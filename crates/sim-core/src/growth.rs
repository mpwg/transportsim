use crate::random::SplitMix64;
use crate::{BuildingId, Cell, JunctionId, LaneId, LoadError, MovementId};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

mod spec;
pub use spec::{GrowthError, GrowthRules, GrowthSite, GrowthSpec, GrowthUse, LocalRoad};

pub(crate) const MAX_POPULATION: u32 = 10_000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ReservedAsset {
    Cell(Cell),
    Lane(LaneId),
    Movement(MovementId),
    Junction(JunctionId),
}

impl GrowthSite {
    fn assets(&self, tiers: u32) -> impl Iterator<Item = ReservedAsset> + '_ {
        let road = self.local_road.iter().filter(move |_| tiers == 0);
        std::iter::once(ReservedAsset::Cell(self.cell)).chain(road.flat_map(|road| {
            std::iter::once(ReservedAsset::Lane(road.lane.id))
                .chain(road.street.cells.iter().copied().map(ReservedAsset::Cell))
                .chain(
                    road.movements
                        .iter()
                        .map(|movement| ReservedAsset::Movement(movement.id)),
                )
                .chain(
                    road.junctions
                        .iter()
                        .map(|junction| ReservedAsset::Junction(junction.id)),
                )
        }))
    }
}

/// Completed capacity across all zones.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GrowthTotals {
    pub residents: u32,
    pub jobs: u32,
    pub shops: u32,
}

impl GrowthTotals {
    fn unclaimed_capacity(self) -> Self {
        Self {
            residents: MAX_POPULATION.saturating_sub(self.residents),
            jobs: self.residents.saturating_sub(self.jobs),
            shops: self.residents.saturating_sub(self.shops),
        }
    }

