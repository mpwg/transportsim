use std::num::NonZeroU32;

use sim_core::{
    DischargeRate, FootEdgeId, FootEdgeSpec, JunctionId, JunctionSpec, LaneId, LaneSpec,
    MovementId, MovementSpec, RuleSet, Scenario, SignalPhase, Simulation, Tick, VehicleKind,
    ZoneId, ZoneSpec,
};

fn foot_network() -> Scenario {
    let mut scenario = Scenario::empty(Tick::new(1));
    scenario.demand.zones = (1..=6)
        .map(|id| ZoneSpec {
            id: ZoneId::new(id),
            residents: 0,
            jobs: 0,
            shops: 0,
            road_access: None,
        })
        .collect();
    scenario.demand.foot_edges = [
        (10, 1, 2, 2, Some(MovementId::new(1))),
        (20, 2, 3, 2, None),
        (30, 1, 4, 1, Some(MovementId::new(2))),
        (40, 5, 1, 1, None),
    ]
    .map(|(id, from, to, ticks, signal)| FootEdgeSpec {
        id: FootEdgeId::new(id),
        from: ZoneId::new(from),
        to: ZoneId::new(to),
        travel_ticks: NonZeroU32::new(ticks).unwrap(),
        capacity: NonZeroU32::MIN,
        discharge: NonZeroU32::MIN,
        signal,
    })
    .into();
    let capacity_mm = NonZeroU32::new(5_000).unwrap();
    scenario.lanes = [1, 2]
        .map(|id| LaneSpec {
            id: LaneId::new(id),
            capacity_mm,
            travel_ticks: NonZeroU32::MIN,
            discharge: DischargeRate {
                millimetres: capacity_mm,
                ticks: NonZeroU32::MIN,
            },
            allowed: vec![VehicleKind::Car],
        })
        .into();
    let junction = JunctionId::new(1);
    scenario.movements = [(1, 1, 2), (2, 2, 1)]
        .map(|(id, from, to)| MovementSpec {
            id: MovementId::new(id),
            from: LaneId::new(from),
            to: LaneId::new(to),
            junction,
            conflict_zones: Vec::new(),
            priority: 0,
        })
        .into();
    scenario.junctions.push(JunctionSpec {
        id: junction,
        phases: vec![SignalPhase {
            green_ticks: NonZeroU32::new(2).unwrap(),
            clearance_ticks: 3,
            movements: vec![MovementId::new(1)],
        }],
    });
    scenario
}

fn path(zone: u64, edges: &[u64]) -> (ZoneId, Vec<FootEdgeId>) {
    (
        ZoneId::new(zone),
        edges.iter().copied().map(FootEdgeId::new).collect(),
    )
}

#[test]
fn outward_access_waits_for_green_and_follows_directed_paths() {
    let mut sim = Simulation::new_game(foot_network(), RuleSet::default(), 42).unwrap();
    let source = ZoneId::new(1);
    assert_eq!(sim.walking_access_from(source, 4), [path(1, &[])]);
    assert_eq!(
        sim.walking_access_from(source, 7),
        [path(1, &[]), path(2, &[10]), path(3, &[10, 20])]
    );
    assert_eq!(
        sim.walking_access_from(source, u32::MAX),
        [path(1, &[]), path(2, &[10]), path(3, &[10, 20])]
    );

    sim.advance(1).unwrap();
    assert_eq!(
        sim.walking_access_from(source, 4),
        [path(1, &[]), path(2, &[10])]
    );
}

#[test]
fn inward_access_respects_the_deadline_and_keeps_edges_in_travel_order() {
    let sim = Simulation::new_game(foot_network(), RuleSet::default(), 42).unwrap();
    let target = ZoneId::new(3);
    assert_eq!(
        sim.walking_access_to(target, 6),
        [path(2, &[20]), path(3, &[])]
    );
    assert_eq!(
        sim.walking_access_to(target, 7),
        [
            path(1, &[10, 20]),
            path(2, &[20]),
            path(3, &[]),
            path(5, &[40, 10, 20])
        ]
    );
    assert_eq!(
        sim.walking_access_to(ZoneId::new(4), u32::MAX),
        [path(4, &[])]
    );
}

#[test]
fn stationary_and_unreachable_queries_leave_the_save_identical() {
    let sim = Simulation::new_game(foot_network(), RuleSet::default(), 42).unwrap();
    let before = sim.save().unwrap();
    let isolated = ZoneId::new(6);
    let unknown = ZoneId::new(7);
    assert_eq!(sim.walking_access_from(isolated, 0), [path(6, &[])]);
    assert_eq!(sim.walking_access_to(isolated, 0), [path(6, &[])]);
    assert!(sim.walking_access_from(unknown, 10).is_empty());
    assert!(sim.walking_access_to(unknown, 10).is_empty());
    let target = ZoneId::new(3);
    let inward_access = sim.walking_access_to(target, 7);
    assert_eq!(sim.save().unwrap(), before);
    let loaded = Simulation::load(&before).unwrap();
    assert_eq!(loaded.walking_access_to(target, 7), inward_access);
}
