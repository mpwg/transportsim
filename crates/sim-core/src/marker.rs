use serde::{Deserialize, Serialize};

use crate::{BuildingId, Cell, LaneId, StopId};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(transparent)]
pub struct MarkerId(u64);

impl MarkerId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MarkerAnchor {
    Building(BuildingId),
    Lane(LaneId),
    Stop(StopId),
}

/// A persistent map location with an optional reference to a simulation object.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub id: MarkerId,
    pub cell: Cell,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<MarkerAnchor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MarkerObservation {
    pub marker: Marker,
    pub anchor_present: bool,
}
