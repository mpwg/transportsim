use std::num::NonZeroU32;

use sim_core::{
    DemandProfile, DemandSpec, FootEdgeId, FootEdgeSpec, JourneyMode, Observation, Query, RuleSet,
    Scenario, Simulation, Tick, TripPurpose, ZoneId, ZoneSpec,
};

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn commute(residents: u32, jobs: u32, numerator: u32, denominator: u32) -> Scenario {
    let mut scenario = Scenario::empty(Tick::new(0));
    let home = ZoneId::new(1);
    let work = ZoneId::new(2);
    scenario.demand = DemandSpec {
        zones: vec![
            ZoneSpec {
                id: home,
                residents,
                jobs: 0,
                shops: 0,
                road_access: None,
            },
            ZoneSpec {
                id: work,
                residents: 0,
                jobs,
                shops: 0,
                road_access: None,
            },
        ],
        foot_edges: [(home, work), (work, home)]
            .into_iter()
            .enumerate()
            .map(|(index, (from, to))| FootEdgeSpec {
                id: FootEdgeId::new(index as u64 + 1),
                from,
                to,
                travel_ticks: nz(3),
                capacity: nz(20),
                discharge: nz(20),
                signal: None,
            })
            .collect(),
        profiles: vec![DemandProfile {
            purpose: TripPurpose::Work,
            departure_offsets: vec![0, 1],
            return_offset: 2,
            rate_numerator: numerator,
            rate_denominator: nz(denominator),
        }],
        ..DemandSpec::default()
    };
    scenario
}

fn demand(sim: &Simulation) -> sim_core::DemandObservation {
    let Observation::Demand(view) = sim.inspect(Query::Demand).unwrap() else {
        panic!("expected demand observation");
    };
    assert_eq!(
        view.generated,
        view.waiting + view.in_transit + view.arrived
    );
    view
}

#[test]
fn half_reisewunsch_is_carried_across_windows_and_save() {
    let mut sim = Simulation::new_game(commute(1, 3, 1, 2), RuleSet::default(), 42).unwrap();
    sim.advance(1).unwrap();
    assert_eq!(demand(&sim).generated, 0);
    assert_eq!(demand(&sim).scheduled_departures, 1);
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    loaded.advance(1).unwrap();

    let state = demand(&loaded);
    assert_eq!(state.generated, 1);
    assert_eq!(state.in_transit, 1);
    assert_eq!(state.active[0].mode, Some(JourneyMode::Walk));
    sim.advance(1).unwrap();
    assert_eq!(loaded.fingerprint().unwrap(), sim.fingerprint().unwrap());
}

#[test]
fn work_capacity_and_late_return_use_real_arrivals() {
    let mut scenario = commute(10, 6, 1, 1);
    scenario.demand.profiles[0].departure_offsets.pop();
    scenario.demand.foot_edges[0].capacity = nz(4);
    let mut sim = Simulation::new_game(scenario, RuleSet::default(), 42).unwrap();
    sim.advance(3).unwrap();
    let before = demand(&sim);
    assert_eq!(
        (
            before.generated,
            before.waiting,
            before.in_transit,
            before.arrived
        ),
        (10, 6, 4, 0)
    );
    assert_eq!(before.unassigned, 4);
    assert_eq!(before.pending_returns, 0);

    sim.advance(1).unwrap();
    let arrived = demand(&sim);
    assert_eq!(arrived.arrived, 4);
    assert_eq!(arrived.pending_returns, 4);
    assert_eq!(arrived.generated, 10);

    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    loaded.advance(1).unwrap();
    let first_returns = demand(&loaded);
    assert_eq!(first_returns.generated, 14);
    assert!(
        first_returns
            .active
            .iter()
            .any(|batch| batch.purpose == TripPurpose::Home
                && batch.origin == ZoneId::new(2)
                && batch.destination == Some(ZoneId::new(1))
                && batch.in_transit == 4)
    );
    loaded.advance(4).unwrap();
    assert_eq!(demand(&loaded).generated, 16);
    assert_eq!(demand(&loaded).unassigned, 4);
    sim.advance(5).unwrap();
    assert_eq!(loaded.fingerprint().unwrap(), sim.fingerprint().unwrap());
}

