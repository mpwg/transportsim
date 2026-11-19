use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BuildingSite, GrowthError, GrowthSite, GrowthUse, Scenario, Simulation, Terrain, edit,
    growth::{Growth, GrowthAssessment},
    traffic::RoadState,
};

pub(crate) fn validate_sites(scenario: &Scenario) -> Result<(), GrowthError> {
    let zones: BTreeSet<_> = scenario.demand.zones.iter().map(|zone| zone.id).collect();
    let terrain: BTreeMap<_, _> = scenario
        .layout
        .tiles
        .iter()
        .map(|tile| (tile.cell, tile.terrain))
        .collect();
    let mut proposed_lanes = BTreeSet::new();
    for site in &scenario.growth.sites {
        if !zones.contains(&site.zone) || terrain.get(&site.cell) != Some(&Terrain::Land) {
            return Err(GrowthError::InvalidSite(site.building));
        }
        if site.initial_tiers != 0
            && !scenario.layout.buildings.iter().any(|building| {
                building.id == site.building
                    && building.zone == site.zone
                    && building.access == site.access
                    && building.cell == site.cell
            })
        {
            return Err(GrowthError::InvalidSite(site.building));
        }
        if let Some(local) = &site.local_road
            && (!zones.contains(&local.connects_zone)
                || !proposed_lanes.insert(local.lane.id)
                || local
                    .street
                    .cells
                    .iter()
                    .any(|cell| terrain.get(cell) != Some(&Terrain::Land)))
        {
            return Err(GrowthError::InvalidLocalRoad(site.building));
        }
    }
    Ok(())
}

/// Prepare a building and its local connection on a disposable scenario.
pub(crate) fn stage(
    scenario: &mut Scenario,
    site: &GrowthSite,
    amount: u32,
    tiers: u32,
) -> Result<(), GrowthError> {
    let invalid_site = || GrowthError::InvalidSite(site.building);
    if let Some(local) = &site.local_road {
        let invalid_road = || GrowthError::InvalidLocalRoad(site.building);
        if tiers == 0 {
            let adjacent = scenario
                .demand
                .zones
                .iter()
                .find(|zone| zone.id == local.connects_zone)
                .and_then(|zone| zone.road_access)
                .ok_or_else(invalid_road)?;
            let has_link = |from, to| {
                local
                    .movements
                    .iter()
                    .any(|movement| movement.from == from && movement.to == to)
            };
            if !scenario.lanes.iter().any(|lane| lane.id == adjacent)
                || !has_link(local.lane.id, adjacent)
                || !has_link(adjacent, local.lane.id)
                || scenario.lanes.iter().any(|lane| lane.id == local.lane.id)
                || scenario
                    .layout
                    .streets
                    .iter()
                    .flat_map(|street| &street.cells)
                    .any(|cell| local.street.cells.contains(cell))
                || local
                    .movements
                    .iter()
                    .any(|new| scenario.movements.iter().any(|old| old.id == new.id))
                || local
                    .junctions
                    .iter()
                    .any(|new| scenario.junctions.iter().any(|old| old.id == new.id))
            {
                return Err(invalid_road());
            }
            scenario.lanes.push(local.lane.clone());
            scenario.layout.streets.push(local.street.clone());
            scenario.movements.extend(local.movements.iter().cloned());
            scenario.junctions.extend(local.junctions.iter().cloned());
        } else if !scenario.lanes.iter().any(|lane| lane.id == local.lane.id) {
            return Err(invalid_road());
        }
    }
    let zone = scenario
        .demand
        .zones
        .iter_mut()
        .find(|zone| zone.id == site.zone)
        .ok_or_else(invalid_site)?;
    let volume = match site.usage {
        GrowthUse::Housing => &mut zone.residents,
        GrowthUse::Jobs => &mut zone.jobs,
        GrowthUse::Shops => &mut zone.shops,
    };
    *volume = volume.checked_add(amount).ok_or_else(invalid_site)?;
    if zone.road_access.is_none() {
        zone.road_access = site.access;
    }
    if let Some(existing) = scenario
        .layout
        .buildings
        .iter()
        .find(|building| building.id == site.building)
    {
        if existing.zone != site.zone
            || existing.access != site.access
            || existing.cell != site.cell
        {
            return Err(invalid_site());
        }
    } else {
        scenario.layout.buildings.push(BuildingSite {
            id: site.building,
            zone: site.zone,
            cell: site.cell,
            access: site.access,
        });
    }
    Ok(())
}

