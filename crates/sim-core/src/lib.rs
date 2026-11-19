//! Deterministic, headless traffic simulation.

mod demand;
mod edit;
mod growth;
mod growth_runtime;
mod ids;
mod junction;
mod layout;
mod marker;
mod network;
mod observe;
mod routing;
mod save;
mod service;
mod simulation;
mod taxi;
mod traffic;
mod walking;
mod world;

pub use ids::{BuildingId, Tick};
pub use observe::{ClockObservation, Observation, Query, QueryError, UndoObservation};
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
    ServiceVehicleObservation, StopId, StopObservation, StopSpec, TimetableObservation,
};

pub use taxi::{
    TaxiAssignment, TaxiError, TaxiObservation, TaxiSpec, TaxiStatus, TaxiVehicleObservation,
    TaxiVehicleSpec,
};

pub use edit::{EditAction, EditCommand, EditError, EditPreview, TurnPocket};

pub use layout::{BuildingSite, Cell, LayoutError, LayoutSpec, StreetSite, Terrain, TileSpec};

pub use marker::{Marker, MarkerAnchor, MarkerId, MarkerObservation};

pub use growth::{
    GrowthError, GrowthObservation, GrowthProject, GrowthReason, GrowthRules, GrowthSite,
    GrowthSiteObservation, GrowthSpec, GrowthTotals, GrowthUse, LocalRoad,
};
