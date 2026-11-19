use std::num::NonZeroU64;

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
        } = sim.scenario.clone();
        building_ids.sort_unstable();
        lanes.sort_unstable_by_key(|lane| lane.id);
        movements.sort_unstable_by_key(|movement| movement.id);
        junctions.sort_unstable_by_key(|junction| junction.id);
        traffic.sort_by_key(|item| item.lane);
        demand.sort_tables();
        service.sort_tables();
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
        }
    }

    pub(super) fn into_simulation(self) -> Result<Simulation, LoadError> {
        let scenario = Scenario {
            start: self.start,
            building_ids: self.building_ids,
            lanes: self.lanes,
            movements: self.movements,
            junctions: self.junctions,
            traffic: self.traffic,
            demand: self.demand,
            service: self.service,
        };
        Simulation::new_game(
            scenario,
            RuleSet::new(self.development_period_ticks),
            self.seed,
        )
        .map_err(LoadError::State)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    scenario: ScenarioV1,
    tick: Tick,
    development_ticks: u64,
    sequence: u64,
    paused: bool,
    growth_paused: bool,
    lanes: Vec<LaneV1>,
    junctions: Vec<JunctionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    demand: Option<DemandSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    service: Option<ServiceSnapshot>,
}

impl Payload {
    pub(super) fn from_simulation(sim: &Simulation) -> Self {
        let (lanes, junctions) = sim.world.road.snapshot();
        Self {
            scenario: ScenarioV1::from_simulation(sim),
            tick: sim.traffic,
            development_ticks: sim.development_ticks,
            sequence: sim.sequence,
            paused: sim.paused,
            growth_paused: sim.growth_paused,
            lanes,
            junctions,
            demand: sim.world.demand.snapshot(),
            service: sim.world.service.snapshot(),
        }
    }

    pub(super) fn into_simulation(self) -> Result<Simulation, LoadError> {
        let mut sim = self.scenario.into_simulation()?;
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
            .demand
            .restore(self.demand, self.tick, &sim.world.road)?;
        sim.world
            .service
            .restore(self.service, self.tick, &sim.world.road)?;
        sim.traffic = self.tick;
        sim.development_ticks = self.development_ticks;
        sim.sequence = self.sequence;
        sim.paused = self.paused;
        sim.growth_paused = self.growth_paused;
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueueItemV1 {
    pub(crate) entry: QueueEntry,
    pub(crate) ready_at: Option<Tick>,
    pub(crate) route: Vec<MovementId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cohort: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) managed: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JunctionV1 {
    pub(crate) id: JunctionId,
    pub(crate) last_granted: Vec<(u16, LaneId)>,
}
