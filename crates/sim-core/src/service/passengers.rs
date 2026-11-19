use super::{Line, Progress, StopObservation};
use crate::{FleetSpec, LineId, LineSpec, LoadError, StopId, StopSpec, Tick, VehicleId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassengerRequest {
    pub(crate) cohort: u64,
    pub(crate) leg: usize,
    pub(crate) line: LineId,
    pub(crate) board: StopId,
    pub(crate) alight: StopId,
    pub(crate) since: Tick,
    pub(crate) count: u32,
}

#[derive(Debug)]
pub(crate) struct Alighting {
    pub(crate) cohort: u64,
    pub(crate) leg: usize,
    pub(crate) count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OnboardPassengers {
    cohort: u64,
    leg: usize,
    board_index: usize,
    alight_index: usize,
    count: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PassengerState {
    waiting: BTreeMap<StopId, Vec<PassengerRequest>>,
    onboard: BTreeMap<VehicleId, Vec<OnboardPassengers>>,
}

pub(super) fn serves(lines: &BTreeMap<LineId, Line>, request: &PassengerRequest) -> bool {
    lines.get(&request.line).is_some_and(|line| {
        line.spec
            .stops
            .iter()
            .skip_while(|&&id| id != request.board)
            .skip(1)
            .any(|&id| id == request.alight)
    })
}

impl PassengerState {
    pub(super) fn is_empty(&self) -> bool {
        self.waiting.is_empty() && self.onboard.is_empty()
    }

    pub(super) fn enter(&mut self, request: PassengerRequest, capacity: u32) -> u32 {
        let queued = self
            .waiting
            .get(&request.board)
            .into_iter()
            .flatten()
            .map(|group| group.count)
            .sum::<u32>();
        let count = request.count.min(capacity - queued);
        if count != 0 {
            let queue = self.waiting.entry(request.board).or_default();
            let position = queue.partition_point(|other| other.since <= request.since);
            queue.insert(position, PassengerRequest { count, ..request });
        }
        count
    }

    pub(super) fn alight(
        &mut self,
        vehicle: VehicleId,
        stop_index: usize,
        completed: &mut Vec<Alighting>,
    ) -> u32 {
        let mut alighted = 0;
        if let Some(groups) = self.onboard.get_mut(&vehicle) {
            groups.retain(|group| {
                if group.alight_index != stop_index {
                    return true;
                }
                alighted += group.count;
                completed.push(Alighting {
                    cohort: group.cohort,
                    leg: group.leg,
                    count: group.count,
                });
                false
            });
            if groups.is_empty() {
                self.onboard.remove(&vehicle);
            }
        }
        alighted
    }

    pub(super) fn board(
        &mut self,
        vehicle: &FleetSpec,
        line: &LineSpec,
        stop_index: usize,
        tick: Tick,
    ) -> u32 {
        let stop = line.stops[stop_index];
        let available = vehicle.capacity.get() - self.onboard_total(vehicle.id);
        let Some(queue) = self.waiting.get_mut(&stop) else {
            return 0;
        };
        let mut boarded = 0;
        queue.retain_mut(|group| {
            if boarded == available || group.line != line.id || group.since > tick {
                return true;
            }
            let following_stops = &line.stops[stop_index + 1..];
            let Some(offset) = following_stops.iter().position(|&id| id == group.alight) else {
                return true;
            };
            let count = group.count.min(available - boarded);
            self.onboard
                .entry(vehicle.id)
                .or_default()
                .push(OnboardPassengers {
                    cohort: group.cohort,
                    leg: group.leg,
                    board_index: stop_index,
                    alight_index: stop_index + 1 + offset,
                    count,
                });
            boarded += count;
            group.count -= count;
            group.count != 0
        });
        if queue.is_empty() {
            self.waiting.remove(&stop);
        }
        boarded
    }

    pub(super) fn observe_stop(&self, id: StopId) -> StopObservation {
        let groups = self.waiting.get(&id).map_or(&[][..], Vec::as_slice);
        StopObservation {
            id,
            waiting_total: groups.iter().map(|group| group.count).sum(),
            oldest_waiting_since: groups.first().map(|group| group.since),
        }
    }

    pub(super) fn onboard_total(&self, vehicle: VehicleId) -> u32 {
        self.onboard
            .get(&vehicle)
            .into_iter()
            .flatten()
            .map(|group| group.count)
            .sum()
    }

    pub(super) fn cohorts(&self) -> impl Iterator<Item = (u64, usize, u32)> + '_ {
        self.waiting
            .values()
            .flatten()
            .map(|group| (group.cohort, group.leg, group.count))
            .chain(
                self.onboard
                    .values()
                    .flatten()
                    .map(|group| (group.cohort, group.leg, group.count)),
            )
    }

    pub(super) fn rides<'a>(
        &'a self,
        fleet: &'a BTreeMap<VehicleId, FleetSpec>,
        lines: &'a BTreeMap<LineId, Line>,
    ) -> impl Iterator<Item = (u64, usize, LineId, StopId, StopId)> + 'a {
        self.waiting
            .values()
            .flatten()
            .map(|group| {
                (
                    group.cohort,
                    group.leg,
                    group.line,
                    group.board,
                    group.alight,
                )
            })
            .chain(self.onboard.iter().flat_map(move |(id, groups)| {
                let line = &lines[&fleet[id].line].spec;
                groups.iter().map(|group| {
                    (
                        group.cohort,
                        group.leg,
                        line.id,
                        line.stops[group.board_index],
                        line.stops[group.alight_index],
                    )
                })
            }))
    }

    pub(super) fn validate(
        &self,
        stops: &BTreeMap<StopId, StopSpec>,
        lines: &BTreeMap<LineId, Line>,
        fleet: &BTreeMap<VehicleId, FleetSpec>,
        progress: &BTreeMap<VehicleId, Progress>,
        tick: Tick,
    ) -> Result<(), LoadError> {
        for (&id, groups) in &self.waiting {
            let invalid = LoadError::InvalidState {
                field: "service.passengers.waiting",
            };
            let Some(stop) = stops.get(&id) else {
                return Err(invalid);
            };
            if groups.is_empty()
                || !groups.iter().map(|group| group.since).is_sorted()
                || groups.iter().any(|group| {
                    group.board != id
                        || group.count == 0
                        || group.since > tick
                        || !serves(lines, group)
                })
                || !fits(groups.iter().map(|group| group.count), stop.capacity.get())
            {
                return Err(invalid);
            }
        }
        for (id, groups) in &self.onboard {
            let invalid = LoadError::InvalidState {
                field: "service.passengers.onboard",
            };
            let Some(vehicle) = fleet.get(id) else {
                return Err(invalid);
            };
            let line = &lines[&vehicle.line].spec;
            let (target, dwelling) = match progress[id] {
                Progress::Approaching { stop_index, .. } => (stop_index, false),
                Progress::Dwelling { stop_index, .. } => (stop_index, true),
                Progress::Idle | Progress::Returning => return Err(invalid),
            };
            if groups.is_empty()
                || groups.iter().any(|group| {
                    let en_route = if dwelling {
                        group.board_index <= target && group.alight_index > target
                    } else {
                        group.board_index < target && group.alight_index >= target
                    };
                    group.count == 0 || !en_route || group.alight_index >= line.stops.len()
                })
                || !fits(
                    groups.iter().map(|group| group.count),
                    vehicle.capacity.get(),
                )
            {
                return Err(invalid);
            }
        }
        Ok(())
    }
}

fn fits(mut counts: impl Iterator<Item = u32>, capacity: u32) -> bool {
    counts.try_fold(capacity, u32::checked_sub).is_some()
}