pub(crate) fn can_place(scenario: &Scenario, site: &GrowthSite, tiers: u32) -> bool {
    let mut proposal = scenario.clone();
    if stage(&mut proposal, site, 0, tiers).is_err() {
        return false;
    }
    let exists_lane = |id| proposal.lanes.iter().any(|lane| lane.id == id);
    let exists_zone = |id| proposal.demand.zones.iter().any(|zone| zone.id == id);
    let exists_edge = |id| proposal.demand.foot_edges.iter().any(|edge| edge.id == id);
    proposal.layout.validate(exists_lane, exists_zone).is_ok()
        && proposal
            .layout
            .validate_connections(&proposal.movements, exists_edge)
            .is_ok()
        && (site.local_road.is_none()
            || tiers != 0
            || RoadState::new(
                proposal.lanes,
                proposal.movements,
                proposal.junctions,
                Vec::new(),
            )
            .is_ok())
}

pub(crate) fn reservations_fit(scenario: &Scenario, growth: &Growth) -> bool {
    scenario.growth.sites.iter().all(|site| {
        growth.project(site.building).is_none()
            || can_place(
                scenario,
                site,
                growth.tiers(site.building).expect("validated growth site"),
            )
    })
}

pub(crate) fn completed_sites_fit(scenario: &Scenario, growth: &Growth) -> bool {
    scenario.growth.sites.iter().all(|site| {
        growth.tiers(site.building) == Some(0)
            || scenario.layout.buildings.iter().any(|building| {
                building.id == site.building
                    && building.zone == site.zone
                    && building.access == site.access
                    && building.cell == site.cell
            })
    })
}

impl Simulation {
    pub(crate) fn grow_city(&mut self) {
        self.world.demand.close_growth_measurements();
        for building in self.world.growth.due() {
            let site = self
                .scenario
                .growth
                .sites
                .iter()
                .find(|site| site.building == building)
                .expect("validated growth project");
            if let Ok(prepared) = edit::prepare_growth(self, site) {
                self.world = prepared.world;
                self.scenario = prepared.scenario;
                self.network_revision = prepared.preview.revision;
            }
        }
        let rules = &self.scenario.growth.rules;
        let horizon = rules.max_travel_ticks.get();
        let assessments = self
            .scenario
            .growth
            .sites
            .iter()
            .map(|site| {
                let score = |zone, horizon| {
                    self.world.demand.growth_accessibility(
                        zone,
                        site.usage,
                        self.traffic,
                        horizon,
                        &self.world,
                    )
                };
                let tiers = self
                    .world
                    .growth
                    .tiers(site.building)
                    .expect("validated growth site");
                let direct = score(site.zone, horizon);
                let potential = site
                    .local_road
                    .as_ref()
                    .filter(|_| tiers == 0)
                    .and_then(|road| {
                        horizon
                            .checked_sub(road.lane.travel_ticks.get())
                            .map(|remaining| score(road.connects_zone, remaining))
                    })
                    .unwrap_or(0);
                (
                    site.building,
                    GrowthAssessment {
                        accessibility: direct.max(potential),
                        available: tiers == site.max_tiers.get()
                            || can_place(&self.scenario, site, tiers),
                    },
                )
            })
            .collect();
        self.world
            .growth
            .choose(self.world.demand.totals(), &assessments);
    }
}
