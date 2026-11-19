use super::*;
use crate::{DischargeRate, LaneSpec, StreetSite, VehicleKind, ZoneId};
use std::num::NonZeroU32;

const TWO: NonZeroU32 = NonZeroU32::new(2).unwrap();
const THREE: NonZeroU32 = NonZeroU32::new(3).unwrap();

fn site(id: u64, usage: GrowthUse) -> GrowthSite {
    GrowthSite {
        building: BuildingId::new(id),
        zone: ZoneId::new(id),
        cell: Cell { x: id as i32, y: 0 },
        access: Some(LaneId::new(id)),
        usage,
        amount: TWO,
        max_tiers: TWO,
        initial_tiers: 0,
        local_road: None,
    }
}

fn assess(scores: &[(u64, u64)]) -> BTreeMap<BuildingId, GrowthAssessment> {
    scores
        .iter()
        .map(|&(id, accessibility)| {
            (
                BuildingId::new(id),
                GrowthAssessment {
                    accessibility,
                    available: true,
                },
            )
        })
        .collect()
}

fn spec(sites: Vec<GrowthSite>) -> GrowthSpec {
    GrowthSpec {
        sites,
        rules: GrowthRules {
            build_periods: THREE,
            projects_per_period: TWO,
            minimum_accessibility: TWO,
            ..GrowthRules::default()
        },
    }
}

#[test]
fn ranks_accessibility_then_building_id() {
    let totals = GrowthTotals::default();
    let mut growth = Growth::new(spec(vec![
        site(1, GrowthUse::Housing),
        site(4, GrowthUse::Housing),
        site(3, GrowthUse::Housing),
        site(2, GrowthUse::Housing),
        site(5, GrowthUse::Housing),
    ]))
    .unwrap();
    growth.choose(totals, &assess(&[(1, 2), (2, 7), (3, 7), (4, 8), (5, 1)]));
    let observation = growth.observe(totals);
    let chosen: Vec<_> = observation
        .projects
        .iter()
        .map(|project| project.building.get())
        .collect();
    assert_eq!(chosen, [2, 4]);
    assert_eq!(observation.sites[2].reason, GrowthReason::Eligible);
    assert_eq!(observation.sites[4].reason, GrowthReason::Inaccessible);
}

#[test]
fn reserves_population_during_construction() {
    let totals = GrowthTotals {
        residents: 9_999,
        ..Default::default()
    };
    let mut growth = Growth::new(spec(vec![
        site(1, GrowthUse::Housing),
        site(2, GrowthUse::Housing),
    ]))
    .unwrap();
    let assessments = assess(&[(1, 7), (2, 2)]);
    growth.choose(totals, &assessments);
    assert_eq!(growth.project(BuildingId::new(1)).unwrap().amount, 1);
    assert_eq!(
        growth.observe(totals).sites[1].reason,
        GrowthReason::NoCapacity
    );
    assert!(growth.due().is_empty());
    growth.choose(totals, &assessments);
    assert!(growth.project(BuildingId::new(2)).is_none());
}

#[test]
fn running_job_and_shop_projects_reserve_their_own_demand() {
    let totals = GrowthTotals {
        residents: 4,
        jobs: 3,
        shops: 3,
    };
    let mut growth = Growth::new(spec(vec![
        site(1, GrowthUse::Jobs),
        site(2, GrowthUse::Jobs),
        site(3, GrowthUse::Shops),
        site(4, GrowthUse::Shops),
    ]))
    .unwrap();
    growth.choose(totals, &assess(&[(1, 5), (2, 4), (3, 3), (4, 2)]));
    let observation = growth.observe(totals);
    let chosen: Vec<_> = observation
        .projects
        .iter()
        .map(|project| (project.building.get(), project.amount))
        .collect();
    assert_eq!(chosen, [(1, 1), (3, 1)]);
    assert_eq!(observation.sites[1].reason, GrowthReason::NoCapacity);
    assert_eq!(observation.sites[3].reason, GrowthReason::NoCapacity);
}

#[test]
fn checkpoint_keeps_construction_time_and_observed_reasons() {
    let id = BuildingId::new(1);
    let spec = spec(vec![site(1, GrowthUse::Housing)]);
    let totals = GrowthTotals::default();
    let mut growth = Growth::new(spec.clone()).unwrap();
    growth.choose(totals, &assess(&[(1, 7)]));
    assert!(growth.complete(id).is_none());
    assert!(growth.due().is_empty());
    let wire = serde_json::to_vec(&growth.snapshot()).unwrap();
    let mut resumed = Growth::new(spec).unwrap();
    resumed
        .restore(serde_json::from_slice(&wire).unwrap(), totals)
        .unwrap();
    assert_eq!(resumed.observe(totals), growth.observe(totals));
    assert!(resumed.due().is_empty());
    assert_eq!(resumed.due(), [id]);
    assert_eq!(resumed.due(), [id]);
    assert_eq!(resumed.complete(id).unwrap().amount, 2);
    assert_eq!(resumed.tiers(id), Some(1));
    assert!(resumed.due().is_empty());
}

