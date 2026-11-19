mod measurements;

use self::measurements::{TravelWindows, decimal};
use std::{collections::BTreeMap, fmt, num::NonZeroU32};

use serde::{Deserialize, Serialize};

use crate::{
    FootEdgeSpec, JourneyMode, JourneyOption, LaneId, LoadError, Tick, WalkingError,
    routing::{JourneyPath, cost_weight, distribute},
    traffic::RoadState,
    walking::{Walking, WalkingSnapshot},
};

const DAY_TICKS: u64 = 86_400;
const DEFAULT_PLANNING_WINDOW: NonZeroU32 = NonZeroU32::new(900).unwrap();

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ZoneId(u64);

impl ZoneId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneSpec {
    pub id: ZoneId,
    pub residents: u32,
    pub jobs: u32,
    pub shops: u32,
    pub road_access: Option<LaneId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TripPurpose {
    Work,
    Shopping,
    Home,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemandProfile {
    pub purpose: TripPurpose,
    /// Zero is the first simulated second of the day.
    pub departure_offsets: Vec<u32>,
    pub return_offset: u32,
    pub rate_numerator: u32,
    pub rate_denominator: NonZeroU32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DemandSpec {
    pub zones: Vec<ZoneSpec>,
    pub foot_edges: Vec<FootEdgeSpec>,
    pub profiles: Vec<DemandProfile>,
    pub walk_weight: u32,
    pub car_weight: u32,
    pub planning_window_ticks: NonZeroU32,
}

impl Default for DemandSpec {
    fn default() -> Self {
        Self {
            zones: Vec::new(),
            foot_edges: Vec::new(),
            profiles: Vec::new(),
            walk_weight: 1,
            car_weight: 1,
            planning_window_ticks: DEFAULT_PLANNING_WINDOW,
        }
    }
}

impl DemandSpec {
    pub(crate) fn is_default(&self) -> bool {
        self.zones.is_empty()
            && self.foot_edges.is_empty()
            && self.profiles.is_empty()
            && self.walk_weight == 1
            && self.car_weight == 1
            && self.planning_window_ticks == DEFAULT_PLANNING_WINDOW
    }

    pub(crate) fn sort_tables(&mut self) {
        self.zones.sort_unstable_by_key(|zone| zone.id);
        self.foot_edges.sort_unstable_by_key(|edge| edge.id);
    }
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DemandError {
    DuplicateZone(ZoneId),
    MissingRoadAccess { zone: ZoneId, lane: LaneId },
    InvalidProfile(usize),
    Walking(WalkingError),
}

impl fmt::Display for DemandError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateZone(id) => write!(out, "duplicate zone {}", id.get()),
            Self::MissingRoadAccess { zone, lane } => {
                write!(
                    out,
                    "zone {} references unknown lane {}",
                    zone.get(),
                    lane.get()
                )
            }
            Self::InvalidProfile(index) => write!(out, "invalid demand profile {index}"),
            Self::Walking(error) => error.fmt(out),
        }
    }
}

impl std::error::Error for DemandError {}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ActiveJourney {
    pub origin: ZoneId,
    pub destination: Option<ZoneId>,
    pub purpose: TripPurpose,
    pub mode: Option<JourneyMode>,
    pub created: Tick,
    pub waiting: u64,
    pub in_transit: u64,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DemandObservation {
    pub generated: u64,
    pub waiting: u64,
    pub in_transit: u64,
    pub arrived: u64,
    pub unassigned: u64,
    pub pending_returns: u64,
    pub scheduled_departures: u64,
    pub total_travel_ticks: u128,
    pub active: Vec<ActiveJourney>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cohort {
    origin: ZoneId,
    destination: Option<ZoneId>,
    purpose: TripPurpose,
    created: Tick,
    return_due: Option<Tick>,
    path: Option<JourneyPath>,
    waiting: u64,
    in_transit: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReturnOrder {
    from: ZoneId,
    to: ZoneId,
    amount: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DemandSnapshot {
    day: Option<u64>,
    remainder: BTreeMap<ZoneId, Vec<u32>>,
    jobs_used: BTreeMap<ZoneId, u32>,
    #[serde(default)]
    scheduled: BTreeMap<Tick, Vec<Cohort>>,
    cohorts: BTreeMap<u64, Cohort>,
    returns: BTreeMap<Tick, Vec<ReturnOrder>>,
    next_id: u64,
    generated: u64,
    arrived: u64,
    #[serde(with = "decimal")]
    total_travel_ticks: u128,
    #[serde(default)]
    measurements: TravelWindows,
    walking: WalkingSnapshot,
}

#[derive(Debug)]
pub(crate) struct DemandState {
    spec: DemandSpec,
    zones: BTreeMap<ZoneId, ZoneSpec>,
    windows: BTreeMap<u32, Vec<usize>>,
    walking: Walking,
    state: DemandSnapshot,
}

impl DemandState {
    pub(crate) fn new(spec: DemandSpec, road: &RoadState) -> Result<Self, DemandError> {
        let mut zones = BTreeMap::new();
        for zone in &spec.zones {
            if zones.insert(zone.id, zone.clone()).is_some() {
                return Err(DemandError::DuplicateZone(zone.id));
            }
            if let Some(lane) = zone.road_access
                && !road.contains_lane(lane)
            {
                return Err(DemandError::MissingRoadAccess {
                    zone: zone.id,
                    lane,
                });
            }
        }
        let mut windows = BTreeMap::<u32, Vec<usize>>::new();
        for (index, profile) in spec.profiles.iter().enumerate() {
            if profile.purpose == TripPurpose::Home
                || u64::from(profile.return_offset) >= DAY_TICKS
                || profile
                    .departure_offsets
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || profile
                    .departure_offsets
                    .iter()
                    .any(|&offset| offset > profile.return_offset)
            {
                return Err(DemandError::InvalidProfile(index));
            }
            for &offset in &profile.departure_offsets {
                windows.entry(offset).or_default().push(index);
            }
        }
        let walking = Walking::new(
            spec.foot_edges.clone(),
            &zones.keys().copied().collect(),
            |id| road.contains_movement(id),
        )
        .map_err(DemandError::Walking)?;
        Ok(Self {
            spec,
            zones,
            windows,
            walking,
            state: DemandSnapshot::default(),
        })
    }

    pub(crate) fn contains_zone(&self, zone: ZoneId) -> bool {
        self.zones.contains_key(&zone)
    }

    pub(crate) fn walk_route(
        &self,
        from: ZoneId,
        to: ZoneId,
    ) -> Option<(u64, Vec<crate::FootEdgeId>)> {
        self.walking.route(from, to)
    }

    fn options(
        &self,
        origin: ZoneId,
        destination: ZoneId,
        road: &RoadState,
    ) -> Vec<(JourneyOption, JourneyPath)> {
        let mut paths = Vec::new();
        if self.spec.walk_weight != 0
            && let Some((cost, edges)) = self.walking.route(origin, destination)
        {
            paths.push((
                self.option(
                    origin,
                    destination,
                    JourneyMode::Walk,
                    self.spec.walk_weight,
                    cost,
                ),
                JourneyPath::Walk(edges),
            ));
        }
        if self.spec.car_weight != 0
            && let (Some(from), Some(to)) = (
                self.zones[&origin].road_access,
                self.zones[&destination].road_access,
            )
            && let Some((cost, movements)) = road.route(from, to)
        {
            paths.push((
                self.option(
                    origin,
                    destination,
                    JourneyMode::Car,
                    self.spec.car_weight,
                    cost,
                ),
                JourneyPath::Car { from, movements },
            ));
        }
        paths
    }

    pub(crate) fn alternatives(
        &self,
        origin: ZoneId,
        destination: ZoneId,
        road: &RoadState,
    ) -> Vec<JourneyOption> {
        self.options(origin, destination, road)
            .into_iter()
            .map(|(info, _)| info)
            .collect()
    }

    pub(crate) fn tick_before_road(&mut self, tick: Tick, road: &mut RoadState) {
        if self.zones.is_empty() {
            return;
        }
        let elapsed = tick.seconds() - 1;
        if elapsed.is_multiple_of(u64::from(self.spec.planning_window_ticks.get())) {
            self.state.measurements.close();
        }
        let day = elapsed / DAY_TICKS;
        if self.state.day != Some(day) {
            self.open_day(day, tick, road);
        }
        for cohort in self.state.scheduled.remove(&tick).into_iter().flatten() {
            self.create(cohort, road);
        }
        for order in self.state.returns.remove(&tick).into_iter().flatten() {
            self.create(
                Cohort {
                    origin: order.from,
                    destination: Some(order.to),
                    purpose: TripPurpose::Home,
                    created: tick,
                    return_due: None,
                    path: None,
                    waiting: order.amount,
                    in_transit: 0,
                },
                road,
            );
        }

        for (id, amount) in self
            .walking
            .tick(tick, |movement, tick| road.signal_allows(movement, tick))
        {
            self.arrive(id, u64::from(amount), tick);
        }
        self.admit(tick, road);
    }

    pub(crate) fn after_road(&mut self, tick: Tick, road: &RoadState) {
        for (id, amount) in road.car_arrivals() {
            self.arrive(id, u64::from(amount), tick);
        }
    }

    fn open_day(&mut self, day: u64, tick: Tick, road: &RoadState) {
        self.state.day = Some(day);
        self.state.jobs_used.clear();
        let first_offset = ((tick.seconds() - 1) % DAY_TICKS) as u32;
        let windows: Vec<_> = self
            .windows
            .range(first_offset..)
            .map(|(&offset, profiles)| (offset, profiles.clone()))
            .collect();
        for (offset, profiles) in windows {
            let Some(departure) = (day * DAY_TICKS)
                .checked_add(u64::from(offset) + 1)
                .map(Tick::new)
            else {
                break;
            };
            for index in profiles {
                self.plan_window(index, day, departure, road);
            }
        }
    }

    fn plan_window(&mut self, index: usize, day: u64, tick: Tick, road: &RoadState) {
        let profile = &self.spec.profiles[index];
        let purpose = profile.purpose;
        let numerator = u64::from(profile.rate_numerator);
        let denominator = u64::from(profile.rate_denominator.get());
        let due = Tick::new((day * DAY_TICKS).saturating_add(u64::from(profile.return_offset) + 1));
        let mut homes: Vec<_> = self
            .zones
            .values()
            .filter(|zone| zone.residents != 0)
            .map(|zone| (zone.id, zone.residents))
            .collect();
        if !homes.is_empty() {
            let shift = (day % homes.len() as u64) as usize;
            homes.rotate_left(shift);
        }
        for (home, residents) in homes {
            let remainder = self
                .state
                .remainder
                .entry(home)
                .or_insert_with(|| vec![0; self.spec.profiles.len()]);
            let scaled = u64::from(residents) * numerator + u64::from(remainder[index]);
            remainder[index] = (scaled % denominator) as u32;
            let amount = scaled / denominator;
            if amount != 0 {
                self.assign(home, purpose, amount, tick, due, road);
            }
        }
    }

    fn assign(
        &mut self,
        origin: ZoneId,
        purpose: TripPurpose,
        mut remaining: u64,
        tick: Tick,
        due: Tick,
        road: &RoadState,
    ) {
        let mut candidates: Vec<_> = self
            .zones
            .values()
            .filter_map(|zone| {
                let capacity = if purpose == TripPurpose::Work {
                    zone.jobs - self.state.jobs_used.get(&zone.id).copied().unwrap_or(0)
                } else {
                    zone.shops
                };
                if capacity == 0 {
                    return None;
                }
                let cost = self
                    .options(origin, zone.id, road)
                    .into_iter()
                    .map(|(info, _)| {
                        (info.access_ticks + info.ride_ticks).saturating_add(info.wait_ticks)
                    })
                    .min()?;
                Some((zone.id, capacity, cost, 0u64))
            })
            .collect();

        while remaining != 0 {
            let weights: Vec<_> = candidates
                .iter()
                .map(|&(_, capacity, cost, assigned)| {
                    let available = if purpose == TripPurpose::Work {
                        capacity - assigned as u32
                    } else {
                        capacity
                    };
                    cost_weight(available, cost)
                })
                .collect();
            let shares = distribute(remaining, &weights);
            let mut assigned_now = 0;
            for ((_, capacity, _, assigned), share) in candidates.iter_mut().zip(shares) {
                let given = if purpose == TripPurpose::Work {
                    share.min(u64::from(*capacity) - *assigned)
                } else {
                    share
                };
                *assigned += given;
                assigned_now += given;
            }
            if assigned_now == 0 {
                break;
            }
            remaining -= assigned_now;
        }

        let schedule = self.state.scheduled.entry(tick).or_default();
        for (destination, _, _, count) in candidates {
            if count == 0 {
                continue;
            }
            if purpose == TripPurpose::Work {
                *self.state.jobs_used.entry(destination).or_default() += count as u32;
            }
            schedule.push(Cohort {
                origin,
                destination: Some(destination),
                purpose,
                created: tick,
                return_due: Some(due),
                path: None,
                waiting: count,
                in_transit: 0,
            });
        }
        if remaining != 0 {
            schedule.push(Cohort {
                origin,
                destination: None,
                purpose,
                created: tick,
                return_due: Some(due),
                path: None,
                waiting: remaining,
                in_transit: 0,
            });
        }
    }

    fn create(&mut self, cohort: Cohort, road: &RoadState) {
        self.state.generated += cohort.waiting;
        let options = cohort
            .destination
            .map_or_else(Vec::new, |to| self.options(cohort.origin, to, road));
        if options.is_empty() {
            self.insert(cohort);
            return;
        }
        let weights: Vec<_> = options.iter().map(|(info, _)| info.weight).collect();
        let shares = distribute(cohort.waiting, &weights);
        for ((_, path), amount) in options.into_iter().zip(shares) {
            if amount != 0 {
                self.insert(Cohort {
                    path: Some(path),
                    waiting: amount,
                    ..cohort.clone()
                });
            }
        }
    }

    fn insert(&mut self, cohort: Cohort) {
        let id = self.state.next_id;
        self.state.next_id += 1;
        self.state.cohorts.insert(id, cohort);
    }

    fn admit(&mut self, tick: Tick, road: &mut RoadState) {
        let mut local_arrivals = Vec::new();
        for (&id, cohort) in &mut self.state.cohorts {
            if cohort.waiting == 0 {
                continue;
            }
            let offered = cohort.waiting.min(u64::from(u32::MAX)) as u32;
            let admitted = match &cohort.path {
                Some(JourneyPath::Walk(path)) if path.is_empty() => {
                    local_arrivals.push((id, u64::from(offered)));
                    offered
                }
                Some(JourneyPath::Walk(path)) => self.walking.enter(path, offered, tick, id),
                Some(JourneyPath::Car { from, movements }) => {
                    road.enter_cars(*from, movements, offered, tick, id)
                }
                None => 0,
            };
            cohort.waiting -= u64::from(admitted);
            cohort.in_transit += u64::from(admitted);
        }
        for (id, amount) in local_arrivals {
            self.arrive(id, amount, tick);
        }
    }

    fn arrive(&mut self, id: u64, amount: u64, tick: Tick) {
        let cohort = self
            .state
            .cohorts
            .get_mut(&id)
            .expect("arrival has an active cohort");
        cohort.in_transit -= amount;
        self.state.arrived += amount;
        let ticks = tick.seconds() - cohort.created.seconds();
        self.state.total_travel_ticks += u128::from(amount) * u128::from(ticks);
        let destination = cohort.destination.expect("arrival has a destination");
        let mode = cohort.path.as_ref().expect("arrival has a route").mode();
        self.state
            .measurements
            .record(cohort.origin, destination, mode, ticks, amount);
        if let Some(due) = cohort.return_due {
            let release = due.max(Tick::new(tick.seconds().saturating_add(1)));
            self.state
                .returns
                .entry(release)
                .or_default()
                .push(ReturnOrder {
                    from: destination,
                    to: cohort.origin,
                    amount,
                });
        }
        if cohort.in_transit == 0 && cohort.waiting == 0 {
            self.state.cohorts.remove(&id);
        }
    }

    pub(crate) fn observe(&self) -> DemandObservation {
        let active: Vec<_> = self
            .state
            .cohorts
            .values()
            .map(|cohort| ActiveJourney {
                origin: cohort.origin,
                destination: cohort.destination,
                purpose: cohort.purpose,
                mode: cohort.path.as_ref().map(JourneyPath::mode),
                created: cohort.created,
                waiting: cohort.waiting,
                in_transit: cohort.in_transit,
            })
            .collect();
        DemandObservation {
            generated: self.state.generated,
            waiting: active.iter().map(|cohort| cohort.waiting).sum(),
            in_transit: active.iter().map(|cohort| cohort.in_transit).sum(),
            arrived: self.state.arrived,
            unassigned: active
                .iter()
                .filter(|cohort| cohort.destination.is_none())
                .map(|cohort| cohort.waiting)
                .sum(),
            pending_returns: self
                .state
                .returns
                .values()
                .flatten()
                .map(|order| order.amount)
                .sum(),
            scheduled_departures: self
                .state
                .scheduled
                .values()
                .flatten()
                .map(|cohort| cohort.waiting)
                .sum(),
            total_travel_ticks: self.state.total_travel_ticks,
            active,
        }
    }

    pub(crate) fn snapshot(&self) -> Option<DemandSnapshot> {
        if self.spec.is_default() && self.state.day.is_none() {
            return None;
        }
        let mut state = self.state.clone();
        state.walking = self.walking.snapshot();
        Some(state)
    }

    pub(crate) fn restore(
        &mut self,
        saved: Option<DemandSnapshot>,
        tick: Tick,
        road: &RoadState,
    ) -> Result<(), LoadError> {
        let error = |field| LoadError::InvalidState { field };
        let Some(mut saved) = saved else {
            if !self.spec.is_default() || road.car_cohorts().next().is_some() {
                return Err(error("demand"));
            }
            return Ok(());
        };
        self.walking
            .restore(std::mem::take(&mut saved.walking), tick)
            .map_err(|_| error("demand.walking"))?;
        if saved
            .day
            .is_some_and(|day| day != tick.seconds().saturating_sub(1) / DAY_TICKS)
            || saved.remainder.iter().any(|(zone, rests)| {
                !self.contains_zone(*zone)
                    || rests.len() != self.spec.profiles.len()
                    || rests
                        .iter()
                        .zip(&self.spec.profiles)
                        .any(|(&rest, profile)| rest >= profile.rate_denominator.get())
            })
            || saved
                .jobs_used
                .iter()
                .any(|(zone, used)| self.zones.get(zone).is_none_or(|zone| *used > zone.jobs))
        {
            return Err(error("demand.daily"));
        }
        if saved.scheduled.iter().any(|(&departure, cohorts)| {
            departure <= tick
                || saved.day != Some(departure.seconds().saturating_sub(1) / DAY_TICKS)
                || cohorts.is_empty()
                || cohorts.iter().any(|cohort| {
                    cohort.created != departure
                        || cohort.waiting == 0
                        || cohort.in_transit != 0
                        || cohort.path.is_some()
                        || cohort.purpose == TripPurpose::Home
                        || cohort.return_due.is_none_or(|due| due < departure)
                        || !self.contains_zone(cohort.origin)
                        || cohort
                            .destination
                            .is_some_and(|zone| !self.contains_zone(zone))
                })
        }) {
            return Err(error("demand.scheduled"));
        }
        let mut active_modes = BTreeMap::<(u64, bool), u128>::new();
        for (id, amount) in road.car_cohorts() {
            *active_modes.entry((id, true)).or_default() += u128::from(amount);
        }
        for (id, amount) in self.walking.cohorts() {
            *active_modes.entry((id, false)).or_default() += u128::from(amount);
        }
        let mut total = u128::from(saved.arrived);
        for (&id, cohort) in &saved.cohorts {
            if id >= saved.next_id
                || cohort.created > tick
                || !self.contains_zone(cohort.origin)
                || cohort.destination.is_some_and(|to| !self.contains_zone(to))
                || (cohort.destination.is_none() && cohort.path.is_some())
                || (cohort.waiting == 0 && cohort.in_transit == 0)
            {
                return Err(error("demand.cohorts"));
            }
            if let Some(path) = &cohort.path {
                let car = matches!(path, JourneyPath::Car { .. });
                let count = active_modes.remove(&(id, car)).unwrap_or(0);
                if count != u128::from(cohort.in_transit) {
                    return Err(error("demand.in_transit"));
                }
                let to = cohort.destination.expect("validated destination");
                let expected = self.options(cohort.origin, to, road);
                if !expected.iter().any(|(_, candidate)| candidate == path) {
                    return Err(error("demand.path"));
                }
            } else if cohort.in_transit != 0 {
                return Err(error("demand.in_transit"));
            }
            total += u128::from(cohort.waiting) + u128::from(cohort.in_transit);
        }
        if !active_modes.is_empty() || total != u128::from(saved.generated) {
            return Err(error("demand.balance"));
        }
        let orders = saved.returns.values().flatten();
        if saved
            .returns
            .keys()
            .any(|&due| due <= tick && due.seconds() != u64::MAX)
            || orders.clone().any(|order| {
                order.amount == 0
                    || !self.contains_zone(order.from)
                    || !self.contains_zone(order.to)
            })
            || orders.map(|order| u128::from(order.amount)).sum::<u128>()
                > u128::from(saved.arrived)
        {
            return Err(error("demand.returns"));
        }
        if !saved
            .measurements
            .valid(|zone| self.contains_zone(zone), saved.arrived)
            || saved.total_travel_ticks > u128::from(saved.arrived) * u128::from(tick.seconds())
        {
            return Err(error("demand.measurements"));
        }
        self.state = saved;
        Ok(())
    }
    fn option(
        &self,
        from: ZoneId,
        to: ZoneId,
        mode: JourneyMode,
        base: u32,
        ticks: u64,
    ) -> JourneyOption {
        let estimated = self
            .state
            .measurements
            .estimate(from, to, mode)
            .unwrap_or(ticks)
            .max(ticks);

        let (access_ticks, ride_ticks) = if mode == JourneyMode::Walk {
            (ticks, 0)
        } else {
            (0, ticks)
        };
        JourneyOption {
            mode,
            access_ticks,
            wait_ticks: estimated - ticks,
            ride_ticks,
            transfer_ticks: 0,
            weight: cost_weight(base, estimated),
        }
    }
}
