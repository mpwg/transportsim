use std::{collections::BTreeMap, fmt, num::NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::demand::DemandState;
use crate::routing::transit::{TimetableLink, TransitCost, itinerary};
use crate::traffic::RoadState;
use crate::{LaneId, LoadError, MovementId, QueueEntry, Tick, VehicleId, VehicleKind, ZoneId};

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

id!(StopId);
id!(LineId);

/// A stop on a shared lane or on a separately modelled bay or track.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StopSpec {
    pub id: StopId,
    pub zone: ZoneId,
    pub lane: LaneId,
    pub capacity: NonZeroU32,
    pub minimum_dwell: NonZeroU32,
    pub passengers_per_tick: NonZeroU32,
}

/// One direction of a line. Vehicles return to its origin between runs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LineSpec {
    pub id: LineId,
    pub kind: VehicleKind,
    pub stops: Vec<StopId>,
    pub headway: NonZeroU32,
    pub first_departure: Tick,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FleetSpec {
    pub id: VehicleId,
    pub line: LineId,
    pub length_mm: NonZeroU32,
    pub capacity: NonZeroU32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServiceSpec {
    pub stops: Vec<StopSpec>,
    pub lines: Vec<LineSpec>,
    pub fleet: Vec<FleetSpec>,
}

impl ServiceSpec {
    pub(crate) fn is_empty(&self) -> bool {
        self.stops.is_empty() && self.lines.is_empty() && self.fleet.is_empty()
    }

    pub(crate) fn sort_tables(&mut self) {
        self.stops.sort_unstable_by_key(|stop| stop.id);
        self.lines.sort_unstable_by_key(|line| line.id);
        self.fleet.sort_unstable_by_key(|vehicle| vehicle.id);
    }
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ServiceError {
    DuplicateStop(StopId),
    DuplicateLine(LineId),
    DuplicateVehicle(VehicleId),
    InvalidStop(StopId),
    InvalidLine(LineId),
    UnknownLine(VehicleId),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStop(id) => write!(out, "duplicate stop {}", id.get()),
            Self::DuplicateLine(id) => write!(out, "duplicate line {}", id.get()),
            Self::DuplicateVehicle(id) => write!(out, "duplicate vehicle {}", id.get()),
            Self::InvalidStop(id) => write!(out, "invalid stop {}", id.get()),
            Self::InvalidLine(id) => write!(out, "invalid line {}", id.get()),
            Self::UnknownLine(id) => write!(out, "vehicle {} references an unknown line", id.get()),
        }
    }
}
impl std::error::Error for ServiceError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Progress {
    Idle,
    Approaching {
        planned: Tick,
        stop_index: usize,
    },
    Dwelling {
        planned: Tick,
        stop_index: usize,
        until: Tick,
    },
    Returning,
}

