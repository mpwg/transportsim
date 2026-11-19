use std::num::NonZeroU32;

use sim_core::{
    Command, DischargeRate, JunctionId, JunctionSpec, LaneId, LaneSpec, MovementId, MovementSpec,
    Observation, Query, QueueEntry, RuleSet, Scenario, SignalPhase, Simulation, Tick, TrafficSeed,
    VehicleId, VehicleKind, WaitReason,
};

fn bridge() -> Simulation {
    let mut scenario = Scenario::empty(Tick::new(9));
    let one = NonZeroU32::MIN;
    let movement = MovementId::new(1);
    let junction = JunctionId::new(1);
    let left = LaneId::new(1);
    let right = LaneId::new(2);
    scenario.lanes = [left, right]
        .map(|id| LaneSpec {
            id,
            capacity_mm: NonZeroU32::new(60_000).unwrap(),
            travel_ticks: one,
            discharge: DischargeRate {
                millimetres: NonZeroU32::new(12_000).unwrap(),
                ticks: one,
            },
            allowed: vec![VehicleKind::Bus],
        })
        .into();
    scenario.movements = vec![MovementSpec {
        id: movement,
        from: left,
        to: right,
        junction,
        conflict_zones: vec![1],
        priority: 0,
    }];
    scenario.junctions = vec![JunctionSpec {
        id: junction,
        phases: vec![
            SignalPhase {
                green_ticks: NonZeroU32::new(6).unwrap(),
                clearance_ticks: 0,
                movements: vec![movement],
            },
            SignalPhase {
                green_ticks: NonZeroU32::new(6).unwrap(),
                clearance_ticks: 0,
                movements: vec![],
            },
        ],
    }];
    scenario.traffic = vec![TrafficSeed {
        lane: left,
        entry: QueueEntry::Vehicle {
            id: VehicleId::new(12),
            kind: VehicleKind::Bus,
            length_mm: NonZeroU32::new(12_000).unwrap(),
        },
        ready_at: Tick::new(10),
        route: vec![movement],
    }];
    Simulation::new_game(scenario, RuleSet::default(), 42).unwrap()
}

#[test]
fn restart_during_red_preserves_first_legal_crossing() {
    let mut uninterrupted = bridge();
    uninterrupted.advance(1).unwrap();
    let source = Query::Lane(LaneId::new(1));
    let Observation::Lane(view) = uninterrupted.inspect(source).unwrap() else {
        panic!("lane observation required");
    };
    assert_eq!(view.wait_reason, Some(WaitReason::RedSignal));
    let bytes = uninterrupted.save().unwrap();
    let fingerprint = uninterrupted.fingerprint().unwrap();
    let _ = uninterrupted.inspect(Query::Clock).unwrap();
    assert_eq!(bytes, uninterrupted.save().unwrap());
    assert_eq!(fingerprint, uninterrupted.fingerprint().unwrap());
    let mut restored = Simulation::load(&bytes).unwrap();

    for _ in 0..3 {
        uninterrupted.advance(1).unwrap();
    }
    restored.advance(3).unwrap();
    let target = || Query::Lane(LaneId::new(2));
    let Observation::Lane(view) = restored.inspect(target()).unwrap() else {
        panic!("lane observation required");
    };
    assert_eq!(view.tick, Tick::new(13));
    let Observation::Lane(source) = restored.inspect(Query::Lane(LaneId::new(1))).unwrap() else {
        panic!("lane observation required");
    };
    let event = source.departure.expect("bus departure at tick 13");
    assert_eq!(event.to, Some(LaneId::new(2)));
    assert!(matches!(event.entry, QueueEntry::Vehicle { id, .. } if id == VehicleId::new(12)));
    let reloaded_boundary = Simulation::load(&restored.save().unwrap()).unwrap();
    assert_eq!(
        reloaded_boundary.inspect(Query::Lane(LaneId::new(1))),
        restored.inspect(Query::Lane(LaneId::new(1)))
    );

    assert!(matches!(view.queue.as_slice(),
        [QueueEntry::Vehicle { id, .. }] if *id == VehicleId::new(12)));
    assert_eq!(restored.inspect(target()), uninterrupted.inspect(target()));
    assert_eq!(
        restored.fingerprint().unwrap(),
        uninterrupted.fingerprint().unwrap()
    );
}

