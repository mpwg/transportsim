use crate::LineId;
use crate::random::RandomState;
use crate::world::World;
use crate::{
    ClockObservation, Observation, Query, QueryError, RuleSet, Scenario, Tick, ValidationError,
};
use crate::{EditAction, Marker, MarkerAnchor, MarkerId, MarkerObservation, UndoObservation};
use crate::{EditCommand, EditError, EditPreview, edit};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

/// A mutation applied at a completed simulation boundary.
#[non_exhaustive]
pub enum Command {
    SetPaused(bool),
    SetGrowthPaused(bool),
    CommitEdit(EditCommand),
    UndoEdit,
    SetMarker(Marker),
    RemoveMarker(MarkerId),
    SetLineHeadway { line: LineId, ticks: NonZeroU32 },
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CommandReceipt {
    /// Sequence at the returned boundary. An undo rewinds the sequence as well.
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
    NoUndo,
    UnknownMarker(MarkerId),
}

impl fmt::Display for CommandError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoUndo => out.write_str("no build is available to undo"),
            Self::UnknownMarker(id) => write!(out, "unknown marker ID {}", id.get()),
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
    pub(crate) random: Option<RandomState>,
    pub(crate) undo: Option<Box<Simulation>>,
    pub(crate) markers: BTreeMap<MarkerId, Marker>,
}

impl Simulation {
    /// Validates and starts a scenario with deterministic initial conditions.
    pub fn new_game(
        scenario: Scenario,
        rules: RuleSet,
        seed: u64,
    ) -> Result<Self, ValidationError> {
        Self::new_with_disconnections(scenario, rules, seed, &BTreeSet::new())
    }

    pub(crate) fn new_with_disconnections(
        mut scenario: Scenario,
        rules: RuleSet,
        seed: u64,
        allowed_disconnected: &BTreeSet<LineId>,
    ) -> Result<Self, ValidationError> {
        scenario.layout.sort_tables();
        let traffic = scenario.start;
        let world = World::from_scenario(scenario.clone(), allowed_disconnected)?;
        Ok(Self {
            world,
            scenario,
            seed,
            random: Some(RandomState::seeded(seed)),
            rules,
            traffic,
            development_ticks: 0,
            sequence: 0,
            network_revision: 0,
            paused: false,
            growth_paused: false,
            undo: None,
            markers: BTreeMap::new(),
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
        if matches!(command, Command::UndoEdit) {
            let before = self.undo.take().ok_or(CommandError::NoUndo)?;
            *self = *before;
            return Ok(self.receipt(None));
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or(CommandError::SequenceOverflow)?;
        match command {
            Command::SetPaused(paused) => self.paused = paused,
            Command::SetGrowthPaused(paused) => self.growth_paused = paused,
            Command::SetMarker(marker) => {
                self.markers.insert(marker.id, marker);
            }
            Command::RemoveMarker(id) => {
                self.markers
                    .remove(&id)
                    .ok_or(CommandError::UnknownMarker(id))?;
            }
            Command::CommitEdit(command) => {
                let prepared = edit::prepare(self, &command).map_err(CommandError::Edit)?;
                return Ok(self.commit(prepared, sequence));
            }
            Command::SetLineHeadway { line, ticks } => {
                let command = EditCommand {
                    expected_revision: self.network_revision,
                    actions: vec![EditAction::SetHeadway { line, ticks }],
                };
                let prepared = edit::prepare(self, &command).map_err(|error| match error {
                    EditError::MissingLine(id) => CommandError::UnknownLine(id),
                    error => CommandError::Edit(error),
                })?;
                return Ok(self.commit(prepared, sequence));
            }
            Command::UndoEdit => unreachable!("undo restores the command sequence itself"),
        }
        self.sequence = sequence;
        Ok(self.receipt(None))
    }

    fn receipt(&self, edit: Option<EditPreview>) -> CommandReceipt {
        CommandReceipt {
            sequence: self.sequence,
            tick: self.traffic,
            edit,
        }
    }

    fn commit(&mut self, prepared: edit::PreparedEdit, sequence: u64) -> CommandReceipt {
        let candidate = Self {
            world: prepared.world,
            scenario: prepared.scenario,
            network_revision: prepared.preview.revision,
            sequence,
            undo: None,
            markers: self.markers.clone(),
            seed: self.seed,
            random: self.random,
            rules: self.rules,
            traffic: self.traffic,
            development_ticks: self.development_ticks,
            paused: self.paused,
            growth_paused: self.growth_paused,
        };
        let mut before = std::mem::replace(self, candidate);
        before.undo = None;
        self.undo = Some(Box::new(before));
        self.receipt(Some(prepared.preview))
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
        let growing = !self.growth_paused && !self.scenario.growth.sites.is_empty();
        let work_phase = self.random.map_or(0, RandomState::demand_phase);
        self.world.demand.sample_growth(growing);
        for offset in 0..ticks {
            let tick = Tick::new(start.seconds() + offset + 1);
            self.world.demand.tick_before_road(
                tick,
                &mut self.world.road,
                &mut self.world.service,
                &mut self.world.taxi,
                work_phase,
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
            self.traffic = tick;
            if !self.growth_paused {
                self.development_ticks += 1;
                if growing
                    && self
                        .development_ticks
                        .is_multiple_of(self.rules.development_period_ticks())
                {
                    self.grow_city();
                    self.world.demand.sample_growth(growing);
                }
            }
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

    /// Returns whether any signal-controlled movement from this lane is green now.
    /// Returns `None` for unknown lanes and lanes without signal-controlled movements.
    pub fn lane_signal(&self, lane: crate::LaneId) -> Option<bool> {
        self.world.road.lane_signal(lane, self.traffic)
    }

    /// Number of active origin-destination journey groups.
    pub fn active_journey_count(&self) -> usize {
        self.world.demand.active_journey_count()
    }

    pub fn inspect(&self, query: Query) -> Result<Observation, QueryError> {
        match query {
            Query::Undo => Ok(Observation::Undo(self.undo.as_ref().map(|before| {
                UndoObservation {
                    target_tick: before.traffic,
                }
            }))),
            Query::Markers => Ok(Observation::Markers(
                self.markers
                    .values()
                    .map(|marker| {
                        let anchor_present = marker.anchor.is_some_and(|anchor| match anchor {
                            MarkerAnchor::Building(id) => self.world.buildings.contains(&id),
                            MarkerAnchor::Lane(id) => self.world.road.contains_lane(id),
                            MarkerAnchor::Stop(id) => self.world.service.stop_zone(id).is_some(),
                        });
                        MarkerObservation {
                            marker: marker.clone(),
                            anchor_present,
                        }
                    })
                    .collect(),
            )),
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
            Query::Service => Ok(Observation::Service(
                self.world.service.observe(&self.world.road),
            )),
            Query::Taxi => Ok(Observation::Taxi(
                self.world.taxi.observe(self.traffic, &self.world.road),
            )),
            Query::Growth => Ok(Observation::Growth(
                self.world.growth.observe(self.world.demand.totals()),
            )),
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
            Query::Layout => Ok(Observation::Layout(self.scenario.layout.clone())),
            Query::FootEdge(id) => self
                .scenario
                .demand
                .foot_edges
                .iter()
                .find(|edge| edge.id == id)
                .cloned()
                .map(Observation::FootEdge)
                .ok_or(QueryError::UnknownFootEdge(id)),
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
