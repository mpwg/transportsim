use std::{fmt, num::NonZeroU32};

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);
        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

id!(LaneId);
id!(MovementId);
id!(JunctionId);
id!(VehicleId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VehicleKind {
    Car,
    Bus,
    Taxi,
    Tram,
}

#[derive(Clone, Copy, Debug)]
pub struct DischargeRate {
    pub millimetres: NonZeroU32,
    pub ticks: NonZeroU32,
}

/// The capacity includes the lengths and standing gaps of queued vehicles.
#[derive(Clone, Debug)]
pub struct LaneSpec {
    pub id: LaneId,
    pub capacity_mm: NonZeroU32,
    pub travel_ticks: NonZeroU32,
    pub discharge: DischargeRate,
    pub allowed: Vec<VehicleKind>,
}

/// Conflict zones are local to a junction. Greater priority is served first.
#[derive(Clone, Debug)]
pub struct MovementSpec {
    pub id: MovementId,
    pub from: LaneId,
    pub to: LaneId,
    pub junction: JunctionId,
    pub conflict_zones: Vec<u32>,
    pub priority: u16,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RoadValidationError {
    DuplicateLane(LaneId),
    DuplicateJunction(JunctionId),
    DuplicateMovement(MovementId),
    DuplicateVehicle(VehicleId),
    UnknownLane(LaneId),
    UnknownJunction(JunctionId),
    InvalidMovement(MovementId),
    InvalidSignal {
        junction: JunctionId,
        phase: usize,
        movement: MovementId,
    },
    InvalidRoute {
        lane: LaneId,
        movement: MovementId,
    },
    VehicleNotAllowed {
        lane: LaneId,
        kind: VehicleKind,
    },
    OverCapacity(LaneId),
}

impl fmt::Display for RoadValidationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateLane(id) => write!(out, "duplicate lane {}", id.get()),
            Self::DuplicateJunction(id) => write!(out, "duplicate junction {}", id.get()),
            Self::DuplicateMovement(id) => write!(out, "duplicate movement {}", id.get()),
            Self::DuplicateVehicle(id) => write!(out, "duplicate vehicle {}", id.get()),
            Self::UnknownLane(id) => write!(out, "unknown lane {}", id.get()),
            Self::UnknownJunction(id) => write!(out, "unknown junction {}", id.get()),
            Self::InvalidMovement(id) => write!(out, "invalid movement {}", id.get()),
            Self::InvalidSignal {
                junction,
                phase,
                movement,
            } => write!(
                out,
                "junction {} phase {phase}: invalid or conflicting movement {}",
                junction.get(),
                movement.get()
            ),
            Self::InvalidRoute { lane, movement } => write!(
                out,
                "lane {}: route cannot use movement {}",
                lane.get(),
                movement.get()
            ),
            Self::VehicleNotAllowed { lane, kind } => {
                write!(out, "lane {} does not allow {kind:?}", lane.get())
            }
            Self::OverCapacity(id) => write!(out, "lane {} exceeds capacity", id.get()),
        }
    }
}

impl std::error::Error for RoadValidationError {}
