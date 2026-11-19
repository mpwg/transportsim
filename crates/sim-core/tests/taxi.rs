use std::num::NonZeroU32;

use sim_core::{
    DemandObservation, DemandProfile, DischargeRate, JunctionId, JunctionSpec, LaneId, LaneSpec,
    MovementId, MovementSpec, Observation, Query, QueueEntry, RuleSet, Scenario, SignalPhase,
    Simulation, TaxiSpec, TaxiVehicleSpec, Tick, TripPurpose, VehicleId, VehicleKind, ZoneId,
    ZoneSpec,
};

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn scenario(red: bool, residents: u32) -> Scenario {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.lanes = [1, 2, 3]
        .map(|id| LaneSpec {
            id: LaneId::new(id),
            capacity_mm: nz(30_000),
            travel_ticks: nz(if id == 2 { 3 } else { 1 }),
            discharge: DischargeRate {
                millimetres: nz(30_000),
                ticks: nz(1),
            },
            allowed: vec![VehicleKind::Taxi],
        })
        .into();
    let junction = JunctionId::new(1);
    spec.movements = [(1, 3, 1), (2, 1, 2), (3, 2, 1)]
        .into_iter()
        .map(|(id, from, to)| MovementSpec {
            id: MovementId::new(id),
            from: LaneId::new(from),
            to: LaneId::new(to),
            junction,
            conflict_zones: vec![id as u32],
            priority: 0,
        })
        .collect();
    spec.junctions.push(JunctionSpec {
        id: junction,
        phases: if red {
            vec![
                SignalPhase {
                    green_ticks: nz(4),
                    clearance_ticks: 0,
                    movements: vec![MovementId::new(2), MovementId::new(3)],
                },
                SignalPhase {
                    green_ticks: nz(60),
                    clearance_ticks: 0,
                    movements: vec![MovementId::new(1), MovementId::new(2), MovementId::new(3)],
                },
            ]
        } else {
            Vec::new()
        },
    });
    spec.demand.zones = [1, 2, 3]
        .map(|id| ZoneSpec {
            id: ZoneId::new(id),
            residents: if id == 1 { residents } else { 0 },
            jobs: if id == 2 { residents } else { 0 },
            shops: 0,
            road_access: Some(LaneId::new(id)),
        })
        .into();
    spec.demand.walk_weight = 0;
    spec.demand.car_weight = 0;
    spec.demand.profiles.push(DemandProfile {
        purpose: TripPurpose::Work,
        departure_offsets: vec![0],
        return_offset: 1000,
        rate_numerator: 1,
        rate_denominator: nz(1),
    });
    spec.taxi = TaxiSpec {
        fleet: [9, 7]
            .map(|id| TaxiVehicleSpec {
                id: VehicleId::new(id),
                start: ZoneId::new(3),
                length_mm: nz(5_000),
                capacity: nz(2),
            })
            .into(),
    };
    spec
}

fn demand(sim: &Simulation) -> DemandObservation {
    let Observation::Demand(view) = sim.inspect(Query::Demand).unwrap() else {
        panic!("demand");
    };
    assert_eq!(
        view.generated,
        view.arrived + view.waiting + view.in_transit
    );
    view
}

fn lane_taxis(sim: &Simulation, id: u64) -> Vec<u64> {
    let Observation::Lane(view) = sim.inspect(Query::Lane(LaneId::new(id))).unwrap() else {
        panic!("lane");
    };
    view.queue
        .iter()
        .filter_map(|entry| match entry {
            QueueEntry::Vehicle {
                id,
                kind: VehicleKind::Taxi,
                ..
            } => Some(id.get()),
            _ => None,
        })
        .collect()
}