#[test]
fn modes_share_whole_trips_and_a_full_road_keeps_the_rest_at_origin() {
    use sim_core::{DischargeRate, LaneId, LaneSpec, VehicleKind};

    let mut scenario = commute(6, 6, 1, 1);
    scenario.demand.profiles[0].departure_offsets.pop();
    scenario.demand.zones[0].road_access = Some(LaneId::new(7));
    scenario.demand.zones[1].road_access = Some(LaneId::new(7));
    scenario.demand.walk_weight = 1;
    scenario.demand.car_weight = 2;
    scenario.lanes.push(LaneSpec {
        id: LaneId::new(7),
        capacity_mm: nz(5_000),
        travel_ticks: nz(3),
        discharge: DischargeRate {
            millimetres: nz(5_000),
            ticks: nz(1),
        },
        allowed: vec![VehicleKind::Car],
    });
    let mut sim = Simulation::new_game(scenario, RuleSet::default(), 42).unwrap();
    let Observation::Journeys(options) = sim
        .inspect(Query::Journeys {
            from: ZoneId::new(1),
            to: ZoneId::new(2),
        })
        .unwrap()
    else {
        panic!("expected journey options");
    };
    assert_eq!(options[0].mode, JourneyMode::Walk);
    assert_eq!(options[0].access_ticks, 3);
    assert_eq!(options[1].mode, JourneyMode::Car);
    assert_eq!(options[1].ride_ticks, 3);
    sim.advance(1).unwrap();

    let observation = demand(&sim);
    let car = observation
        .active
        .iter()
        .find(|batch| batch.mode == Some(JourneyMode::Car))
        .unwrap();
    let walk = observation
        .active
        .iter()
        .find(|batch| batch.mode == Some(JourneyMode::Walk))
        .unwrap();
    assert_eq!((car.waiting, car.in_transit), (3, 1));
    assert_eq!((walk.waiting, walk.in_transit), (0, 2));

    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.advance(4).unwrap();
    sim.advance(4).unwrap();
    assert_eq!(restored.fingerprint().unwrap(), sim.fingerprint().unwrap());
    assert!(demand(&restored).total_travel_ticks >= 9);
}

#[test]
fn completed_planning_window_reports_missed_time_at_source() {
    use sim_core::{DischargeRate, LaneId, LaneSpec, VehicleKind};

    let mut scenario = commute(3, 3, 1, 1);
    let lane = LaneId::new(1);
    for zone in &mut scenario.demand.zones {
        zone.road_access = Some(lane);
    }
    scenario.demand.walk_weight = 0;
    scenario.demand.planning_window_ticks = nz(6);
    scenario.demand.profiles[0].departure_offsets = vec![0];
    scenario.demand.profiles[0].return_offset = 100;
    scenario.lanes.push(LaneSpec {
        id: lane,
        capacity_mm: nz(5_000),
        travel_ticks: nz(1),
        discharge: DischargeRate {
            millimetres: nz(5_000),
            ticks: nz(1),
        },
        allowed: vec![VehicleKind::Car],
    });
    let mut sim = Simulation::new_game(scenario, RuleSet::default(), 42).unwrap();
    let cost = |sim: &Simulation| {
        let Observation::Journeys(options) = sim
            .inspect(Query::Journeys {
                from: ZoneId::new(1),
                to: ZoneId::new(2),
            })
            .unwrap()
        else {
            panic!("expected journey options");
        };
        options[0].wait_ticks
    };
    sim.advance(6).unwrap();
    assert_eq!(demand(&sim).arrived, 3);
    assert_eq!(demand(&sim).total_travel_ticks, 9);
    assert_eq!(cost(&sim), 0);
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    loaded.advance(1).unwrap();
    sim.advance(1).unwrap();
    assert_eq!(cost(&loaded), 2);
    assert_eq!(loaded.fingerprint().unwrap(), sim.fingerprint().unwrap());
}

