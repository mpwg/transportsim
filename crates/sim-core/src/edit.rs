use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

use serde::{Deserialize, Serialize};

use crate::{
    FootEdgeId, FootEdgeSpec, JunctionSpec, LaneId, LaneSpec, LineId, LoadError, MovementId,
    MovementSpec, Scenario, Simulation, ValidationError,
    demand::DemandSnapshot,
    save::v1::{CarRecoveryV1, JunctionV1, LaneV1},
    world::World,
};

mod pocket;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EditCommand {
    pub expected_revision: u64,
    pub actions: Vec<EditAction>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum EditAction {
    SetCapacity {
        lane: LaneId,
        capacity_mm: NonZeroU32,
    },
    UpsertLane(LaneSpec),
    RemoveLane(LaneId),
    UpsertMovement(MovementSpec),
    RemoveMovement(MovementId),
    SetSignal(JunctionSpec),
    UpsertFootEdge(FootEdgeSpec),
    RemoveFootEdge(FootEdgeId),
    AddTurnPocket(TurnPocket),
    SetHeadway {
        line: LineId,
        ticks: NonZeroU32,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TurnPocket {
    pub source: LaneId,
    pub lane: LaneSpec,
    pub feeder: MovementSpec,
    pub turns: Vec<MovementId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EditPreview {
    pub revision: u64,
    pub affected_lanes: Vec<LaneId>,
    pub affected_foot_edges: Vec<FootEdgeId>,
    pub relocated_vehicles: u64,
    pub rerouted_vehicles: u64,
    pub blocked_vehicles: u64,
    pub vehicles_en_route_to_exit: u64,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EditError {
    Outdated { expected: u64, current: u64 },
    RevisionOverflow,
    Empty,
    MissingLane(LaneId),
    MissingMovement(MovementId),
    MissingFootEdge(FootEdgeId),
    MissingLine(LineId),
    Occupied(LaneId),
    InvalidPocket(LaneId),
    WouldInvalidate(LoadError),
}

impl fmt::Display for EditError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outdated { expected, current } => write!(
                out,
                "outdated network revision {expected}; current revision is {current}"
            ),
            Self::RevisionOverflow => out.write_str("network revision overflow"),
            Self::Empty => out.write_str("edit has no actions"),
            Self::MissingLane(lane) => write!(out, "unknown lane {}", lane.get()),
            Self::MissingFootEdge(id) => write!(out, "unknown foot edge {}", id.get()),
            Self::MissingMovement(id) => write!(out, "unknown movement {}", id.get()),
            Self::MissingLine(id) => write!(out, "unknown line {}", id.get()),
            Self::Occupied(lane) => write!(
                out,
                "cannot fit the current occupancy of lane {}",
                lane.get()
            ),
            Self::InvalidPocket(lane) => write!(out, "invalid turn pocket {}", lane.get()),
            Self::WouldInvalidate(error) => {
                write!(out, "edit would invalidate the current simulation: {error}")
            }
        }
    }
}

impl std::error::Error for EditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WouldInvalidate(error) => Some(error),
            _ => None,
        }
    }
}

pub(crate) struct PreparedEdit {
    pub(crate) scenario: Scenario,
    pub(crate) world: World,
    pub(crate) preview: EditPreview,
}

#[derive(Default)]
struct RouteChanges {
    rerouted: u64,
    blocked: u64,
    en_route_to_exit: u64,
}

struct Draft {
    scenario: Scenario,
    lanes: Vec<LaneV1>,
    junctions: Vec<JunctionV1>,
    demand: Option<DemandSnapshot>,
    affected: BTreeSet<LaneId>,
    affected_foot_edges: BTreeSet<FootEdgeId>,
    relocated_vehicles: u64,
    headways: BTreeMap<LineId, NonZeroU32>,
}

impl Draft {
    fn new(simulation: &Simulation) -> Self {
        let (lanes, junctions) = simulation.world.road.snapshot();
        let mut scenario = simulation.scenario.clone();
        // The live queues supply current occupancy after topology changes.
        scenario.traffic.clear();
        Self {
            scenario,
            lanes,
            junctions,
            demand: simulation.world.demand.snapshot(),
            affected: BTreeSet::new(),
            affected_foot_edges: BTreeSet::new(),
            relocated_vehicles: 0,
            headways: BTreeMap::new(),
        }
    }