#[test]
fn equal_pickup_cost_prefers_smaller_id_and_reservation_remains_waiting() {
    let mut sim = Simulation::new_game(scenario(false, 1), RuleSet::default(), 42).unwrap();
    let Observation::Journeys(options) = sim
        .inspect(Query::Journeys {
            from: ZoneId::new(1),
            to: ZoneId::new(2),
        })
        .unwrap()
    else {
        panic!("expected travel modes");
    };
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].mode, sim_core::JourneyMode::Taxi);
    assert_eq!((options[0].wait_ticks, options[0].ride_ticks), (2, 3));
    sim.advance(1).unwrap();
    assert_eq!(lane_taxis(&sim, 3), [7]);
    assert_eq!((demand(&sim).waiting, demand(&sim).in_transit), (1, 0));
    sim.advance(2).unwrap();
    assert_eq!(lane_taxis(&sim, 2), [7]);
    assert_eq!((demand(&sim).waiting, demand(&sim).in_transit), (0, 1));
    sim.advance(3).unwrap();
    assert_eq!(demand(&sim).arrived, 1);
    assert_eq!(demand(&sim).pending_returns, 1);
}

#[test]
fn obstruction_delays_pickup_and_both_legs_resume_after_loading() {
    let mut clear = Simulation::new_game(scenario(false, 1), RuleSet::default(), 42).unwrap();
    clear.advance(6).unwrap();
    assert_eq!(demand(&clear).total_travel_ticks, 5);
    let mut sim = Simulation::new_game(scenario(true, 1), RuleSet::default(), 42).unwrap();
    sim.advance(4).unwrap();
    assert_eq!(lane_taxis(&sim, 3), [7]);
    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.advance(2).unwrap();
    assert_eq!(
        (demand(&restored).waiting, demand(&restored).in_transit),
        (0, 1)
    );
    assert_eq!(lane_taxis(&restored, 2), [7]);
    let mut occupied = Simulation::load(&restored.save().unwrap()).unwrap();
    occupied.advance(3).unwrap();
    sim.advance(5).unwrap();
    assert_eq!(sim.fingerprint().unwrap(), occupied.fingerprint().unwrap());
    assert_eq!(demand(&occupied).arrived, 1);
    assert_eq!(demand(&occupied).total_travel_ticks, 8);
}

#[test]
fn distinct_orders_cannot_reserve_the_same_vehicle_and_a_group_can_split() {
    let mut spec = scenario(false, 3);
    spec.taxi
        .fleet
        .retain(|vehicle| vehicle.id == VehicleId::new(7));
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(3).unwrap();
    assert_eq!((demand(&sim).waiting, demand(&sim).in_transit), (1, 2));
    assert_eq!(lane_taxis(&sim, 2), [7]);
    sim.advance(3).unwrap();
    assert_eq!(demand(&sim).arrived, 2);
    sim.advance(20).unwrap();
    assert_eq!(demand(&sim).arrived, 3);
}

fn taxis(sim: &Simulation) -> sim_core::TaxiObservation {
    let Observation::Taxi(view) = sim.inspect(Query::Taxi).unwrap() else {
        panic!("taxi");
    };
    view
}

#[test]
fn same_origin_and_destination_bundle_but_other_destination_waits() {
    use sim_core::TaxiStatus;
    let mut spec = scenario(false, 1);
    spec.taxi
        .fleet
        .retain(|vehicle| vehicle.id == VehicleId::new(7));
    spec.demand.zones[1].jobs = 2;
    spec.demand.zones[2].shops = 1;
    let work = spec.demand.profiles[0].clone();
    spec.demand.profiles.push(work.clone());
    spec.demand.profiles.push(DemandProfile {
        purpose: TripPurpose::Shopping,
        ..work
    });
    spec.movements.push(MovementSpec {
        id: MovementId::new(4),
        from: LaneId::new(1),
        to: LaneId::new(3),
        junction: JunctionId::new(1),
        conflict_zones: vec![4],
        priority: 0,
    });
    let mut sim = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    sim.advance(1).unwrap();
    let view = taxis(&sim);
    assert_eq!(view.waiting_total, 3);
    assert_eq!(view.unassigned_total, 1);
    assert_eq!(view.oldest_waiting_since, Some(Tick::new(1)));
    let TaxiStatus::ToPickup { assignment } = &view.vehicles[0].status else {
        panic!("one pickup is reserved");
    };
    assert_eq!(assignment.destination, ZoneId::new(2));
    assert_eq!(assignment.passengers, 2);
    sim.advance(2).unwrap();
    assert_eq!(taxis(&sim).waiting_total, 1);
    assert_eq!(demand(&sim).in_transit, 2);
    sim.advance(15).unwrap();
    assert_eq!(demand(&sim).arrived, 3);
}

