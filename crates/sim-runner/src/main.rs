mod file_store;
mod replay;

use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use sim_core::Simulation;

use file_store::FileStore;
use replay::{Replay, Snapshot};

#[derive(Parser)]
#[command(version, about = "Run a headless traffic simulation")]
struct Cli {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Run a scenario or continue from a saved checkpoint.
    Run(RunArgs),
}

#[derive(Args)]
#[command(group(clap::ArgGroup::new("input").required(true).args(["scenario", "load"])))]
struct RunArgs {
    /// Start from a scenario JSON file.
    #[arg(long, value_name = "PATH")]
    scenario: Option<PathBuf>,
    /// Continue from a checkpoint JSON file.
    #[arg(long, value_name = "PATH")]
    load: Option<PathBuf>,
    /// Apply a JSON list of commands, e.g. [{"tick":5,"command":{"set_paused":true}}].
    #[arg(long, value_name = "PATH")]
    commands: Option<PathBuf>,
    /// Stop at this absolute traffic tick. Without it, an undo can leave an earlier boundary.
    #[arg(long, value_name = "N")]
    until_tick: Option<u64>,
    /// Write the final checkpoint to this file.
    #[arg(long, value_name = "PATH")]
    save: Option<PathBuf>,
    /// Write a JSON line per completed tick with traffic, demand, service, taxi and growth
    /// measurements, before commands at that boundary.
    #[arg(long, value_name = "PATH")]
    trace: Option<PathBuf>,
}

#[derive(Serialize)]
struct Summary {
    #[serde(flatten)]
    state: Snapshot,
    fingerprint: String,
}

fn main() -> ExitCode {
    let result = match Cli::parse().action {
        Action::Run(args) => run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sim-runner: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: RunArgs) -> Result<()> {
    let trace = args.trace.as_deref().map(output_entry).transpose()?;
    if let Some(save) = &args.save {
        let (_, save_entry) = output_entry(save)?;
        for input in [&args.scenario, &args.commands].into_iter().flatten() {
            let (_, input_entry) = output_entry(input)?;
            anyhow::ensure!(
                save_entry != input_entry && !same_file(&save_entry, &input_entry)?,
                "--save must use a separate file from {}",
                input.display()
            );
        }
    }
    if let Some((_, trace)) = &trace {
        for other in [&args.scenario, &args.load, &args.commands, &args.save]
            .into_iter()
            .flatten()
        {
            let (_, other_entry) = output_entry(other)?;
            anyhow::ensure!(
                trace != &other_entry && !same_file(trace, &other_entry)?,
                "--trace must use a separate file from {}",
                other.display()
            );
        }
    }

    let mut sim = if let Some(path) = args.scenario {
        Simulation::from_scenario_json(&read(&path)?)
            .with_context(|| format!("cannot start scenario {}", path.display()))?
    } else {
        let path = args.load.context("expected --scenario or --load")?;
        Simulation::load(&read(&path)?)
            .with_context(|| format!("cannot load checkpoint {}", path.display()))?
    };
    let commands = args.commands.as_deref().map(read).transpose()?;
    let replay = Replay::new(&sim, args.until_tick, commands.as_deref())?;

    if let Some((parent, path)) = trace {
        let run_with_trace = || -> Result<()> {
            let mut trace = tempfile::NamedTempFile::new_in(parent)?;
            {
                let mut writer = BufWriter::new(trace.as_file_mut());
                replay.execute(&mut sim, Some(&mut writer))?;
                writer.flush()?;
            }
            trace.persist(&path)?;
            Ok(())
        };
        run_with_trace().with_context(|| format!("cannot complete trace {}", path.display()))?;
    } else {
        replay.execute(&mut sim, None)?;
    }

    let summary = Summary {
        state: Snapshot::inspect(&sim)?,
        fingerprint: sim
            .fingerprint()?
            .into_iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    };
    if let Some(path) = args.save {
        FileStore
            .write(&path, &sim.save()?)
            .with_context(|| format!("cannot write checkpoint {}", path.display()))?;
    }

    let mut writer = io::stdout().lock();
    serde_json::to_writer(&mut writer, &summary)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn read(path: &Path) -> Result<Vec<u8>> {
    FileStore
        .read(path)
        .with_context(|| format!("cannot read {}", path.display()))
}

fn output_entry(path: &Path) -> Result<(PathBuf, PathBuf)> {
    let name = path
        .file_name()
        .with_context(|| format!("expected a file path, got {}", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent
        .canonicalize()
        .with_context(|| format!("cannot resolve directory {}", parent.display()))?;
    let entry = parent.join(name);
    Ok((parent, entry))
}

fn same_file(first: &Path, second: &Path) -> Result<bool> {
    match same_file::is_same_file(first, second) {
        Ok(same) => Ok(same),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| {
            format!(
                "cannot compare {} and {}",
                first.display(),
                second.display()
            )
        }),
    }
}
