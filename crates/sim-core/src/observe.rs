use crate::{BuildingId, Tick};
use std::fmt;

/// Read-only requests against the last completed boundary.
#[non_exhaustive]
pub enum Query {
    Clock,
    Building(BuildingId),
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Observation {
    Clock(ClockObservation),
    Building(BuildingId),
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ClockObservation {
    pub traffic: Tick,
    pub development_ticks: u64,
    pub development_period: u64,
    pub paused: bool,
    pub growth_paused: bool,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QueryError {
    UnknownBuilding(BuildingId),
}

impl fmt::Display for QueryError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownBuilding(id) => write!(out, "unknown building ID {}", id.get()),
        }
    }
}

impl std::error::Error for QueryError {}
