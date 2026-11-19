use std::num::NonZeroU32;

use sim_core::{
    Command, CommandError, DischargeRate, EditAction, EditCommand, EditError, JunctionId,
    JunctionSpec, LaneId, LaneObservation, LaneSpec, MovementId, MovementSpec, Observation, Query,
    QueueEntry, SignalPhase, Simulation, Tick, TurnPocket, VehicleId, VehicleKind, WaitReason,
};

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn bridge() -> Simulation {
    Simulation::from_scenario_json(include_bytes!("../../../scenarios/bridge-test.json")).unwrap()
}

fn lane(sim: &Simulation, id: u64) -> LaneObservation {
    let Observation::Lane(view) = sim.inspect(Query::Lane(LaneId::new(id))).unwrap() else {
        panic!("lane observation");
    };
    view
}

fn pocket(capacity_mm: u32) -> EditCommand {
    EditCommand {
        expected_revision: 0,
        actions: vec![
            EditAction::SetSignal(JunctionSpec {
                id: JunctionId::new(1),
                phases: vec![SignalPhase {
                    green_ticks: nz(1),
                    clearance_ticks: 0,
                    movements: vec![MovementId::new(2)],
                }],
            }),
            EditAction::SetSignal(JunctionSpec {
                id: JunctionId::new(2),
                phases: Vec::new(),
            }),
            EditAction::AddTurnPocket(TurnPocket {
                source: LaneId::new(1),
                lane: LaneSpec {
                    id: LaneId::new(4),
                    capacity_mm: nz(capacity_mm),
                    travel_ticks: nz(1),
                    discharge: DischargeRate {
                        millimetres: nz(30_000),
                        ticks: nz(1),
                    },
                    allowed: vec![VehicleKind::Car],
                },
                feeder: MovementSpec {
                    id: MovementId::new(3),
                    from: LaneId::new(1),
                    to: LaneId::new(4),
                    junction: JunctionId::new(2),
                    conflict_zones: Vec::new(),
                    priority: 0,
                },
                turns: vec![MovementId::new(1)],
            }),
        ],
    }
}

#[test]
fn full_turn_pocket_releases_bus_and_short_one_leaves_a_car_in_front() {
    for (space, moved) in [(15_000, 3), (10_000, 2)] {
        let mut sim = bridge();
        sim.advance(10).unwrap();
        let before = sim.fingerprint().unwrap();
        let edit = pocket(space);
        let preview = sim.preview(&edit).unwrap();
        assert_eq!(preview.relocated_vehicles, moved);
        assert!(preview.affected_lanes.contains(&LaneId::new(1)));
        assert_eq!(sim.fingerprint().unwrap(), before);
        let receipt = sim.apply(Command::CommitEdit(edit)).unwrap();
        assert_eq!(receipt.edit, Some(preview));
        assert_eq!(sim.network_revision(), 1);
        assert_eq!(
            lane(&sim, 4).queue,
            [QueueEntry::PrivateCars(nz(moved as u32))]
        );
        let mut resumed = Simulation::load(&sim.save().unwrap()).unwrap();
        resumed.advance(1).unwrap();
        sim.advance(1).unwrap();
        assert_eq!(resumed.fingerprint().unwrap(), sim.fingerprint().unwrap());
        if moved == 3 {
            assert!(matches!(lane(&resumed, 3).queue.as_slice(),
                [QueueEntry::Vehicle { id, .. }] if *id == VehicleId::new(12)));
        } else {
            assert_eq!(lane(&resumed, 1).queue[0], QueueEntry::PrivateCars(nz(1)));
            assert_eq!(lane(&resumed, 1).wait_reason, Some(WaitReason::TargetFull));
        }
    }
}

#[test]
fn commit_rechecks_occupancy_after_read_only_preview() {
    let mut sim = bridge();
    sim.advance(10).unwrap();
    let edit = EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::SetCapacity {
            lane: LaneId::new(2),
            capacity_mm: nz(10_000),
        }],
    };
    sim.preview(&edit).unwrap();
    sim.advance(1).unwrap();
    let before = sim.fingerprint().unwrap();
    assert!(matches!(sim.apply(Command::CommitEdit(edit)),
        Err(CommandError::Edit(EditError::Occupied(id))) if id == LaneId::new(2)));
    assert_eq!(sim.fingerprint().unwrap(), before);
    assert_eq!(sim.network_revision(), 0);
    assert_eq!(sim.apply(Command::SetPaused(true)).unwrap().sequence, 1);
}

