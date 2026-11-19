use crate::LayoutSpec;
use crate::Marker;
use crate::random::RandomState;
use crate::taxi::{TaxiSnapshot, TaxiSpec};
use crate::{GrowthSpec, growth::GrowthSnapshot, growth_runtime};
use std::{collections::BTreeSet, num::NonZeroU64};

use serde::{Deserialize, Serialize};

use super::LoadError;
use crate::{
    BuildingId, JunctionId, JunctionSpec, LaneDeparture, LaneId, LaneSpec, MovementId,
    MovementSpec, QueueEntry, RuleSet, Scenario, Simulation, Tick, TrafficSeed, WaitReason,
};
use crate::{DemandSpec, ServiceSpec, demand::DemandSnapshot, service::ServiceSnapshot};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScenarioV1 {
    seed: u64,
    start: Tick,
    development_period_ticks: NonZeroU64,
    building_ids: Vec<BuildingId>,
    lanes: Vec<LaneSpec>,
    movements: Vec<MovementSpec>,
    junctions: Vec<JunctionSpec>,
    traffic: Vec<TrafficSeed>,
    #[serde(default, skip_serializing_if = "DemandSpec::is_default")]
    demand: DemandSpec,
    #[serde(default, skip_serializing_if = "ServiceSpec::is_empty")]
    service: ServiceSpec,
    #[serde(default, skip_serializing_if = "TaxiSpec::is_empty")]
    taxi: TaxiSpec,
    #[serde(default, skip_serializing_if = "LayoutSpec::is_empty")]
    layout: LayoutSpec,
    #[serde(default, skip_serializing_if = "GrowthSpec::is_empty")]
    growth: GrowthSpec,
}

impl ScenarioV1 {
    fn from_simulation(sim: &Simulation) -> Self {
        let Scenario {
            start,
            mut building_ids,
            mut lanes,
            mut movements,
            mut junctions,
            mut traffic,
            mut demand,
            mut service,
            mut taxi,
            mut layout,
            mut growth,
        } = sim.scenario.clone();
        building_ids.sort_unstable();
        lanes.sort_unstable_by_key(|lane| lane.id);
        movements.sort_unstable_by_key(|movement| movement.id);
        junctions.sort_unstable_by_key(|junction| junction.id);
        traffic.sort_by_key(|item| item.lane);
        demand.sort_tables();
        service.sort_tables();
        taxi.sort_tables();
        layout.sort_tables();
        growth.sort_tables();
        Self {
            seed: sim.seed,
            start,
            development_period_ticks: NonZeroU64::new(sim.rules.development_period_ticks())
                .expect("validated rule period"),
            building_ids,
            lanes,
            movements,
            junctions,
            traffic,
            demand,
            service,
            taxi,
            layout,
            growth,
        }
    }

    pub(super) fn into_simulation(self) -> Result<Simulation, LoadError> {
        self.into_simulation_with_disconnections(&BTreeSet::new())
    }

    fn into_simulation_with_disconnections(
        self,
        allowed: &BTreeSet<crate::LineId>,
    ) -> Result<Simulation, LoadError> {
        let scenario = Scenario {
            start: self.start,
            building_ids: self.building_ids,
            lanes: self.lanes,
            movements: self.movements,
            junctions: self.junctions,
            traffic: self.traffic,
            demand: self.demand,
            service: self.service,
            taxi: self.taxi,
            layout: self.layout,
            growth: self.growth,
        };
        Simulation::new_with_disconnections(
            scenario,
            RuleSet::new(self.development_period_ticks),
            self.seed,
            allowed,
        )
        .map_err(LoadError::State)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    scenario: ScenarioV1,
    tick: Tick,
    #[serde(default, skip_serializing_if = "is_zero")]
    network_revision: u64,
    development_ticks: u64,
    sequence: u64,
    paused: bool,
    growth_paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    random: Option<RandomState>,
    lanes: Vec<LaneV1>,
    junctions: Vec<JunctionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    demand: Option<DemandSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    service: Option<ServiceSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    taxi: Option<TaxiSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    growth: Option<GrowthSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    markers: Vec<Marker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    undo: Option<Box<Payload>>,
}

impl Payload {
    pub(super) fn from_simulation(sim: &Simulation) -> Self {
        let (lanes, junctions) = sim.world.road.snapshot();
        Self {
            scenario: ScenarioV1::from_simulation(sim),
            tick: sim.traffic,
            network_revision: sim.network_revision,
            development_ticks: sim.development_ticks,
            sequence: sim.sequence,
            paused: sim.paused,
            growth_paused: sim.growth_paused,
            random: sim.random,
            lanes,
            junctions,
            demand: sim.world.demand.snapshot(),
            service: sim.world.service.snapshot(),
            taxi: sim.world.taxi.snapshot(),
            growth: sim.world.growth.snapshot(),
            markers: sim.markers.values().cloned().collect(),
            undo: sim
                .undo
                .as_ref()
                .map(|before| Box::new(Payload::from_simulation(before))),
        }
    }

