use crate::LineId;
use crate::world::World;
use crate::{
    ClockObservation, Observation, Query, QueryError, RuleSet, Scenario, Tick, ValidationError,
};
use crate::{EditCommand, EditError, EditPreview, edit};
use std::{fmt, num::NonZeroU32};

/// A mutation applied at a completed simulation boundary.
#[non_exhaustive]
pub enum Command {
    SetPaused(bool),
    SetGrowthPaused(bool),
    CommitEdit(EditCommand),
    SetLineHeadway { line: LineId, ticks: NonZeroU32 },
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CommandReceipt {
    pub sequence: u64,
    pub tick: Tick,
    pub edit: Option<EditPreview>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CommandError {
    SequenceOverflow,
    UnknownLine(LineId),
    Edit(EditError),
}

impl fmt::Display for CommandError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Edit(error) => error.fmt(out),
            Self::UnknownLine(line) => write!(out, "unknown line ID {}", line.get()),
            Self::SequenceOverflow => out.write_str("command sequence overflow"),
        }
    }
}

impl std::error::Error for CommandError {}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AdvanceError {
    TrafficClockOverflow,
    DevelopmentClockOverflow,
    NotPaused,
}

impl fmt::Display for AdvanceError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrafficClockOverflow => out.write_str("traffic clock overflow"),
            Self::DevelopmentClockOverflow => out.write_str("development clock overflow"),
            Self::NotPaused => out.write_str("single step requires a paused simulation"),
        }
    }
}

impl std::error::Error for AdvanceError {}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AdvanceReport {
    pub start: Tick,
    pub end: Tick,
    pub executed_ticks: u64,
}

/// Exclusive owner of a running simulation.
#[derive(Debug)]
pub struct Simulation {
    pub(crate) world: World,
    pub(crate) rules: RuleSet,
    pub(crate) traffic: Tick,
    pub(crate) development_ticks: u64,
    pub(crate) sequence: u64,
    pub(crate) network_revision: u64,
    pub(crate) paused: bool,
    pub(crate) growth_paused: bool,
    pub(crate) scenario: Scenario,
    pub(crate) seed: u64,
}

impl Simulation {
    /// Validates and starts a scenario with deterministic initial conditions.
    pub fn new_game(
        scenario: Scenario,
        rules: RuleSet,
        seed: u64,
    ) -> Result<Self, ValidationError> {
        let traffic = scenario.start;
        let world = World::from_scenario(scenario.clone())?;
        Ok(Self {
            world,
            scenario,
            seed,
            rules,
            traffic,
            development_ticks: 0,
            sequence: 0,
            network_revision: 0,
            paused: false,
            growth_paused: false,
        })
    }

    /// Current revision of the editable network.
    pub fn network_revision(&self) -> u64 {
        self.network_revision
    }

    /// Tests an edit against a copy of the current state.
    pub fn preview(&self, edit: &EditCommand) -> Result<EditPreview, EditError> {
        Ok(crate::edit::prepare(self, edit)?.preview)
    }

    pub fn apply(&mut self, command: Command) -> Result<CommandReceipt, CommandError> {
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(CommandError::SequenceOverflow)?;
        let mut receipt_edit = None;
        match command {
            Command::SetPaused(paused) => self.paused = paused,
            Command::SetGrowthPaused(paused) => self.growth_paused = paused,
            Command::CommitEdit(edit) => {
                let prepared = edit::prepare(self, &edit).map_err(CommandError::Edit)?;
                self.scenario = prepared.scenario;
                self.world = prepared.world;
                self.network_revision = prepared.preview.revision;
                receipt_edit = Some(prepared.preview);
            }
            Command::SetLineHeadway { line, ticks } => {
                if !self.world.service.set_headway(line, self.traffic, ticks) {
                    return Err(CommandError::UnknownLine(line));
                }
            }
        }
        self.sequence = sequence;
        Ok(CommandReceipt {
            sequence,
            tick: self.traffic,
            edit: receipt_edit,
        })
    }