    fn capacity_for(&mut self, usage: GrowthUse) -> &mut u32 {
        match usage {
            GrowthUse::Housing => &mut self.residents,
            GrowthUse::Jobs => &mut self.jobs,
            GrowthUse::Shops => &mut self.shops,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GrowthAssessment {
    pub(crate) accessibility: u64,
    pub(crate) available: bool,
}

/// The latest recorded decision for a site.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrowthReason {
    Unassessed,
    Unavailable,
    Inaccessible,
    NoCapacity,
    SpaceReserved,
    Reserved,
    Complete,
    Eligible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GrowthProject {
    pub building: BuildingId,
    pub usage: GrowthUse,
    pub amount: u32,
    /// Zero means construction is due and is waiting to be applied to the city.
    pub remaining_periods: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GrowthSiteObservation {
    pub building: BuildingId,
    pub tiers: u32,
    pub accessibility: u64,
    pub reason: GrowthReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrowthObservation {
    pub totals: GrowthTotals,
    pub projects: Vec<GrowthProject>,
    pub sites: Vec<GrowthSiteObservation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GrowthSnapshot {
    projects: Vec<GrowthProject>,
    sites: Vec<GrowthSiteObservation>,
}

#[derive(Clone, Debug)]
pub(crate) struct Growth {
    rules: GrowthRules,
    sites: BTreeMap<BuildingId, GrowthSite>,
    states: BTreeMap<BuildingId, GrowthSiteObservation>,
    projects: BTreeMap<BuildingId, GrowthProject>,
}

impl Growth {
    pub(crate) fn new(spec: GrowthSpec) -> Result<Self, GrowthError> {
        let mut sites = BTreeMap::new();
        let mut states = BTreeMap::new();
        for site in spec.sites {
            let id = site.building;
            if site.initial_tiers > site.max_tiers.get() {
                return Err(GrowthError::InvalidInitialTiers(id));
            }
            if spec
                .rules
                .build_periods
                .get()
                .checked_add(site.additional_build_periods)
                .is_none()
            {
                return Err(GrowthError::BuildDurationOverflow(id));
            }
            if site.local_road.as_ref().is_some_and(|road| {
                road.lane.id != road.street.lane
                    || site.access != Some(road.lane.id)
                    || road.street.bridge
                    || road.street.cells.is_empty()
                    || road.street.cells.contains(&site.cell)
            }) {
                return Err(GrowthError::InvalidLocalRoad(id));
            }
            let reason = if site.initial_tiers == site.max_tiers.get() {
                GrowthReason::Complete
            } else {
                GrowthReason::Unassessed
            };
            states.insert(
                id,
                GrowthSiteObservation {
                    building: id,
                    tiers: site.initial_tiers,
                    accessibility: 0,
                    reason,
                },
            );
            if sites.insert(id, site).is_some() {
                return Err(GrowthError::DuplicateSite(id));
            }
        }
        Ok(Self {
            rules: spec.rules,
            sites,
            states,
            projects: BTreeMap::new(),
        })
    }

    pub(crate) fn due(&mut self) -> Vec<BuildingId> {
        self.projects
            .values_mut()
            .filter_map(|project| {
                project.remaining_periods = project.remaining_periods.saturating_sub(1);
                (project.remaining_periods == 0).then_some(project.building)
            })
            .collect()
    }

    pub(crate) fn project(&self, id: BuildingId) -> Option<&GrowthProject> {
        self.projects.get(&id)
    }

    pub(crate) fn complete(&mut self, id: BuildingId) -> Option<GrowthProject> {
        if self.projects.get(&id)?.remaining_periods != 0 {
            return None;
        }
        let project = self.projects.remove(&id)?;
        let state = self.states.get_mut(&id).expect("project has a site");
        state.tiers += 1;
        state.reason = if state.tiers == self.sites[&id].max_tiers.get() {
            GrowthReason::Complete
        } else {
            GrowthReason::Eligible
        };
        Some(project)
    }

    pub(crate) fn tiers(&self, id: BuildingId) -> Option<u32> {
        self.states.get(&id).map(|state| state.tiers)
    }

    pub(crate) fn choose(
        &mut self,
        totals: GrowthTotals,
        assessments: &BTreeMap<BuildingId, GrowthAssessment>,
        mut random: Option<&mut SplitMix64>,
    ) {
        let mut capacity = totals.unclaimed_capacity();
        let mut reserved = BTreeSet::new();
        for project in self.projects.values() {
            let unclaimed = capacity.capacity_for(project.usage);
            *unclaimed = unclaimed.saturating_sub(project.amount);
            reserved
                .extend(self.sites[&project.building].assets(self.states[&project.building].tiers));
        }
        let mut ranked: Vec<_> = self
            .sites
            .keys()
            .copied()
            .map(|id| (id, assessments.get(&id).copied().unwrap_or_default()))
            .collect();
        ranked.sort_unstable_by_key(|(id, assessment)| (Reverse(assessment.accessibility), *id));

        let minimum = u64::from(self.rules.minimum_accessibility.get());
        let mut slots = self.rules.projects_per_period.get();
        for (id, assessment) in ranked {
            let site = &self.sites[&id];
            let state = self.states.get_mut(&id).expect("known site");
            let unclaimed = capacity.capacity_for(site.usage);
            state.accessibility = assessment.accessibility;
            state.reason = if self.projects.contains_key(&id) {
                GrowthReason::Reserved
            } else if state.tiers == site.max_tiers.get() {
                GrowthReason::Complete
            } else if !assessment.available {
                GrowthReason::Unavailable
            } else if assessment.accessibility < minimum {
                GrowthReason::Inaccessible
            } else if site
                .assets(state.tiers)
                .any(|asset| reserved.contains(&asset))
            {
                GrowthReason::SpaceReserved
            } else if *unclaimed == 0 {
                GrowthReason::NoCapacity
            } else if slots == 0 {
                GrowthReason::Eligible
            } else {
                let amount = (*unclaimed).min(site.amount.get());
                let extra_periods = random.as_deref_mut().map_or(0, |stream| {
                    stream.sample_up_to(site.additional_build_periods)
                });
                self.projects.insert(
                    id,
                    GrowthProject {
                        building: id,
                        usage: site.usage,
                        amount,
                        remaining_periods: self.rules.build_periods.get() + extra_periods,
                    },
                );
                *unclaimed -= amount;
                slots -= 1;
                reserved.extend(site.assets(state.tiers));
                GrowthReason::Reserved
            };
        }
    }

    pub(crate) fn observe(&self, totals: GrowthTotals) -> GrowthObservation {
        GrowthObservation {
            totals,
            projects: self.projects.values().copied().collect(),
            sites: self.states.values().copied().collect(),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<GrowthSnapshot> {
        (!self.sites.is_empty()).then(|| GrowthSnapshot {
            projects: self.projects.values().copied().collect(),
            sites: self.states.values().copied().collect(),
        })
    }

    pub(crate) fn restore(
        &mut self,
        snapshot: Option<GrowthSnapshot>,
        totals: GrowthTotals,
    ) -> Result<(), LoadError> {
        const GROWTH: LoadError = LoadError::InvalidState { field: "growth" };
        const SITES: LoadError = LoadError::InvalidState {
            field: "growth.sites",
        };
        const PROJECTS: LoadError = LoadError::InvalidState {
            field: "growth.projects",
        };

        let Some(snapshot) = snapshot else {
            return if self.sites.is_empty() {
                Ok(())
            } else {
                Err(GROWTH)
            };
        };
        if self.sites.is_empty() || totals.residents > MAX_POPULATION {
            return Err(GROWTH);
        }
        if self.sites.len() != snapshot.sites.len() {
            return Err(SITES);
        }

        let mut states = BTreeMap::new();
        for state in snapshot.sites {
            let site = self.sites.get(&state.building).ok_or(SITES)?;
            if !(site.initial_tiers..=site.max_tiers.get()).contains(&state.tiers)
                || (state.reason == GrowthReason::Complete) != (state.tiers == site.max_tiers.get())
                || states.insert(state.building, state).is_some()
            {
                return Err(SITES);
            }
        }

        let mut projects = BTreeMap::new();
        let mut capacity = totals.unclaimed_capacity();
        let mut reserved = BTreeSet::new();
        for project in snapshot.projects {
            let site = self.sites.get(&project.building).ok_or(PROJECTS)?;
            let state = &states[&project.building];
            let unclaimed = capacity.capacity_for(project.usage);
            if site.usage != project.usage
                || project.amount == 0
                || project.amount > site.amount.get().min(*unclaimed)
                || project.remaining_periods
                    > self.rules.build_periods.get() + site.additional_build_periods
                || state.reason != GrowthReason::Reserved
                || site
                    .assets(state.tiers)
                    .any(|asset| reserved.contains(&asset))
                || projects.insert(project.building, project).is_some()
            {
                return Err(PROJECTS);
            }
            *unclaimed -= project.amount;
            reserved.extend(site.assets(state.tiers));
        }
        if states.values().any(|state| {
            state.reason == GrowthReason::Reserved && !projects.contains_key(&state.building)
        }) {
            return Err(PROJECTS);
        }

        self.states = states;
        self.projects = projects;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