#[derive(Debug)]
struct Line {
    spec: LineSpec,
    // Each route starts at the corresponding stop. The last is the empty run back.
    routes: Vec<Vec<MovementId>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LineStatus {
    last_order: Option<Tick>,
    pending: Option<Tick>,
    departed: u64,
    cancelled: u64,
    last_departure: Option<Tick>,
    last_delay: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceSnapshot {
    lines: BTreeMap<LineId, LineStatus>,
    vehicles: BTreeMap<VehicleId, Progress>,
    #[serde(default)]
    released: BTreeMap<VehicleId, Tick>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ServiceObservation {
    pub lines: Vec<LineObservation>,
    pub vehicles: Vec<ServiceVehicleObservation>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TimetableObservation {
    pub first_line: LineId,
    pub first_kind: VehicleKind,
    pub board: StopId,
    pub alight: StopId,
    pub rides: usize,
    pub cost: TransitCost,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LineObservation {
    pub id: LineId,
    pub pending: Option<Tick>,
    pub departed: u64,
    pub cancelled: u64,
    pub last_departure: Option<Tick>,
    pub last_delay: Option<u64>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ServiceVehicleObservation {
    pub id: VehicleId,
    pub line: LineId,
    pub kind: VehicleKind,
    pub capacity: u32,
    pub planned_departure: Option<Tick>,
    pub next_stop: Option<StopId>,
    pub dwell_until: Option<Tick>,
}

#[derive(Debug)]
pub(crate) struct Service {
    stops: BTreeMap<StopId, StopSpec>,
    lines: BTreeMap<LineId, Line>,
    fleet: BTreeMap<VehicleId, FleetSpec>,
    current: ServiceSnapshot,
    links: Vec<TimetableLink>,
}

impl Service {
    pub(crate) fn new(
        spec: ServiceSpec,
        road: &RoadState,
        zone_exists: impl Fn(ZoneId) -> bool,
        vehicle_used: impl Fn(VehicleId) -> bool,
    ) -> Result<Self, ServiceError> {
        let mut stops = BTreeMap::new();
        for stop in spec.stops {
            let id = stop.id;
            if !zone_exists(stop.zone) || !road.contains_lane(stop.lane) {
                return Err(ServiceError::InvalidStop(id));
            }
            if stops.insert(id, stop).is_some() {
                return Err(ServiceError::DuplicateStop(id));
            }
        }
        let mut lines = BTreeMap::new();
        let mut links = Vec::new();
        let mut current = ServiceSnapshot::default();
        for line in spec.lines {
            let id = line.id;
            if line.stops.len() < 2 || !matches!(line.kind, VehicleKind::Bus | VehicleKind::Tram) {
                return Err(ServiceError::InvalidLine(id));
            }
            let largest = spec
                .fleet
                .iter()
                .filter(|vehicle| vehicle.line == id)
                .map(|vehicle| vehicle.length_mm)
                .max();
            let mut routes = Vec::new();
            let mut leg_costs = Vec::new();
            for (index, (&from, &to)) in line
                .stops
                .iter()
                .zip(line.stops.iter().cycle().skip(1))
                .enumerate()
            {
                let (Some(from), Some(to)) = (stops.get(&from), stops.get(&to)) else {
                    return Err(ServiceError::InvalidLine(id));
                };
                if from.lane == to.lane && index + 1 != line.stops.len() {
                    return Err(ServiceError::InvalidLine(id));
                }
                let path = if let Some(length) = largest {
                    road.route_for_length(line.kind, from.lane, to.lane, length)
                } else {
                    road.route_for(line.kind, from.lane, to.lane)
                };
                let Some((cost, route)) = path else {
                    return Err(ServiceError::InvalidLine(id));
                };
                leg_costs.push(cost - u64::from(road.lane_travel_ticks(from.lane).unwrap()));
                routes.push(route);
            }
            if largest.is_some() {
                links.extend(timetable(&line, &stops, &leg_costs, road));
            }
            if lines.insert(id, Line { spec: line, routes }).is_some() {
                return Err(ServiceError::DuplicateLine(id));
            }
            current.lines.insert(id, LineStatus::default());
        }
        let mut fleet = BTreeMap::new();
        for vehicle in spec.fleet {
            let id = vehicle.id;
            if !lines.contains_key(&vehicle.line) {
                return Err(ServiceError::UnknownLine(id));
            }
            if vehicle_used(id) || fleet.insert(id, vehicle).is_some() {
                return Err(ServiceError::DuplicateVehicle(id));
            }
            current.vehicles.insert(id, Progress::Idle);
        }
        Ok(Self {
            stops,
            lines,
            fleet,
            current,
            links,
        })
    }

    pub(crate) fn tick(&mut self, tick: Tick, road: &mut RoadState) {
        for (&line_id, line) in &self.lines {
            let line_state = self.current.lines.get_mut(&line_id).unwrap();
            let seconds = tick.seconds();
            let first = line.spec.first_departure.seconds();
            if let Some(elapsed) = seconds.checked_sub(first) {
                let headway = u64::from(line.spec.headway.get());
                let order = Tick::new(seconds - elapsed % headway);
                if line_state.last_order != Some(order) {
                    if line_state.pending.replace(order).is_some() {
                        line_state.cancelled += 1;
                    }
                    line_state.last_order = Some(order);
                }
            }
        }

        for (&id, phase) in &mut self.current.vehicles {
            let vehicle = &self.fleet[&id];
            let line = &self.lines[&vehicle.line];
            let line_state = self.current.lines.get_mut(&vehicle.line).unwrap();
            match *phase {
                Progress::Idle => {
                    if let Some(planned) = line_state.pending {
                        let first = &self.stops[&line.spec.stops[0]];
                        let entry = QueueEntry::Vehicle {
                            id,
                            kind: line.spec.kind,
                            length_mm: vehicle.length_mm,
                        };
                        if road.enter_vehicle(first.lane, entry, &[], tick) {
                            line_state.pending = None;
                            *phase = Progress::Approaching {
                                planned,
                                stop_index: 0,
                            };
                        }
                    }
                }
                Progress::Approaching {
                    planned,
                    stop_index,
                } => {
                    let stop = &self.stops[&line.spec.stops[stop_index]];
                    if road.vehicle_at_stop(id, tick) == Some(stop.lane) {
                        let until = Tick::new(
                            tick.seconds()
                                .saturating_add(u64::from(stop.minimum_dwell.get())),
                        );
                        *phase = Progress::Dwelling {
                            planned,
                            stop_index,
                            until,
                        };
                    }
                }
                Progress::Dwelling {
                    planned,
                    stop_index,
                    until,
                } if tick >= until => {
                    let route = &line.routes[stop_index];
                    let finishing = stop_index + 1 == line.spec.stops.len();
                    let moved = if finishing && route.is_empty() {
                        road.park_vehicle(id, tick)
                    } else {
                        road.depart_vehicle(id, route, tick)
                    };
                    if moved {
                        if stop_index == 0 {
                            self.current.released.insert(id, planned);
                        }
                        *phase = if !finishing {
                            Progress::Approaching {
                                planned,
                                stop_index: stop_index + 1,
                            }
                        } else if route.is_empty() {
                            Progress::Idle
                        } else {
                            Progress::Returning
                        };
                    }
                }
                Progress::Dwelling { .. } => {}
                Progress::Returning => {
                    let origin = self.stops[&line.spec.stops[0]].lane;
                    if road.vehicle_at_stop(id, tick) == Some(origin) && road.park_vehicle(id, tick)
                    {
                        *phase = Progress::Idle;
                    }
                }
            }
        }
    }

    pub(crate) fn after_road(&mut self, tick: Tick, road: &RoadState) {
        self.current.released.retain(|&id, planned| {
            let line_id = self.fleet[&id].line;
            let first = self.lines[&line_id].spec.stops[0];
            let lane = self.stops[&first].lane;
            if !road.departed_vehicle(lane, id) {
                return true;
            }
            let state = self.current.lines.get_mut(&line_id).unwrap();
            state.departed += 1;
            state.last_departure = Some(tick);
            state.last_delay = Some(tick.seconds() - planned.seconds());
            false
        });
    }

    pub(crate) fn timetable(
        &self,
        from: ZoneId,
        to: ZoneId,
        tick: Tick,
        demand: &DemandState,
    ) -> Option<TimetableObservation> {
        let (path, cost) = itinerary(from, to, tick, &self.links, |a, b| demand.walk_route(a, b))?;
        let first = path.rides.first()?;
        let last = path.rides.last()?;
        Some(TimetableObservation {
            first_line: first.line,
            first_kind: self.lines[&first.line].spec.kind,
            board: first.board,
            alight: last.alight,
            rides: path.rides.len(),
            cost,
        })
    }

    pub(crate) fn observe(&self) -> ServiceObservation {
        ServiceObservation {
            lines: self
                .current
                .lines
                .iter()
                .map(|(&id, status)| LineObservation {
                    id,
                    pending: status.pending,
                    departed: status.departed,
                    cancelled: status.cancelled,
                    last_departure: status.last_departure,
                    last_delay: status.last_delay,
                })
                .collect(),
            vehicles: self
                .current
                .vehicles
                .iter()
                .map(|(&id, phase)| {
                    let line = self.fleet[&id].line;
                    let spec = &self.lines[&line].spec;
                    let (planned_departure, next_stop, dwell_until) = match *phase {
                        Progress::Idle => (None, None, None),
                        Progress::Returning => (None, Some(spec.stops[0]), None),
                        Progress::Approaching {
                            planned,
                            stop_index,
                        } => (Some(planned), Some(spec.stops[stop_index]), None),
                        Progress::Dwelling {
                            planned,
                            stop_index,
                            until,
                        } => (Some(planned), Some(spec.stops[stop_index]), Some(until)),
                    };
                    ServiceVehicleObservation {
                        id,
                        line,
                        kind: spec.kind,
                        capacity: self.fleet[&id].capacity.get(),
                        planned_departure,
                        next_stop,
                        dwell_until,
                    }
                })
                .collect(),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<ServiceSnapshot> {
        (!self.lines.is_empty()).then(|| self.current.clone())
    }

    pub(crate) fn restore(
        &mut self,
        snapshot: Option<ServiceSnapshot>,
        tick: Tick,
        road: &RoadState,
    ) -> Result<(), LoadError> {
        let invalid = |field| LoadError::InvalidState { field };
        let Some(snapshot) = snapshot else {
            if self.lines.is_empty() && road.managed_vehicles().next().is_none() {
                return Ok(());
            }
            return Err(invalid("service"));
        };
        if !snapshot.lines.keys().eq(self.lines.keys())
            || !snapshot.vehicles.keys().eq(self.fleet.keys())
        {
            return Err(invalid("service.ids"));
        }
        if snapshot.lines.values().any(|status| {
            [status.last_order, status.pending, status.last_departure]
                .into_iter()
                .flatten()
                .any(|time| time > tick)
                || status.pending.is_some() && status.pending != status.last_order
        }) {
            return Err(invalid("service.lines"));
        }
        if snapshot.released.iter().any(|(vehicle, &plan)| {
            plan > tick
                || !matches!(
                    snapshot.vehicles.get(vehicle),
                    Some(Progress::Approaching { planned, stop_index: 1 }) if *planned == plan
                )
        }) {
            return Err(invalid("service.released"));
        }
        let mut managed: BTreeMap<_, _> = road
            .managed_vehicles()
            .map(|(id, lane, kind)| (id, (lane, kind)))
            .collect();
        for (&id, &phase) in &snapshot.vehicles {
            let line = &self.lines[&self.fleet[&id].line].spec;
            let physical = managed.remove(&id);
            if phase == Progress::Idle {
                if physical.is_some() {
                    return Err(invalid("service.vehicle_position"));
                }
                continue;
            }
            let Some((lane, kind)) = physical else {
                return Err(invalid("service.vehicle_position"));
            };
            if kind != line.kind {
                return Err(invalid("service.vehicle_kind"));
            }
            match phase {
                Progress::Approaching {
                    planned,
                    stop_index,
                } if planned > tick || stop_index >= line.stops.len() => {
                    return Err(invalid("service.vehicle_progress"));
                }
                Progress::Dwelling {
                    planned,
                    stop_index,
                    ..
                } if planned > tick
                    || line
                        .stops
                        .get(stop_index)
                        .is_none_or(|stop| self.stops[stop].lane != lane) =>
                {
                    return Err(invalid("service.vehicle_progress"));
                }
                _ => {}
            }
        }
        if !managed.is_empty() {
            return Err(invalid("service.vehicle_position"));
        }
        self.current = snapshot;
        Ok(())
    }
}

fn timetable(
    line: &LineSpec,
    stops: &BTreeMap<StopId, StopSpec>,
    leg_costs: &[u64],
    road: &RoadState,
) -> Vec<TimetableLink> {
    let first = &stops[&line.stops[0]];
    let mut arrival = line
        .first_departure
        .seconds()
        .max(1)
        .saturating_add(u64::from(road.lane_travel_ticks(first.lane).unwrap()));
    let mut times = Vec::new();
    for (index, &id) in line.stops.iter().enumerate() {
        let leave = arrival.saturating_add(u64::from(stops[&id].minimum_dwell.get()));
        times.push((arrival, leave));
        arrival = leave.saturating_add(leg_costs[index]);
    }
    let mut links = Vec::new();
    for (board, &id) in line.stops.iter().enumerate() {
        let first_board = Tick::new(times[board].1);
        for next in board + 1..line.stops.len() {
            let alight = line.stops[next];
            links.push(TimetableLink {
                line: line.id,
                board: id,
                alight,
                from: stops[&id].zone,
                to: stops[&alight].zone,
                first_board,
                headway: line.headway,
                ride_ticks: times[next].0 - first_board.seconds(),
            });
        }
    }
    links
}