    fn action(&mut self, action: &EditAction, tick: crate::Tick) -> Result<(), EditError> {
        match action {
            EditAction::SetCapacity { lane, capacity_mm } => {
                let spec = self
                    .scenario
                    .lanes
                    .iter_mut()
                    .find(|spec| spec.id == *lane)
                    .ok_or(EditError::MissingLane(*lane))?;
                spec.capacity_mm = *capacity_mm;
                self.affected.insert(*lane);
            }
            EditAction::UpsertLane(lane) => {
                if let Some(spec) = self
                    .scenario
                    .lanes
                    .iter_mut()
                    .find(|spec| spec.id == lane.id)
                {
                    *spec = lane.clone();
                } else {
                    self.scenario.lanes.push(lane.clone());
                    self.lanes.push(empty_lane(lane.id));
                }
                self.affected.insert(lane.id);
            }
            EditAction::RemoveLane(id) => {
                let index = self
                    .lanes
                    .iter()
                    .position(|lane| lane.id == *id)
                    .ok_or(EditError::MissingLane(*id))?;
                if !self.lanes[index].queue.is_empty() {
                    return Err(EditError::Occupied(*id));
                }
                self.lanes.remove(index);
                self.scenario.lanes.retain(|lane| lane.id != *id);
                self.affected.insert(*id);
            }
            EditAction::UpsertMovement(movement) => {
                if let Some(previous) = self
                    .scenario
                    .movements
                    .iter_mut()
                    .find(|old| old.id == movement.id)
                {
                    self.affected.extend([previous.from, previous.to]);
                    *previous = movement.clone();
                } else {
                    self.scenario.movements.push(movement.clone());
                }
                self.affected.extend([movement.from, movement.to]);
            }
            EditAction::RemoveMovement(id) => {
                let index = self
                    .scenario
                    .movements
                    .iter()
                    .position(|movement| movement.id == *id)
                    .ok_or(EditError::MissingMovement(*id))?;
                let movement = self.scenario.movements.remove(index);
                self.affected.extend([movement.from, movement.to]);
            }
            EditAction::SetSignal(junction) => {
                if let Some(spec) = self
                    .scenario
                    .junctions
                    .iter_mut()
                    .find(|spec| spec.id == junction.id)
                {
                    *spec = junction.clone();
                } else {
                    self.scenario.junctions.push(junction.clone());
                    self.junctions.push(JunctionV1 {
                        id: junction.id,
                        last_granted: Vec::new(),
                    });
                }
                self.affected.extend(
                    self.scenario
                        .movements
                        .iter()
                        .filter(|m| m.junction == junction.id)
                        .flat_map(|m| [m.from, m.to]),
                );
            }
            EditAction::UpsertFootEdge(edge) => {
                let edges = &mut self.scenario.demand.foot_edges;
                if let Some(previous) = edges.iter_mut().find(|old| old.id == edge.id) {
                    *previous = edge.clone();
                } else {
                    edges.push(edge.clone());
                }
                self.affected_foot_edges.insert(edge.id);
            }
            EditAction::RemoveFootEdge(id) => {
                let edges = &mut self.scenario.demand.foot_edges;
                let index = edges
                    .iter()
                    .position(|edge| edge.id == *id)
                    .ok_or(EditError::MissingFootEdge(*id))?;
                edges.remove(index);
                self.affected_foot_edges.insert(*id);
            }
            EditAction::AddTurnPocket(pocket) => self.pocket(pocket, tick)?,
            EditAction::SetHeadway { line, ticks } => {
                if !self
                    .scenario
                    .service
                    .lines
                    .iter()
                    .any(|spec| spec.id == *line)
                {
                    return Err(EditError::MissingLine(*line));
                }
                self.headways.insert(*line, *ticks);
            }
        }
        Ok(())
    }

