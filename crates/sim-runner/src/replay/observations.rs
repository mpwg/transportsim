use anyhow::{Result, bail};
use serde::Serialize;
use sim_core::{
    DemandObservation, GrowthProject, GrowthSiteObservation, GrowthTotals, LineId, LineObservation,
    Observation, Query, ServiceVehicleObservation, Simulation, StopId, StopObservation,
    TaxiAssignment, TaxiStatus, TaxiVehicleObservation, Tick, VehicleId, VehicleKind, ZoneId,
};

#[derive(Serialize)]
pub(super) struct Measurements {
    demand: Demand,
    service: Service,
    taxi: Taxi,
    growth: Growth,
}

impl Measurements {
    pub(super) fn inspect(sim: &Simulation) -> Result<Self> {
        let Observation::Demand(demand) = sim.inspect(Query::Demand)? else {
            bail!("expected a demand observation");
        };
        let Observation::Service(service) = sim.inspect(Query::Service)? else {
            bail!("expected a service observation");
        };
        let Observation::Taxi(taxi) = sim.inspect(Query::Taxi)? else {
            bail!("expected a taxi observation");
        };
        let Observation::Growth(growth) = sim.inspect(Query::Growth)? else {
            bail!("expected a growth observation");
        };
        Ok(Self {
            demand: demand.into(),
            service: Service {
                lines: service.lines.into_iter().map(Line).collect(),
                vehicles: service.vehicles.into_iter().map(LineVehicle).collect(),
                stops: service.stops.into_iter().map(Stop).collect(),
            },
            taxi: Taxi {
                waiting_total: taxi.waiting_total,
                unassigned_total: taxi.unassigned_total,
                oldest_waiting_since: taxi.oldest_waiting_since,
                vehicles: taxi.vehicles.into_iter().map(TaxiVehicle::from).collect(),
            },
            growth: Growth {
                totals: growth.totals,
                sites: growth.sites,
                projects: growth.projects,
            },
        })
    }
}

#[derive(Serialize)]
struct Demand {
    generated: u64,
    waiting: u64,
    in_transit: u64,
    arrived: u64,
    unassigned: u64,
    pending_returns: u64,
    transit_transferring: u64,
    scheduled_departures: u64,
    // The value can exceed the exact integer range of JSON consumers.
    total_travel_ticks: String,
    active_cohorts: usize,
}

impl From<DemandObservation> for Demand {
    fn from(demand: DemandObservation) -> Self {
        Self {
            generated: demand.generated,
            waiting: demand.waiting,
            in_transit: demand.in_transit,
            arrived: demand.arrived,
            unassigned: demand.unassigned,
            pending_returns: demand.pending_returns,
            transit_transferring: demand.transit_transferring,
            scheduled_departures: demand.scheduled_departures,
            total_travel_ticks: demand.total_travel_ticks.to_string(),
            active_cohorts: demand.active.len(),
        }
    }
}

#[derive(Serialize)]
struct Service {
    lines: Vec<Line>,
    vehicles: Vec<LineVehicle>,
    stops: Vec<Stop>,
}

#[derive(Serialize)]
#[serde(transparent)]
struct Line(#[serde(with = "LineFields")] LineObservation);

#[derive(Serialize)]
#[serde(remote = "LineObservation")]
struct LineFields {
    id: LineId,
    disconnected: bool,
    headway: u32,
    pending: Option<Tick>,
    departed: u64,
    cancelled: u64,
    last_departure: Option<Tick>,
    last_delay: Option<u64>,
}

#[derive(Serialize)]
#[serde(transparent)]
struct LineVehicle(#[serde(with = "LineVehicleFields")] ServiceVehicleObservation);

#[derive(Serialize)]
#[serde(remote = "ServiceVehicleObservation")]
struct LineVehicleFields {
    id: VehicleId,
    line: LineId,
    disconnected: bool,
    kind: VehicleKind,
    capacity: u32,
    onboard: u32,
    planned_departure: Option<Tick>,
    next_stop: Option<StopId>,
    dwell_until: Option<Tick>,
}

#[derive(Serialize)]
#[serde(transparent)]
struct Stop(#[serde(with = "StopFields")] StopObservation);

#[derive(Serialize)]
#[serde(remote = "StopObservation")]
struct StopFields {
    id: StopId,
    waiting_total: u32,
    oldest_waiting_since: Option<Tick>,
}

#[derive(Serialize)]
struct Taxi {
    waiting_total: u64,
    unassigned_total: u64,
    oldest_waiting_since: Option<Tick>,
    vehicles: Vec<TaxiVehicle>,
}

#[derive(Serialize)]
struct TaxiVehicle {
    id: VehicleId,
    capacity: u32,
    disconnected: bool,
    #[serde(flatten)]
    progress: TaxiProgress,
}

impl From<TaxiVehicleObservation> for TaxiVehicle {
    fn from(vehicle: TaxiVehicleObservation) -> Self {
        let progress = match vehicle.status {
            TaxiStatus::Idle { zone } => TaxiProgress::Idle { zone },
            TaxiStatus::ToPickup { assignment } => TaxiProgress::ToPickup { assignment },
            TaxiStatus::Occupied { assignment } => TaxiProgress::Occupied { assignment },
            _ => TaxiProgress::Unknown,
        };
        Self {
            id: vehicle.id,
            capacity: vehicle.capacity,
            disconnected: vehicle.disconnected,
            progress,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum TaxiProgress {
    Idle {
        zone: ZoneId,
    },
    ToPickup {
        #[serde(with = "TaxiAssignmentFields")]
        assignment: TaxiAssignment,
    },
    Occupied {
        #[serde(with = "TaxiAssignmentFields")]
        assignment: TaxiAssignment,
    },
    Unknown,
}

#[derive(Serialize)]
#[serde(remote = "TaxiAssignment")]
struct TaxiAssignmentFields {
    origin: ZoneId,
    destination: ZoneId,
    passengers: u32,
}

#[derive(Serialize)]
struct Growth {
    #[serde(with = "GrowthTotalsFields")]
    totals: GrowthTotals,
    sites: Vec<GrowthSiteObservation>,
    projects: Vec<GrowthProject>,
}

#[derive(Serialize)]
#[serde(remote = "GrowthTotals")]
struct GrowthTotalsFields {
    residents: u32,
    jobs: u32,
    shops: u32,
}
