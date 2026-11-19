use crate::junction::{JunctionSpec, JunctionState};
use crate::network::{
    JunctionId, LaneId, LaneSpec, MovementId, MovementSpec, RoadValidationError, VehicleId,
    VehicleKind,
};
use crate::save::v1::{CarRecoveryV1, JunctionV1, LaneV1, QueueItemV1};
use crate::{LoadError, Tick, ValidationError, ZoneId};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::num::NonZeroU32;

const CAR_SPACE_MM: u32 = 5_000;

/// A private traffic batch or one indivisible deployed vehicle.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum QueueEntry {
    PrivateCars(NonZeroU32),
    Vehicle {
        id: VehicleId,
        kind: VehicleKind,
        length_mm: NonZeroU32,
    },
}

impl QueueEntry {
    pub(crate) fn unit_space(&self) -> u32 {
        match self {
            Self::PrivateCars(_) => CAR_SPACE_MM,
            Self::Vehicle { length_mm, .. } => length_mm.get(),
        }
    }

    pub(crate) fn units(&self) -> u32 {
        match self {
            Self::PrivateCars(count) => count.get(),
            Self::Vehicle { .. } => 1,
        }
    }

    pub(crate) fn kind(&self) -> VehicleKind {
        match self {
            Self::PrivateCars(_) => VehicleKind::Car,
            Self::Vehicle { kind, .. } => *kind,
        }
    }

    fn vehicle_id(&self) -> Option<VehicleId> {
        match self {
            Self::Vehicle { id, .. } => Some(*id),
            Self::PrivateCars(_) => None,
        }
    }

    pub(crate) fn first_units(&self, units: NonZeroU32) -> Self {
        match self {
            Self::PrivateCars(_) => Self::PrivateCars(units),
            vehicle => vehicle.clone(),
        }
    }
}

/// Initial physical occupancy in scenario order on each lane.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficSeed {
    pub lane: LaneId,
    pub entry: QueueEntry,
    pub ready_at: Tick,
    pub route: Vec<MovementId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum WaitReason {
    TravelTime,
    VehicleStop,
    RedSignal,
    DischargeLimit,
    TargetFull,
    Conflict,
    Disconnected,
}

/// Traffic that cleared the lane on the most recent tick.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LaneDeparture {
    pub entry: QueueEntry,
    /// The next lane, or the end of the route if absent.
    pub to: Option<LaneId>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LaneObservation {
    pub tick: Tick,
    pub capacity_mm: u32,
    pub occupied_mm: u32,
    pub queue: Vec<QueueEntry>,
    pub wait_reason: Option<WaitReason>,
    pub departure: Option<LaneDeparture>,
}

#[derive(Debug)]
struct QueueItem {
    entry: QueueEntry,
    ready_at: Option<Tick>,
    route: VecDeque<MovementId>,
    cohort: Option<u64>,
    managed: bool,
    recovery: Option<CarRecoveryV1>,
}

#[derive(Debug)]
struct LaneState {
    spec: LaneSpec,
    outgoing: Vec<MovementId>,
    occupied_mm: u32,
    credit: u64,
    queue: VecDeque<QueueItem>,
    wait: Option<WaitReason>,
    departure: Option<LaneDeparture>,
}

impl LaneState {
    fn ready_after_entry(&self, tick: Tick) -> Option<Tick> {
        tick.seconds()
            .checked_add(u64::from(self.spec.travel_ticks.get()))
            .map(Tick::new)
    }
}

#[derive(Debug)]
pub(crate) struct RoadState {
    lanes: BTreeMap<LaneId, LaneState>,
    movements: BTreeMap<MovementId, MovementSpec>,
    junctions: BTreeMap<JunctionId, JunctionState>,
    car_arrivals: Vec<(u64, u32)>,
    car_exits: Vec<(u64, ZoneId, u32)>,
}

#[derive(Clone, Copy)]
struct MoveRequest {
    from: LaneId,
    movement: Option<MovementId>,
    units: u32,
    unit_mm: u32,
}

struct GrantedMove {
    from: LaneId,
    to: Option<LaneId>,
    units: NonZeroU32,
    space_mm: u32,
}

impl RoadState {
    /// Free-flow cost includes every lane from the source to the destination.
    pub(crate) fn route(&self, from: LaneId, to: LaneId) -> Option<(u64, Vec<MovementId>)> {
        self.route_for(VehicleKind::Car, from, to)
    }

    pub(crate) fn route_for(
        &self,
        kind: VehicleKind,
        from: LaneId,
        to: LaneId,
    ) -> Option<(u64, Vec<MovementId>)> {
        let length_mm = if kind == VehicleKind::Car {
            NonZeroU32::new(CAR_SPACE_MM).expect("positive car space")
        } else {
            NonZeroU32::MIN
        };
        self.route_for_length(kind, from, to, length_mm)
    }