    /// Advances whole traffic ticks. While paused, the number executed is zero.
    pub fn advance(&mut self, ticks: u64) -> Result<AdvanceReport, AdvanceError> {
        self.advance_clocks(if self.paused { 0 } else { ticks })
    }

    /// Runs one tick during a total pause and leaves the simulation paused.
    pub fn step(&mut self) -> Result<AdvanceReport, AdvanceError> {
        if !self.paused {
            return Err(AdvanceError::NotPaused);
        }
        self.advance_clocks(1)
    }

    fn advance_clocks(&mut self, ticks: u64) -> Result<AdvanceReport, AdvanceError> {
        let start = self.traffic;
        let end = start
            .seconds()
            .checked_add(ticks)
            .map(Tick::new)
            .ok_or(AdvanceError::TrafficClockOverflow)?;
        let growth_ticks = if self.growth_paused { 0 } else { ticks };
        let development = self
            .development_ticks
            .checked_add(growth_ticks)
            .ok_or(AdvanceError::DevelopmentClockOverflow)?;
        for offset in 0..ticks {
            let tick = Tick::new(start.seconds() + offset + 1);
            self.world.demand.tick_before_road(
                tick,
                &mut self.world.road,
                &mut self.world.service,
                &mut self.world.taxi,
            );
            let taxi_arrivals = self.world.taxi.tick(tick, &mut self.world.road);
            self.world.demand.taxi_arrivals(taxi_arrivals, tick);
            let alighting = self.world.service.arrive_at_stops(tick, &self.world.road);
            if !alighting.is_empty() {
                self.world.demand.alighting(alighting, tick);
                self.world
                    .demand
                    .move_transit(tick, &mut self.world.service);
            }
            self.world.service.tick(tick, &mut self.world.road);
            self.world.road.tick(tick);
            self.world.service.after_road(tick, &self.world.road);
            self.world.demand.after_road(tick, &self.world.road);
        }
        self.traffic = end;
        self.development_ticks = development;
        Ok(AdvanceReport {
            start,
            end,
            executed_ticks: ticks,
        })
    }

    /// Iterates through existing lane IDs in stable order.
    pub fn lane_ids(&self) -> impl Iterator<Item = crate::LaneId> + '_ {
        self.world.road.lane_ids()
    }

    pub fn inspect(&self, query: Query) -> Result<Observation, QueryError> {
        match query {
            Query::Clock => Ok(Observation::Clock(ClockObservation {
                traffic: self.traffic,
                development_ticks: self.development_ticks,
                development_period: self.development_ticks / self.rules.development_period_ticks(),
                paused: self.paused,
                growth_paused: self.growth_paused,
            })),
            Query::TimetableJourney { from, to } => {
                for zone in [from, to] {
                    if !self.world.demand.contains_zone(zone) {
                        return Err(QueryError::UnknownZone(zone));
                    }
                }
                Ok(Observation::TimetableJourney(self.world.service.timetable(
                    from,
                    to,
                    self.traffic,
                    &self.world.demand,
                )))
            }
            Query::Service => Ok(Observation::Service(self.world.service.observe())),
            Query::Taxi => Ok(Observation::Taxi(self.world.taxi.observe())),
            Query::Demand => Ok(Observation::Demand(
                self.world.demand.observe(&self.world.taxi),
            )),
            Query::Journeys { from, to } => {
                for zone in [from, to] {
                    if !self.world.demand.contains_zone(zone) {
                        return Err(QueryError::UnknownZone(zone));
                    }
                }
                Ok(Observation::Journeys(self.world.demand.alternatives(
                    from,
                    to,
                    self.traffic,
                    &self.world.road,
                    &self.world.service,
                    &self.world.taxi,
                )))
            }
            Query::Lane(id) => self
                .world
                .road
                .observe(id, self.traffic)
                .map(Observation::Lane)
                .ok_or(QueryError::UnknownLane(id)),
            Query::Building(id) => {
                if self.world.buildings.contains(&id) {
                    Ok(Observation::Building(id))
                } else {
                    Err(QueryError::UnknownBuilding(id))
                }
            }
        }
    }
}
