use std::{borrow::Cow, collections::BTreeMap, fmt, num::NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::demand::DemandState;
use crate::routing::transit::{TimetableLink, TransitCost, TransitPath, itinerary};
use crate::traffic::RoadState;
use crate::{
    FootEdgeId, LaneId, LoadError, MovementId, QueueEntry, Tick, VehicleId, VehicleKind, ZoneId,
};

mod passengers;

use passengers::PassengerState;
pub(crate) use passengers::{Alighting, PassengerRequest};

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
        #[serde(default, skip_serializing_if = "is_zero")]
        exchanged: u64,
    },
    Returning,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug)]
struct Line {
    spec: LineSpec,
    // Each route starts at the corresponding stop. The last is the empty run back.
    routes: Vec<Vec<MovementId>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HeadwayChange {
    ticks: NonZeroU32,
    anchor: Option<Tick>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LineStatus {
    last_order: Option<Tick>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    headway: Option<HeadwayChange>,
    pending: Option<Tick>,
    departed: u64,
    cancelled: u64,
    last_departure: Option<Tick>,
    last_delay: Option<u64>,
}

impl LineStatus {
    fn schedule(self, line: &LineSpec) -> (Option<Tick>, NonZeroU32) {
        self.headway
            .map_or((Some(line.first_departure), line.headway), |change| {
                (change.anchor, change.ticks)
            })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceSnapshot {
    lines: BTreeMap<LineId, LineStatus>,
    vehicles: BTreeMap<VehicleId, Progress>,
    #[serde(default)]
    released: BTreeMap<VehicleId, Tick>,
    #[serde(default, skip_serializing_if = "PassengerState::is_empty")]
    passengers: PassengerState,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ServiceObservation {
    pub lines: Vec<LineObservation>,
    pub vehicles: Vec<ServiceVehicleObservation>,
    pub stops: Vec<StopObservation>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct StopObservation {
    pub id: StopId,
    pub waiting_total: u32,
    pub oldest_waiting_since: Option<Tick>,
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
    pub headway: u32,
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
    pub onboard: u32,
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

    pub(crate) fn enter_passengers(&mut self, request: PassengerRequest) -> u32 {
        let Some(stop) = self.stops.get(&request.board) else {
            return 0;
        };
        if !passengers::serves(&self.lines, &request) {
            return 0;
        }
        self.current.passengers.enter(request, stop.capacity.get())
    }

    pub(crate) fn set_headway(&mut self, id: LineId, tick: Tick, ticks: NonZeroU32) -> bool {
        let Some(state) = self.current.lines.get_mut(&id) else {
            return false;
        };
        let line = &self.lines[&id].spec;
        if state.schedule(line).1 == ticks {
            return true;
        }
        let anchor = if tick < line.first_departure {
            Some(line.first_departure)
        } else {
            tick.seconds()
                .checked_add(u64::from(ticks.get()))
                .map(Tick::new)
        };
        state.headway = Some(HeadwayChange { ticks, anchor });
        true
    }

    pub(crate) fn arrive_at_stops(&mut self, tick: Tick, road: &RoadState) -> Vec<Alighting> {
        let mut alighting = Vec::new();
        for (&id, phase) in &mut self.current.vehicles {
            let Progress::Approaching {
                planned,
                stop_index,
            } = *phase
            else {
                continue;
            };
            let stops = &self.lines[&self.fleet[&id].line].spec.stops;
            let stop = &self.stops[&stops[stop_index]];
            if road.vehicle_at_stop(id, tick) != Some(stop.lane) {
                continue;
            }
            let exchanged = u64::from(self.current.passengers.alight(
                id,
                stop_index,
                &mut alighting,
            ));
            let until = Tick::new(tick.seconds().saturating_add(
                u64::from(stop.minimum_dwell.get())
                    + exchanged.div_ceil(u64::from(stop.passengers_per_tick.get())),
            ));
            *phase = Progress::Dwelling {
                planned,
                stop_index,
                until,
                exchanged,
            };
        }
        alighting
    }

    pub(crate) fn tick(&mut self, tick: Tick, road: &mut RoadState) {
        for (&line_id, line) in &self.lines {
            let line_state = self.current.lines.get_mut(&line_id).unwrap();
            let seconds = tick.seconds();
            let (anchor, headway) = line_state.schedule(&line.spec);
            if let Some(elapsed) = anchor.and_then(|from| seconds.checked_sub(from.seconds())) {
                let order = Tick::new(seconds - elapsed % u64::from(headway.get()));
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
                Progress::Approaching { .. } => {}
                Progress::Dwelling {
                    planned,
                    stop_index,
                    until,
                    exchanged,
                } => {
                    let boarded = u64::from(
                        self.current
                            .passengers
                            .board(vehicle, &line.spec, stop_index, tick),
                    );
                    if boarded != 0 {
                        let stop = &self.stops[&line.spec.stops[stop_index]];
                        let rate = u64::from(stop.passengers_per_tick.get());
                        let total = exchanged + boarded;
                        let additional = total.div_ceil(rate) - exchanged.div_ceil(rate);
                        let until = Tick::new(
                            until
                                .seconds()
                                .saturating_add(additional)
                                .max(tick.seconds().saturating_add(boarded.div_ceil(rate))),
                        );
                        *phase = Progress::Dwelling {
                            planned,
                            stop_index,
                            until,
                            exchanged: total,
                        };
                        continue;
                    }
                    if tick < until {
                        continue;
                    }
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
        let (path, cost) = self.transit_path(from, to, tick, |a, b| demand.walk_route(a, b))?;
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

    pub(crate) fn transit_path(
        &self,
        from: ZoneId,
        to: ZoneId,
        tick: Tick,
        walk: impl Fn(ZoneId, ZoneId) -> Option<(u64, Vec<FootEdgeId>)>,
    ) -> Option<(TransitPath, TransitCost)> {
        let links = if self
            .current
            .lines
            .values()
            .any(|line| line.headway.is_some())
        {
            Cow::Owned(
                self.links
                    .iter()
                    .filter_map(|link| {
                        let line = &self.lines[&link.line].spec;
                        let (anchor, headway) = self.current.lines[&link.line].schedule(line);
                        let offset =
                            link.first_board.seconds() - line.first_departure.seconds().max(1);
                        let first_board = anchor?
                            .seconds()
                            .max(1)
                            .checked_add(offset)
                            .map(Tick::new)?;
                        Some(TimetableLink {
                            first_board,
                            headway,
                            ..*link
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            Cow::Borrowed(self.links.as_slice())
        };
        itinerary(from, to, tick, &links, walk)
    }

    pub(crate) fn valid_transit_path(
        &self,
        origin: ZoneId,
        destination: ZoneId,
        path: &TransitPath,
        walk: impl Fn(ZoneId, ZoneId, &[FootEdgeId]) -> bool,
    ) -> bool {
        let mut zone = origin;
        let mut foot = path.access.as_slice();
        if path.rides.is_empty() {
            return false;
        }
        for ride in &path.rides {
            let Some(link) = self.links.iter().find(|link| {
                (link.line, link.board, link.alight) == (ride.line, ride.board, ride.alight)
            }) else {
                return false;
            };
            if !walk(zone, link.from, foot) {
                return false;
            }
            zone = link.to;
            foot = &ride.transfer_after;
        }
        foot.is_empty() && walk(zone, destination, &path.egress)
    }

    pub(crate) fn passenger_cohorts(&self) -> impl Iterator<Item = (u64, usize, u32)> + '_ {
        self.current.passengers.cohorts()
    }

    pub(crate) fn passenger_rides(
        &self,
    ) -> impl Iterator<Item = (u64, usize, LineId, StopId, StopId)> + '_ {
        self.current.passengers.rides(&self.fleet, &self.lines)
    }

    pub(crate) fn observe(&self) -> ServiceObservation {
        ServiceObservation {
            stops: self
                .stops
                .keys()
                .map(|&id| self.current.passengers.observe_stop(id))
                .collect(),
            lines: self
                .current
                .lines
                .iter()
                .map(|(&id, status)| LineObservation {
                    id,
                    headway: status.schedule(&self.lines[&id].spec).1.get(),
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
                            ..
                        } => (Some(planned), Some(spec.stops[stop_index]), Some(until)),
                    };
                    ServiceVehicleObservation {
                        id,
                        line,
                        kind: spec.kind,
                        capacity: self.fleet[&id].capacity.get(),
                        onboard: self.current.passengers.onboard_total(id),
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
            if self.lines.is_empty()
                && !road
                    .managed_vehicles()
                    .any(|(_, _, kind)| kind != VehicleKind::Taxi)
            {
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
            .filter(|(_, _, kind)| *kind != VehicleKind::Taxi)
            .map(|(id, lane, kind)| (id, (lane, kind)))
            .collect();
        for (&id, &phase) in &snapshot.vehicles {
            let vehicle = &self.fleet[&id];
            let line = &self.lines[&vehicle.line].spec;
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
                    until,
                    exchanged,
                } if planned > tick
                    || until < planned
                    || exchanged > 2 * u64::from(vehicle.capacity.get())
                    || line
                        .stops
                        .get(stop_index)
                        .is_none_or(|stop| self.stops[stop].lane != lane)
                    || road.vehicle_at_stop(id, tick) != Some(lane) =>
                {
                    return Err(invalid("service.vehicle_progress"));
                }
                _ => {}
            }
        }
        if !managed.is_empty() {
            return Err(invalid("service.vehicle_position"));
        }
        snapshot.passengers.validate(
            &self.stops,
            &self.lines,
            &self.fleet,
            &snapshot.vehicles,
            tick,
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DischargeRate, JunctionId, JunctionSpec, LaneSpec, MovementSpec};

    fn nonzero(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    fn setup() -> (Service, RoadState) {
        let lanes = [1, 2, 3]
            .map(|id| LaneSpec {
                id: LaneId::new(id),
                capacity_mm: nonzero(100),
                travel_ticks: nonzero(1),
                discharge: DischargeRate {
                    millimetres: nonzero(100),
                    ticks: nonzero(1),
                },
                allowed: vec![VehicleKind::Bus],
            })
            .to_vec();
        let junction = JunctionId::new(1);
        let movements = [(1, 2), (2, 3), (3, 1)]
            .map(|(from, to)| MovementSpec {
                id: MovementId::new(from),
                from: LaneId::new(from),
                to: LaneId::new(to),
                junction,
                conflict_zones: vec![1],
                priority: 0,
            })
            .to_vec();
        let mut road = RoadState::new(
            lanes,
            movements,
            vec![JunctionSpec {
                id: junction,
                phases: vec![],
            }],
            vec![],
        )
        .unwrap();
        let stops = [1, 2, 3]
            .map(|id| StopSpec {
                id: StopId::new(id),
                zone: ZoneId::new(id),
                lane: LaneId::new(id),
                capacity: nonzero(3),
                minimum_dwell: nonzero(3),
                passengers_per_tick: nonzero(2),
            })
            .to_vec();
        let line = |id| LineSpec {
            id: LineId::new(id),
            kind: VehicleKind::Bus,
            stops: vec![StopId::new(1), StopId::new(2), StopId::new(3)],
            headway: nonzero(20),
            first_departure: Tick::new(1),
        };
        let mut service = Service::new(
            ServiceSpec {
                stops,
                lines: vec![line(1), line(2)],
                fleet: vec![FleetSpec {
                    id: VehicleId::new(1),
                    line: LineId::new(1),
                    length_mm: nonzero(20),
                    capacity: nonzero(2),
                }],
            },
            &road,
            |_| true,
            |_| false,
        )
        .unwrap();
        step(&mut service, &mut road, 1);
        (service, road)
    }

    fn step(service: &mut Service, road: &mut RoadState, time: u64) -> Vec<Alighting> {
        let tick = Tick::new(time);
        let alighting = service.arrive_at_stops(tick, road);
        service.tick(tick, road);
        road.tick(tick);
        service.after_road(tick, road);
        alighting
    }

    fn request(cohort: u64, board: u64, alight: u64, count: u32) -> PassengerRequest {
        PassengerRequest {
            cohort,
            leg: 0,
            line: LineId::new(1),
            board: StopId::new(board),
            alight: StopId::new(alight),
            since: Tick::new(1),
            count,
        }
    }

    #[test]
    fn boarding_uses_passenger_arrival_order_and_skips_other_lines() {
        let (mut service, mut road) = setup();
        let mut other_line = request(1, 1, 2, 1);
        other_line.line = LineId::new(2);
        let mut older = request(3, 1, 2, 1);
        older.since = Tick::new(0);
        assert_eq!(service.enter_passengers(other_line), 1);
        assert_eq!(service.enter_passengers(request(2, 1, 2, 1)), 1);
        assert_eq!(service.enter_passengers(older), 1);
        assert_eq!(service.enter_passengers(request(4, 1, 2, 1)), 0);
        step(&mut service, &mut road, 2);

        let status = service.observe();
        assert_eq!(status.stops[0].waiting_total, 1);
        assert_eq!(status.vehicles[0].onboard, 2);
        assert_eq!(status.vehicles[0].dwell_until, Some(Tick::new(6)));
        for tick in 3..7 {
            assert!(step(&mut service, &mut road, tick).is_empty());
        }
        let arrived = step(&mut service, &mut road, 7);
        assert_eq!(
            arrived.iter().map(|group| group.cohort).collect::<Vec<_>>(),
            [3, 2]
        );
    }

    #[test]
    fn alighting_frees_capacity_before_boarding_at_the_same_stop() {
        let (mut service, mut road) = setup();
        assert_eq!(service.enter_passengers(request(1, 1, 2, 2)), 2);
        assert_eq!(service.enter_passengers(request(2, 2, 3, 3)), 3);
        for tick in 2..7 {
            step(&mut service, &mut road, tick);
        }
        let arrived = step(&mut service, &mut road, 7);
        assert_eq!(arrived[0].count, 2);
        let status = service.observe();
        assert_eq!(status.stops[1].waiting_total, 1);
        assert_eq!(status.stops[1].oldest_waiting_since, Some(Tick::new(1)));
        assert_eq!(status.vehicles[0].onboard, 2);
        assert_eq!(status.vehicles[0].dwell_until, Some(Tick::new(12)));
    }

    #[test]
    fn a_passenger_who_arrives_on_departure_tick_can_extend_dwell() {
        let (mut service, mut road) = setup();
        for tick in 2..5 {
            step(&mut service, &mut road, tick);
        }
        let mut late = request(1, 1, 2, 1);
        late.since = Tick::new(5);
        assert_eq!(service.enter_passengers(late), 1);
        step(&mut service, &mut road, 5);

        let status = service.observe();
        assert_eq!(status.stops[0].waiting_total, 0);
        assert_eq!(status.vehicles[0].onboard, 1);
        assert_eq!(status.vehicles[0].dwell_until, Some(Tick::new(6)));
    }
}
