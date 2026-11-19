use std::num::NonZeroU32;

use sim_core::{
    DischargeRate, FleetSpec, JunctionId, JunctionSpec, LaneId, LaneSpec, LineId, LineSpec,
    MovementId, MovementSpec, Observation, Query, QueueEntry, RuleSet, Scenario, ServiceSpec,
    Simulation, StopId, StopSpec, Tick, TrafficSeed, VehicleId, VehicleKind, ZoneId, ZoneSpec,
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
