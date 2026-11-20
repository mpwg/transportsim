use std::num::NonZeroU32;

use sim_core::{
    DischargeRate, JunctionId, JunctionSpec, LaneId, LaneSpec, MovementId, MovementSpec, RuleSet,
    Scenario, Simulation, Tick,
    VehicleKind::{Bus, Car},
};

fn scenario() -> Simulation {
    let mut scenario = Scenario::empty(Tick::new(0));
    let capacity = NonZeroU32::new(30_000).unwrap();
    scenario.lanes = [
        (1, 1, vec![Car, Bus]),
        (2, 8, vec![Car, Bus]),
        (3, 2, vec![Car]),
        (4, 1, vec![Car, Bus]),
        (5, 1, vec![Bus]),
        (6, 1, vec![Car]),
    ]
    .into_iter()
    .map(|(id, ticks, allowed)| LaneSpec {
        id: LaneId::new(id),
        capacity_mm: capacity,
        travel_ticks: NonZeroU32::new(ticks).unwrap(),
        discharge: DischargeRate {
            millimetres: capacity,
            ticks: NonZeroU32::MIN,
        },
        allowed,
    })
    .collect();
    let junction = JunctionId::new(1);
    scenario.junctions.push(JunctionSpec {
        id: junction,
        phases: vec![],
    });
    scenario.movements = [
        (101, 1, 2),
        (102, 2, 4),
        (103, 1, 3),
        (104, 3, 4),
        (105, 1, 5),
        (106, 5, 4),
    ]
    .into_iter()
    .map(|(id, from, to)| MovementSpec {
        id: MovementId::new(id),
        from: LaneId::new(from),
        to: LaneId::new(to),
        junction,
        conflict_zones: vec![1],
        priority: 0,
    })
    .collect();
    Simulation::new_game(scenario, RuleSet::default(), 1).unwrap()
}

#[test]
fn route_preview_lists_lanes_on_the_fastest_route_for_each_vehicle_kind() {
    let sim = scenario();
    let lane = LaneId::new;
    let before = sim.fingerprint().unwrap();
    assert_eq!(
        sim.preview_vehicle_route(lane(1), lane(4), Car),
        Some(vec![lane(1), lane(3), lane(4)])
    );
    assert_eq!(
        sim.preview_vehicle_route(lane(1), lane(4), Bus),
        Some(vec![lane(1), lane(5), lane(4)])
    );
    assert_eq!(sim.fingerprint().unwrap(), before);
}

#[test]
fn route_preview_on_an_isolated_lane_includes_the_lane_once() {
    let sim = scenario();
    let isolated = LaneId::new(6);
    assert_eq!(
        sim.preview_vehicle_route(isolated, isolated, Car),
        Some(vec![isolated])
    );
}

#[test]
fn route_preview_rejects_unknown_restricted_or_unreachable_endpoints() {
    let sim = scenario();
    let lane = LaneId::new;
    for (from, to) in [(99, 4), (1, 99), (99, 99), (5, 4), (1, 5), (5, 5), (4, 1)] {
        assert_eq!(
            sim.preview_vehicle_route(lane(from), lane(to), Car),
            None,
            "unexpected car route from {from} to {to}"
        );
    }
}
