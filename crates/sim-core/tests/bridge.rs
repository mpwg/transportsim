use std::num::NonZeroU32;

use sim_core::{
    DischargeRate, JunctionId, JunctionSpec, LaneId, LaneObservation, LaneSpec, MovementId,
    MovementSpec, Observation, Query, QueueEntry, RuleSet, Scenario, SignalPhase, Simulation, Tick,
    TrafficSeed, VehicleId, VehicleKind, WaitReason,
};

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn lane(id: u64, capacity: u32) -> LaneSpec {
    LaneSpec {
        id: LaneId::new(id),
        capacity_mm: nz(capacity),
        travel_ticks: nz(1),
        discharge: DischargeRate {
            millimetres: nz(30_000),
            ticks: nz(1),
        },
        allowed: vec![VehicleKind::Car, VehicleKind::Bus],
    }
}

fn movement(id: u64, from: u64, to: u64) -> MovementSpec {
    MovementSpec {
        id: MovementId::new(id),
        from: LaneId::new(from),
        to: LaneId::new(to),
        junction: JunctionId::new(1),
        conflict_zones: vec![1],
        priority: 0,
    }
}

fn seed(lane: u64, entry: QueueEntry, ready: u64, moves: &[u64]) -> TrafficSeed {
    TrafficSeed {
        lane: LaneId::new(lane),
        entry,
        ready_at: Tick::new(ready),
        route: moves.iter().copied().map(MovementId::new).collect(),
    }
}

fn cars(count: u32) -> QueueEntry {
    QueueEntry::PrivateCars(nz(count))
}

fn bus() -> QueueEntry {
    QueueEntry::Vehicle {
        id: VehicleId::new(12),
        kind: VehicleKind::Bus,
        length_mm: nz(12_000),
    }
}

fn observe(sim: &Simulation, lane: u64) -> LaneObservation {
    let Observation::Lane(view) = sim.inspect(Query::Lane(LaneId::new(lane))).unwrap() else {
        panic!("expected lane view");
    };
    view
}

fn phase(ticks: u32, moves: &[u64]) -> SignalPhase {
    SignalPhase {
        green_ticks: nz(ticks),
        clearance_ticks: 0,
        movements: moves.iter().copied().map(MovementId::new).collect(),
    }
}

#[test]
fn bus_waits_behind_left_turners_and_crosses_as_one_vehicle() {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = vec![lane(1, 60_000), lane(2, 60_000), lane(3, 60_000)];
    spec.movements = vec![movement(1, 1, 2), movement(2, 1, 3)];
    spec.junctions = vec![JunctionSpec {
        id: JunctionId::new(1),
        phases: vec![phase(1, &[2]), phase(1, &[1]), phase(1, &[2])],
    }];
    spec.traffic = vec![
        seed(1, cars(3), 1, &[1]),
        seed(1, bus(), 1, &[2]),
        seed(1, cars(5), 1, &[2]),
    ];
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 4).unwrap();

    sim.advance(1).unwrap();
    assert_eq!(observe(&sim, 1).queue, [cars(3), bus(), cars(5)]);
    assert_eq!(observe(&sim, 1).wait_reason, Some(WaitReason::RedSignal));
    assert!(observe(&sim, 3).queue.is_empty());
    sim.advance(2).unwrap();
    assert_eq!(observe(&sim, 3).queue, [bus()]);
    assert_eq!(observe(&sim, 3).occupied_mm, 12_000);
    assert_eq!(observe(&sim, 1).queue, [cars(5)]);
}

#[test]
fn full_target_blocks_and_a_car_may_not_cross_twice_in_a_tick() {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = vec![lane(1, 30_000), lane(2, 5_000), lane(3, 30_000)];
    spec.movements = vec![movement(1, 1, 2), movement(2, 2, 3)];
    spec.junctions = vec![JunctionSpec {
        id: JunctionId::new(1),
        phases: vec![],
    }];
    spec.traffic = vec![seed(1, cars(2), 1, &[1, 2]), seed(2, cars(1), 2, &[2])];
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 4).unwrap();

    sim.advance(1).unwrap();
    assert_eq!(observe(&sim, 1).wait_reason, Some(WaitReason::TargetFull));
    sim.advance(2).unwrap();
    assert_eq!(observe(&sim, 1).queue, [cars(1)]);
    assert_eq!(observe(&sim, 2).queue, [cars(1)]);
    assert!(observe(&sim, 3).queue.is_empty());
}

