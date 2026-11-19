use crate::Tick;
use crate::junction::{JunctionSpec, JunctionState};
use crate::network::{
    JunctionId, LaneId, LaneSpec, MovementId, MovementSpec, RoadValidationError, VehicleId,
    VehicleKind,
};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroU32;

const CAR_SPACE_MM: u32 = 5_000;

/// A private traffic batch or one indivisible deployed vehicle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QueueEntry {
    PrivateCars(NonZeroU32),
    Vehicle {
        id: VehicleId,
        kind: VehicleKind,
        length_mm: NonZeroU32,
    },
}

impl QueueEntry {
    fn unit_space(&self) -> u32 {
        match self {
            Self::PrivateCars(_) => CAR_SPACE_MM,
            Self::Vehicle { length_mm, .. } => length_mm.get(),
        }
    }

    fn units(&self) -> u32 {
        match self {
            Self::PrivateCars(count) => count.get(),
            Self::Vehicle { .. } => 1,
        }
    }

    fn kind(&self) -> VehicleKind {
        match self {
            Self::PrivateCars(_) => VehicleKind::Car,
            Self::Vehicle { kind, .. } => *kind,
        }
    }

    fn first_units(&self, units: NonZeroU32) -> Self {
        match self {
            Self::PrivateCars(_) => Self::PrivateCars(units),
            vehicle => vehicle.clone(),
        }
    }
}

/// Initial physical occupancy in scenario order on each lane.
#[derive(Clone, Debug)]
pub struct TrafficSeed {
    pub lane: LaneId,
    pub entry: QueueEntry,
    pub ready_at: Tick,
    pub route: Vec<MovementId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WaitReason {
    TravelTime,
    RedSignal,
    DischargeLimit,
    TargetFull,
    Conflict,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LaneObservation {
    pub tick: Tick,
    pub capacity_mm: u32,
    pub occupied_mm: u32,
    pub queue: Vec<QueueEntry>,
    pub wait_reason: Option<WaitReason>,
}

#[derive(Debug)]
struct QueueItem {
    entry: QueueEntry,
    ready_at: Option<Tick>,
    route: VecDeque<MovementId>,
}

#[derive(Debug)]
struct LaneState {
    spec: LaneSpec,
    occupied_mm: u32,
    credit: u64,
    queue: VecDeque<QueueItem>,
    wait: Option<WaitReason>,
}

#[derive(Debug)]
pub(crate) struct RoadState {
    lanes: BTreeMap<LaneId, LaneState>,
    movements: BTreeMap<MovementId, MovementSpec>,
    junctions: BTreeMap<JunctionId, JunctionState>,
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
        };
        for spec in lanes {
            let id = spec.id;
            let state = LaneState {
                spec,
                occupied_mm: 0,
                credit: 0,
                queue: VecDeque::new(),
                wait: None,
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
        let mut lane_id = seed.lane;
        self.validate_lane(lane_id, &seed.entry)?;
        for &id in &seed.route {
            let movement = self
                .movements
                .get(&id)
                .filter(|m| m.from == lane_id)
                .ok_or(RoadValidationError::InvalidRoute {
                    lane: lane_id,
                    movement: id,
                })?;
            lane_id = movement.to;
            self.validate_lane(lane_id, &seed.entry)?;
        }
        let lane = self.lanes.get_mut(&seed.lane).expect("validated seed lane");
        let occupied = u64::from(lane.occupied_mm)
            + u64::from(seed.entry.units()) * u64::from(seed.entry.unit_space());
        if occupied > u64::from(lane.spec.capacity_mm.get()) {
            return Err(RoadValidationError::OverCapacity(seed.lane));
        }
        lane.occupied_mm = occupied as u32;
        lane.queue.push_back(QueueItem {
            entry: seed.entry,
            ready_at: Some(seed.ready_at),
            route: seed.route.into(),
        });
        Ok(())
    }

    fn validate_lane(&self, id: LaneId, entry: &QueueEntry) -> Result<(), RoadValidationError> {
        let lane = self
            .lanes
            .get(&id)
            .ok_or(RoadValidationError::UnknownLane(id))?;
        let kind = entry.kind();
        if !lane.spec.allowed.contains(&kind) {
            return Err(RoadValidationError::VehicleNotAllowed { lane: id, kind });
        }
        if entry.unit_space() > lane.spec.capacity_mm.get() {
            return Err(RoadValidationError::OverCapacity(id));
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
        })
    }

    pub(crate) fn tick(&mut self, tick: Tick) {
        let requests = self.plan_moves(tick);
        let granted = self.reserve_moves(&requests);
        self.commit_moves(tick, &granted);
    }

    fn plan_moves(&mut self, tick: Tick) -> Vec<MoveRequest> {
        let mut requests = Vec::new();
        for (&from, lane) in &mut self.lanes {
            lane.wait = None;
            let Some(head) = lane.queue.front() else {
                lane.credit = 0;
                continue;
            };
            if head.ready_at.is_none_or(|ready| ready > tick) {
                lane.wait = Some(WaitReason::TravelTime);
                continue;
            }
            let movement = head.route.front().copied();
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
            if let Some(remaining) = NonZeroU32::new(head.entry.units() - movement.units.get()) {
                lane.queue.push_front(QueueItem {
                    entry: QueueEntry::PrivateCars(remaining),
                    ready_at: head.ready_at,
                    route: head.route.clone(),
                });
            }
            if let Some(to) = movement.to {
                let target = self.lanes.get_mut(&to).unwrap();
                target.occupied_mm += movement.space_mm;
                head.route.pop_front();
                target.queue.push_back(QueueItem {
                    entry,
                    ready_at: tick
                        .seconds()
                        .checked_add(u64::from(target.spec.travel_ticks.get()))
                        .map(Tick::new),
                    route: head.route,
                });
            }
        }
    }
}
