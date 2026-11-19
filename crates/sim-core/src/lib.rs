//! Deterministic, headless traffic simulation.

mod ids;
mod observe;
mod simulation;
mod world;

pub use ids::{BuildingId, Tick};
pub use observe::{ClockObservation, Observation, Query, QueryError};
pub use simulation::{
    AdvanceError, AdvanceReport, Command, CommandError, CommandReceipt, Simulation,
};
pub use world::{RuleSet, Scenario, ValidationError};
