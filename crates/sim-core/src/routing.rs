pub(crate) mod transit;

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::traffic::RoadState;
use crate::{
    EditCommand, EditError, FootEdgeId, LaneId, LineId, MovementId, Simulation, VehicleKind,
};

impl Simulation {
    /// Returns the fastest free-flow route for `kind`, including both endpoint lanes.
    /// A route within a single lane contains that lane once.
    /// Returns `None` if no compatible route exists, including when either lane is unknown.
    pub fn preview_vehicle_route(
        &self,
        from: LaneId,
        to: LaneId,
        kind: VehicleKind,
    ) -> Option<Vec<LaneId>> {
        self.world.road.route_lanes(kind, from, to)
    }

    /// Routes through the specified lanes in order for a vehicle of the given length.
    /// An empty list or an unreachable section returns `None`.
    pub fn preview_vehicle_route_through(
        &self,
        lanes: &[LaneId],
        kind: VehicleKind,
        length_mm: NonZeroU32,
    ) -> Option<Vec<LaneId>> {
        let (&from, rest) = lanes.split_first()?;
        let (to, waypoints) = rest.split_last().unwrap_or((&from, &[]));
        let road = &self.world.road;
        let (_, movements) = vehicle_route_through(road, from, waypoints, *to, kind, length_mm)?;
        Some(road.lanes_for_movements(from, &movements))
    }

    /// Validates an edit and returns the resulting route of a connected line.
    /// Each leg includes its starting and ending lane. The last leg returns to the origin.
    pub fn preview_line_route(
        &self,
        edit: &EditCommand,
        line: LineId,
    ) -> Result<Option<Vec<Vec<LaneId>>>, EditError> {
        let prepared = crate::edit::prepare(self, edit)?;
        Ok(prepared
            .world
            .service
            .line_route(line, &prepared.world.road))
    }
}

pub(crate) fn vehicle_route_through(
    road: &RoadState,
    mut from: LaneId,
    waypoints: &[LaneId],
    to: LaneId,
    kind: VehicleKind,
    length_mm: NonZeroU32,
) -> Option<(u64, Vec<MovementId>)> {
    if waypoints.is_empty() {
        return road.route_for_length(kind, from, to, length_mm);
    }
    let mut ticks = u64::from(road.lane_travel_ticks(from)?);
    let mut route = Vec::new();
    for target in waypoints.iter().copied().chain(std::iter::once(to)) {
        let (leg_ticks, movements) = road.route_for_length(kind, from, target, length_mm)?;
        let overlap = u64::from(road.lane_travel_ticks(from)?);
        ticks = ticks.checked_add(leg_ticks - overlap)?;
        route.extend(movements);
        from = target;
    }
    Some((ticks, route))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JourneyMode {
    Walk,
    Car,
    Transit,
    Taxi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct JourneyOption {
    pub mode: JourneyMode,
    pub access_ticks: u64,
    pub wait_ticks: u64,
    pub ride_ticks: u64,
    pub transfer_ticks: u64,
    pub weight: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum JourneyPath {
    Walk(Vec<FootEdgeId>),
    Transit(transit::TransitPath),
    Taxi,
    CarWaiting,
    Car {
        from: LaneId,
        movements: Vec<MovementId>,
    },
}

impl JourneyPath {
    pub(crate) fn mode(&self) -> JourneyMode {
        match self {
            Self::Taxi => JourneyMode::Taxi,
            Self::Transit(_) => JourneyMode::Transit,
            Self::Walk(_) => JourneyMode::Walk,
            Self::CarWaiting | Self::Car { .. } => JourneyMode::Car,
        }
    }
}

// Largest remainder with input order breaking ties.
pub(crate) fn distribute(amount: u64, weights: &[u64]) -> Vec<u64> {
    let denominator: u128 = weights.iter().map(|&weight| u128::from(weight)).sum();
    if denominator == 0 {
        return vec![0; weights.len()];
    }
    let mut remainders = Vec::with_capacity(weights.len());
    let mut counts = Vec::with_capacity(weights.len());
    let mut assigned = 0;
    for (index, &weight) in weights.iter().enumerate() {
        let product = u128::from(amount) * u128::from(weight);
        let count = (product / denominator) as u64;
        counts.push(count);
        assigned += count;
        remainders.push((std::cmp::Reverse(product % denominator), index));
    }
    remainders.sort_unstable();
    for (_, index) in remainders.into_iter().take((amount - assigned) as usize) {
        counts[index] += 1;
    }
    counts
}

pub(crate) fn cost_weight(base: u32, ticks: u64) -> u64 {
    u64::from(base) * (1_000_000 / ticks.saturating_add(1)).max(1)
}
