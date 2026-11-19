pub(crate) mod transit;

use serde::{Deserialize, Serialize};

use crate::{FootEdgeId, LaneId, MovementId};

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
