use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

use serde::{Deserialize, Serialize};

use crate::traffic::RoadState;
use crate::{LaneId, LoadError, MovementId, QueueEntry, Tick, VehicleId, VehicleKind, ZoneId};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaxiVehicleSpec {
    pub id: VehicleId,
    pub start: ZoneId,
    pub length_mm: NonZeroU32,
    pub capacity: NonZeroU32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TaxiSpec {
    pub fleet: Vec<TaxiVehicleSpec>,
}

impl TaxiSpec {
    pub(crate) fn is_empty(&self) -> bool {
        self.fleet.is_empty()
    }

    pub(crate) fn sort_tables(&mut self) {
        self.fleet.sort_unstable_by_key(|vehicle| vehicle.id);
    }
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TaxiError {
    DuplicateVehicle(VehicleId),
    InvalidStart(VehicleId),
}

impl fmt::Display for TaxiError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateVehicle(id) => write!(out, "duplicate vehicle {}", id.get()),
            Self::InvalidStart(id) => write!(out, "invalid starting zone for taxi {}", id.get()),
        }
    }
}

impl std::error::Error for TaxiError {}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TaxiObservation {
    pub vehicles: Vec<TaxiVehicleObservation>,
    /// Includes passengers reserved for a taxi that has yet to pick them up.
    pub waiting_total: u64,
    pub unassigned_total: u64,
    pub oldest_waiting_since: Option<Tick>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TaxiVehicleObservation {
    pub id: VehicleId,
    pub capacity: u32,
    pub status: TaxiStatus,
    /// The assigned trip is waiting for a road connection.
    pub disconnected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TaxiStatus {
    Idle { zone: ZoneId },
    ToPickup { assignment: TaxiAssignment },
    Occupied { assignment: TaxiAssignment },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TaxiAssignment {
    pub origin: ZoneId,
    pub destination: ZoneId,
    pub passengers: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaxiRequest {
    pub(crate) cohort: u64,
    pub(crate) origin: ZoneId,
    pub(crate) destination: ZoneId,
    pub(crate) since: Tick,
    pub(crate) count: u32,
}

impl TaxiRequest {
    fn priority(&self) -> (Tick, u64) {
        (self.since, self.cohort)
    }

    fn relation(&self) -> (ZoneId, ZoneId) {
        (self.origin, self.destination)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Progress {
    Idle { zone: ZoneId },
    ToPickup { passengers: Vec<TaxiRequest> },
    Occupied { passengers: Vec<TaxiRequest> },
}

impl Progress {
    fn passengers(&self) -> &[TaxiRequest] {
        match self {
            Self::Idle { .. } => &[],
            Self::ToPickup { passengers } | Self::Occupied { passengers } => passengers,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaxiSnapshot {
    queue: Vec<TaxiRequest>,
    vehicles: BTreeMap<VehicleId, Progress>,
}

#[derive(Debug)]
pub(crate) struct Taxi {
    fleet: BTreeMap<VehicleId, TaxiVehicleSpec>,
    access: BTreeMap<ZoneId, LaneId>,
    current: TaxiSnapshot,
}

impl Taxi {
    pub(crate) fn new(
        spec: TaxiSpec,
        access: BTreeMap<ZoneId, LaneId>,
        road: &RoadState,
        vehicle_used: impl Fn(VehicleId) -> bool,
    ) -> Result<Self, TaxiError> {
        let mut taxi = Self {
            fleet: BTreeMap::new(),
            access,
            current: TaxiSnapshot::default(),
        };
        for vehicle in spec.fleet {
            let id = vehicle.id;
            if vehicle_used(id) || taxi.fleet.contains_key(&id) {
                return Err(TaxiError::DuplicateVehicle(id));
            }
            if taxi
                .route(vehicle.start, vehicle.start, vehicle.length_mm, road)
                .is_none()
            {
                return Err(TaxiError::InvalidStart(id));
            }
            taxi.current.vehicles.insert(
                id,
                Progress::Idle {
                    zone: vehicle.start,
                },
            );
            taxi.fleet.insert(id, vehicle);
        }
        Ok(taxi)
    }

    fn route(
        &self,
        from: ZoneId,
        to: ZoneId,
        length: NonZeroU32,
        road: &RoadState,
    ) -> Option<(u64, Vec<MovementId>)> {
        road.route_for_length(
            VehicleKind::Taxi,
            *self.access.get(&from)?,
            *self.access.get(&to)?,
            length,
        )
    }

    pub(crate) fn has_access_pair(&self, origin: ZoneId, destination: ZoneId) -> bool {
        !self.fleet.is_empty()
            && self.access.contains_key(&origin)
            && self.access.contains_key(&destination)
    }

    pub(crate) fn estimate(
        &self,
        origin: ZoneId,
        destination: ZoneId,
        road: &RoadState,
    ) -> Option<(u64, u64)> {
        let &pickup = self.access.get(&origin)?;
        let pickup_travel = u64::from(road.lane_travel_ticks(pickup)?);
        let costs = self.current.vehicles.iter().filter_map(|(&id, progress)| {
            let length = self.fleet[&id].length_mm;
            let (available_at, busy_for) = match progress {
                Progress::Idle { zone } => (*zone, 0),
                Progress::ToPickup { passengers } | Progress::Occupied { passengers } => {
                    let first = passengers.first()?;
                    let to_pickup = matches!(progress, Progress::ToPickup { .. });
                    let next = if to_pickup {
                        first.origin
                    } else {
                        first.destination
                    };
                    let (_, current_lane, _) = road
                        .managed_vehicles()
                        .find(|(vehicle, _, _)| *vehicle == id)?;
                    let (mut remaining, _) = road.route_for_length(
                        VehicleKind::Taxi,
                        current_lane,
                        self.access[&next],
                        length,
                    )?;
                    if to_pickup {
                        let (ride, _) = self.route(next, first.destination, length, road)?;
                        remaining = remaining.saturating_add(
                            (ride - u64::from(road.lane_travel_ticks(self.access[&next])?)).max(1),
                        );
                    }
                    (first.destination, remaining)
                }
            };
            let (ride, _) = self.route(origin, destination, length, road)?;
            let (approach, empty_route) = self.route(available_at, origin, length, road)?;
            let (wait, ride) = if empty_route.is_empty() {
                (busy_for, ride)
            } else {
                (
                    busy_for.saturating_add(approach),
                    (ride - pickup_travel).max(1),
                )
            };
            Some((wait, id, ride))
        });
        costs.min().map(|(wait, _, ride)| (wait, ride))
    }

    pub(crate) fn request(&mut self, request: TaxiRequest) {
        if request.count != 0 {
            let index = self
                .current
                .queue
                .partition_point(|queued| queued.priority() <= request.priority());
            self.current.queue.insert(index, request);
        }
    }

    pub(crate) fn tick(&mut self, tick: Tick, road: &mut RoadState) -> Vec<(u64, u32)> {
        let mut arrivals = Vec::new();
        for (&id, progress) in &mut self.current.vehicles {
            if matches!(progress, Progress::Idle { .. }) {
                continue;
            }
            let Some(lane) = road.vehicle_at_stop(id, tick) else {
                continue;
            };
            match progress {
                Progress::ToPickup { passengers } => {
                    let first = &passengers[0];
                    let length = self.fleet[&id].length_mm;
                    if lane == self.access[&first.origin]
                        && let Some((_, route)) = road.route_for_length(
                            VehicleKind::Taxi,
                            lane,
                            self.access[&first.destination],
                            length,
                        )
                        && road.depart_vehicle(id, &route, tick)
                    {
                        *progress = Progress::Occupied {
                            passengers: std::mem::take(passengers),
                        };
                    }
                }
                Progress::Occupied { passengers } => {
                    let zone = passengers[0].destination;
                    if lane == self.access[&zone] && road.park_vehicle(id, tick) {
                        arrivals.extend(
                            passengers
                                .iter()
                                .map(|request| (request.cohort, request.count)),
                        );
                        *progress = Progress::Idle { zone };
                    }
                }
                Progress::Idle { .. } => {}
            }
        }
        self.dispatch(tick, road);
        arrivals
    }

    fn dispatch(&mut self, tick: Tick, road: &mut RoadState) {
        if self.current.queue.is_empty() {
            return;
        }
        let mut free: BTreeMap<_, _> = self
            .current
            .vehicles
            .iter()
            .filter_map(|(&id, progress)| match progress {
                Progress::Idle { zone } => Some((id, *zone)),
                _ => None,
            })
            .collect();
        let mut index = 0;
        let mut skipped_relations = BTreeSet::new();
        while !free.is_empty() && index < self.current.queue.len() {
            let first = self.current.queue[index];
            if first.since > tick {
                break;
            }
            if skipped_relations.contains(&first.relation()) {
                index += 1;
                continue;
            }
            let mut candidates: Vec<_> = free
                .iter()
                .filter_map(|(&id, &zone)| {
                    let vehicle = &self.fleet[&id];
                    let (approach, empty_route) =
                        self.route(zone, first.origin, vehicle.length_mm, road)?;
                    let (_, occupied_route) =
                        self.route(first.origin, first.destination, vehicle.length_mm, road)?;
                    let at_pickup = empty_route.is_empty();
                    let (cost, route) = if at_pickup {
                        (0, occupied_route)
                    } else {
                        (approach, empty_route)
                    };
                    Some((cost, id, self.access[&zone], at_pickup, route))
                })
                .collect();
            candidates.sort_unstable_by_key(|&(cost, id, ..)| (cost, id));
            let dispatched = candidates.into_iter().find(|&(_, id, from, _, ref route)| {
                road.enter_vehicle(
                    from,
                    QueueEntry::Vehicle {
                        id,
                        kind: VehicleKind::Taxi,
                        length_mm: self.fleet[&id].length_mm,
                    },
                    route,
                    tick,
                )
            });
            if let Some((_, id, _, at_pickup, _)) = dispatched {
                free.remove(&id);
                let passengers = self.reserve(first, self.fleet[&id].capacity, tick);
                let progress = if at_pickup {
                    Progress::Occupied { passengers }
                } else {
                    Progress::ToPickup { passengers }
                };
                self.current.vehicles.insert(id, progress);
            } else {
                // The set of free vehicles and road space can only shrink in this pass.
                skipped_relations.insert(first.relation());
                index += 1;
            }
        }
    }

    fn reserve(
        &mut self,
        first: TaxiRequest,
        capacity: NonZeroU32,
        tick: Tick,
    ) -> Vec<TaxiRequest> {
        let mut remaining = capacity.get();
        let mut passengers = Vec::new();
        self.current.queue.retain_mut(|request| {
            if remaining != 0 && request.since <= tick && request.relation() == first.relation() {
                let count = remaining.min(request.count);
                passengers.push(TaxiRequest { count, ..*request });
                request.count -= count;
                remaining -= count;
            }
            request.count != 0
        });
        passengers
    }

    fn waiting(&self) -> impl Iterator<Item = &TaxiRequest> {
        self.current.queue.iter().chain(
            self.current
                .vehicles
                .values()
                .filter_map(|progress| match progress {
                    Progress::ToPickup { passengers } => Some(passengers.as_slice()),
                    _ => None,
                })
                .flatten(),
        )
    }

    pub(crate) fn waiting_cohorts(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.waiting()
            .map(|request| (request.cohort, request.count))
    }

    pub(crate) fn cohorts(&self) -> impl Iterator<Item = (u64, ZoneId, ZoneId, u32)> + '_ {
        self.current
            .queue
            .iter()
            .chain(
                self.current
                    .vehicles
                    .values()
                    .flat_map(Progress::passengers),
            )
            .map(|request| {
                (
                    request.cohort,
                    request.origin,
                    request.destination,
                    request.count,
                )
            })
    }

    pub(crate) fn observe(&self, tick: Tick, road: &RoadState) -> TaxiObservation {
        let mut waiting_total = 0;
        let mut oldest_waiting_since = None;
        for request in self.waiting() {
            waiting_total += u64::from(request.count);
            oldest_waiting_since = oldest_waiting_since
                .into_iter()
                .chain([request.since])
                .min();
        }
        TaxiObservation {
            vehicles: self
                .current
                .vehicles
                .iter()
                .map(|(&id, progress)| {
                    let assignment = |passengers: &[TaxiRequest]| TaxiAssignment {
                        origin: passengers[0].origin,
                        destination: passengers[0].destination,
                        passengers: passengers.iter().map(|request| request.count).sum(),
                    };
                    let status = match progress {
                        Progress::Idle { zone } => TaxiStatus::Idle { zone: *zone },
                        Progress::ToPickup { passengers } => TaxiStatus::ToPickup {
                            assignment: assignment(passengers),
                        },
                        Progress::Occupied { passengers } => TaxiStatus::Occupied {
                            assignment: assignment(passengers),
                        },
                    };
                    let vehicle = &self.fleet[&id];
                    let disconnected = match status {
                        TaxiStatus::Idle { .. } => false,
                        TaxiStatus::Occupied { .. } => road.managed_vehicle_blocked(id),
                        TaxiStatus::ToPickup { assignment } => {
                            road.managed_vehicle_blocked(id)
                                || (road.vehicle_at_stop(id, tick)
                                    == Some(self.access[&assignment.origin])
                                    && self
                                        .route(
                                            assignment.origin,
                                            assignment.destination,
                                            vehicle.length_mm,
                                            road,
                                        )
                                        .is_none())
                        }
                    };
                    TaxiVehicleObservation {
                        id,
                        capacity: vehicle.capacity.get(),
                        status,
                        disconnected,
                    }
                })
                .collect(),
            waiting_total,
            unassigned_total: self
                .current
                .queue
                .iter()
                .map(|request| u64::from(request.count))
                .sum(),
            oldest_waiting_since,
        }
    }

    pub(crate) fn snapshot(&self) -> Option<TaxiSnapshot> {
        (!self.fleet.is_empty()).then(|| self.current.clone())
    }

    pub(crate) fn restore(
        &mut self,
        snapshot: Option<TaxiSnapshot>,
        tick: Tick,
        road: &RoadState,
    ) -> Result<(), LoadError> {
        let invalid = |field| LoadError::InvalidState { field };
        let mut managed: BTreeMap<_, _> = road
            .managed_vehicles()
            .filter(|&(_, _, kind)| kind == VehicleKind::Taxi)
            .map(|(id, lane, _)| (id, lane))
            .collect();
        let Some(snapshot) = snapshot else {
            return if self.fleet.is_empty() && managed.is_empty() {
                Ok(())
            } else {
                Err(invalid("taxi"))
            };
        };
        if !snapshot.vehicles.keys().eq(self.fleet.keys()) {
            return Err(invalid("taxi.ids"));
        }
        if self.fleet.is_empty() && !snapshot.queue.is_empty() {
            return Err(invalid("taxi.queue"));
        }
        if !ordered(&snapshot.queue) {
            return Err(invalid("taxi.queue"));
        }
        for (&id, progress) in &snapshot.vehicles {
            let vehicle = &self.fleet[&id];
            let deployed = managed.remove(&id).is_some();
            if let Progress::Idle { zone } = progress {
                if deployed || self.route(*zone, *zone, vehicle.length_mm, road).is_none() {
                    return Err(invalid("taxi.vehicle_position"));
                }
                continue;
            }
            let passengers = progress.passengers();
            let Some(first) = passengers.first() else {
                return Err(invalid("taxi.assignment"));
            };
            if !ordered(passengers)
                || passengers
                    .iter()
                    .any(|request| request.relation() != first.relation())
                || passengers
                    .iter()
                    .map(|request| u64::from(request.count))
                    .sum::<u64>()
                    > u64::from(vehicle.capacity.get())
                || !self.has_access_pair(first.origin, first.destination)
            {
                return Err(invalid("taxi.assignment"));
            }
            let target = match progress {
                Progress::ToPickup { .. } => first.origin,
                _ => first.destination,
            };
            if !deployed
                || road.managed_vehicle_endpoint(id)
                    != Some((self.access[&target], vehicle.length_mm))
            {
                return Err(invalid("taxi.vehicle_position"));
            }
        }
        if !managed.is_empty() {
            return Err(invalid("taxi.vehicle_position"));
        }
        let mut cohorts = BTreeMap::new();
        for request in snapshot
            .queue
            .iter()
            .chain(snapshot.vehicles.values().flat_map(Progress::passengers))
        {
            if request.since > tick
                || request.count == 0
                || !self.access.contains_key(&request.origin)
                || !self.access.contains_key(&request.destination)
            {
                return Err(invalid("taxi.requests"));
            }
            let (since, relation, count) =
                cohorts
                    .entry(request.cohort)
                    .or_insert((request.since, request.relation(), 0_u64));
            if *since != request.since || *relation != request.relation() {
                return Err(invalid("taxi.requests"));
            }
            *count = count
                .checked_add(u64::from(request.count))
                .ok_or_else(|| invalid("taxi.requests"))?;
        }
        self.current = snapshot;
        Ok(())
    }
}

fn ordered(requests: &[TaxiRequest]) -> bool {
    requests
        .windows(2)
        .all(|pair| pair[0].priority() <= pair[1].priority())
}

#[cfg(test)]
mod tests;