#[test]
fn save_after_command_preserves_sequence_and_both_pauses() {
    let mut simulation = bridge();
    simulation.apply(Command::SetGrowthPaused(true)).unwrap();
    simulation.apply(Command::SetPaused(true)).unwrap();
    let mut restored = Simulation::load(&simulation.save().unwrap()).unwrap();
    assert_eq!(restored.advance(3).unwrap().executed_ticks, 0);
    assert_eq!(restored.step().unwrap().end, Tick::new(10));
    let Observation::Clock(clock) = restored.inspect(Query::Clock).unwrap() else {
        panic!("clock observation required");
    };
    assert!(clock.paused && clock.growth_paused);
    assert_eq!(clock.development_ticks, 0);
    assert_eq!(
        restored.apply(Command::SetPaused(false)).unwrap().sequence,
        3
    );
}

#[test]
fn restart_keeps_fractional_discharge_and_rotating_priority() {
    let mut scenario = Scenario::empty(Tick::new(0));
    let nz = |value| NonZeroU32::new(value).unwrap();
    scenario.lanes = (1..=3)
        .map(|id| LaneSpec {
            id: LaneId::new(id),
            capacity_mm: nz(30_000),
            travel_ticks: nz(1),
            discharge: DischargeRate {
                millimetres: nz(5_000),
                ticks: nz(2),
            },
            allowed: vec![VehicleKind::Car],
        })
        .collect();
    let junction = JunctionId::new(1);
    scenario.junctions = vec![JunctionSpec {
        id: junction,
        phases: vec![],
    }];
    for id in 1..=2 {
        let movement = MovementId::new(id);
        let from = LaneId::new(id);
        scenario.movements.push(MovementSpec {
            id: movement,
            from,
            to: LaneId::new(3),
            junction,
            conflict_zones: vec![1],
            priority: 0,
        });
        scenario.traffic.push(TrafficSeed {
            lane: from,
            entry: QueueEntry::PrivateCars(nz(4)),
            ready_at: Tick::new(1),
            route: vec![movement],
        });
    }
    let mut original = Simulation::new_game(scenario, RuleSet::default(), 42).unwrap();
    original.advance(2).unwrap();
    let mut restored = Simulation::load(&original.save().unwrap()).unwrap();
    let queue = |sim: &Simulation, id| {
        let Observation::Lane(view) = sim.inspect(Query::Lane(LaneId::new(id))).unwrap() else {
            panic!("lane observation required");
        };
        view.queue
    };
    assert_eq!(queue(&restored, 1), [QueueEntry::PrivateCars(nz(3))]);
    assert_eq!(queue(&restored, 2), [QueueEntry::PrivateCars(nz(4))]);

    for (a, b) in [(3, 3), (2, 3), (2, 2)] {
        original.advance(1).unwrap();
        restored.advance(1).unwrap();
        assert_eq!(queue(&restored, 1), [QueueEntry::PrivateCars(nz(a))]);
        assert_eq!(queue(&restored, 2), [QueueEntry::PrivateCars(nz(b))]);
        assert_eq!(
            restored.fingerprint().unwrap(),
            original.fingerprint().unwrap()
        );
    }
}

#[test]
fn correctly_signed_unreachable_junction_cursor_is_rejected() {
    let mut sim = bridge();
    sim.advance(4).unwrap();
    let bytes = sim.save().unwrap();
    let mut document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    document["payload"]["junctions"][0]["last_granted"][0][1] = 2.into();
    let payload = serde_json::to_vec(&document["payload"]).unwrap();
    document["checksum"] = blake3::hash(&payload).to_hex().as_str().into();
    let corrupted = serde_json::to_vec(&document).unwrap();
    assert!(matches!(
        Simulation::load(&corrupted),
        Err(sim_core::LoadError::InvalidState {
            field: "junctions.last_granted"
        })
    ));
    assert_eq!(sim.save().unwrap(), bytes);
}

#[test]
fn independent_scenario_table_order_does_not_change_fingerprint() {
    let mut scenario = Scenario::empty(Tick::new(0));
    scenario.building_ids = [3, 1, 2]
        .into_iter()
        .map(sim_core::BuildingId::new)
        .collect();
    let first = Simulation::new_game(scenario.clone(), RuleSet::default(), 7).unwrap();
    scenario.building_ids.reverse();
    let second = Simulation::new_game(scenario, RuleSet::default(), 7).unwrap();
    assert_eq!(first.fingerprint().unwrap(), second.fingerprint().unwrap());
}
