use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::{
    JourneyMode, LoadError, Tick,
    routing::{JourneyPath, transit::TransitPath},
    service::{Alighting, PassengerRequest, Service},
};

use super::{Cohort, DemandState};

/// Walk tokens occupy a separate range from direct journey cohort IDs.
pub(super) const WALK_TOKEN: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
enum Stage {
    Walk,
    Board,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Part {
    cohort: u64,
    leg: usize,
    amount: u32,
    since: Tick,
    stage: Stage,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct TransitState {
    pending: VecDeque<Part>,
    walking: BTreeMap<u64, Part>,
    next_token: u64,
}

impl TransitState {
    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.walking.is_empty() && self.next_token == 0
    }

    pub(super) fn start(&mut self, cohort: u64, amount: u32, tick: Tick) {
        self.pending.push_back(Part {
            cohort,
            leg: 0,
            amount,
            since: tick,
            stage: Stage::Walk,
        });
    }

    pub(super) fn transferring(&self, cohorts: &BTreeMap<u64, Cohort>) -> u64 {
        self.walking
            .values()
            .filter(|part| {
                part.leg != 0 && part.leg < transit_path(cohorts, part.cohort).rides.len()
            })
            .map(|part| u64::from(part.amount))
            .sum()
    }

    pub(super) fn validate(
        &self,
        cohorts: &BTreeMap<u64, Cohort>,
        tick: Tick,
        service: &Service,
        walkers: impl Iterator<Item = (u64, u32)>,
        active: &mut BTreeMap<(u64, JourneyMode), u128>,
    ) -> Result<(), LoadError> {
        let error = || LoadError::InvalidState {
            field: "demand.transit",
        };
        let check_part = |part: &Part| {
            if part.amount == 0 || part.since > tick {
                return false;
            }
            let Some(JourneyPath::Transit(path)) =
                cohorts.get(&part.cohort).and_then(|c| c.path.as_ref())
            else {
                return false;
            };
            match part.stage {
                Stage::Walk => part.leg <= path.rides.len(),
                Stage::Board => part.leg < path.rides.len(),
            }
        };
        let mut physical_walkers = BTreeMap::<u64, u128>::new();
        for (token, count) in walkers.filter(|(id, _)| *id >= WALK_TOKEN) {
            *physical_walkers.entry(token).or_default() += u128::from(count);
        }
        for (&token, part) in &self.walking {
            if !check_part(part)
                || !matches!(part.stage, Stage::Walk)
                || token < WALK_TOKEN
                || token - WALK_TOKEN >= self.next_token
                || physical_walkers.remove(&token) != Some(u128::from(part.amount))
                || walking_leg(transit_path(cohorts, part.cohort), part.leg).is_empty()
            {
                return Err(error());
            }
            *active
                .entry((part.cohort, JourneyMode::Transit))
                .or_default() += u128::from(part.amount);
        }
        if !physical_walkers.is_empty() || self.next_token >= WALK_TOKEN {
            return Err(error());
        }
        for part in &self.pending {
            if !check_part(part) {
                return Err(error());
            }
            *active
                .entry((part.cohort, JourneyMode::Transit))
                .or_default() += u128::from(part.amount);
        }
        for (id, leg, line, board, alight) in service.passenger_rides() {
            let Some(JourneyPath::Transit(path)) = cohorts.get(&id).and_then(|c| c.path.as_ref())
            else {
                return Err(error());
            };
            if path
                .rides
                .get(leg)
                .is_none_or(|ride| (ride.line, ride.board, ride.alight) != (line, board, alight))
            {
                return Err(error());
            }
        }
        for (id, _, amount) in service.passenger_cohorts() {
            *active.entry((id, JourneyMode::Transit)).or_default() += u128::from(amount);
        }
        Ok(())
    }
}

impl DemandState {
    pub(super) fn walking_arrival(&mut self, id: u64, amount: u32, tick: Tick) {
        if id < WALK_TOKEN {
            self.arrive(id, u64::from(amount), tick);
            return;
        }
        let part = self
            .state
            .transit
            .walking
            .get_mut(&id)
            .expect("transfer token is active");
        let cohort = part.cohort;
        let leg = part.leg;
        part.amount -= amount;
        if part.amount == 0 {
            self.state.transit.walking.remove(&id);
        }
        self.finish_walk(cohort, leg, amount, tick);
    }

    fn finish_walk(&mut self, cohort: u64, leg: usize, amount: u32, tick: Tick) {
        let rides = transit_path(&self.state.cohorts, cohort).rides.len();
        if leg == rides {
            self.arrive(cohort, u64::from(amount), tick);
        } else {
            self.state.transit.pending.push_back(Part {
                cohort,
                leg,
                amount,
                since: tick,
                stage: Stage::Board,
            });
        }
    }

    pub(crate) fn alighting(&mut self, events: Vec<Alighting>, tick: Tick) {
        for event in events {
            let part = Part {
                cohort: event.cohort,
                leg: event.leg + 1,
                amount: event.count,
                since: tick,
                stage: Stage::Walk,
            };
            self.begin_walk(part, tick);
        }
    }

    pub(crate) fn move_transit(&mut self, tick: Tick, service: &mut Service) {
        // Pass 1 prepares walks and releases groups that already reached their stop.
        // Pass 2 admits them in arrival order before the vehicles process the tick.
        let mut boarding = Vec::new();
        for part in std::mem::take(&mut self.state.transit.pending) {
            match part.stage {
                Stage::Walk => self.begin_walk(part, tick),
                Stage::Board => boarding.push(part),
            }
        }
        self.state.transit.pending.retain(|part| {
            if matches!(part.stage, Stage::Board) {
                boarding.push(part.clone());
                false
            } else {
                true
            }
        });
        boarding.sort_by_key(|part| part.since);
        for mut part in boarding {
            let ride = &transit_path(&self.state.cohorts, part.cohort).rides[part.leg];
            let admitted = service.enter_passengers(PassengerRequest {
                cohort: part.cohort,
                leg: part.leg,
                line: ride.line,
                board: ride.board,
                alight: ride.alight,
                since: part.since,
                count: part.amount,
            });
            part.amount -= admitted;
            if part.amount != 0 {
                self.state.transit.pending.push_back(part);
            }
        }
    }

    fn begin_walk(&mut self, mut part: Part, tick: Tick) {
        let path = transit_path(&self.state.cohorts, part.cohort);
        let walk = walking_leg(path, part.leg);
        if walk.is_empty() {
            self.finish_walk(part.cohort, part.leg, part.amount, part.since);
            return;
        }
        let serial = self.state.transit.next_token;
        let id = WALK_TOKEN | serial;
        let accepted = self.walking.enter(walk, part.amount, tick, id);
        if accepted != 0 {
            self.state.transit.next_token += 1;
            self.state.transit.walking.insert(
                id,
                Part {
                    amount: accepted,
                    ..part.clone()
                },
            );
            part.amount -= accepted;
        }
        if part.amount != 0 {
            self.state.transit.pending.push_back(part);
        }
    }
}

fn transit_path(cohorts: &BTreeMap<u64, Cohort>, cohort: u64) -> &TransitPath {
    let Some(JourneyPath::Transit(path)) = cohorts[&cohort].path.as_ref() else {
        unreachable!("transfer requires a transit path");
    };
    path
}

fn walking_leg(path: &TransitPath, next: usize) -> &[crate::FootEdgeId] {
    if next == 0 {
        &path.access
    } else if next == path.rides.len() {
        &path.egress
    } else {
        &path.rides[next - 1].transfer_after
    }
}