    /// Free-flow routing through lanes that can accommodate this vehicle.
    pub(crate) fn route_for_length(
        &self,
        kind: VehicleKind,
        from: LaneId,
        to: LaneId,
        length_mm: NonZeroU32,
    ) -> Option<(u64, Vec<MovementId>)> {
        let unit_space = length_mm.get();
        self.validate_lane(from, kind, unit_space).ok()?;
        self.validate_lane(to, kind, unit_space).ok()?;
        let initial_cost = u64::from(self.lanes[&from].spec.travel_ticks.get());
        let mut best = BTreeMap::from([(from, (initial_cost, None))]);
        let mut pending = BinaryHeap::from([Reverse((initial_cost, from))]);

        // Equal costs are visited in lane ID order. Outgoing movements are sorted
        // by ID, and an equal candidate retains the first predecessor.
        while let Some(Reverse((cost, lane))) = pending.pop() {
            if cost != best[&lane].0 {
                continue;
            }
            if lane == to {
                let mut current = to;
                let mut route = Vec::new();
                while let Some(id) = best[&current].1 {
                    route.push(id);
                    current = self.movements[&id].from;
                }
                route.reverse();
                return Some((cost, route));
            }
            for &id in &self.lanes[&lane].outgoing {
                let target = self.movements[&id].to;
                if self.validate_lane(target, kind, unit_space).is_err() {
                    continue;
                }
                let travel = u64::from(self.lanes[&target].spec.travel_ticks.get());
                if let Some(candidate) = cost.checked_add(travel)
                    && best
                        .get(&target)
                        .is_none_or(|&(known, _)| candidate < known)
                {
                    best.insert(target, (candidate, Some(id)));
                    pending.push(Reverse((candidate, target)));
                }
            }
        }
        None
    }

    pub(crate) fn enter_cars(
        &mut self,
        from: LaneId,
        route: &[MovementId],
        count: u32,
        tick: Tick,
        cohort: u64,
    ) -> u32 {
        let Some(lane) = self.lanes.get(&from) else {
            return 0;
        };
        let free = lane.spec.capacity_mm.get() - lane.occupied_mm;
        let Some(admitted) = NonZeroU32::new(count.min(free / CAR_SPACE_MM)) else {
            return 0;
        };
        let item = QueueItem {
            entry: QueueEntry::PrivateCars(admitted),
            ready_at: lane.ready_after_entry(tick),
            route: route.iter().copied().collect(),
            cohort: Some(cohort),
            managed: false,
            recovery: None,
        };
        self.push_queue_item(from, item)
            .map_or(0, |()| admitted.get())
    }

    pub(crate) fn enter_vehicle(
        &mut self,
        from: LaneId,
        entry: QueueEntry,
        route: &[MovementId],
        tick: Tick,
    ) -> bool {
        let (Some(id), Some(lane)) = (entry.vehicle_id(), self.lanes.get(&from)) else {
            return false;
        };
        if self
            .lanes
            .values()
            .flat_map(|lane| &lane.queue)
            .any(|item| item.entry.vehicle_id() == Some(id))
        {
            return false;
        }
        let item = QueueItem {
            entry,
            ready_at: lane.ready_after_entry(tick),
            route: route.iter().copied().collect(),
            cohort: None,
            managed: true,
            recovery: None,
        };
        self.push_queue_item(from, item).is_ok()
    }

    pub(crate) fn vehicle_at_stop(&self, id: VehicleId, tick: Tick) -> Option<LaneId> {
        self.lanes.iter().find_map(|(&lane_id, lane)| {
            let head = lane.queue.front()?;
            (head.managed
                && head.route.is_empty()
                && head.ready_at.is_some_and(|ready| ready <= tick)
                && head.entry.vehicle_id() == Some(id))
            .then_some(lane_id)
        })
    }

    /// Replaces a completed route without repeating the current lane's travel time.
    pub(crate) fn depart_vehicle(
        &mut self,
        id: VehicleId,
        route: &[MovementId],
        not_before: Tick,
    ) -> bool {
        let Some(lane_id) = self.vehicle_at_stop(id, not_before) else {
            return false;
        };
        let head = self.lanes[&lane_id].queue.front().expect("vehicle at stop");
        if self.validate_route(lane_id, &head.entry, route).is_err() {
            return false;
        }
        let lane = self.lanes.get_mut(&lane_id).expect("vehicle at stop");
        let head = lane.queue.front_mut().expect("vehicle at stop");
        head.route.extend(route.iter().copied());
        head.ready_at = Some(not_before);
        true
    }

    pub(crate) fn park_vehicle(&mut self, id: VehicleId, tick: Tick) -> bool {
        let Some(lane_id) = self.vehicle_at_stop(id, tick) else {
            return false;
        };
        let lane = self.lanes.get_mut(&lane_id).expect("vehicle at stop");
        let head = lane.queue.pop_front().expect("vehicle at stop");
        lane.occupied_mm -= head.entry.unit_space();
        if lane.wait == Some(WaitReason::VehicleStop) {
            lane.wait = None;
        }
        true
    }

