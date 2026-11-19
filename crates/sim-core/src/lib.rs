//! Deterministic, headless traffic simulation.

mod ids;
mod junction;
mod network;
mod observe;
mod save;
mod simulation;
mod traffic;
mod world;

pub use ids::{BuildingId, Tick};
pub use observe::{ClockObservation, Observation, Query, QueryError};
pub use save::{LoadError, SaveError};
pub use simulation::{
    AdvanceError, AdvanceReport, Command, CommandError, CommandReceipt, Simulation,
};
pub use world::{RuleSet, Scenario, ValidationError};

pub use junction::{JunctionSpec, SignalPhase};
pub use network::{
    DischargeRate, JunctionId, LaneId, LaneSpec, MovementId, MovementSpec, RoadValidationError,
    VehicleId, VehicleKind,
};
pub use traffic::{LaneDeparture, LaneObservation, QueueEntry, TrafficSeed, WaitReason};