#[test]
fn conflicting_signals_and_old_revisions_do_not_change_state() {
    let mut sim = bridge();
    let signal = EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::SetSignal(JunctionSpec {
            id: JunctionId::new(1),
            phases: vec![SignalPhase {
                green_ticks: nz(20),
                clearance_ticks: 1,
                movements: vec![MovementId::new(1), MovementId::new(2)],
            }],
        })],
    };
    assert!(matches!(
        sim.preview(&signal),
        Err(EditError::WouldInvalidate(_))
    ));
    let ok = EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::SetCapacity {
            lane: LaneId::new(2),
            capacity_mm: nz(90_000),
        }],
    };
    let first = sim.apply(Command::CommitEdit(ok.clone())).unwrap();
    assert_eq!(first.tick, Tick::new(0));
    let before = sim.fingerprint().unwrap();
    assert!(matches!(
        sim.apply(Command::CommitEdit(ok)),
        Err(CommandError::Edit(EditError::Outdated {
            expected: 0,
            current: 1
        }))
    ));
    assert_eq!(before, sim.fingerprint().unwrap());
}

fn commute_scenario() -> sim_core::Scenario {
    use sim_core::{DemandProfile, DischargeRate, Scenario, TripPurpose, ZoneId, ZoneSpec};

    let mut spec = Scenario::empty(Tick::new(0));
    let junction = JunctionId::new(1);
    spec.lanes = (1..=3)
        .map(|id| LaneSpec {
            id: LaneId::new(id),
            capacity_mm: nz(10_000),
            travel_ticks: nz(2),
            discharge: DischargeRate {
                millimetres: nz(5_000),
                ticks: nz(1),
            },
            allowed: vec![VehicleKind::Car],
        })
        .collect();
    spec.movements = vec![movement(1, 1, 2), movement(2, 2, 3)];
    spec.junctions = vec![JunctionSpec {
        id: junction,
        phases: Vec::new(),
    }];
    spec.demand.zones = vec![
        ZoneSpec {
            id: ZoneId::new(1),
            residents: 1,
            jobs: 0,
            shops: 0,
            road_access: Some(LaneId::new(1)),
        },
        ZoneSpec {
            id: ZoneId::new(2),
            residents: 0,
            jobs: 1,
            shops: 0,
            road_access: Some(LaneId::new(3)),
        },
    ];
    spec.demand.profiles = vec![DemandProfile {
        purpose: TripPurpose::Work,
        departure_offsets: vec![0],
        return_offset: 100,
        rate_numerator: 1,
        rate_denominator: nz(1),
    }];
    spec
}

fn active_commute() -> Simulation {
    Simulation::new_game(commute_scenario(), sim_core::RuleSet::default(), 42).unwrap()
}

fn movement(id: u64, from: u64, to: u64) -> MovementSpec {
    MovementSpec {
        id: MovementId::new(id),
        from: LaneId::new(from),
        to: LaneId::new(to),
        junction: JunctionId::new(1),
        conflict_zones: Vec::new(),
        priority: 0,
    }
}

#[test]
fn a_new_shortcut_preserves_the_valid_route_of_a_trip_already_underway() {
    let mut sim = active_commute();
    sim.advance(1).unwrap();
    sim.apply(Command::CommitEdit(EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::UpsertMovement(movement(3, 1, 3))],
    }))
    .unwrap();
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    loaded.advance(2).unwrap();
    assert_eq!(lane(&loaded, 2).queue, [QueueEntry::PrivateCars(nz(1))]);
    assert!(lane(&loaded, 3).queue.is_empty());
}

#[test]
fn closing_a_movement_reroutes_an_active_commute_when_an_alternative_exists() {
    let mut sim = active_commute();
    sim.advance(1).unwrap();
    let edit = EditCommand {
        expected_revision: 0,
        actions: vec![
            EditAction::RemoveMovement(MovementId::new(1)),
            EditAction::UpsertMovement(movement(3, 1, 3)),
        ],
    };
    let preview = sim.preview(&edit).unwrap();
    assert_eq!(preview.rerouted_vehicles, 1);
    sim.apply(Command::CommitEdit(edit)).unwrap();
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    loaded.advance(2).unwrap();
    assert!(lane(&loaded, 2).queue.is_empty());
    assert_eq!(lane(&loaded, 3).queue, [QueueEntry::PrivateCars(nz(1))]);
    loaded.advance(2).unwrap();
    let Observation::Demand(view) = loaded.inspect(Query::Demand).unwrap() else {
        panic!("demand");
    };
    assert_eq!((view.generated, view.in_transit, view.arrived), (1, 0, 1));
}

fn demand(sim: &Simulation) -> sim_core::DemandObservation {
    let Observation::Demand(view) = sim.inspect(Query::Demand).unwrap() else {
        panic!("demand");
    };
    assert_eq!(
        view.generated,
        view.waiting + view.in_transit + view.arrived
    );
    view
}

