use crate::{BuildingId, Cell, JunctionSpec, LaneId, LaneSpec, MovementSpec, StreetSite, ZoneId};
use serde::{Deserialize, Serialize};
use std::{fmt, num::NonZeroU32};

/// Parameters for the candidate assessment and construction cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GrowthRules {
    /// Full development periods elapsed between approval and completion.
    pub build_periods: NonZeroU32,
    /// Maximum number of new projects approved per development period.
    pub projects_per_period: NonZeroU32,
    pub max_travel_ticks: NonZeroU32,
    pub minimum_accessibility: NonZeroU32,
}

impl Default for GrowthRules {
    fn default() -> Self {
        Self {
            build_periods: NonZeroU32::new(3).unwrap(),
            projects_per_period: NonZeroU32::MIN,
            max_travel_ticks: NonZeroU32::new(3_600).unwrap(),
            minimum_accessibility: NonZeroU32::MIN,
        }
    }
}

/// Configured sites and development rules for one scenario.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GrowthSpec {
    pub rules: GrowthRules,
    pub sites: Vec<GrowthSite>,
}

impl GrowthSpec {
    pub(crate) fn is_empty(&self) -> bool {
        self.sites.is_empty() && self.rules == GrowthRules::default()
    }

    pub(crate) fn sort_tables(&mut self) {
        self.sites.sort_unstable_by_key(|site| site.building);
        for road in self
            .sites
            .iter_mut()
            .filter_map(|site| site.local_road.as_mut())
        {
            road.movements.sort_unstable_by_key(|movement| movement.id);
            road.junctions.sort_unstable_by_key(|junction| junction.id);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrowthUse {
    Housing,
    Jobs,
    Shops,
}

/// A parcel that can be developed or an existing building that can be extended.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GrowthSite {
    pub building: BuildingId,
    pub zone: ZoneId,
    pub cell: Cell,
    pub access: Option<LaneId>,
    pub usage: GrowthUse,
    /// Maximum capacity added by one construction project at this site.
    pub amount: NonZeroU32,
    pub max_tiers: NonZeroU32,
    /// Tiers already completed in the starting scenario.
    #[serde(default)]
    pub initial_tiers: u32,
    /// Maximum extra development periods drawn when a project is approved.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub additional_build_periods: u32,
    pub local_road: Option<LocalRoad>,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// The optional street built when a parcel is developed for the first time.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalRoad {
    pub lane: LaneSpec,
    pub street: StreetSite,
    #[serde(default)]
    pub movements: Vec<MovementSpec>,
    #[serde(default)]
    pub junctions: Vec<JunctionSpec>,
    pub connects_zone: ZoneId,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GrowthError {
    DuplicateSite(BuildingId),
    InvalidInitialTiers(BuildingId),
    BuildDurationOverflow(BuildingId),
    InvalidLocalRoad(BuildingId),
    InvalidSite(BuildingId),
    PopulationLimit,
}

impl fmt::Display for GrowthError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSite(id) => {
                write!(out, "duplicate growth site for building {}", id.get())
            }
            Self::InvalidInitialTiers(id) => write!(
                out,
                "growth site for building {} exceeds its tier limit",
                id.get()
            ),
            Self::BuildDurationOverflow(id) => {
                write!(
                    out,
                    "construction duration overflows at growth site {}",
                    id.get()
                )
            }
            Self::InvalidLocalRoad(id) => {
                write!(out, "invalid local road for growth site {}", id.get())
            }
            Self::InvalidSite(id) => write!(out, "invalid growth site for building {}", id.get()),
            Self::PopulationLimit => {
                out.write_str("population exceeds the growth limit of 10,000 residents")
            }
        }
    }
}

impl std::error::Error for GrowthError {}
