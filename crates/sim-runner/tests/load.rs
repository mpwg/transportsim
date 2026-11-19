#[path = "../src/file_store.rs"]
mod file_store;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sim_core::{DemandObservation, Observation, Query, Simulation};

use file_store::FileStore;

const TICKS_PER_DAY: u64 = 86_400;
const MONTHS_PER_YEAR: u64 = 12;
const MONTHS_IN_FIVE_YEARS: u64 = 5 * MONTHS_PER_YEAR;

#[test]
#[ignore = "eintägige Lastabnahme im Release-Build"]
fn one_day() -> Result<()> {
    let (mut sim, _) = start_scenario("load-10000.json")?;
    assert_eq!(residents(&sim)?, 10_000);

    let mut run = RunMeasurement::default();
    run.advance(&mut sim, TICKS_PER_DAY)?;
    let demand = check_boundary(&sim, 1)?;
    assert!(demand.generated > 0 && demand.arrived > 0);
    assert!(run.peak_active > 0);
    assert_eq!(residents(&sim)?, 10_000);
    println!(
        "Einwohner 10000 | Wege erzeugt {} | wartend {} | unterwegs {} | angekommen {}",
        demand.generated, demand.waiting, demand.in_transit, demand.arrived,
    );

    let dir = tempfile::tempdir()?;
    let checkpoint = dir.path().join("lasttag.json");
    write_checkpoint(&sim, &checkpoint)?;
    let restored = read_checkpoint(&checkpoint)?;
    assert_same_observations(&sim, &restored)?;
    assert_eq!(restored.fingerprint()?, sim.fingerprint()?);

    run.report(TICKS_PER_DAY);
    assert!(
        run.ticks_per_second(TICKS_PER_DAY) >= 60.0,
        "Der Lasttag unterschreitet das Ziel von 60 Ticks/s"
    );
    Ok(())
}

#[test]
#[ignore = "Fünfjahresabnahme mit monatlichen Prozessneustarts im Release-Build"]
fn five_years() -> Result<()> {
    // The bridge city can grow from its initial population over the five years.
    let (mut continuous, scenario) = start_scenario("bridge-city.json")?;
    let initial_residents = residents(&continuous)?;
    assert!((1..10_000).contains(&initial_residents));

    let dir = tempfile::tempdir()?;
    let checkpoint = dir.path().join("brueckenstadt.json");
    let mut run = RunMeasurement::default();
    let mut runner_elapsed = Duration::ZERO;
    let mut load_elapsed = Duration::ZERO;
    let mut largest_checkpoint = 0;

    for month in 1..=MONTHS_IN_FIVE_YEARS {
        let tick = month * TICKS_PER_DAY;
        run.advance(&mut continuous, TICKS_PER_DAY)?;
        let demand = check_boundary(&continuous, month)?;
        let inhabitants = residents(&continuous)?;
        assert!(inhabitants <= 10_000, "Bevölkerungsgrenze in Monat {month}");

        let start = Instant::now();
        let summary = if month == 1 {
            run_segment("--scenario", &scenario, tick, &checkpoint)?
        } else {
            run_segment("--load", &checkpoint, tick, &checkpoint)?
        };
        runner_elapsed += start.elapsed();
        assert_eq!(summary.tick, tick);

        let start = Instant::now();
        let bytes = FileStore.read(&checkpoint)?;
        let resumed = Simulation::load(&bytes)?;
        load_elapsed += start.elapsed();
        largest_checkpoint = largest_checkpoint.max(bytes.len());
        assert_same_observations(&continuous, &resumed)
            .with_context(|| format!("Abweichung nach Neustart in Monat {month}"))?;
        let fingerprint = blake3::Hash::from_bytes(continuous.fingerprint()?);
        assert_eq!(
            summary.fingerprint,
            fingerprint.to_hex().as_str(),
            "Monat {month}"
        );
        assert_eq!(
            resumed.fingerprint()?,
            *fingerprint.as_bytes(),
            "Monat {month}"
        );

        println!(
            "Monat {month} | Tick {tick} | Einwohner {inhabitants} | Wege erzeugt {} | wartend {} | unterwegs {} | angekommen {} | aktive Mengen {}",
            demand.generated,
            demand.waiting,
            demand.in_transit,
            demand.arrived,
            demand.active.len(),
        );
        if month % MONTHS_PER_YEAR == 0 {
            println!(
                "Jahr {} | Fingerabdruck {fingerprint}",
                month / MONTHS_PER_YEAR
            );
        }
    }

    let final_residents = residents(&continuous)?;
    assert!(
        final_residents > initial_residents,
        "Die Brückenstadt ist nicht gewachsen"
    );
    write_checkpoint(&continuous, &dir.path().join("durchgehend.json"))?;
    println!(
        "{MONTHS_IN_FIVE_YEARS} Runner-Prozesse in {:.3} s | {:.2} Ticks/s inklusive Neustarts und E/A | größte Runner-Datei {largest_checkpoint} Bytes | kumulierte Ladezeit der Kontrollstände {:.3} s",
        runner_elapsed.as_secs_f64(),
        (MONTHS_IN_FIVE_YEARS * TICKS_PER_DAY) as f64 / runner_elapsed.as_secs_f64(),
        load_elapsed.as_secs_f64(),
    );
    run.report(MONTHS_IN_FIVE_YEARS * TICKS_PER_DAY);
    Ok(())
}

