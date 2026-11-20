use std::num::NonZeroU32;

use sim_core::{
    Command, DischargeRate, EditAction, EditCommand, FleetSpec, JunctionId, JunctionSpec, LaneId,
    LaneSpec, LineId, LineSpec, MovementId, MovementSpec, Observation, Query, QueueEntry, RuleSet,
    Scenario, ServiceSpec, Simulation, StopId, StopSpec, Tick, TrafficSeed, VehicleId, VehicleKind,
    ZoneId, ZoneSpec,
};

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

fn scenario(fleet: bool) -> Scenario {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = [1, 2]
        .map(|id| LaneSpec {
            id: LaneId::new(id),
            capacity_mm: nz(60_000),
            travel_ticks: nz(1),
            discharge: DischargeRate {
                millimetres: nz(60_000),
                ticks: nz(1),
            },
            allowed: vec![VehicleKind::Bus, VehicleKind::Car],
        })
        .into();
    let junction = JunctionId::new(1);
    spec.junctions.push(JunctionSpec {
        id: junction,
        phases: vec![],
    });
    spec.movements = [(1, 2), (2, 1)]
        .into_iter()
        .map(|(from, to)| MovementSpec {
            id: MovementId::new(from),
            from: LaneId::new(from),
            to: LaneId::new(to),
            junction,
            conflict_zones: vec![1],
            priority: 0,
        })
        .collect();
    spec.demand.zones = [1, 2]
        .map(|id| ZoneSpec {
            id: ZoneId::new(id),
            residents: 0,
            jobs: 0,
            shops: 0,
            road_access: None,
        })
        .into();
    spec.service = ServiceSpec {
        stops: [1, 2]
            .map(|id| StopSpec {
                id: StopId::new(id),
                zone: ZoneId::new(id),
                lane: LaneId::new(id),
                capacity: nz(20),
                minimum_dwell: nz(3),
                passengers_per_tick: nz(16),
            })
            .into(),
        lines: vec![LineSpec {
            id: LineId::new(1),
            kind: VehicleKind::Bus,
            stops: vec![StopId::new(1), StopId::new(2)],
            headway: nz(20),
            first_departure: Tick::new(1),
        }],
        fleet: if fleet {
            vec![FleetSpec {
                id: VehicleId::new(12),
                line: LineId::new(1),
                length_mm: nz(12_000),
                capacity: nz(8),
            }]
        } else {
            vec![]
        },
    };
    spec
}

fn lane(sim: &Simulation, id: u64) -> sim_core::LaneObservation {
    let Observation::Lane(view) = sim.inspect(Query::Lane(LaneId::new(id))).unwrap() else {
        panic!("expected lane");
    };
    view
}
fn service(sim: &Simulation) -> sim_core::ServiceObservation {
    let Observation::Service(view) = sim.inspect(Query::Service).unwrap() else {
        panic!("expected service");
    };
    view
}

fn waypoint_scenario(capacity: u32) -> Scenario {
    let mut spec = scenario(true);
    let mut waypoint = spec.lanes[0].clone();
    waypoint.id = LaneId::new(3);
    waypoint.capacity_mm = nz(capacity);
    waypoint.travel_ticks = nz(5);
    spec.lanes.push(waypoint);
    spec.movements
        .extend([(3, 1, 3), (4, 3, 2)].map(|(id, from, to)| MovementSpec {
            id: MovementId::new(id),
            from: LaneId::new(from),
            to: LaneId::new(to),
            ..spec.movements[0].clone()
        }));
    spec
}

fn set_waypoint(sim: &Simulation, waypoint: u64) -> EditCommand {
    EditCommand {
        expected_revision: sim.network_revision(),
        actions: vec![EditAction::SetLineWaypoints {
            line: LineId::new(1),
            legs: vec![vec![LaneId::new(waypoint)], vec![]],
        }],
    }
}

#[test]
fn line_route_preview_matches_waypoint_route_after_save_and_load() {
    let mut sim = Simulation::new_game(waypoint_scenario(30_000), RuleSet::default(), 42).unwrap();
    let before = sim.fingerprint().unwrap();
    let waypoint = set_waypoint(&sim, 3);
    let lane_id = LaneId::new;
    let through = vec![lane_id(1), lane_id(3), lane_id(2)];
    assert_eq!(
        sim.preview_vehicle_route_through(&through, VehicleKind::Bus, nz(12_000)),
        Some(through.clone())
    );
    assert_eq!(
        sim.preview_line_route(&waypoint, LineId::new(1)).unwrap(),
        Some(vec![through, vec![lane_id(2), lane_id(1)]])
    );
    assert_eq!(sim.fingerprint().unwrap(), before);

    sim.apply(Command::CommitEdit(waypoint)).unwrap();
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    assert_eq!(loaded.fingerprint().unwrap(), sim.fingerprint().unwrap());
    let Observation::TimetableJourney(Some(itinerary)) = loaded
        .inspect(Query::TimetableJourney {
            from: ZoneId::new(1),
            to: ZoneId::new(2),
        })
        .unwrap()
    else {
        panic!("expected a timetable journey");
    };
    assert_eq!(itinerary.cost.ride_ticks, 6);
    loaded.advance(6).unwrap();
    sim.advance(6).unwrap();
    assert_eq!(loaded.fingerprint().unwrap(), sim.fingerprint().unwrap());
    assert_eq!(lane(&loaded, 3).vehicles[0].id, VehicleId::new(12));
    assert!(lane(&loaded, 2).vehicles.is_empty());
}

