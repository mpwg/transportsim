use crate::taxi::{Taxi, TaxiError, TaxiSpec};
use crate::{
    BuildingId, JunctionSpec, LaneSpec, MovementSpec, RoadValidationError, Tick, TrafficSeed,
};
use crate::{DemandError, DemandSpec, demand::DemandState, traffic::RoadState};
use crate::{
    GrowthError, GrowthSpec,
    growth::{Growth, MAX_POPULATION},
    growth_runtime,
};
use crate::{LaneId, LineId, QueueEntry, ServiceError, ServiceSpec, service::Service};
use crate::{LayoutError, LayoutSpec};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU64,
};

/// Initial conditions supplied by a scenario author.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Scenario {
    pub start: Tick,
    pub building_ids: Vec<BuildingId>,
    pub lanes: Vec<LaneSpec>,
    pub movements: Vec<MovementSpec>,
    pub junctions: Vec<JunctionSpec>,
    pub traffic: Vec<TrafficSeed>,
    pub demand: DemandSpec,
    pub service: ServiceSpec,
    /// Required lane waypoints by line and leg, with the return trip last.
    pub line_waypoints: BTreeMap<LineId, Vec<Vec<LaneId>>>,
    pub taxi: TaxiSpec,
    pub layout: LayoutSpec,
    pub growth: GrowthSpec,
}

impl Scenario {
    pub fn empty(start: Tick) -> Self {
        Self {
            start,
            building_ids: Vec::new(),
            lanes: Vec::new(),
            movements: Vec::new(),
            junctions: Vec::new(),
            traffic: Vec::new(),
            demand: DemandSpec::default(),
            service: ServiceSpec::default(),
            line_waypoints: BTreeMap::new(),
            taxi: TaxiSpec::default(),
            layout: LayoutSpec::default(),
            growth: GrowthSpec::default(),
        }
    }
}

/// Effective simulation rules.
#[derive(Clone, Copy, Debug)]
pub struct RuleSet {
    development_period_ticks: NonZeroU64,
}

impl RuleSet {
    pub const fn new(development_period_ticks: NonZeroU64) -> Self {
        Self {
            development_period_ticks,
        }
    }

    pub(crate) const fn development_period_ticks(self) -> u64 {
        self.development_period_ticks.get()
    }
}

impl Default for RuleSet {
    fn default() -> Self {
        Self::new(NonZeroU64::new(86_400).unwrap())
    }
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ValidationError {
    DuplicateBuildingId { id: BuildingId, index: usize },
    Road(RoadValidationError),
    Demand(DemandError),
    Service(ServiceError),
    Taxi(TaxiError),
    Layout(LayoutError),
    Growth(GrowthError),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Road(error) => error.fmt(out),
            Self::Demand(error) => error.fmt(out),
            Self::Service(error) => error.fmt(out),
            Self::Taxi(error) => error.fmt(out),
            Self::Layout(error) => error.fmt(out),
            Self::Growth(error) => error.fmt(out),
            Self::DuplicateBuildingId { id, index } => {
                write!(
                    out,
                    "building_ids[{index}]: duplicate building ID {}",
                    id.get()
                )
            }
        }
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug)]
pub(crate) struct World {
    pub(crate) buildings: BTreeSet<BuildingId>,
    pub(crate) road: RoadState,
    pub(crate) demand: DemandState,
    pub(crate) service: Service,
    pub(crate) taxi: Taxi,
    pub(crate) growth: Growth,
}

impl World {
    pub(crate) fn from_scenario(
        scenario: Scenario,
        allowed_disconnected: &BTreeSet<LineId>,
    ) -> Result<Self, ValidationError> {
        let mut buildings = BTreeSet::new();
        for (index, id) in scenario.building_ids.iter().copied().enumerate() {
            if !buildings.insert(id) {
                return Err(ValidationError::DuplicateBuildingId { id, index });
            }
        }
        let seeded_vehicles: BTreeSet<_> = scenario
            .traffic
            .iter()
            .filter_map(|seed| match seed.entry {
                QueueEntry::Vehicle { id, .. } => Some(id),
                QueueEntry::PrivateCars(_) => None,
            })
            .collect();
        let lanes: BTreeSet<_> = scenario.lanes.iter().map(|lane| lane.id).collect();
        let zones: BTreeSet<_> = scenario.demand.zones.iter().map(|zone| zone.id).collect();
        let feet: BTreeSet<_> = scenario
            .demand
            .foot_edges
            .iter()
            .map(|edge| edge.id)
            .collect();
        scenario
            .layout
            .validate(|id| lanes.contains(&id), |id| zones.contains(&id))
            .map_err(ValidationError::Layout)?;
        scenario
            .layout
            .validate_connections(&scenario.movements, |id| feet.contains(&id))
            .map_err(ValidationError::Layout)?;
        buildings.extend(scenario.layout.buildings.iter().map(|building| building.id));
        growth_runtime::validate_sites(&scenario).map_err(ValidationError::Growth)?;
        let road = RoadState::new(
            scenario.lanes,
            scenario.movements,
            scenario.junctions,
            scenario.traffic,
        )
        .map_err(ValidationError::Road)?;
        let demand = DemandState::new(scenario.demand, &road).map_err(ValidationError::Demand)?;
        let line_vehicles: BTreeSet<_> = scenario
            .service
            .fleet
            .iter()
            .map(|vehicle| vehicle.id)
            .collect();
        let service = Service::new_with_disconnections(
            scenario.service,
            &road,
            |id| demand.contains_zone(id),
            |id| seeded_vehicles.contains(&id),
            allowed_disconnected,
            scenario.line_waypoints,
        )
        .map_err(ValidationError::Service)?;
        let taxi = Taxi::new(scenario.taxi, demand.road_accesses(), &road, |id| {
            seeded_vehicles.contains(&id) || line_vehicles.contains(&id)
        })
        .map_err(ValidationError::Taxi)?;
        let growth = Growth::new(scenario.growth).map_err(ValidationError::Growth)?;
        if demand.totals().residents > MAX_POPULATION {
            return Err(ValidationError::Growth(GrowthError::PopulationLimit));
        }
        Ok(Self {
            buildings,
            road,
            demand,
            service,
            taxi,
            growth,
        })
    }
}