#[test]
fn duplicate_vehicle_id_between_a_line_and_taxi_is_rejected() {
    use sim_core::{FleetSpec, LineId, LineSpec, StopId, StopSpec, TaxiError, ValidationError};
    let mut spec = scenario(false, 1);
    for lane in &mut spec.lanes[..2] {
        lane.allowed.push(VehicleKind::Bus);
    }
    for id in 1..=2 {
        spec.service.stops.push(StopSpec {
            id: StopId::new(id),
            zone: ZoneId::new(id),
            lane: LaneId::new(id),
            capacity: nz(6),
            minimum_dwell: nz(1),
            passengers_per_tick: nz(3),
        });
    }
    spec.service.lines.push(LineSpec {
        id: LineId::new(1),
        kind: VehicleKind::Bus,
        stops: vec![StopId::new(1), StopId::new(2)],
        headway: nz(5),
        first_departure: Tick::new(1),
    });
    spec.service.fleet.push(FleetSpec {
        id: VehicleId::new(7),
        line: LineId::new(1),
        capacity: nz(6),
        length_mm: nz(10_000),
    });
    assert!(matches!(Simulation::new_game(spec, RuleSet::default(), 42),
        Err(ValidationError::Taxi(TaxiError::DuplicateVehicle(id))) if id == VehicleId::new(7)
    ));
}

#[test]
fn same_checkpoint_restores_a_dwelling_bus_and_an_occupied_taxi() {
    use sim_core::{FleetSpec, LineId, LineSpec, StopId, StopSpec};

    let mut spec = scenario(false, 1);
    spec.demand.transit_weight = 0;
    for index in 0..2 {
        let zone = index as u64 + 1;
        let lane_id = LaneId::new(zone + 3);
        let mut lane = spec.lanes[index].clone();
        lane.id = lane_id;
        lane.allowed = vec![VehicleKind::Bus];
        spec.lanes.push(lane);
        spec.service.stops.push(StopSpec {
            id: StopId::new(zone),
            zone: ZoneId::new(zone),
            lane: lane_id,
            capacity: nz(8),
            minimum_dwell: nz(4),
            passengers_per_tick: nz(8),
        });
    }
    for (from, to) in [(4, 5), (5, 4)] {
        spec.movements.push(MovementSpec {
            id: MovementId::new(from),
            from: LaneId::new(from),
            to: LaneId::new(to),
            junction: JunctionId::new(1),
            conflict_zones: vec![from as u32],
            priority: 0,
        });
    }
    spec.service.lines.push(LineSpec {
        id: LineId::new(1),
        kind: VehicleKind::Bus,
        stops: vec![StopId::new(1), StopId::new(2)],
        headway: nz(10),
        first_departure: Tick::new(1),
    });
    spec.service.fleet.push(FleetSpec {
        id: VehicleId::new(12),
        line: LineId::new(1),
        capacity: nz(8),
        length_mm: nz(10_000),
    });
    let mut simulation = Simulation::new_game(spec, RuleSet::default(), 42).unwrap();
    simulation.advance(3).unwrap();
    assert_eq!(lane_taxis(&simulation, 2), [7]);
    let Observation::Service(line) = simulation.inspect(Query::Service).unwrap() else {
        panic!("service");
    };
    assert_eq!(line.vehicles[0].dwell_until, Some(Tick::new(6)));
    let mut loaded = Simulation::load(&simulation.save().unwrap()).unwrap();
    assert_eq!(
        loaded.inspect(Query::Service).unwrap(),
        Observation::Service(line)
    );
    assert_eq!(loaded.inspect(Query::Taxi), simulation.inspect(Query::Taxi));
    loaded.advance(8).unwrap();
    simulation.advance(8).unwrap();
    assert_eq!(
        loaded.fingerprint().unwrap(),
        simulation.fingerprint().unwrap()
    );
}