#[test]
fn equal_priority_alternates_under_continuous_contention() {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = vec![lane(1, 30_000), lane(2, 30_000), lane(3, 60_000)];
    spec.lanes[0].discharge.millimetres = nz(5_000);
    spec.lanes[1].discharge.millimetres = nz(5_000);
    spec.movements = vec![movement(1, 1, 3), movement(2, 2, 3)];
    spec.junctions = vec![JunctionSpec {
        id: JunctionId::new(1),
        phases: vec![],
    }];
    spec.traffic = vec![seed(1, cars(4), 1, &[1]), seed(2, cars(4), 1, &[2])];
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 4).unwrap();

    for (a, b) in [(3, 4), (3, 3), (2, 3), (2, 2)] {
        sim.advance(1).unwrap();
        assert_eq!(observe(&sim, 1).queue, [cars(a)]);
        assert_eq!(observe(&sim, 2).queue, [cars(b)]);
    }
}

#[test]
fn red_does_not_accumulate_discharge_and_fractional_green_does() {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = vec![lane(1, 30_000), lane(2, 30_000)];
    spec.lanes[0].discharge = DischargeRate {
        millimetres: nz(5_000),
        ticks: nz(2),
    };
    spec.movements = vec![movement(1, 1, 2)];
    spec.junctions = vec![JunctionSpec {
        id: JunctionId::new(1),
        phases: vec![phase(8, &[]), phase(3, &[1])],
    }];
    spec.traffic = vec![seed(1, cars(3), 1, &[1])];
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 4).unwrap();

    sim.advance(9).unwrap();
    assert_eq!(observe(&sim, 1).queue, [cars(3)]);
    assert_eq!(
        observe(&sim, 1).wait_reason,
        Some(WaitReason::DischargeLimit)
    );
    sim.advance(1).unwrap();
    assert_eq!(observe(&sim, 1).queue, [cars(2)]);
}

#[test]
fn tram_on_a_separate_track_uses_its_own_signal_and_travel_time() {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = vec![
        lane(1, 30_000),
        lane(2, 30_000),
        lane(3, 30_000),
        lane(4, 30_000),
    ];
    for track in &mut spec.lanes[2..] {
        track.allowed = vec![VehicleKind::Tram];
    }
    spec.lanes[3].travel_ticks = nz(3);
    spec.movements = vec![movement(1, 1, 2), movement(2, 3, 4)];
    spec.junctions = vec![JunctionSpec {
        id: JunctionId::new(1),
        phases: vec![phase(1, &[2])],
    }];
    let tram = QueueEntry::Vehicle {
        id: VehicleId::new(13),
        kind: VehicleKind::Tram,
        length_mm: nz(20_000),
    };
    spec.traffic = vec![seed(1, cars(1), 1, &[1]), seed(3, tram.clone(), 1, &[2])];
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 4).unwrap();

    sim.advance(1).unwrap();
    assert_eq!(observe(&sim, 1).queue, [cars(1)]);
    assert_eq!(observe(&sim, 4).queue, std::slice::from_ref(&tram));
    sim.advance(2).unwrap();
    assert_eq!(observe(&sim, 4).queue, [tram]);
    sim.advance(1).unwrap();
    assert!(observe(&sim, 4).queue.is_empty());
}

#[test]
fn conflicting_signal_plan_is_rejected() {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = vec![lane(1, 30_000), lane(2, 30_000), lane(3, 30_000)];
    spec.movements = vec![movement(1, 1, 3), movement(2, 2, 3)];
    spec.junctions = vec![JunctionSpec {
        id: JunctionId::new(1),
        phases: vec![phase(1, &[1, 2])],
    }];
    assert!(Simulation::new_game(spec, RuleSet::default(), 4).is_err());
}

#[test]
fn independent_movement_does_not_erase_contending_lanes_turn() {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = (1..=6).map(|id| lane(id, 60_000)).collect();
    for spec in &mut spec.lanes {
        spec.discharge.millimetres = nz(5_000);
    }
    spec.movements = vec![movement(1, 1, 4), movement(2, 2, 5), movement(3, 3, 6)];
    spec.movements[2].conflict_zones = vec![2];
    spec.junctions = vec![JunctionSpec {
        id: JunctionId::new(1),
        phases: vec![],
    }];
    spec.traffic = vec![
        seed(1, cars(4), 1, &[1]),
        seed(2, cars(4), 1, &[2]),
        seed(3, cars(4), 1, &[3]),
    ];
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 4).unwrap();

    sim.advance(2).unwrap();
    assert_eq!(observe(&sim, 2).queue, [cars(3)]);
    assert_eq!(observe(&sim, 3).queue, [cars(2)]);
}
