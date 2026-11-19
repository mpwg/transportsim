/// A boundary on the traffic clock, in simulated seconds.
#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[serde(transparent)]
pub struct Tick(u64);

impl Tick {
    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }

    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// A stable identity for a building.
#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[serde(transparent)]
pub struct BuildingId(u64);

impl BuildingId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}
