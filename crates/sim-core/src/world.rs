use crate::{BuildingId, Tick};
use std::{collections::BTreeSet, fmt, num::NonZeroU64};

/// Initial conditions supplied by a scenario author.
#[derive(Debug)]
#[non_exhaustive]
pub struct Scenario {
    pub start: Tick,
    pub building_ids: Vec<BuildingId>,
}

impl Scenario {
    pub fn empty(start: Tick) -> Self {
        Self {
            start,
            building_ids: Vec::new(),
        }
    }
}

/// Effective simulation rules.
#[derive(Clone, Copy, Debug)]
pub struct RuleSet {
    development_period_ticks: NonZeroU64,
}

impl RuleSet {
    pub const fn new(development_period_ticks: NonZeroU64) -> Self {
        Self {
            development_period_ticks,
        }
    }

    pub(crate) const fn development_period_ticks(self) -> u64 {
        self.development_period_ticks.get()
    }
}

impl Default for RuleSet {
    fn default() -> Self {
        Self::new(NonZeroU64::new(86_400).unwrap())
    }
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ValidationError {
    DuplicateBuildingId { id: BuildingId, index: usize },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateBuildingId { id, index } => {
                write!(
                    out,
                    "building_ids[{index}]: duplicate building ID {}",
                    id.get()
                )
            }
        }
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug)]
pub(crate) struct World {
    pub(crate) buildings: BTreeSet<BuildingId>,
}

impl World {
    pub(crate) fn from_scenario(scenario: Scenario) -> Result<Self, ValidationError> {
        let mut buildings = BTreeSet::new();
        for (index, id) in scenario.building_ids.into_iter().enumerate() {
            if !buildings.insert(id) {
                return Err(ValidationError::DuplicateBuildingId { id, index });
            }
        }
        Ok(Self { buildings })
    }
}