    fn pocket(&mut self, pocket: &TurnPocket, tick: crate::Tick) -> Result<(), EditError> {
        let invalid = || EditError::InvalidPocket(pocket.lane.id);
        let turns: BTreeSet<_> = pocket.turns.iter().copied().collect();
        if turns.is_empty()
            || turns.len() != pocket.turns.len()
            || pocket.source == pocket.lane.id
            || (pocket.feeder.from, pocket.feeder.to) != (pocket.source, pocket.lane.id)
            || self
                .scenario
                .lanes
                .iter()
                .any(|lane| lane.id == pocket.lane.id)
            || self
                .scenario
                .movements
                .iter()
                .any(|m| m.id == pocket.feeder.id)
        {
            return Err(invalid());
        }
        let source = self
            .scenario
            .lanes
            .iter()
            .find(|spec| spec.id == pocket.source)
            .ok_or(EditError::MissingLane(pocket.source))?;
        if pocket.lane.capacity_mm > source.capacity_mm {
            return Err(invalid());
        }
        for turn in turns {
            let movement = self
                .scenario
                .movements
                .iter_mut()
                .find(|m| m.id == turn && m.from == pocket.source)
                .ok_or_else(invalid)?;
            self.affected.insert(movement.to);
            movement.from = pocket.lane.id;
        }
        self.scenario.lanes.push(pocket.lane.clone());
        self.scenario.movements.push(pocket.feeder.clone());
        self.relocated_vehicles += pocket::divert(pocket, tick, &mut self.lanes)?;
        if let Some(demand) = &mut self.demand {
            demand.redirect_turns(&pocket.turns, pocket.feeder.id);
        }
        self.affected.extend([pocket.source, pocket.lane.id]);
        Ok(())
    }

    fn normalize(&mut self) -> Result<(), EditError> {
        let specs: BTreeMap<_, _> = self
            .scenario
            .lanes
            .iter()
            .map(|lane| (lane.id, lane))
            .collect();
        let adjacent: BTreeSet<_> = self
            .scenario
            .movements
            .iter()
            .map(|m| (m.from, m.to))
            .collect();
        for lane in &mut self.lanes {
            let spec = specs[&lane.id];
            let occupied: u64 = lane.queue.iter().map(|item| space(&item.entry)).sum();
            if occupied > u64::from(spec.capacity_mm.get())
                || lane
                    .queue
                    .iter()
                    .any(|item| !spec.allowed.contains(&item.entry.kind()))
            {
                return Err(EditError::Occupied(lane.id));
            }
            if self.affected.contains(&lane.id) {
                lane.wait = None;
            }
            let maximum_credit = u64::from(spec.capacity_mm.get())
                * u64::from(spec.discharge.ticks.get())
                + u64::from(spec.discharge.millimetres.get())
                - 1;
            lane.credit = lane.credit.min(maximum_credit);
            if lane.departure.as_ref().is_some_and(|departure| {
                space(&departure.entry) > u64::from(spec.capacity_mm.get())
                    || !spec.allowed.contains(&departure.entry.kind())
                    || departure
                        .to
                        .is_some_and(|to| !adjacent.contains(&(lane.id, to)))
            }) {
                lane.departure = None;
            }
        }
        let cursors: BTreeSet<_> = self
            .scenario
            .movements
            .iter()
            .map(|m| (m.junction, m.priority, m.from))
            .collect();
        for saved in &mut self.junctions {
            saved
                .last_granted
                .retain(|&(priority, source)| cursors.contains(&(saved.id, priority, source)));
        }
        Ok(())
    }