#[test]
fn line_waypoint_must_exist_and_fit_the_longest_vehicle() {
    let sim = Simulation::new_game(waypoint_scenario(9_000), RuleSet::default(), 42).unwrap();
    assert_eq!(
        sim.preview_vehicle_route_through(
            &[LaneId::new(1), LaneId::new(3), LaneId::new(2)],
            VehicleKind::Bus,
            nz(12_000)
        ),
        None,
    );
    for invalid in [3, 99] {
        let error = sim
            .preview_line_route(&set_waypoint(&sim, invalid), LineId::new(1))
            .unwrap_err();
        assert!(matches!(
            error,
            sim_core::EditError::WouldInvalidate(sim_core::LoadError::State(
                sim_core::ValidationError::Service(sim_core::ServiceError::InvalidLine(line))
            )) if line == LineId::new(1)
        ));
    }
}

#[test]
fn closing_a_waypoint_cannot_divert_a_bus_that_already_started_its_leg() {
    let mut spec = waypoint_scenario(30_000);
    spec.junctions[0].phases.push(sim_core::SignalPhase {
        green_ticks: nz(100),
        clearance_ticks: 0,
        movements: vec![MovementId::new(1)],
    });
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.apply(Command::CommitEdit(set_waypoint(&sim, 3)))
        .unwrap();
    sim.advance(6).unwrap();
    assert_eq!(lane(&sim, 1).vehicles[0].id, VehicleId::new(12));
    assert_eq!(service(&sim).vehicles[0].next_stop, Some(StopId::new(2)));
    let close = EditCommand {
        expected_revision: sim.network_revision(),
        actions: vec![EditAction::RemoveMovement(MovementId::new(3))],
    };
    let before = sim.fingerprint().unwrap();
    assert!(matches!(
        sim.apply(Command::CommitEdit(close)),
        Err(sim_core::CommandError::Edit(
            sim_core::EditError::WouldInvalidate(sim_core::LoadError::InvalidState {
                field: "service.active_vehicle_route"
            })
        ))
    ));
    assert_eq!(sim.fingerprint().unwrap(), before);
}

#[test]
fn an_idle_line_keeps_its_waypoint_after_the_road_is_closed_and_reopened() {
    let spec = waypoint_scenario(30_000);
    let closed = spec.movements[2].clone();
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    let save = sim.save().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&save).unwrap();
    let scenario = json["payload"]["scenario"].as_object().unwrap();
    assert!(!scenario.contains_key("line_waypoints"));

    sim.apply(Command::CommitEdit(set_waypoint(&sim, 3)))
        .unwrap();
    let close = EditCommand {
        expected_revision: sim.network_revision(),
        actions: vec![EditAction::RemoveMovement(closed.id)],
    };
    sim.apply(Command::CommitEdit(close)).unwrap();
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    assert!(service(&loaded).lines[0].disconnected);

    let reopen = EditCommand {
        expected_revision: loaded.network_revision(),
        actions: vec![EditAction::UpsertMovement(closed)],
    };
    assert_eq!(
        loaded.preview_line_route(&reopen, LineId::new(1)).unwrap(),
        Some(vec![
            [1, 3, 2].map(LaneId::new).into(),
            [2, 1].map(LaneId::new).into()
        ])
    );
    loaded.apply(Command::CommitEdit(reopen)).unwrap();
    assert!(!service(&loaded).lines[0].disconnected);
}

#[test]
fn bus_holds_up_following_cars_and_resumes_exactly_after_reload() {
    let mut spec = scenario(true);
    spec.traffic.push(TrafficSeed {
        lane: LaneId::new(2),
        entry: QueueEntry::PrivateCars(nz(1)),
        ready_at: Tick::new(1),
        route: vec![MovementId::new(2)],
    });
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(4).unwrap();
    assert!(matches!(lane(&sim, 1).queue.as_slice(),
        [QueueEntry::Vehicle { id, .. }, QueueEntry::PrivateCars(_)] if *id == VehicleId::new(12)
    ));
    let mut resumed = Simulation::load(&sim.save().unwrap()).unwrap();
    resumed.advance(1).unwrap();
    sim.advance(1).unwrap();
    assert!(
        matches!(lane(&resumed, 2).queue.as_slice(), [QueueEntry::Vehicle { id, .. }] if *id == VehicleId::new(12))
    );
    assert_eq!(lane(&resumed, 1).queue, [QueueEntry::PrivateCars(nz(1))]);
    assert_eq!(resumed.fingerprint().unwrap(), sim.fingerprint().unwrap());
}