    pub(super) fn into_simulation(self) -> Result<Simulation, LoadError> {
        let undo = self
            .undo
            .map(|before| {
                let invalid = || LoadError::InvalidState { field: "undo" };
                if before.undo.is_some()
                    || before.tick > self.tick
                    || before.development_ticks > self.development_ticks
                    || before.sequence >= self.sequence
                    || before.network_revision >= self.network_revision
                    || before.scenario.start != self.scenario.start
                    || before.scenario.seed != self.scenario.seed
                    || before.random.is_some() != self.random.is_some()
                    || before.scenario.development_period_ticks
                        != self.scenario.development_period_ticks
                {
                    return Err(invalid());
                }
                before
                    .into_simulation()
                    .map(Box::new)
                    .map_err(|_| invalid())
            })
            .transpose()?;
        let empty = BTreeSet::new();
        let allowed = self
            .service
            .as_ref()
            .map_or(&empty, ServiceSnapshot::disconnected_lines);
        let mut sim = self.scenario.into_simulation_with_disconnections(allowed)?;
        let elapsed = self
            .tick
            .seconds()
            .checked_sub(sim.traffic.seconds())
            .ok_or(LoadError::InvalidState { field: "tick" })?;
        if self.development_ticks > elapsed {
            return Err(LoadError::InvalidState {
                field: "development_ticks",
            });
        }
        sim.world
            .road
            .restore(self.tick, self.lanes, self.junctions)?;
        sim.world
            .service
            .restore(self.service, self.tick, &sim.world.road)?;
        sim.world
            .taxi
            .restore(self.taxi, self.tick, &sim.world.road)?;
        sim.world.demand.restore(
            self.demand,
            self.tick,
            &sim.world.road,
            &sim.world.service,
            &sim.world.taxi,
        )?;
        sim.world
            .growth
            .restore(self.growth, sim.world.demand.totals())?;
        if !growth_runtime::reservations_fit(&sim.scenario, &sim.world.growth)
            || !growth_runtime::completed_sites_fit(&sim.scenario, &sim.world.growth)
        {
            return Err(LoadError::InvalidState {
                field: "growth.layout",
            });
        }
        sim.traffic = self.tick;
        sim.network_revision = self.network_revision;
        sim.development_ticks = self.development_ticks;
        sim.sequence = self.sequence;
        sim.paused = self.paused;
        sim.growth_paused = self.growth_paused;
        sim.random = self.random;
        for marker in self.markers {
            if sim.markers.insert(marker.id, marker).is_some() {
                return Err(LoadError::InvalidState {
                    field: "markers.id",
                });
            }
        }
        sim.undo = undo;
        Ok(sim)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneV1 {
    pub(crate) id: LaneId,
    pub(crate) credit: u64,
    pub(crate) wait: Option<WaitReason>,
    pub(crate) departure: Option<LaneDeparture>,
    pub(crate) queue: Vec<QueueItemV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CarRecoveryV1 {
    pub(crate) target: LaneId,
    pub(crate) exit: Option<crate::ZoneId>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueueItemV1 {
    pub(crate) entry: QueueEntry,
    pub(crate) ready_at: Option<Tick>,
    pub(crate) route: Vec<MovementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cohort: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) managed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery: Option<CarRecoveryV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) blocked_destination: Option<LaneId>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JunctionV1 {
    pub(crate) id: JunctionId,
    pub(crate) last_granted: Vec<(u16, LaneId)>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}