fn start_scenario(name: &str) -> Result<(Simulation, PathBuf)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios")
        .join(name);
    let bytes = fs::read(&path).with_context(|| format!("Szenario {}", path.display()))?;
    #[derive(Deserialize)]
    struct ScenarioHeader {
        seed: u64,
    }
    let header: ScenarioHeader = serde_json::from_slice(&bytes)?;
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "Szenario {name} | Seed {} | Build {profile} | {} | {}",
        header.seed,
        platform(),
        cpu_model(),
    );
    let sim = Simulation::from_scenario_json(&bytes)?;
    let Observation::Clock(clock) = sim.inspect(Query::Clock)? else {
        anyhow::bail!("Uhrenstand fehlt");
    };
    assert_eq!(clock.traffic.seconds(), 0);
    assert_eq!(clock.development_ticks, 0);
    assert!(!clock.paused && !clock.growth_paused);
    Ok((sim, path))
}

fn residents(sim: &Simulation) -> Result<u32> {
    let Observation::Growth(growth) = sim.inspect(Query::Growth)? else {
        anyhow::bail!("Wachstumsbeobachtung fehlt");
    };
    Ok(growth.totals.residents)
}

fn check_boundary(sim: &Simulation, month: u64) -> Result<DemandObservation> {
    let Observation::Clock(clock) = sim.inspect(Query::Clock)? else {
        anyhow::bail!("Uhrenstand fehlt");
    };
    assert_eq!(clock.traffic.seconds(), month * TICKS_PER_DAY);
    assert_eq!(clock.development_ticks, month * TICKS_PER_DAY);
    assert_eq!(clock.development_period, month);

    let Observation::Demand(demand) = sim.inspect(Query::Demand)? else {
        anyhow::bail!("Nachfragebeobachtung fehlt");
    };
    assert_eq!(
        demand.generated,
        demand.waiting + demand.in_transit + demand.arrived,
        "Wegebilanz in Monat {month}"
    );
    assert_eq!(sim.active_journey_count(), demand.active.len());
    Ok(demand)
}

fn assert_same_observations(continuous: &Simulation, resumed: &Simulation) -> Result<()> {
    let queries = [
        ("Uhr", (|| Query::Clock) as fn() -> Query),
        ("Wachstum", || Query::Growth),
        ("Nachfrage", || Query::Demand),
        ("Linienbetrieb", || Query::Service),
        ("Taxi", || Query::Taxi),
    ];
    for (name, query) in queries {
        ensure!(
            continuous.inspect(query())? == resumed.inspect(query())?,
            "{name} weicht nach dem Laden ab"
        );
    }
    for lane in continuous.lane_ids() {
        ensure!(
            continuous.inspect(Query::Lane(lane))? == resumed.inspect(Query::Lane(lane))?,
            "Spur {} weicht nach dem Laden ab",
            lane.get()
        );
    }
    Ok(())
}

#[derive(Default)]
struct RunMeasurement {
    elapsed: Duration,
    peak_active: usize,
}

impl RunMeasurement {
    fn advance(&mut self, sim: &mut Simulation, ticks: u64) -> Result<()> {
        let start = Instant::now();
        self.peak_active = self.peak_active.max(sim.active_journey_count());
        for _ in 0..ticks {
            assert_eq!(sim.advance(1)?.executed_ticks, 1);
            self.peak_active = self.peak_active.max(sim.active_journey_count());
        }
        self.elapsed += start.elapsed();
        Ok(())
    }

    fn ticks_per_second(&self, ticks: u64) -> f64 {
        ticks as f64 / self.elapsed.as_secs_f64()
    }

    fn report(&self, ticks: u64) {
        println!(
            "{ticks} Ticks in {:.3} s | {:.2} Ticks/s | maximal {} aktive Mengen | Spitzen-RSS des Testprozesses {}",
            self.elapsed.as_secs_f64(),
            self.ticks_per_second(ticks),
            self.peak_active,
            peak_rss().as_deref().unwrap_or("unbekannt"),
        );
    }
}

#[derive(Deserialize)]
struct RunnerSummary {
    tick: u64,
    fingerprint: String,
}

fn run_segment(input_flag: &str, input: &Path, tick: u64, save: &Path) -> Result<RunnerSummary> {
    let output = Command::new(env!("CARGO_BIN_EXE_sim-runner"))
        .arg("run")
        .arg(input_flag)
        .arg(input)
        .arg("--until-tick")
        .arg(tick.to_string())
        .arg("--save")
        .arg(save)
        .output()
        .with_context(|| format!("Runner bis Tick {tick} starten"))?;
    ensure!(
        output.status.success(),
        "Runner bis Tick {tick} mit {} beendet\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).context("Runner-Zusammenfassung lesen")
}

fn write_checkpoint(sim: &Simulation, path: &Path) -> Result<()> {
    let start = Instant::now();
    let bytes = sim.save()?;
    FileStore.write(path, &bytes)?;
    println!(
        "Spielstand {} Bytes | Speicherzeit mit Datei- und Verzeichnissynchronisierung {:.3} ms",
        bytes.len(),
        start.elapsed().as_secs_f64() * 1000.0,
    );
    Ok(())
}

fn read_checkpoint(path: &Path) -> Result<Simulation> {
    let start = Instant::now();
    let sim = Simulation::load(&FileStore.read(path)?)?;
    println!(
        "Ladezeit inklusive Datei und Validierung {:.3} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(sim)
}

fn platform() -> String {
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    format!(
        "{} {} {}",
        std::env::consts::OS,
        kernel.trim(),
        std::env::consts::ARCH
    )
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|cpuinfo| {
            cpuinfo.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                (key.trim() == "model name").then(|| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "CPU-Modell unbekannt".to_owned())
}

fn peak_rss() -> Option<String> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .map(|value| value.trim().to_owned())
}
