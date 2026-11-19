use crate::taxi::TaxiObservation;
use crate::{
    BuildingId, DemandObservation, JourneyOption, LaneId, LaneObservation, ServiceObservation,
    Tick, TimetableObservation, ZoneId,
};
use crate::{FootEdgeId, FootEdgeSpec, LayoutSpec};
use std::fmt;

/// Read-only requests against the last completed boundary.
#[non_exhaustive]
pub enum Query {
    Clock,
    Demand,
    Service,
    Taxi,
    TimetableJourney { from: ZoneId, to: ZoneId },
    Journeys { from: ZoneId, to: ZoneId },
    Building(BuildingId),
    Lane(LaneId),
    FootEdge(FootEdgeId),
    Layout,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Observation {
    Clock(ClockObservation),
    Demand(DemandObservation),
    Service(ServiceObservation),
    Taxi(TaxiObservation),
    TimetableJourney(Option<TimetableObservation>),
    Journeys(Vec<JourneyOption>),
    Building(BuildingId),
    Lane(LaneObservation),
    FootEdge(FootEdgeSpec),
    Layout(LayoutSpec),
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
    UnknownLane(LaneId),
    UnknownFootEdge(FootEdgeId),
    UnknownZone(ZoneId),
}

impl fmt::Display for QueryError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFootEdge(id) => write!(out, "unknown foot edge ID {}", id.get()),
            Self::UnknownZone(id) => write!(out, "unknown zone ID {}", id.get()),
            Self::UnknownLane(id) => write!(out, "unknown lane ID {}", id.get()),
            Self::UnknownBuilding(id) => write!(out, "unknown building ID {}", id.get()),
        }
    }
}

impl std::error::Error for QueryError {}