    fn repair_routes(
        &mut self,
        previous: &Scenario,
        road: &crate::traffic::RoadState,
    ) -> RouteChanges {
        let destinations: BTreeMap<_, _> = previous
            .movements
            .iter()
            .map(|movement| (movement.id, movement.to))
            .collect();
        let has_foot_access: BTreeSet<_> = self
            .scenario
            .demand
            .foot_edges
            .iter()
            .map(|edge| edge.from)
            .collect();
        let exits: BTreeMap<_, _> = self
            .scenario
            .demand
            .zones
            .iter()
            .filter(|zone| has_foot_access.contains(&zone.id))
            .filter_map(|zone| Some((zone.id, zone.road_access?)))
            .collect();
        let mut changes = RouteChanges::default();
        for lane in &mut self.lanes {
            for item in &mut lane.queue {
                let target = item.recovery.map(|recovery| recovery.target).or_else(|| {
                    item.route
                        .last()
                        .and_then(|id| destinations.get(id))
                        .copied()
                });
                let Some(target) = target else {
                    continue;
                };
                if item.recovery.is_none()
                    && road.supports_vehicle_route(lane.id, target, &item.entry, &item.route)
                {
                    continue;
                }
                let units = u64::from(item.entry.units());
                let length =
                    NonZeroU32::new(item.entry.unit_space()).expect("positive vehicle length");
                if let Some((_, replacement)) =
                    road.route_for_length(item.entry.kind(), lane.id, target, length)
                {
                    item.route = replacement;
                    item.recovery = None;
                    changes.rerouted += units;
                } else if matches!(item.entry, crate::QueueEntry::PrivateCars(_)) {
                    let exit_in_progress = item
                        .recovery
                        .and_then(|r| r.exit)
                        .and_then(|zone| exits.get(&zone))
                        .is_some_and(|&exit_lane| {
                            road.supports_car_route(lane.id, exit_lane, &item.route)
                        });
                    if exit_in_progress {
                        continue;
                    }
                    let exit = exits
                        .iter()
                        .filter_map(|(&zone, &lane_id)| {
                            let (cost, route) = road.route(lane.id, lane_id)?;
                            Some((cost, zone, route))
                        })
                        .min();
                    if let Some((_, zone, route)) = exit {
                        item.route = route;
                        item.recovery = Some(CarRecoveryV1 {
                            target,
                            exit: Some(zone),
                        });
                        changes.en_route_to_exit += units;
                    } else {
                        item.route.clear();
                        item.recovery = Some(CarRecoveryV1 { target, exit: None });
                        changes.blocked += units;
                    }
                } else {
                    // Other vehicle services still validate their routes at this boundary.
                    continue;
                }
                lane.wait = None;
                self.affected.insert(lane.id);
            }
        }
        if let Some(demand) = &mut self.demand {
            demand.repair_car_paths(&self.scenario.demand, road);
        }
        changes
    }

    fn finish(mut self, sim: &Simulation, revision: u64) -> Result<PreparedEdit, EditError> {
        let mut world =
            World::from_scenario(self.scenario.clone()).map_err(|error: ValidationError| {
                EditError::WouldInvalidate(LoadError::State(error))
            })?;
        let invalid = EditError::WouldInvalidate;
        let tick = sim.traffic;
        let changes = self.repair_routes(&sim.scenario, &world.road);
        world
            .road
            .restore(tick, self.lanes, self.junctions)
            .map_err(invalid)?;
        world
            .service
            .restore(sim.world.service.snapshot(), tick, &world.road)
            .map_err(invalid)?;
        world
            .taxi
            .restore(sim.world.taxi.snapshot(), tick, &world.road)
            .map_err(invalid)?;
        world
            .demand
            .restore(self.demand, tick, &world.road, &world.service, &world.taxi)
            .map_err(invalid)?;
        for (line, interval) in self.headways {
            if !world.service.set_headway(line, tick, interval) {
                return Err(EditError::MissingLine(line));
            }
        }
        Ok(PreparedEdit {
            scenario: self.scenario,
            world,
            preview: EditPreview {
                revision,
                affected_lanes: self.affected.into_iter().collect(),
                affected_foot_edges: self.affected_foot_edges.into_iter().collect(),
                relocated_vehicles: self.relocated_vehicles,
                rerouted_vehicles: changes.rerouted,
                blocked_vehicles: changes.blocked,
                vehicles_en_route_to_exit: changes.en_route_to_exit,
            },
        })
    }
}

pub(crate) fn prepare(sim: &Simulation, edit: &EditCommand) -> Result<PreparedEdit, EditError> {
    if edit.expected_revision != sim.network_revision {
        return Err(EditError::Outdated {
            expected: edit.expected_revision,
            current: sim.network_revision,
        });
    }
    if edit.actions.is_empty() {
        return Err(EditError::Empty);
    }
    let revision = sim
        .network_revision
        .checked_add(1)
        .ok_or(EditError::RevisionOverflow)?;
    let mut draft = Draft::new(sim);
    for action in &edit.actions {
        draft.action(action, sim.traffic)?;
    }
    draft.normalize()?;
    draft.finish(sim, revision)
}

fn empty_lane(id: LaneId) -> LaneV1 {
    LaneV1 {
        id,
        credit: 0,
        wait: None,
        departure: None,
        queue: Vec::new(),
    }
}

fn space(entry: &crate::QueueEntry) -> u64 {
    u64::from(entry.units()) * u64::from(entry.unit_space())
}