#[test]
fn severed_trip_holds_the_lane_and_resumes_after_the_connection_returns() {
    let mut sim = active_commute();
    sim.advance(1).unwrap();
    let receipt = sim
        .apply(Command::CommitEdit(EditCommand {
            expected_revision: 0,
            actions: vec![EditAction::RemoveMovement(MovementId::new(1))],
        }))
        .unwrap();
    assert_eq!(receipt.edit.unwrap().blocked_vehicles, 1);
    sim.advance(2).unwrap();
    assert_eq!(lane(&sim, 1).queue, [QueueEntry::PrivateCars(nz(1))]);
    assert_eq!(lane(&sim, 1).wait_reason, Some(WaitReason::Disconnected));
    assert_eq!(demand(&sim).arrived, 0);

    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.advance(1).unwrap();
    assert_eq!(
        lane(&restored, 1).wait_reason,
        Some(WaitReason::Disconnected)
    );
    restored
        .apply(Command::CommitEdit(EditCommand {
            expected_revision: 1,
            actions: vec![EditAction::UpsertMovement(movement(1, 1, 2))],
        }))
        .unwrap();
    restored.advance(5).unwrap();
    assert_eq!(demand(&restored).arrived, 1);
}

#[test]
fn private_cars_reach_the_exit_before_their_occupants_continue_on_foot() {
    use sim_core::{FootEdgeId, FootEdgeSpec, RuleSet, ZoneId, ZoneSpec};
    let mut spec = commute_scenario();
    let exit = ZoneId::new(3);
    let goal = ZoneId::new(2);
    spec.demand.zones.push(ZoneSpec {
        id: exit,
        residents: 0,
        jobs: 0,
        shops: 0,
        road_access: Some(LaneId::new(2)),
    });
    spec.demand.foot_edges.push(FootEdgeSpec {
        id: FootEdgeId::new(1),
        from: exit,
        to: goal,
        travel_ticks: nz(2),
        capacity: nz(3),
        discharge: nz(3),
        signal: None,
    });
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(1).unwrap();
    let receipt = sim
        .apply(Command::CommitEdit(EditCommand {
            expected_revision: 0,
            actions: vec![EditAction::RemoveMovement(MovementId::new(2))],
        }))
        .unwrap()
        .edit
        .unwrap();
    assert_eq!(receipt.vehicles_en_route_to_exit, 1);
    assert_eq!(
        demand(&sim).active[0].mode,
        Some(sim_core::JourneyMode::Car)
    );
    sim.advance(3).unwrap();
    assert_eq!(lane(&sim, 2).queue, [QueueEntry::PrivateCars(nz(1))]);
    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.advance(1).unwrap();
    assert!(lane(&restored, 2).queue.is_empty());
    assert_eq!(demand(&restored).arrived, 0);
    assert_eq!(demand(&restored).waiting, 1);
    assert_eq!(demand(&restored).active[0].location, exit);
    let mut after_exit = Simulation::load(&restored.save().unwrap()).unwrap();
    after_exit.advance(3).unwrap();
    let state = demand(&after_exit);
    assert_eq!(state.arrived, 1);
    assert_eq!(state.total_travel_ticks, 7);
    sim.advance(4).unwrap();
    assert_eq!(
        sim.fingerprint().unwrap(),
        after_exit.fingerprint().unwrap()
    );
}

#[test]
fn an_offroad_waiter_replans_when_a_foot_connection_is_built() {
    use sim_core::{FootEdgeId, FootEdgeSpec, RuleSet, ZoneId, ZoneSpec};
    let mut spec = commute_scenario();
    let exit = ZoneId::new(3);
    spec.demand.zones.push(ZoneSpec {
        id: exit,
        residents: 0,
        jobs: 0,
        shops: 0,
        road_access: Some(LaneId::new(2)),
    });
    let edge = FootEdgeSpec {
        id: FootEdgeId::new(1),
        from: exit,
        to: ZoneId::new(1),
        travel_ticks: nz(2),
        capacity: nz(3),
        discharge: nz(3),
        signal: None,
    };
    spec.demand.foot_edges.push(edge.clone());
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(1).unwrap();
    sim.apply(Command::CommitEdit(EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::RemoveMovement(MovementId::new(2))],
    }))
    .unwrap();
    sim.advance(7).unwrap();
    assert_eq!(demand(&sim).active[0].location, exit);
    assert_eq!(demand(&sim).waiting, 1);
    assert!(lane(&sim, 2).queue.is_empty());
    let mut resumed = Simulation::load(&sim.save().unwrap()).unwrap();
    resumed
        .apply(Command::CommitEdit(EditCommand {
            expected_revision: 1,
            actions: vec![EditAction::UpsertFootEdge(FootEdgeSpec {
                id: FootEdgeId::new(2),
                to: ZoneId::new(2),
                ..edge
            })],
        }))
        .unwrap();
    resumed.advance(3).unwrap();
    assert_eq!(demand(&resumed).arrived, 1);
    assert_eq!(demand(&resumed).generated, 1);
}