    pub(crate) fn managed_vehicles(
        &self,
    ) -> impl Iterator<Item = (VehicleId, LaneId, VehicleKind)> + '_ {
        self.lanes.iter().flat_map(|(&lane_id, lane)| {
            lane.queue.iter().filter_map(move |item| match item.entry {
                QueueEntry::Vehicle { id, kind, .. } if item.managed => Some((id, lane_id, kind)),
                _ => None,
            })
        })
    }

    /// Arrivals at the original destination produced by the most recent tick.
    pub(crate) fn car_arrivals(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.car_arrivals.iter().copied()
    }

    /// Recovery departures at alternative access zones on the most recent tick.
    pub(crate) fn car_exits(&self) -> impl Iterator<Item = (u64, ZoneId, u32)> + '_ {
        self.car_exits.iter().copied()
    }

    /// Each part of a cohort that still occupies a lane.
    pub(crate) fn car_cohorts(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.lanes
            .values()
            .flat_map(|lane| &lane.queue)
            .filter_map(|item| Some((item.cohort?, item.entry.units())))
    }

    pub(crate) fn car_recoveries(
        &self,
    ) -> impl Iterator<Item = (Option<u64>, LaneId, CarRecoveryV1)> + '_ {
        self.lanes.iter().flat_map(move |(&lane, state)| {
            state.queue.iter().filter_map(move |item| {
                let recovery = item.recovery?;
                let endpoint = item.route.back().map_or(lane, |id| self.movements[id].to);
                Some((item.cohort, endpoint, recovery))
            })
        })
    }

    pub(crate) fn contains_lane(&self, id: LaneId) -> bool {
        self.lanes.contains_key(&id)
    }

    pub(crate) fn managed_vehicle_endpoint(
        &self,
        vehicle: VehicleId,
    ) -> Option<(LaneId, NonZeroU32)> {
        self.lanes.iter().find_map(|(&lane, state)| {
            state.queue.iter().find_map(|item| {
                let QueueEntry::Vehicle { id, length_mm, .. } = item.entry else {
                    return None;
                };
                (item.managed && id == vehicle).then(|| {
                    let endpoint = item.route.back().map_or(lane, |id| self.movements[id].to);
                    (endpoint, length_mm)
                })
            })
        })
    }

    pub(crate) fn departed_vehicle(&self, lane: LaneId, vehicle: VehicleId) -> bool {
        self.lanes
            .get(&lane)
            .and_then(|lane| lane.departure.as_ref())
            .is_some_and(|departure| departure.entry.vehicle_id() == Some(vehicle))
    }

    pub(crate) fn lane_travel_ticks(&self, id: LaneId) -> Option<u32> {
        self.lanes.get(&id).map(|lane| lane.spec.travel_ticks.get())
    }

    pub(crate) fn contains_movement(&self, id: MovementId) -> bool {
        self.movements.contains_key(&id)
    }

    pub(crate) fn signal_allows(&self, id: MovementId, tick: Tick) -> bool {
        self.movements
            .get(&id)
            .is_some_and(|movement| self.junctions[&movement.junction].allows(id, tick))
    }

    pub(crate) fn new(
        lanes: Vec<LaneSpec>,
        movements: Vec<MovementSpec>,
        junctions: Vec<JunctionSpec>,
        seeds: Vec<TrafficSeed>,
    ) -> Result<Self, RoadValidationError> {
        let mut road = Self {
            lanes: BTreeMap::new(),
            movements: BTreeMap::new(),
            junctions: BTreeMap::new(),
            car_arrivals: Vec::new(),
            car_exits: Vec::new(),
        };
        for spec in lanes {
            let id = spec.id;
            let state = LaneState {
                spec,
                outgoing: Vec::new(),
                occupied_mm: 0,
                credit: 0,
                queue: VecDeque::new(),
                wait: None,
                departure: None,
            };
            if road.lanes.insert(id, state).is_some() {
                return Err(RoadValidationError::DuplicateLane(id));
            }
        }
        for spec in junctions {
            let id = spec.id;
            if road
                .junctions
                .insert(id, JunctionState::new(spec))
                .is_some()
            {
                return Err(RoadValidationError::DuplicateJunction(id));
            }
        }
        for spec in movements {
            let id = spec.id;
            for lane in [spec.from, spec.to] {
                if !road.lanes.contains_key(&lane) {
                    return Err(RoadValidationError::UnknownLane(lane));
                }
            }
            if spec.from == spec.to {
                return Err(RoadValidationError::InvalidMovement(id));
            }
            if !road.junctions.contains_key(&spec.junction) {
                return Err(RoadValidationError::UnknownJunction(spec.junction));
            }
            if road.movements.insert(id, spec).is_some() {
                return Err(RoadValidationError::DuplicateMovement(id));
            }
        }
        road.validate_signals()?;
        for (&id, movement) in &road.movements {
            road.lanes
                .get_mut(&movement.from)
                .expect("validated movement source")
                .outgoing
                .push(id);
        }
        let mut ids = BTreeSet::new();
        for seed in seeds {
            if let QueueEntry::Vehicle { id, .. } = seed.entry
                && !ids.insert(id)
            {
                return Err(RoadValidationError::DuplicateVehicle(id));
            }
            road.seed(seed)?;
        }
        Ok(road)
    }

    fn validate_signals(&self) -> Result<(), RoadValidationError> {
        for junction in self.junctions.values() {
            for (index, phase) in junction.spec.phases.iter().enumerate() {
                let mut zones = BTreeSet::new();
                let mut seen = BTreeSet::new();
                for &id in &phase.movements {
                    let error = RoadValidationError::InvalidSignal {
                        junction: junction.spec.id,
                        phase: index,
                        movement: id,
                    };
                    let Some(movement) = self.movements.get(&id) else {
                        return Err(error);
                    };
                    if movement.junction != junction.spec.id
                        || !seen.insert(id)
                        || !movement
                            .conflict_zones
                            .iter()
                            .copied()
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .all(|zone| zones.insert(zone))
                    {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }

    fn seed(&mut self, seed: TrafficSeed) -> Result<(), RoadValidationError> {
        self.push_queue_item(
            seed.lane,
            QueueItem {
                entry: seed.entry,
                ready_at: Some(seed.ready_at),
                route: seed.route.into(),
                cohort: None,
                managed: false,
                recovery: None,
            },
        )
    }

    fn push_queue_item(
        &mut self,
        start: LaneId,
        item: QueueItem,
    ) -> Result<(), RoadValidationError> {
        self.validate_route(start, &item.entry, &item.route)?;
        let lane = self.lanes.get_mut(&start).expect("validated queue lane");
        let occupied = u64::from(lane.occupied_mm)
            + u64::from(item.entry.units()) * u64::from(item.entry.unit_space());
        if occupied > u64::from(lane.spec.capacity_mm.get()) {
            return Err(RoadValidationError::OverCapacity(start));
        }
        lane.occupied_mm = occupied as u32;
        lane.queue.push_back(item);
        Ok(())
    }

    fn validate_route<'a>(
        &self,
        mut lane_id: LaneId,
        entry: &QueueEntry,
        route: impl IntoIterator<Item = &'a MovementId>,
    ) -> Result<LaneId, RoadValidationError> {
        let kind = entry.kind();
        let unit_space = entry.unit_space();
        self.validate_lane(lane_id, kind, unit_space)?;
        for &id in route {
            let movement = self
                .movements
                .get(&id)
                .filter(|m| m.from == lane_id)
                .ok_or(RoadValidationError::InvalidRoute {
                    lane: lane_id,
                    movement: id,
                })?;
            lane_id = movement.to;
            self.validate_lane(lane_id, kind, unit_space)?;
        }
        Ok(lane_id)
    }

    pub(crate) fn supports_vehicle_route(
        &self,
        from: LaneId,
        to: LaneId,
        entry: &QueueEntry,
        movements: &[MovementId],
    ) -> bool {
        self.validate_route(from, entry, movements)
            .is_ok_and(|end| end == to)
    }

    pub(crate) fn supports_car_route(
        &self,
        from: LaneId,
        to: LaneId,
        movements: &[MovementId],
    ) -> bool {
        self.validate_route(from, &QueueEntry::PrivateCars(NonZeroU32::MIN), movements)
            .is_ok_and(|end| end == to)
    }

    fn validate_lane(
        &self,
        id: LaneId,
        kind: VehicleKind,
        unit_space: u32,
    ) -> Result<(), RoadValidationError> {
        let lane = self
            .lanes
            .get(&id)
            .ok_or(RoadValidationError::UnknownLane(id))?;
        if !lane.spec.allowed.contains(&kind) {
            return Err(RoadValidationError::VehicleNotAllowed { lane: id, kind });
        }
        if unit_space > lane.spec.capacity_mm.get() {
            return Err(RoadValidationError::OverCapacity(id));
        }
        Ok(())
    }

    pub(crate) fn lane_ids(&self) -> impl Iterator<Item = LaneId> + '_ {
        self.lanes.keys().copied()
    }

    pub(crate) fn snapshot(&self) -> (Vec<LaneV1>, Vec<JunctionV1>) {
        let lanes = self
            .lanes
            .iter()
            .map(|(&id, lane)| LaneV1 {
                id,
                credit: lane.credit,
                wait: lane.wait,
                departure: lane.departure.clone(),
                queue: lane
                    .queue
                    .iter()
                    .map(|item| QueueItemV1 {
                        entry: item.entry.clone(),
                        ready_at: item.ready_at,
                        route: item.route.iter().copied().collect(),
                        cohort: item.cohort,
                        managed: item.managed,
                        recovery: item.recovery,
                    })
                    .collect(),
            })
            .collect();
        let junctions = self
            .junctions
            .iter()
            .map(|(&id, junction)| JunctionV1 {
                id,
                last_granted: junction
                    .last_granted
                    .iter()
                    .map(|(&priority, &lane)| (priority, lane))
                    .collect(),
            })
            .collect();
        (lanes, junctions)
    }

    // Used only to construct a new simulation from a save. The running instance
    // is kept by the caller until this entire restoration succeeds.
    pub(crate) fn restore(
        &mut self,
        tick: Tick,
        lanes: Vec<LaneV1>,
        junctions: Vec<JunctionV1>,
    ) -> Result<(), LoadError> {
        if lanes.len() != self.lanes.len() {
            return Err(LoadError::InvalidState { field: "lanes" });
        }
        if junctions.len() != self.junctions.len() {
            return Err(LoadError::InvalidState { field: "junctions" });
        }
        self.car_arrivals.clear();
        self.car_exits.clear();
        for lane in self.lanes.values_mut() {
            lane.queue.clear();
            lane.occupied_mm = 0;
        }
        let mut seen = BTreeSet::new();
        let mut vehicles = BTreeSet::new();
        let road_error = |error| LoadError::State(ValidationError::Road(error));
        for saved in lanes {
            let lane = self
                .lanes
                .get_mut(&saved.id)
                .filter(|_| seen.insert(saved.id))
                .ok_or(LoadError::InvalidState { field: "lanes.id" })?;
            let credit_limit = u64::from(lane.spec.capacity_mm.get())
                * u64::from(lane.spec.discharge.ticks.get())
                + u64::from(lane.spec.discharge.millimetres.get())
                - 1;
            if saved.credit > credit_limit {
                return Err(LoadError::InvalidState {
                    field: "lanes.credit",
                });
            }
            let can_exceed_clock = tick
                .seconds()
                .checked_add(u64::from(lane.spec.travel_ticks.get()))
                .is_none();
            lane.credit = saved.credit;
            lane.wait = saved.wait;
            lane.departure = saved.departure;
            if let Some(departure) = &lane.departure {
                let span =
                    u64::from(departure.entry.units()) * u64::from(departure.entry.unit_space());
                if span > u64::from(lane.spec.capacity_mm.get())
                    || !lane.spec.allowed.contains(&departure.entry.kind())
                    || departure.to.is_some_and(|to| {
                        !self
                            .movements
                            .values()
                            .any(|movement| movement.from == saved.id && movement.to == to)
                    })
                {
                    return Err(LoadError::InvalidState {
                        field: "lanes.departure",
                    });
                }
            }
            for item in saved.queue {
                if item.ready_at.is_none() && !can_exceed_clock {
                    return Err(LoadError::InvalidState {
                        field: "lanes.queue.ready_at",
                    });
                }
                if let Some(recovery) = item.recovery
                    && (!matches!(item.entry, QueueEntry::PrivateCars(_))
                        || (recovery.exit.is_none() && !item.route.is_empty()))
                {
                    return Err(LoadError::InvalidState {
                        field: "lanes.queue.recovery",
                    });
                }
                if let Some(id) = item.entry.vehicle_id() {
                    if item.cohort.is_some() {
                        return Err(LoadError::InvalidState {
                            field: "lanes.queue.cohort",
                        });
                    }
                    if !vehicles.insert(id) {
                        return Err(road_error(RoadValidationError::DuplicateVehicle(id)));
                    }
                } else if item.managed {
                    return Err(LoadError::InvalidState {
                        field: "lanes.queue.managed",
                    });
                }
                self.push_queue_item(
                    saved.id,
                    QueueItem {
                        entry: item.entry,
                        ready_at: item.ready_at,
                        route: item.route.into(),
                        cohort: item.cohort,
                        managed: item.managed,
                        recovery: item.recovery,
                    },
                )
                .map_err(road_error)?;
            }
        }

        let valid_grants: BTreeSet<_> = self
            .movements
            .values()
            .map(|movement| (movement.junction, movement.priority, movement.from))
            .collect();
        let mut seen = BTreeSet::new();
        for saved in junctions {
            let junction = self
                .junctions
                .get_mut(&saved.id)
                .filter(|_| seen.insert(saved.id))
                .ok_or(LoadError::InvalidState {
                    field: "junctions.id",
                })?;
            for (priority, lane) in saved.last_granted {
                if !valid_grants.contains(&(saved.id, priority, lane))
                    || junction.last_granted.insert(priority, lane).is_some()
                {
                    return Err(LoadError::InvalidState {
                        field: "junctions.last_granted",
                    });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn observe(&self, id: LaneId, tick: Tick) -> Option<LaneObservation> {
        let lane = self.lanes.get(&id)?;
        Some(LaneObservation {
            tick,
            capacity_mm: lane.spec.capacity_mm.get(),
            occupied_mm: lane.occupied_mm,
            queue: lane.queue.iter().map(|item| item.entry.clone()).collect(),
            wait_reason: lane.wait,
            departure: lane.departure.clone(),
        })
    }

    pub(crate) fn tick(&mut self, tick: Tick) {
        self.car_arrivals.clear();
        self.car_exits.clear();
        let requests = self.plan_moves(tick);
        let granted = self.reserve_moves(&requests);
        self.commit_moves(tick, &granted);
    }

    fn plan_moves(&mut self, tick: Tick) -> Vec<MoveRequest> {
        let mut requests = Vec::new();
        for (&from, lane) in &mut self.lanes {
            lane.wait = None;
            lane.departure = None;
            let Some(head) = lane.queue.front() else {
                lane.credit = 0;
                continue;
            };
            if head.ready_at.is_none_or(|ready| ready > tick) {
                lane.wait = Some(WaitReason::TravelTime);
                continue;
            }
            if head
                .recovery
                .is_some_and(|recovery| recovery.exit.is_none())
            {
                lane.wait = Some(WaitReason::Disconnected);
                continue;
            }
            let movement = head.route.front().copied();
            if movement.is_none() && head.managed {
                lane.wait = Some(WaitReason::VehicleStop);
                continue;
            }
            if let Some(id) = movement
                && !self.junctions[&self.movements[&id].junction].allows(id, tick)
            {
                lane.wait = Some(WaitReason::RedSignal);
                continue;
            }
            let unit_mm = head.entry.unit_space();
            let cost = u64::from(unit_mm) * u64::from(lane.spec.discharge.ticks.get());
            let earned = u64::from(lane.spec.discharge.millimetres.get());
            // At most one tick of service plus a partially served vehicle.
            lane.credit = lane.credit.saturating_add(earned).min(cost + earned - 1);
            let units = (lane.credit / cost).min(u64::from(head.entry.units())) as u32;
            if units == 0 {
                lane.wait = Some(WaitReason::DischargeLimit);
                continue;
            }
            requests.push(MoveRequest {
                from,
                movement,
                units,
                unit_mm,
            });
        }
        requests.sort_by_key(|request| {
            let movement = request.movement.map(|id| &self.movements[&id]);
            let junction = movement.map(|m| m.junction);
            let last = movement.and_then(|m| {
                self.junctions[&m.junction]
                    .last_granted
                    .get(&m.priority)
                    .copied()
            });
            (
                junction,
                Reverse(movement.map(|m| m.priority)),
                last.is_some_and(|id| request.from <= id),
                request.from,
            )
        });
        requests
    }

    fn reserve_moves(&mut self, requests: &[MoveRequest]) -> Vec<GrantedMove> {
        let mut reserved = BTreeMap::<LaneId, u32>::new();
        let mut conflicts = BTreeSet::new();
        let mut granted = Vec::new();
        let mut advanced = BTreeSet::new();
        for request in requests {
            let movement = request.movement.map(|id| &self.movements[&id]);
            let mut units = request.units;
            if let Some(movement) = movement {
                let target = &self.lanes[&movement.to];
                let free = target.spec.capacity_mm.get()
                    - target.occupied_mm
                    - reserved.get(&movement.to).copied().unwrap_or(0);
                units = units.min(free / request.unit_mm);
                if units == 0 {
                    self.lanes.get_mut(&request.from).unwrap().wait = Some(WaitReason::TargetFull);
                    continue;
                }
                if movement
                    .conflict_zones
                    .iter()
                    .any(|zone| conflicts.contains(&(movement.junction, *zone)))
                {
                    self.lanes.get_mut(&request.from).unwrap().wait = Some(WaitReason::Conflict);
                    continue;
                }
                conflicts.extend(
                    movement
                        .conflict_zones
                        .iter()
                        .map(|zone| (movement.junction, *zone)),
                );
                *reserved.entry(movement.to).or_default() += units * request.unit_mm;
                if advanced.insert((movement.junction, movement.priority)) {
                    self.junctions
                        .get_mut(&movement.junction)
                        .unwrap()
                        .last_granted
                        .insert(movement.priority, request.from);
                }
            }
            granted.push(GrantedMove {
                from: request.from,
                to: movement.map(|m| m.to),
                units: NonZeroU32::new(units).expect("grant must move at least one unit"),
                space_mm: units * request.unit_mm,
            });
        }
        granted
    }

    fn commit_moves(&mut self, tick: Tick, moves: &[GrantedMove]) {
        for movement in moves {
            let lane = self.lanes.get_mut(&movement.from).unwrap();
            let mut head = lane.queue.pop_front().expect("grant has a queue head");
            lane.occupied_mm -= movement.space_mm;
            lane.credit -=
                u64::from(movement.space_mm) * u64::from(lane.spec.discharge.ticks.get());
            let entry = head.entry.first_units(movement.units);
            lane.departure = Some(LaneDeparture {
                entry: entry.clone(),
                to: movement.to,
            });
            if let Some(remaining) = NonZeroU32::new(head.entry.units() - movement.units.get()) {
                lane.queue.push_front(QueueItem {
                    entry: QueueEntry::PrivateCars(remaining),
                    ready_at: head.ready_at,
                    route: head.route.clone(),
                    cohort: head.cohort,
                    managed: false,
                    recovery: head.recovery,
                });
            }
            if let Some(to) = movement.to {
                let target = self.lanes.get_mut(&to).unwrap();
                target.occupied_mm += movement.space_mm;
                head.route.pop_front();
                target.queue.push_back(QueueItem {
                    entry,
                    ready_at: target.ready_after_entry(tick),
                    route: head.route,
                    cohort: head.cohort,
                    managed: head.managed,
                    recovery: head.recovery,
                });
            } else if let Some(cohort) = head.cohort {
                let units = movement.units.get();
                match head.recovery {
                    Some(CarRecoveryV1 {
                        exit: Some(zone), ..
                    }) => {
                        self.car_exits.push((cohort, zone, units));
                    }
                    None => self.car_arrivals.push((cohort, units)),
                    Some(CarRecoveryV1 { exit: None, .. }) => {
                        unreachable!("disconnected cars cannot depart");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DischargeRate, SignalPhase};

    fn lane(id: u64, travel_ticks: u32, capacity_mm: u32, kind: VehicleKind) -> LaneSpec {
        LaneSpec {
            id: LaneId::new(id),
            capacity_mm: NonZeroU32::new(capacity_mm).unwrap(),
            travel_ticks: NonZeroU32::new(travel_ticks).unwrap(),
            discharge: DischargeRate {
                millimetres: NonZeroU32::new(5_000).unwrap(),
                ticks: NonZeroU32::MIN,
            },
            allowed: vec![kind],
        }
    }

    fn movement(id: u64, from: u64, to: u64) -> MovementSpec {
        MovementSpec {
            id: MovementId::new(id),
            from: LaneId::new(from),
            to: LaneId::new(to),
            junction: JunctionId::new(1),
            conflict_zones: Vec::new(),
            priority: 0,
        }
    }

    fn road(
        lanes: Vec<LaneSpec>,
        movements: Vec<MovementSpec>,
        phases: Vec<SignalPhase>,
    ) -> RoadState {
        RoadState::new(
            lanes,
            movements,
            vec![JunctionSpec {
                id: JunctionId::new(1),
                phases,
            }],
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn car_route_minimizes_travel_time_over_compatible_lanes() {
        let road = road(
            vec![
                lane(1, 2, 5_000, VehicleKind::Car),
                lane(2, 9, 5_000, VehicleKind::Car),
                lane(3, 3, 5_000, VehicleKind::Car),
                lane(4, 5, 5_000, VehicleKind::Car),
                lane(5, 1, 5_000, VehicleKind::Bus),
                lane(6, 1, 4_999, VehicleKind::Car),
            ],
            vec![
                movement(1, 1, 2),
                movement(2, 2, 4),
                movement(4, 1, 3),
                movement(7, 3, 4),
                movement(3, 1, 3),
                movement(5, 1, 5),
                movement(6, 5, 4),
                movement(8, 1, 6),
                movement(9, 6, 4),
                movement(10, 3, 1),
            ],
            Vec::new(),
        );
        assert_eq!(
            road.route(LaneId::new(1), LaneId::new(4)),
            Some((10, vec![MovementId::new(3), MovementId::new(7)])),
        );
        assert_eq!(
            road.route(LaneId::new(1), LaneId::new(1)),
            Some((2, vec![]))
        );
        assert_eq!(road.route(LaneId::new(4), LaneId::new(1)), None);
        assert_eq!(road.route(LaneId::new(1), LaneId::new(5)), None);
        assert_eq!(road.route(LaneId::new(1), LaneId::new(999)), None);
    }

    #[test]
    fn vehicle_route_uses_lanes_open_to_its_kind() {
        let mut start = lane(1, 2, 20_000, VehicleKind::Bus);
        start.allowed.push(VehicleKind::Car);
        let mut end = lane(4, 3, 20_000, VehicleKind::Bus);
        end.allowed.push(VehicleKind::Car);
        let road = road(
            vec![
                start,
                lane(2, 4, 20_000, VehicleKind::Bus),
                lane(3, 1, 5_000, VehicleKind::Car),
                end,
                lane(5, 1, 5_000, VehicleKind::Bus),
            ],
            vec![
                movement(1, 1, 2),
                movement(2, 2, 4),
                movement(3, 1, 3),
                movement(4, 3, 4),
                movement(5, 1, 5),
                movement(6, 5, 4),
            ],
            Vec::new(),
        );
        let from = LaneId::new(1);
        let to = LaneId::new(4);
        assert_eq!(
            road.route_for(VehicleKind::Bus, from, to),
            Some((6, vec![MovementId::new(5), MovementId::new(6)])),
        );
        assert_eq!(
            road.route_for_length(VehicleKind::Bus, from, to, NonZeroU32::new(10_000).unwrap(),),
            Some((9, vec![MovementId::new(1), MovementId::new(2)])),
        );
        assert_eq!(
            road.route(from, to),
            Some((6, vec![MovementId::new(3), MovementId::new(4)])),
        );
        assert_eq!(road.route_for(VehicleKind::Tram, from, to), None);
        assert_eq!(road.lane_travel_ticks(to), Some(3));
        assert_eq!(road.lane_travel_ticks(LaneId::new(9)), None);
    }

    #[test]
    fn managed_vehicle_waits_for_release_at_the_end_of_each_route() {
        let make_road = || {
            let mut stop = lane(2, 3, 15_000, VehicleKind::Bus);
            stop.allowed.push(VehicleKind::Car);
            let mut start = lane(1, 2, 15_000, VehicleKind::Bus);
            start.discharge.millimetres = NonZeroU32::new(10_000).unwrap();
            road(
                vec![start, stop, lane(3, 1, 10_000, VehicleKind::Bus)],
                vec![movement(1, 1, 2), movement(2, 2, 3)],
                Vec::new(),
            )
        };
        let mut road = make_road();
        let (start, stop, terminal) = (LaneId::new(1), LaneId::new(2), LaneId::new(3));
        let id = VehicleId::new(10);
        let bus = QueueEntry::Vehicle {
            id,
            kind: VehicleKind::Bus,
            length_mm: NonZeroU32::new(10_000).unwrap(),
        };
        let route = [MovementId::new(1)];
        assert!(!road.enter_vehicle(start, bus.clone(), &[MovementId::new(2)], Tick::new(0)));
        assert!(!road.enter_vehicle(
            start,
            QueueEntry::PrivateCars(NonZeroU32::MIN),
            &route,
            Tick::new(0),
        ));
        assert!(road.enter_vehicle(start, bus.clone(), &route, Tick::new(0)));
        assert!(!road.enter_vehicle(stop, bus, &[], Tick::new(0)));
        assert!(!road.park_vehicle(id, Tick::new(0)));

        road.tick(Tick::new(1));
        assert_eq!(
            road.managed_vehicles().collect::<Vec<_>>(),
            vec![(id, start, VehicleKind::Bus)],
        );
        road.tick(Tick::new(2));
        assert_eq!(road.enter_cars(stop, &[], 1, Tick::new(2), 12), 1);
        road.tick(Tick::new(3));
        road.tick(Tick::new(4));
        assert_eq!(road.vehicle_at_stop(id, Tick::new(4)), None);
        assert!(!road.depart_vehicle(id, &[MovementId::new(2)], Tick::new(4)));
        road.tick(Tick::new(5));
        assert_eq!(road.vehicle_at_stop(id, Tick::new(5)), Some(stop));
        assert_eq!(
            road.observe(stop, Tick::new(5)).unwrap().wait_reason,
            Some(WaitReason::VehicleStop),
        );

        let (lanes, junctions) = road.snapshot();
        let mut road = make_road();
        road.restore(Tick::new(5), lanes, junctions).unwrap();
        road.tick(Tick::new(6));
        assert_eq!(road.vehicle_at_stop(id, Tick::new(6)), Some(stop));
        assert_eq!(
            road.observe(stop, Tick::new(6)).unwrap().occupied_mm,
            15_000
        );
        assert!(road.car_arrivals().next().is_none());
        assert!(!road.depart_vehicle(id, &route, Tick::new(6)));
        assert!(road.depart_vehicle(id, &[MovementId::new(2)], Tick::new(7)));

        road.tick(Tick::new(7));
        assert_eq!(
            road.observe(stop, Tick::new(7)).unwrap().occupied_mm,
            15_000
        );
        road.tick(Tick::new(8));
        assert_eq!(road.observe(stop, Tick::new(8)).unwrap().occupied_mm, 5_000);
        assert!(!road.park_vehicle(id, Tick::new(8)));
        road.tick(Tick::new(9));
        assert_eq!(road.car_arrivals().collect::<Vec<_>>(), vec![(12, 1)]);
        assert_eq!(road.vehicle_at_stop(id, Tick::new(9)), Some(terminal));
        assert!(road.park_vehicle(id, Tick::new(9)));
        assert_eq!(road.observe(terminal, Tick::new(9)).unwrap().occupied_mm, 0);
        assert!(road.managed_vehicles().next().is_none());
    }

    #[test]
    fn car_entries_take_source_travel_time_and_report_split_terminal_arrivals() {
        let make_road = || {
            road(
                vec![
                    lane(1, 2, 10_000, VehicleKind::Car),
                    lane(2, 3, 10_000, VehicleKind::Car),
                ],
                vec![movement(1, 1, 2)],
                Vec::new(),
            )
        };
        let mut road = make_road();
        let from = LaneId::new(1);
        let route = [MovementId::new(1)];
        assert_eq!(
            road.enter_cars(from, &[MovementId::new(9)], 1, Tick::new(0), 1),
            0
        );
        assert_eq!(road.enter_cars(from, &route, 3, Tick::new(0), 42), 2);
        assert_eq!(road.enter_cars(from, &route, 1, Tick::new(0), 43), 0);
        road.tick(Tick::new(1));
        assert!(
            road.observe(from, Tick::new(1))
                .unwrap()
                .departure
                .is_none()
        );
        road.tick(Tick::new(2));
        assert!(
            road.observe(from, Tick::new(2))
                .unwrap()
                .departure
                .is_some()
        );
        assert!(road.car_arrivals().next().is_none());
        assert_eq!(
            road.car_cohorts().collect::<Vec<_>>(),
            vec![(42, 1), (42, 1)]
        );

        let (lanes, junctions) = road.snapshot();
        let mut road = make_road();
        road.restore(Tick::new(2), lanes, junctions).unwrap();
        for i in 3..5 {
            road.tick(Tick::new(i));
            assert!(road.car_arrivals().next().is_none());
        }
        for i in 5..7 {
            road.tick(Tick::new(i));
            assert_eq!(road.car_arrivals().collect::<Vec<_>>(), vec![(42, 1)]);
        }
        road.tick(Tick::new(7));
        assert!(road.car_arrivals().next().is_none());
        assert!(road.car_cohorts().next().is_none());
    }

    #[test]
    fn car_recovery_survives_split_and_restore() {
        let make_road = || {
            road(
                (1..=5)
                    .map(|id| lane(id, 1, 10_000, VehicleKind::Car))
                    .collect(),
                vec![movement(1, 1, 2)],
                Vec::new(),
            )
        };
        let mut road = make_road();
        let route = [MovementId::new(1)];
        let zero = Tick::new(0);
        assert_eq!(road.enter_cars(LaneId::new(1), &route, 2, zero, 42), 2);
        assert_eq!(road.enter_cars(LaneId::new(3), &[], 1, zero, 43), 1);
        assert_eq!(road.enter_cars(LaneId::new(3), &[], 1, zero, 44), 1);
        assert_eq!(road.enter_cars(LaneId::new(4), &[], 1, zero, 45), 1);
        road.seed(TrafficSeed {
            lane: LaneId::new(5),
            entry: QueueEntry::PrivateCars(NonZeroU32::MIN),
            ready_at: Tick::new(1),
            route: Vec::new(),
        })
        .unwrap();

        let (lanes, junctions) = road.snapshot();
        let mut saved = serde_json::to_value(lanes).unwrap();
        saved[0]["queue"][0]["recovery"] = serde_json::json!({ "target": 90, "exit": 7 });
        saved[2]["queue"][0]["recovery"] = serde_json::json!({ "target": 91, "exit": null });
        saved[4]["queue"][0]["recovery"] = serde_json::json!({ "target": 92, "exit": 8 });
        road.restore(zero, serde_json::from_value(saved).unwrap(), junctions)
            .unwrap();

        road.tick(Tick::new(1));
        let blocked = road.observe(LaneId::new(3), Tick::new(1)).unwrap();
        assert_eq!(blocked.wait_reason, Some(WaitReason::Disconnected));
        assert_eq!(blocked.occupied_mm, 10_000);
        assert!(blocked.departure.is_none());
        assert_eq!(road.car_arrivals().collect::<Vec<_>>(), vec![(45, 1)]);
        assert!(road.car_exits().next().is_none());
        assert_eq!(
            road.observe(LaneId::new(5), Tick::new(1))
                .unwrap()
                .occupied_mm,
            0
        );

        let snapshot = serde_json::to_vec(&road.snapshot()).unwrap();
        let (lanes, junctions) = serde_json::from_slice(&snapshot).unwrap();
        let mut road = make_road();
        road.restore(Tick::new(1), lanes, junctions).unwrap();
        let recoveries: Vec<_> = road
            .car_recoveries()
            .map(|(cohort, end, recovery)| (cohort, end, recovery.target, recovery.exit))
            .collect();
        assert_eq!(
            recoveries,
            [
                (
                    Some(42),
                    LaneId::new(2),
                    LaneId::new(90),
                    Some(ZoneId::new(7))
                ),
                (
                    Some(42),
                    LaneId::new(2),
                    LaneId::new(90),
                    Some(ZoneId::new(7))
                ),
                (Some(43), LaneId::new(3), LaneId::new(91), None),
            ]
        );
        for tick in [Tick::new(2), Tick::new(3)] {
            road.tick(tick);
            assert!(road.car_arrivals().next().is_none());
            assert_eq!(
                road.car_exits().collect::<Vec<_>>(),
                vec![(42, ZoneId::new(7), 1)]
            );
        }
        road.tick(Tick::new(4));
        assert!(road.car_exits().next().is_none());
        let blocked = road.observe(LaneId::new(3), Tick::new(4)).unwrap();
        assert_eq!(blocked.wait_reason, Some(WaitReason::Disconnected));
        assert_eq!(blocked.occupied_mm, 10_000);
        assert_eq!(
            road.car_cohorts().collect::<Vec<_>>(),
            vec![(43, 1), (44, 1)]
        );
    }

    #[test]
    fn road_references_resolve_only_known_lanes_and_movements() {
        let road = road(
            vec![
                lane(1, 1, 5_000, VehicleKind::Car),
                lane(2, 1, 5_000, VehicleKind::Car),
            ],
            vec![movement(1, 1, 2)],
            vec![SignalPhase {
                green_ticks: NonZeroU32::MIN,
                clearance_ticks: 1,
                movements: vec![MovementId::new(1)],
            }],
        );
        assert!(road.contains_lane(LaneId::new(1)));
        assert!(!road.contains_lane(LaneId::new(3)));
        assert!(road.contains_movement(MovementId::new(1)));
        assert!(!road.contains_movement(MovementId::new(3)));
        assert!(road.signal_allows(MovementId::new(1), Tick::new(1)));
        assert!(!road.signal_allows(MovementId::new(1), Tick::new(2)));
        assert!(!road.signal_allows(MovementId::new(3), Tick::new(1)));
    }
}
