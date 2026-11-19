use std::io::Write;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sim_core::{
    AdvanceReport, ClockObservation, Command, Observation, Query, QueueEntry, Simulation,
    VehicleKind, WaitReason,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimedCommand {
    tick: u64,
    command: CommandInput,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CommandInput {
    SetPaused(bool),
    SetGrowthPaused(bool),
    Step,
}

impl CommandInput {
    fn apply(self, simulation: &mut Simulation) -> Result<Option<AdvanceReport>> {
        let command = match self {
            Self::SetPaused(paused) => Command::SetPaused(paused),
            Self::SetGrowthPaused(paused) => Command::SetGrowthPaused(paused),
            Self::Step => return Ok(Some(simulation.step()?)),
        };
        simulation.apply(command)?;
        Ok(None)
    }
}

pub(crate) struct Replay {
    start: u64,
    target: u64,
    commands: Vec<TimedCommand>,
}

impl Replay {
    pub(crate) fn new(
        sim: &Simulation,
        target: Option<u64>,
        commands_json: Option<&[u8]>,
    ) -> Result<Self> {
        let start = clock(sim)?.traffic.seconds();
        let target = target.unwrap_or(start);
        ensure!(
            target >= start,
            "target tick {target} precedes current tick {start}"
        );
        let commands: Vec<TimedCommand> = if let Some(bytes) = commands_json {
            serde_json::from_slice(bytes).context("invalid command list JSON")?
        } else {
            Vec::new()
        };
        let mut previous_tick = start;
        for command in &commands {
            ensure!(
                command.tick >= start,
                "command at tick {} precedes current tick {start}",
                command.tick
            );
            ensure!(
                command.tick >= previous_tick,
                "command at tick {} is out of order after tick {previous_tick}",
                command.tick
            );
            previous_tick = command.tick;
        }
        Ok(Self {
            start,
            target,
            commands,
        })
    }

    pub(crate) fn execute(
        self,
        sim: &mut Simulation,
        mut trace: Option<&mut dyn Write>,
    ) -> Result<()> {
        let mut tick = self.start;
        for entry in self
            .commands
            .into_iter()
            .take_while(|entry| entry.tick <= self.target)
        {
            ensure!(
                entry.tick >= tick,
                "command at tick {} precedes current tick {tick}",
                entry.tick
            );
            advance_to(sim, &mut tick, entry.tick, &mut trace)?;
            ensure!(
                !matches!(entry.command, CommandInput::Step) || tick < self.target,
                "single step at tick {tick} would exceed requested target {}",
                self.target,
            );
            if let Some(report) = entry
                .command
                .apply(sim)
                .with_context(|| format!("cannot apply command at tick {tick}"))?
            {
                tick = report.end.seconds();
                emit_trace(sim, &mut trace)?;
            }
        }
        advance_to(sim, &mut tick, self.target, &mut trace)?;
        Ok(())
    }
}

fn advance_to(
    sim: &mut Simulation,
    tick: &mut u64,
    target: u64,
    trace: &mut Option<&mut dyn Write>,
) -> Result<()> {
    while *tick < target {
        let requested = if trace.is_some() { 1 } else { target - *tick };
        let report = sim.advance(requested)?;
        ensure!(
            report.executed_ticks != 0,
            "cannot reach tick {target}: simulation is paused at tick {tick}"
        );
        *tick = report.end.seconds();
        emit_trace(sim, trace)?;
    }
    Ok(())
}

fn emit_trace(sim: &Simulation, trace: &mut Option<&mut dyn Write>) -> Result<()> {
    if let Some(writer) = trace.as_deref_mut() {
        serde_json::to_writer(&mut *writer, &Snapshot::inspect(sim)?)?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn clock(sim: &Simulation) -> Result<ClockObservation> {
    let Observation::Clock(observation) = sim.inspect(Query::Clock)? else {
        anyhow::bail!("expected a clock observation");
    };
    Ok(observation)
}

#[derive(Serialize)]
pub(crate) struct Snapshot {
    tick: u64,
    clock: Clock,
    lanes: Vec<Lane>,
}

impl Snapshot {
    pub(crate) fn inspect(sim: &Simulation) -> Result<Self> {
        let clock = clock(sim)?;
        let mut lanes = Vec::new();
        for id in sim.lane_ids() {
            let Observation::Lane(lane) = sim.inspect(Query::Lane(id))? else {
                anyhow::bail!("expected an observation for lane {}", id.get());
            };
            lanes.push(Lane {
                id: id.get(),
                capacity_mm: lane.capacity_mm,
                occupied_mm: lane.occupied_mm,
                queue: lane.queue.into_iter().map(Entry::from).collect(),
                wait_reason: lane.wait_reason.map(wait_reason),
                departure: lane.departure.map(|departure| Departure {
                    entry: departure.entry.into(),
                    to: departure.to.map(sim_core::LaneId::get),
                }),
            });
        }
        Ok(Self {
            tick: clock.traffic.seconds(),
            clock: Clock {
                development_ticks: clock.development_ticks,
                development_period: clock.development_period,
                paused: clock.paused,
                growth_paused: clock.growth_paused,
            },
            lanes,
        })
    }
}

#[derive(Serialize)]
struct Clock {
    development_ticks: u64,
    development_period: u64,
    paused: bool,
    growth_paused: bool,
}

#[derive(Serialize)]
struct Lane {
    id: u64,
    capacity_mm: u32,
    occupied_mm: u32,
    queue: Vec<Entry>,
    wait_reason: Option<&'static str>,
    departure: Option<Departure>,
}

#[derive(Serialize)]
struct Departure {
    entry: Entry,
    to: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Entry {
    PrivateCars(u32),
    Vehicle {
        id: u64,
        kind: &'static str,
        length_mm: u32,
    },
}

impl From<QueueEntry> for Entry {
    fn from(entry: QueueEntry) -> Self {
        match entry {
            QueueEntry::PrivateCars(count) => Self::PrivateCars(count.get()),
            QueueEntry::Vehicle {
                id,
                kind,
                length_mm,
            } => Self::Vehicle {
                id: id.get(),
                kind: match kind {
                    VehicleKind::Car => "car",
                    VehicleKind::Bus => "bus",
                    VehicleKind::Taxi => "taxi",
                    VehicleKind::Tram => "tram",
                },
                length_mm: length_mm.get(),
            },
        }
    }
}

fn wait_reason(reason: WaitReason) -> &'static str {
    match reason {
        WaitReason::TravelTime => "travel_time",
        WaitReason::RedSignal => "red_signal",
        WaitReason::DischargeLimit => "discharge_limit",
        WaitReason::TargetFull => "target_full",
        WaitReason::Conflict => "conflict",
        WaitReason::VehicleStop => "vehicle_stop",
        _ => "unknown",
    }
}
