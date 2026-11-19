//! Deterministic, headless traffic simulation.

mod demand;
mod ids;
mod junction;
mod network;
mod observe;
mod routing;
mod save;
mod service;
mod simulation;
mod traffic;
mod walking;
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

pub use demand::{
    ActiveJourney, DemandError, DemandObservation, DemandProfile, DemandSpec, TripPurpose, ZoneId,
    ZoneSpec,
};
pub use routing::{JourneyMode, JourneyOption, transit::TransitCost};
pub use walking::{FootEdgeId, FootEdgeSpec, WalkingError};

pub use service::{
    FleetSpec, LineId, LineObservation, LineSpec, ServiceError, ServiceObservation, ServiceSpec,
    ServiceVehicleObservation, StopId, StopSpec, TimetableObservation,
};
