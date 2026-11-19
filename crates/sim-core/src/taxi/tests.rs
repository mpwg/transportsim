use super::*;
use crate::{DischargeRate, JunctionId, JunctionSpec, LaneSpec, MovementSpec, TrafficSeed};

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn road(traffic: Vec<TrafficSeed>) -> RoadState {
    let lanes = [(1, 1), (2, 1), (3, 3), (4, 1)]
        .into_iter()
        .map(|(id, travel)| LaneSpec {
            id: LaneId::new(id),
            capacity_mm: nonzero(5_000),
            travel_ticks: nonzero(travel),
            discharge: DischargeRate {
                millimetres: nonzero(5_000),
                ticks: nonzero(1),
            },
            allowed: vec![VehicleKind::Taxi, VehicleKind::Car],
        })
        .collect();
    let junction = JunctionId::new(1);
    let movements = [(1, 2), (2, 3), (3, 1)]
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
    RoadState::new(
        lanes,
        movements,
        vec![JunctionSpec {
            id: junction,
            phases: vec![],
        }],
        traffic,
    )
    .unwrap()
}

fn vehicle(id: u64, start: u64, capacity: u32) -> TaxiVehicleSpec {
    TaxiVehicleSpec {
        id: VehicleId::new(id),
        start: ZoneId::new(start),
        length_mm: nonzero(5_000),
        capacity: nonzero(capacity),
    }
}

fn taxi(fleet: Vec<TaxiVehicleSpec>, road: &RoadState) -> Taxi {
    Taxi::new(
        TaxiSpec { fleet },
        (1..=4)
            .map(|id| (ZoneId::new(id), LaneId::new(id)))
            .collect(),
        road,
        |_| false,
    )
    .unwrap()
}

fn request(cohort: u64, origin: u64, destination: u64, since: u64, count: u32) -> TaxiRequest {
    TaxiRequest {
        cohort,
        origin: ZoneId::new(origin),
        destination: ZoneId::new(destination),
        since: Tick::new(since),
        count,
    }
}

fn step(time: u64, taxi: &mut Taxi, road: &mut RoadState) -> Vec<(u64, u32)> {
    let tick = Tick::new(time);
    let arrivals = taxi.tick(tick, road);
    road.tick(tick);
    arrivals
}

#[test]
fn dispatch_breaks_a_tie_by_vehicle_id() {
    let mut road = road(vec![]);
    let mut taxi = taxi(vec![vehicle(9, 1, 2), vehicle(7, 1, 2)], &road);
    taxi.request(request(20, 2, 3, 1, 1));
    assert_eq!(
        taxi.estimate(ZoneId::new(2), ZoneId::new(3), &road),
        Some((2, 3))
    );
    assert!(step(1, &mut taxi, &mut road).is_empty());
    let vehicles = taxi.observe(Tick::new(1), &road).vehicles;
    assert_eq!(vehicles[0].id, VehicleId::new(7));
    assert!(matches!(vehicles[0].status, TaxiStatus::ToPickup { .. }));
    assert!(matches!(vehicles[1].status, TaxiStatus::Idle { .. }));
    assert_eq!(taxi.waiting_cohorts().collect::<Vec<_>>(), vec![(20, 1)]);
    assert_eq!(
        road.managed_vehicles().collect::<Vec<_>>(),
        vec![(VehicleId::new(7), LaneId::new(1), VehicleKind::Taxi)]
    );
}

#[test]
fn dispatch_skips_unreachable_requests_and_bundles_up_to_capacity() {
    let mut road = road(vec![]);
    let mut taxi = taxi(vec![vehicle(7, 1, 2)], &road);
    taxi.request(request(5, 4, 3, 1, 1));
    taxi.request(request(7, 2, 3, 2, 2));
    taxi.request(request(9, 2, 1, 2, 1));
    taxi.request(request(6, 2, 3, 2, 1));
    assert!(step(2, &mut taxi, &mut road).is_empty());
    let observation = taxi.observe(Tick::new(2), &road);
    assert_eq!(observation.waiting_total, 5);
    assert_eq!(observation.unassigned_total, 3);
    assert!(
        matches!(observation.vehicles[0].status, TaxiStatus::ToPickup { assignment } if assignment.passengers == 2 && assignment.destination == ZoneId::new(3))
    );
    assert!(step(3, &mut taxi, &mut road).is_empty());
    assert!(step(4, &mut taxi, &mut road).is_empty());
    let observation = taxi.observe(Tick::new(4), &road);
    assert_eq!(observation.waiting_total, 3);
    assert!(matches!(
        observation.vehicles[0].status,
        TaxiStatus::Occupied { .. }
    ));
    assert!(step(5, &mut taxi, &mut road).is_empty());
    assert!(step(6, &mut taxi, &mut road).is_empty());
    assert_eq!(step(7, &mut taxi, &mut road), vec![(6, 1), (7, 1)]);
}

#[test]
fn congestion_delays_pickup_and_arrival() {
    let mut road = road(vec![TrafficSeed {
        lane: LaneId::new(2),
        entry: QueueEntry::PrivateCars(nonzero(1)),
        ready_at: Tick::new(5),
        route: vec![],
    }]);
    let mut taxi = taxi(vec![vehicle(7, 1, 2)], &road);
    taxi.request(request(1, 2, 3, 1, 1));
    for tick in 1..=6 {
        assert!(step(tick, &mut taxi, &mut road).is_empty());
        assert_eq!(taxi.observe(Tick::new(tick), &road).waiting_total, 1);
    }
    assert!(step(7, &mut taxi, &mut road).is_empty());
    assert_eq!(taxi.observe(Tick::new(7), &road).waiting_total, 0);
    assert!(step(8, &mut taxi, &mut road).is_empty());
    assert!(step(9, &mut taxi, &mut road).is_empty());
    assert_eq!(step(10, &mut taxi, &mut road), vec![(1, 1)]);
}