#[test]
fn a_signalized_foot_crossing_stays_blocked_until_green_across_reload() {
    use sim_core::{
        DischargeRate, JunctionId, JunctionSpec, LaneId, LaneSpec, MovementId, MovementSpec,
        SignalPhase, VehicleKind,
    };

    let mut scenario = commute(3, 3, 1, 1);
    scenario.demand.profiles[0].departure_offsets = vec![0];
    scenario.demand.profiles[0].return_offset = 20;
    scenario.demand.foot_edges[0].travel_ticks = nz(1);
    scenario.demand.foot_edges[0].signal = Some(MovementId::new(1));
    scenario.lanes = [1, 2]
        .map(|id| LaneSpec {
            id: LaneId::new(id),
            capacity_mm: nz(5_000),
            travel_ticks: nz(1),
            discharge: DischargeRate {
                millimetres: nz(5_000),
                ticks: nz(1),
            },
            allowed: vec![VehicleKind::Car],
        })
        .into();
    let junction = JunctionId::new(1);
    scenario.movements.push(MovementSpec {
        id: MovementId::new(1),
        from: LaneId::new(1),
        to: LaneId::new(2),
        junction,
        conflict_zones: vec![1],
        priority: 0,
    });
    scenario.junctions.push(JunctionSpec {
        id: junction,
        phases: vec![
            SignalPhase {
                green_ticks: nz(4),
                clearance_ticks: 0,
                movements: vec![],
            },
            SignalPhase {
                green_ticks: nz(1),
                clearance_ticks: 0,
                movements: vec![MovementId::new(1)],
            },
        ],
    });
    let mut sim = Simulation::new_game(scenario, RuleSet::default(), 42).unwrap();
    sim.advance(4).unwrap();
    assert_eq!(demand(&sim).arrived, 0);
    assert_eq!(demand(&sim).in_transit, 3);
    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.advance(1).unwrap();
    assert_eq!(demand(&restored).arrived, 3);
    assert_eq!(demand(&restored).total_travel_ticks, 12);
}

fn contested_work_scenario() -> Scenario {
    let start = Tick::new(86_398);
    let mut scenario = commute(1, 1, 1, 1);
    scenario.start = start;
    scenario.demand.profiles[0].departure_offsets = vec![86_398];
    scenario.demand.profiles[0].return_offset = 86_399;
    scenario.demand.zones.push(ZoneSpec {
        id: ZoneId::new(3),
        residents: 1,
        jobs: 0,
        shops: 0,
        road_access: None,
    });
    let mut connection = scenario.demand.foot_edges[0].clone();
    connection.id = FootEdgeId::new(3);
    connection.from = ZoneId::new(3);
    connection.travel_ticks = nz(86_401);
    scenario.demand.foot_edges.push(connection);
    scenario
}

#[test]
fn residential_zones_alternate_when_the_only_work_zone_has_one_space() {
    let mut sim = Simulation::new_game(contested_work_scenario(), RuleSet::default(), 42).unwrap();
    sim.advance(1).unwrap();
    assert!(
        demand(&sim).active.iter().any(
            |batch| batch.origin == ZoneId::new(1) && batch.destination == Some(ZoneId::new(2))
        )
    );
    sim.advance(86_400).unwrap();
    assert!(
        demand(&sim).active.iter().any(
            |batch| batch.origin == ZoneId::new(3) && batch.destination == Some(ZoneId::new(2))
        )
    );
}

#[test]
fn seed_selects_the_initial_phase_of_work_rotation() {
    let mut sim = Simulation::new_game(contested_work_scenario(), RuleSet::default(), 1).unwrap();
    sim.advance(1).unwrap();
    assert!(demand(&sim).active.iter().any(|batch| {
        batch.origin == ZoneId::new(3) && batch.destination == Some(ZoneId::new(2))
    }));
    let mut resumed = Simulation::load(&sim.save().unwrap()).unwrap();
    resumed.advance(86_400).unwrap();
    assert!(demand(&resumed).active.iter().any(|batch| {
        batch.origin == ZoneId::new(1) && batch.destination == Some(ZoneId::new(2))
    }));
}
