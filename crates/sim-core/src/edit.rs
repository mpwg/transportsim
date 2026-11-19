use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

use serde::{Deserialize, Serialize};

use crate::{
    BuildingId, Cell, FleetSpec, FootEdgeId, FootEdgeSpec, JunctionSpec, LaneId, LaneSpec, LineId,
    LineSpec, LoadError, MovementId, MovementSpec, Scenario, Simulation, StopId, StopSpec,
    StreetSite, ValidationError, VehicleId,
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
    MoveBuilding {
        building: BuildingId,
        target: Cell,
    },
    UpsertStreet(StreetSite),
    SetStreetWidths {
        lane: LaneId,
        road_width_mm: u32,
        foot_width_mm: u32,
        greenery_width_mm: u32,
    },
    AddTurnPocket(TurnPocket),
    UpsertStop(StopSpec),
    RemoveStop(StopId),
    UpsertLine(LineSpec),
    RemoveLine(LineId),
    UpsertLineVehicle(FleetSpec),
    RemoveLineVehicle(VehicleId),
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
    pub affected_buildings: Vec<BuildingId>,
    pub affected_stops: Vec<StopId>,
    pub affected_lines: Vec<LineId>,
    pub affected_line_vehicles: Vec<VehicleId>,
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
    MissingStreet(LaneId),
    MissingLine(LineId),
    MissingStop(StopId),
    MissingLineVehicle(VehicleId),
    VehicleInUse(VehicleId),
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
            Self::MissingStreet(id) => write!(out, "no street profile for lane {}", id.get()),
            Self::MissingFootEdge(id) => write!(out, "unknown foot edge {}", id.get()),
            Self::MissingMovement(id) => write!(out, "unknown movement {}", id.get()),
            Self::MissingStop(id) => write!(out, "unknown stop {}", id.get()),
            Self::VehicleInUse(id) => write!(out, "vehicle ID {} is already on the road", id.get()),
            Self::MissingLineVehicle(id) => write!(out, "unknown line vehicle {}", id.get()),
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
    affected_buildings: BTreeSet<BuildingId>,
    affected_stops: BTreeSet<StopId>,
    affected_lines: BTreeSet<LineId>,
    affected_line_vehicles: BTreeSet<VehicleId>,
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
            affected_buildings: BTreeSet::new(),
            affected_stops: BTreeSet::new(),
            affected_lines: BTreeSet::new(),
            affected_line_vehicles: BTreeSet::new(),
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
                self.scenario
                    .layout
                    .streets
                    .retain(|street| street.lane != *id);
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
                for street in &mut self.scenario.layout.streets {
                    street.foot_edges.retain(|&edge| edge != *id);
                }
                self.affected_foot_edges.insert(*id);
            }
            EditAction::MoveBuilding { building, target } => {
                self.scenario
                    .layout
                    .move_building(*building, *target)
                    .map_err(|error| {
                        EditError::WouldInvalidate(LoadError::State(ValidationError::Layout(error)))
                    })?;
                self.affected_buildings.insert(*building);
            }
            EditAction::UpsertStreet(street) => {
                let streets = &mut self.scenario.layout.streets;
                if let Some(previous) = streets.iter_mut().find(|old| old.lane == street.lane) {
                    *previous = street.clone();
                } else {
                    streets.push(street.clone());
                }
                self.affected.insert(street.lane);
            }
            EditAction::SetStreetWidths {
                lane,
                road_width_mm,
                foot_width_mm,
                greenery_width_mm,
            } => {
                self.set_street_widths(*lane, *road_width_mm, *foot_width_mm, *greenery_width_mm)?;
            }
            EditAction::AddTurnPocket(pocket) => self.pocket(pocket, tick)?,
            EditAction::UpsertStop(stop) => {
                upsert(&mut self.scenario.service.stops, stop, |old| {
                    old.id == stop.id
                });
                self.affected_stops.insert(stop.id);
            }
            EditAction::RemoveStop(id) => {
                remove(&mut self.scenario.service.stops, |stop| stop.id == *id)
                    .ok_or(EditError::MissingStop(*id))?;
                self.affected_stops.insert(*id);
            }
            EditAction::UpsertLine(line) => {
                upsert(&mut self.scenario.service.lines, line, |old| {
                    old.id == line.id
                });
                self.affected_lines.insert(line.id);
            }
            EditAction::RemoveLine(id) => {
                remove(&mut self.scenario.service.lines, |line| line.id == *id)
                    .ok_or(EditError::MissingLine(*id))?;
                self.headways.remove(id);
                self.affected_lines.insert(*id);
            }
            EditAction::UpsertLineVehicle(vehicle) => {
                upsert(&mut self.scenario.service.fleet, vehicle, |old| {
                    old.id == vehicle.id
                });
                self.affected_line_vehicles.insert(vehicle.id);
            }
            EditAction::RemoveLineVehicle(id) => {
                remove(&mut self.scenario.service.fleet, |vehicle| {
                    vehicle.id == *id
                })
                .ok_or(EditError::MissingLineVehicle(*id))?;
                self.affected_line_vehicles.insert(*id);
            }
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
                self.affected_lines.insert(*line);
            }
        }
        Ok(())
    }

    fn set_street_widths(
        &mut self,
        lane: LaneId,
        road_width_mm: u32,
        foot_width_mm: u32,
        greenery_width_mm: u32,
    ) -> Result<(), EditError> {
        let street = self
            .scenario
            .layout
            .streets
            .iter_mut()
            .find(|street| street.lane == lane)
            .ok_or(EditError::MissingStreet(lane))?;
        street.road_width_mm = road_width_mm;
        street.greenery_width_mm = greenery_width_mm;
        if street.foot_width_mm != foot_width_mm {
            street.foot_width_mm = foot_width_mm;
            if let Some(discharge) = NonZeroU32::new(foot_width_mm.div_ceil(1_000)) {
                for &id in &street.foot_edges {
                    let edge = self
                        .scenario
                        .demand
                        .foot_edges
                        .iter_mut()
                        .find(|edge| edge.id == id)
                        .ok_or(EditError::MissingFootEdge(id))?;
                    edge.discharge = discharge;
                    self.affected_foot_edges.insert(id);
                }
            }
        }
        self.affected.insert(lane);
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
        let assigned_vehicles: BTreeSet<_> = self
            .scenario
            .service
            .fleet
            .iter()
            .map(|vehicle| vehicle.id)
            .chain(self.scenario.taxi.fleet.iter().map(|vehicle| vehicle.id))
            .collect();
        for item in self.lanes.iter().flat_map(|lane| &lane.queue) {
            if let crate::QueueEntry::Vehicle { id, .. } = item.entry
                && !item.managed
                && assigned_vehicles.contains(&id)
            {
                return Err(EditError::VehicleInUse(id));
            }
        }
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
                let target = item
                    .blocked_destination
                    .or(item.recovery.map(|recovery| recovery.target))
                    .or_else(|| {
                        item.route
                            .last()
                            .and_then(|id| destinations.get(id))
                            .copied()
                    });
                let Some(target) = target else {
                    continue;
                };
                if item.recovery.is_none()
                    && item.blocked_destination.is_none()
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
                    item.blocked_destination = None;
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
                } else if item.managed {
                    item.route.clear();
                    item.blocked_destination = Some(target);
                    changes.blocked += units;
                } else {
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
        self.scenario.layout.sort_tables();
        let permitted = unchanged_lines(&sim.scenario.service, &self.scenario.service);
        let mut world = World::from_scenario(self.scenario.clone(), &permitted)
            .map_err(|error| EditError::WouldInvalidate(LoadError::State(error)))?;
        let invalid = EditError::WouldInvalidate;
        let tick = sim.traffic;
        for line in &self.scenario.service.lines {
            if world.service.line_disconnected(line.id)
                != sim.world.service.line_disconnected(line.id)
            {
                self.affected_lines.insert(line.id);
            }
        }
        let changes = self.repair_routes(&sim.scenario, &world.road);
        world
            .road
            .restore(tick, self.lanes, self.junctions)
            .map_err(invalid)?;
        world
            .service
            .reconcile(&sim.world.service, tick, &world.road)
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
                affected_buildings: self.affected_buildings.into_iter().collect(),
                affected_stops: self.affected_stops.into_iter().collect(),
                affected_lines: self.affected_lines.into_iter().collect(),
                affected_line_vehicles: self.affected_line_vehicles.into_iter().collect(),
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

fn upsert<T: Clone>(items: &mut Vec<T>, value: &T, matches: impl Fn(&T) -> bool) {
    if let Some(previous) = items.iter_mut().find(|item| matches(item)) {
        *previous = value.clone();
    } else {
        items.push(value.clone());
    }
}

fn remove<T>(items: &mut Vec<T>, matches: impl Fn(&T) -> bool) -> Option<T> {
    let index = items.iter().position(matches)?;
    Some(items.remove(index))
}

fn unchanged_lines(
    previous: &crate::ServiceSpec,
    current: &crate::ServiceSpec,
) -> BTreeSet<LineId> {
    let stops = |spec: &crate::ServiceSpec| -> BTreeMap<_, _> {
        spec.stops
            .iter()
            .map(|stop| (stop.id, (stop.zone, stop.lane)))
            .collect()
    };
    let longest = |spec: &crate::ServiceSpec, id: LineId| {
        spec.fleet
            .iter()
            .filter(|vehicle| vehicle.line == id)
            .map(|vehicle| vehicle.length_mm)
            .max()
    };
    let previous_stops = stops(previous);
    let current_stops = stops(current);
    current
        .lines
        .iter()
        .filter(|line| {
            previous
                .lines
                .iter()
                .find(|old| old.id == line.id)
                .is_some_and(|old| {
                    old.kind == line.kind
                        && old.stops == line.stops
                        && line
                            .stops
                            .iter()
                            .all(|id| previous_stops.get(id) == current_stops.get(id))
                        && longest(previous, line.id) == longest(current, line.id)
                })
        })
        .map(|line| line.id)
        .collect()
}