#[test]
fn checkpoint_continues_both_phases_without_changing_arrivals() {
    for saved_at in [1, 4] {
        let fleet = vec![vehicle(7, 1, 1)];
        let mut original_road = road(vec![]);
        let mut original = taxi(fleet.clone(), &original_road);
        original.request(request(11, 2, 3, 1, 2));
        for tick in 1..=saved_at {
            step(tick, &mut original, &mut original_road);
        }
        let tick = Tick::new(saved_at);
        let (lanes, junctions) = original_road.snapshot();
        let wire = serde_json::to_vec(&original.snapshot()).unwrap();
        let mut restored_road = road(vec![]);
        restored_road.restore(tick, lanes, junctions).unwrap();
        let mut restored = taxi(fleet, &restored_road);
        restored
            .restore(serde_json::from_slice(&wire).unwrap(), tick, &restored_road)
            .unwrap();
        for tick in saved_at + 1..=20 {
            let observed_at = Tick::new(tick - 1);
            assert_eq!(
                original.observe(observed_at, &original_road),
                restored.observe(observed_at, &restored_road)
            );
            assert_eq!(
                step(tick, &mut original, &mut original_road),
                step(tick, &mut restored, &mut restored_road)
            );
        }
        assert_eq!(restored.cohorts().count(), 0);
    }
}

#[test]
fn checkpoint_rejects_a_missing_managed_taxi() {
    let fleet = vec![vehicle(7, 1, 1)];
    let mut active_road = road(vec![]);
    let mut active = taxi(fleet.clone(), &active_road);
    active.request(request(1, 2, 3, 1, 1));
    step(1, &mut active, &mut active_road);
    let empty_road = road(vec![]);
    let mut restored = taxi(fleet, &empty_road);
    assert!(
        restored
            .restore(active.snapshot(), Tick::new(1), &empty_road)
            .is_err()
    );
}

#[test]
fn at_pickup_the_taxi_starts_the_occupied_trip_without_an_empty_leg() {
    let mut road = road(vec![]);
    let mut taxi = taxi(vec![vehicle(7, 2, 1)], &road);
    assert_eq!(
        taxi.estimate(ZoneId::new(2), ZoneId::new(3), &road),
        Some((0, 4))
    );
    taxi.request(request(1, 2, 3, 1, 1));
    assert!(step(1, &mut taxi, &mut road).is_empty());
    assert!(matches!(
        taxi.observe(Tick::new(1), &road).vehicles[0].status,
        TaxiStatus::Occupied { .. }
    ));
    assert_eq!(taxi.waiting_cohorts().count(), 0);
    for tick in 2..=4 {
        assert!(step(tick, &mut taxi, &mut road).is_empty());
    }
    assert_eq!(step(5, &mut taxi, &mut road), vec![(1, 1)]);
}

#[test]
fn checkpoint_rejects_assignment_larger_than_vehicle_capacity() {
    let mut road = road(vec![]);
    let mut original = taxi(vec![vehicle(7, 1, 2)], &road);
    original.request(request(1, 2, 3, 1, 2));
    step(1, &mut original, &mut road);
    let mut restored = taxi(vec![vehicle(7, 1, 1)], &road);
    assert!(matches!(
        restored.restore(original.snapshot(), Tick::new(1), &road),
        Err(LoadError::InvalidState {
            field: "taxi.assignment"
        })
    ));
}

#[test]
fn future_request_is_not_reserved_with_a_current_one() {
    let mut road = road(vec![]);
    let mut taxi = taxi(vec![vehicle(7, 1, 2)], &road);
    taxi.request(request(1, 2, 3, 1, 1));
    taxi.request(request(2, 2, 3, 9, 1));
    step(1, &mut taxi, &mut road);
    assert!(
        matches!(taxi.observe(Tick::new(1), &road).vehicles[0].status, TaxiStatus::ToPickup { assignment } if assignment.passengers == 1)
    );
}

#[test]
fn checkpoint_accepts_multiple_chunks_of_the_same_cohort() {
    let mut road = road(vec![]);
    let fleet = vec![vehicle(7, 1, 1)];
    let mut taxi = taxi(fleet, &road);
    taxi.request(request(1, 2, 3, 1, u32::MAX));
    taxi.request(request(1, 2, 3, 1, 2));
    step(1, &mut taxi, &mut road);
    assert_eq!(
        taxi.observe(Tick::new(1), &road).waiting_total,
        u64::from(u32::MAX) + 2
    );
    taxi.restore(taxi.snapshot(), Tick::new(1), &road).unwrap();
}

#[test]
fn checkpoint_continues_a_trip_within_one_zone() {
    let mut road = road(vec![]);
    let mut taxi = taxi(vec![vehicle(7, 1, 1)], &road);
    taxi.request(request(1, 1, 1, 1, 1));
    assert!(step(1, &mut taxi, &mut road).is_empty());
    taxi.restore(taxi.snapshot(), Tick::new(1), &road).unwrap();
    assert_eq!(step(2, &mut taxi, &mut road), vec![(1, 1)]);
}