#[test]
fn empty_fleet_records_expired_departures() {
    let mut sim = Simulation::new_game(scenario(false), RuleSet::default(), 42).unwrap();
    sim.advance(21).unwrap();
    let status = service(&sim);
    assert!(status.vehicles.is_empty());
    assert_eq!(status.lines[0].cancelled, 1);
    assert_eq!(status.lines[0].pending, Some(Tick::new(21)));
}

#[test]
fn release_from_the_stop_is_not_counted_until_the_bus_moves() {
    use sim_core::SignalPhase;
    let mut spec = scenario(true);
    spec.junctions[0].phases = vec![
        SignalPhase {
            green_ticks: nz(7),
            clearance_ticks: 0,
            movements: vec![],
        },
        SignalPhase {
            green_ticks: nz(1),
            clearance_ticks: 0,
            movements: vec![MovementId::new(1)],
        },
    ];
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(5).unwrap();
    assert_eq!(service(&sim).lines[0].departed, 0);
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    loaded.advance(3).unwrap();
    assert_eq!(service(&loaded).lines[0].last_departure, Some(Tick::new(8)));
    assert_eq!(service(&loaded).lines[0].last_delay, Some(7));
}

#[test]
fn timetable_reports_access_wait_and_travel_time_for_the_available_fleet() {
    let mut sim = Simulation::new_game(scenario(true), RuleSet::default(), 42).unwrap();
    let query = || Query::TimetableJourney {
        from: ZoneId::new(1),
        to: ZoneId::new(2),
    };
    let Observation::TimetableJourney(Some(itinerary)) = sim.inspect(query()).unwrap() else {
        panic!("expected bus itinerary");
    };
    assert_eq!(itinerary.first_line, LineId::new(1));
    assert_eq!(itinerary.cost.wait_ticks, 5);
    assert_eq!(itinerary.cost.ride_ticks, 1);
    sim.advance(5).unwrap();
    let Observation::TimetableJourney(Some(itinerary)) = sim.inspect(query()).unwrap() else {
        panic!("expected bus itinerary");
    };
    assert_eq!(itinerary.cost.wait_ticks, 0);
}

fn add_transit_demand(spec: &mut Scenario, residents: u32, destination: u64) {
    use sim_core::{DemandProfile, TripPurpose};

    spec.demand.zones[0].residents = residents;
    spec.demand
        .zones
        .iter_mut()
        .find(|zone| zone.id == ZoneId::new(destination))
        .unwrap()
        .jobs = residents;
    spec.demand.walk_weight = 0;
    spec.demand.car_weight = 0;
    spec.demand.profiles.push(DemandProfile {
        purpose: TripPurpose::Work,
        departure_offsets: vec![0],
        return_offset: 1000,
        rate_numerator: 1,
        rate_denominator: nz(1),
    });
}

fn ledger(sim: &Simulation) -> sim_core::DemandObservation {
    let Observation::Demand(view) = sim.inspect(Query::Demand).unwrap() else {
        panic!("demand");
    };
    assert_eq!(
        view.generated,
        view.arrived + view.in_transit + view.waiting
    );
    view
}

#[test]
fn eight_board_and_four_keep_their_original_waiting_age_after_reload() {
    let mut spec = scenario(true);
    add_transit_demand(&mut spec, 12, 2);
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(2).unwrap();
    assert_eq!(service(&sim).vehicles[0].onboard, 8);
    assert_eq!(service(&sim).stops[0].waiting_total, 4);
    assert_eq!(
        service(&sim).stops[0].oldest_waiting_since,
        Some(Tick::new(1))
    );

    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.advance(5).unwrap();
    sim.advance(5).unwrap();
    assert_eq!(ledger(&restored).arrived, 8);
    assert_eq!(service(&restored).stops[0].waiting_total, 4);
    assert_eq!(restored.fingerprint().unwrap(), sim.fingerprint().unwrap());
}