#[test]
fn checkpoint_rejects_invalid_project_values_without_mutating_state() {
    let totals = GrowthTotals {
        residents: 9_999,
        ..Default::default()
    };
    let spec = spec(vec![site(1, GrowthUse::Housing)]);
    let mut growth = Growth::new(spec).unwrap();
    growth.choose(totals, &assess(&[(1, 7)]));
    let before = growth.observe(totals);
    for (key, value) in [
        ("building", serde_json::json!(2)),
        ("usage", serde_json::json!("shops")),
        ("amount", serde_json::json!(0)),
        ("amount", serde_json::json!(2)),
        ("remaining_periods", serde_json::json!(4)),
    ] {
        let mut input = serde_json::to_value(growth.snapshot()).unwrap();
        input["projects"][0][key] = value;
        let snapshot = serde_json::from_value(input).unwrap();
        assert!(
            matches!(
                growth.restore(snapshot, totals),
                Err(LoadError::InvalidState { .. })
            ),
            "{key}"
        );
        assert_eq!(growth.observe(totals), before);
    }
}

fn road(lane: LaneId, cells: Vec<Cell>) -> LocalRoad {
    LocalRoad {
        lane: LaneSpec {
            id: lane,
            capacity_mm: TWO,
            travel_ticks: TWO,
            discharge: DischargeRate {
                millimetres: TWO,
                ticks: TWO,
            },
            allowed: vec![VehicleKind::Car],
        },
        street: StreetSite {
            lane,
            cells,
            bridge: false,
            road_width_mm: 2,
            foot_width_mm: 0,
            greenery_width_mm: 0,
            width_mm: TWO,
            foot_edges: vec![],
        },
        movements: vec![],
        junctions: vec![],
        connects_zone: ZoneId::new(3),
    }
}

#[test]
fn reserves_local_road_cells_against_other_building_projects() {
    let mut first = site(1, GrowthUse::Housing);
    let second = site(2, GrowthUse::Housing);
    first.local_road = Some(road(first.access.unwrap(), vec![second.cell]));
    let totals = GrowthTotals::default();
    let mut growth = Growth::new(spec(vec![first, second])).unwrap();
    growth.choose(totals, &assess(&[(1, 7), (2, 2)]));
    assert_eq!(
        growth.observe(totals).sites[1].reason,
        GrowthReason::SpaceReserved
    );
    assert!(growth.project(BuildingId::new(2)).is_none());
}

#[test]
fn checkpoint_restores_a_road_reservation_visiting_one_cell_twice() {
    let mut first = site(1, GrowthUse::Housing);
    let cell = Cell { x: 2, y: 0 };
    let neighbor = Cell { x: 2, y: 1 };
    first.local_road = Some(road(first.access.unwrap(), vec![cell, neighbor, cell]));
    let spec = spec(vec![first]);
    let totals = GrowthTotals::default();
    let mut growth = Growth::new(spec.clone()).unwrap();
    growth.choose(totals, &assess(&[(1, 7)]));
    let mut resumed = Growth::new(spec).unwrap();
    resumed.restore(growth.snapshot(), totals).unwrap();
    assert_eq!(resumed.observe(totals), growth.observe(totals));
}

#[test]
fn only_growth_scenarios_require_a_growth_checkpoint() {
    let totals = GrowthTotals::default();
    let mut empty = Growth::new(GrowthSpec::default()).unwrap();
    assert!(empty.snapshot().is_none());
    empty.restore(None, totals).unwrap();

    let mut growth = Growth::new(spec(vec![site(1, GrowthUse::Housing)])).unwrap();
    assert!(matches!(
        growth.restore(None, totals),
        Err(LoadError::InvalidState { .. })
    ));
    assert!(matches!(
        empty.restore(growth.snapshot(), totals),
        Err(LoadError::InvalidState { .. })
    ));
}

#[test]
fn rejects_initial_tiers_outside_the_site_limit() {
    let mut populated = site(1, GrowthUse::Housing);
    populated.initial_tiers = 3;
    assert_eq!(
        Growth::new(spec(vec![populated])).unwrap_err(),
        GrowthError::InvalidInitialTiers(BuildingId::new(1))
    );
}