fn transfer_scenario(second_start: u64) -> Scenario {
    use sim_core::{FootEdgeId, FootEdgeSpec};

    let mut spec = scenario(true);
    let new_lanes: Vec<_> = spec
        .lanes
        .iter()
        .cloned()
        .map(|mut lane| {
            lane.id = LaneId::new(lane.id.get() + 2);
            lane.allowed = vec![VehicleKind::Tram];
            lane
        })
        .collect();
    spec.lanes.extend(new_lanes);
    let connections: Vec<_> = spec
        .movements
        .iter()
        .cloned()
        .map(|mut movement| {
            movement.id = MovementId::new(movement.id.get() + 2);
            movement.from = LaneId::new(movement.from.get() + 2);
            movement.to = LaneId::new(movement.to.get() + 2);
            movement.conflict_zones = vec![2];
            movement
        })
        .collect();
    spec.movements.extend(connections);
    for id in [3, 4] {
        spec.demand.zones.push(ZoneSpec {
            id: ZoneId::new(id),
            residents: 0,
            jobs: 0,
            shops: 0,
            road_access: None,
        });
        let mut stop = spec.service.stops[0].clone();
        stop.id = StopId::new(id);
        stop.lane = LaneId::new(id);
        stop.zone = ZoneId::new(id);
        spec.service.stops.push(stop);
    }
    spec.service.lines.push(LineSpec {
        id: LineId::new(2),
        kind: VehicleKind::Tram,
        stops: vec![StopId::new(3), StopId::new(4)],
        headway: nz(20),
        first_departure: Tick::new(second_start),
    });
    spec.service.fleet.push(FleetSpec {
        id: VehicleId::new(21),
        line: LineId::new(2),
        length_mm: nz(20_000),
        capacity: nz(8),
    });
    spec.demand.foot_edges.push(FootEdgeSpec {
        id: FootEdgeId::new(1),
        from: ZoneId::new(2),
        to: ZoneId::new(3),
        travel_ticks: nz(2),
        capacity: nz(10),
        discharge: nz(10),
        signal: None,
    });
    add_transit_demand(&mut spec, 4, 4);
    spec
}

#[test]
fn two_tick_transfer_catches_the_aligned_tram_but_misses_the_earlier_one() {
    let mut aligned = Simulation::new_game(transfer_scenario(5), RuleSet::default(), 42).unwrap();
    let mut earlier = Simulation::new_game(transfer_scenario(4), RuleSet::default(), 42).unwrap();

    aligned.advance(8).unwrap();
    assert_eq!(ledger(&aligned).transit_transferring, 4);
    assert_eq!(service(&aligned).stops[2].waiting_total, 0);
    assert_eq!(service(&aligned).vehicles[1].onboard, 0);
    let mut restored = Simulation::load(&aligned.save().unwrap()).unwrap();
    restored.advance(1).unwrap();
    assert_eq!(ledger(&restored).transit_transferring, 0);
    assert_eq!(service(&restored).vehicles[1].onboard, 4);
    earlier.advance(9).unwrap();
    assert_eq!(service(&earlier).vehicles[1].onboard, 0);
    assert_eq!(service(&earlier).stops[2].waiting_total, 4);

    restored.advance(2).unwrap();
    assert_eq!(ledger(&restored).arrived, 4);
    assert_eq!(ledger(&restored).total_travel_ticks, 40);
    aligned.advance(3).unwrap();
    assert_eq!(
        restored.fingerprint().unwrap(),
        aligned.fingerprint().unwrap()
    );
    earlier.advance(21).unwrap();
    assert_eq!(ledger(&earlier).arrived, 4);
    assert!(ledger(&earlier).total_travel_ticks > ledger(&restored).total_travel_ticks);
}

#[test]
fn same_zone_arrival_can_feed_another_vehicle_at_its_departure_tick() {
    let mut spec = transfer_scenario(3);
    spec.demand.foot_edges.clear();
    spec.service.stops[2].zone = ZoneId::new(2);
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(7).unwrap();
    assert_eq!(service(&sim).vehicles[1].onboard, 4);
    sim.advance(2).unwrap();
    assert_eq!(ledger(&sim).arrived, 4);
}

#[test]
fn headway_change_keeps_the_pending_order_and_resumes_the_new_schedule() {
    let mut sim = Simulation::new_game(scenario(false), RuleSet::default(), 42).unwrap();
    sim.advance(1).unwrap();
    sim.apply(sim_core::Command::SetLineHeadway {
        line: LineId::new(1),
        ticks: nz(3),
    })
    .unwrap();
    assert_eq!(service(&sim).lines[0].pending, Some(Tick::new(1)));
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    loaded.advance(2).unwrap();
    assert_eq!(service(&loaded).lines[0].pending, Some(Tick::new(1)));
    loaded.advance(1).unwrap();
    assert_eq!(service(&loaded).lines[0].pending, Some(Tick::new(4)));
    assert_eq!(service(&loaded).lines[0].cancelled, 1);
    assert_eq!(service(&loaded).lines[0].headway, 3);
}
