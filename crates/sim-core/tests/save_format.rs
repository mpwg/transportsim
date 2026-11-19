use std::num::NonZeroU32;

use serde_json::{Value, json};
use sim_core::{
    DemandProfile, DischargeRate, JunctionId, JunctionSpec, LaneId, LaneSpec, LoadError,
    MovementId, MovementSpec, QueueEntry, RoadValidationError, RuleSet, Scenario, Simulation,
    TaxiVehicleSpec, Tick, TrafficSeed, TripPurpose, ValidationError, VehicleId, VehicleKind,
    ZoneId, ZoneSpec,
};

const LIMIT: usize = 128 * 1024 * 1024;

fn lane(id: u64, kind: VehicleKind) -> LaneSpec {
    LaneSpec {
        id: LaneId::new(id),
        capacity_mm: NonZeroU32::new(30_000).unwrap(),
        travel_ticks: NonZeroU32::new(1).unwrap(),
        discharge: DischargeRate {
            millimetres: NonZeroU32::new(5_000).unwrap(),
            ticks: NonZeroU32::new(1).unwrap(),
        },
        allowed: vec![kind],
    }
}

fn simulation() -> Simulation {
    let mut scenario = Scenario::empty(Tick::new(10));
    scenario.lanes = vec![lane(7, VehicleKind::Car)];
    scenario.traffic = vec![TrafficSeed {
        lane: LaneId::new(7),
        entry: QueueEntry::PrivateCars(NonZeroU32::new(3).unwrap()),
        ready_at: Tick::new(11),
        route: vec![],
    }];
    Simulation::new_game(scenario, RuleSet::default(), 0).unwrap()
}

fn taxi_en_route_to_pickup() -> Simulation {
    let mut scenario = Scenario::empty(Tick::new(0));
    scenario.lanes = vec![lane(7, VehicleKind::Taxi), lane(8, VehicleKind::Taxi)];
    let junction = JunctionId::new(1);
    scenario.junctions.push(JunctionSpec {
        id: junction,
        phases: vec![],
    });
    scenario.movements = [(1, 7, 8), (2, 8, 7)]
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
    scenario.demand.zones = [(1, 7, 1, 0), (2, 8, 0, 1)]
        .into_iter()
        .map(|(id, access, residents, jobs)| ZoneSpec {
            id: ZoneId::new(id),
            residents,
            jobs,
            shops: 0,
            road_access: Some(LaneId::new(access)),
        })
        .collect();
    scenario.demand.profiles.push(DemandProfile {
        purpose: TripPurpose::Work,
        departure_offsets: vec![0],
        return_offset: 100,
        rate_numerator: 1,
        rate_denominator: NonZeroU32::MIN,
    });
    scenario.taxi.fleet.push(TaxiVehicleSpec {
        id: VehicleId::new(42),
        start: ZoneId::new(2),
        length_mm: NonZeroU32::new(5_000).unwrap(),
        capacity: NonZeroU32::MIN,
    });
    let mut sim = Simulation::new_game(scenario, RuleSet::default(), 0).unwrap();
    sim.advance(1).unwrap();
    sim
}

fn document() -> Value {
    serde_json::from_slice(&simulation().save().unwrap()).unwrap()
}

fn taxi_document() -> Value {
    serde_json::from_slice(&taxi_en_route_to_pickup().save().unwrap()).unwrap()
}

fn payload_hash(mut payload: Value) -> blake3::Hash {
    payload.sort_all_objects();
    blake3::hash(&serde_json::to_vec(&payload).unwrap())
}

fn with_updated_checksum(mut document: Value) -> Vec<u8> {
    document["checksum"] = json!(payload_hash(document["payload"].clone()).to_hex().as_str());
    serde_json::to_vec(&document).unwrap()
}

#[test]
fn truncated_document_is_a_json_error() {
    let mut bytes = simulation().save().unwrap();
    bytes.pop();
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::Json { .. })
    ));
}

#[test]
fn modified_payload_is_a_checksum_error() {
    let mut document = document();
    document["payload"]["paused"] = json!(true);
    let bytes = serde_json::to_vec(&document).unwrap();
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::ChecksumMismatch)
    ));
}

#[test]
fn unsupported_format_is_distinct_from_incompatible_rules() {
    let mut document = document();
    document["format_version"] = json!(2);
    let bytes = serde_json::to_vec(&document).unwrap();
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::UnsupportedFormat { version: 2 })
    ));

    document["format_version"] = json!(1);
    document["rules_version"] = json!(2);
    let bytes = serde_json::to_vec(&document).unwrap();
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::UnsupportedRules { version: 2 })
    ));
}

#[test]
fn size_over_the_limit_is_rejected_before_parsing() {
    let bytes = vec![0; LIMIT + 1];
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::TooLarge { size, limit }) if size == LIMIT + 1 && limit == LIMIT
    ));
}

#[test]
fn duplicate_object_keys_are_rejected_at_any_depth() {
    let bytes = simulation().save().unwrap();
    let serialized = std::str::from_utf8(&bytes).unwrap();
    for (original, duplicate) in [
        (
            r#""format_version":1"#,
            r#""format_version":1,"format_version":1"#,
        ),
        (
            r#""capacity_mm":30000"#,
            r#""capacity_mm":30000,"capac\u0069ty_mm":30000"#,
        ),
    ] {
        assert!(serialized.contains(original));
        let malformed = serialized.replacen(original, duplicate, 1);
        assert!(matches!(
            Simulation::load(malformed.as_bytes()),
            Err(LoadError::Json { .. })
        ));
    }
}

#[test]
fn checksum_and_fingerprint_use_canonical_payload_bytes() {
    let simulation = simulation();
    let bytes = simulation.save().unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    let expected = payload_hash(document["payload"].clone());
    assert_eq!(
        document["checksum"].as_str(),
        Some(expected.to_hex().as_str())
    );
    assert_eq!(simulation.fingerprint().unwrap(), *expected.as_bytes());
}

#[test]
fn whitespace_and_object_key_order_leave_the_checkpoint_equivalent() {
    let simulation = simulation();
    let bytes = simulation.save().unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    let payload = document["payload"].as_object().unwrap();
    let reversed_payload_fields = payload
        .iter()
        .rev()
        .map(|(key, value)| {
            format!(
                "  {} : {}",
                serde_json::to_string(key).unwrap(),
                serde_json::to_string_pretty(value).unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let rearranged = format!(
        " {{ \n \"payload\": {{\n{reversed_payload_fields}\n}}, \"checksum\": {}, \"rules_version\": 1, \"format_version\": 1 \n }} \n",
        document["checksum"]
    );
    let loaded = Simulation::load(rearranged.as_bytes()).unwrap();
    assert_eq!(loaded.save().unwrap(), bytes);
}

#[test]
fn unknown_lane_is_validated_after_checksum_verification() {
    let mut document = document();
    document["payload"]["scenario"]["traffic"][0]["lane"] = json!(999);
    let bytes = with_updated_checksum(document);
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::State(ValidationError::Road(RoadValidationError::UnknownLane(id))))
            if id == LaneId::new(999)
    ));
}

#[test]
fn negative_car_count_is_rejected_with_a_valid_checksum() {
    let mut document = document();
    document["payload"]["scenario"]["traffic"][0]["entry"]["private_cars"] = json!(-1);
    let bytes = with_updated_checksum(document);
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::Json { .. })
    ));
}

#[test]
fn restored_queue_cannot_exceed_lane_capacity() {
    let mut document = document();
    document["payload"]["lanes"][0]["queue"][0]["entry"]["private_cars"] = json!(7);
    let error = Simulation::load(&with_updated_checksum(document)).unwrap_err();
    assert_eq!(
        error,
        LoadError::State(ValidationError::Road(RoadValidationError::OverCapacity(
            LaneId::new(7)
        )))
    );
}

#[test]
fn impossible_checkpoint_times_are_rejected() {
    for (pointer, impossible, field) in [
        ("/tick", json!(9), "tick"),
        ("/development_ticks", json!(1), "development_ticks"),
        (
            "/lanes/0/queue/0/ready_at",
            Value::Null,
            "lanes.queue.ready_at",
        ),
    ] {
        let mut document = document();
        *document["payload"].pointer_mut(pointer).unwrap() = impossible;
        let error = Simulation::load(&with_updated_checksum(document)).unwrap_err();
        assert_eq!(error, LoadError::InvalidState { field });
    }
}

#[test]
fn restored_lane_ids_must_be_unique() {
    let mut document = taxi_document();
    document["payload"]["lanes"][1]["id"] = json!(7);
    let error = Simulation::load(&with_updated_checksum(document)).unwrap_err();
    assert_eq!(error, LoadError::InvalidState { field: "lanes.id" });
}

#[test]
fn restored_vehicle_cannot_occupy_two_lanes() {
    let mut document = taxi_document();
    let mut taxi = document["payload"]["lanes"][1]["queue"][0].clone();
    assert_eq!(taxi["entry"]["vehicle"]["id"], json!(42));
    taxi["route"] = json!([]);
    document["payload"]["lanes"][0]["queue"] = json!([taxi]);
    let error = Simulation::load(&with_updated_checksum(document)).unwrap_err();
    assert_eq!(
        error,
        LoadError::State(ValidationError::Road(
            RoadValidationError::DuplicateVehicle(VehicleId::new(42))
        ))
    );
}

#[test]
fn taxi_reservation_requires_an_existing_passenger_cohort() {
    let mut document = taxi_document();
    let cohort = document["payload"]
        .pointer_mut("/taxi/vehicles/42/to_pickup/passengers/0/cohort")
        .unwrap();
    *cohort = json!(999);
    let error = Simulation::load(&with_updated_checksum(document)).unwrap_err();
    assert_eq!(
        error,
        LoadError::InvalidState {
            field: "demand.taxi"
        }
    );
}

#[test]
fn taxi_reservation_cannot_predate_its_passenger_cohort() {
    let mut document = taxi_document();
    let since = document["payload"]
        .pointer_mut("/taxi/vehicles/42/to_pickup/passengers/0/since")
        .unwrap();
    assert_eq!(*since, json!(1));
    *since = json!(0);
    let error = Simulation::load(&with_updated_checksum(document)).err();
    assert_eq!(
        error,
        Some(LoadError::InvalidState {
            field: "demand.taxi"
        })
    );
}

#[test]
fn unbalanced_demand_is_rejected_and_running_game_continues() {
    let mut running = taxi_en_route_to_pickup();
    let checkpoint = running.save().unwrap();
    let mut control = Simulation::load(&checkpoint).unwrap();
    let mut document: Value = serde_json::from_slice(&checkpoint).unwrap();
    assert_eq!(document["payload"]["demand"]["generated"], json!(1));
    document["payload"]["demand"]["generated"] = json!(2);

    let error = Simulation::load(&with_updated_checksum(document)).unwrap_err();
    assert_eq!(
        error,
        LoadError::InvalidState {
            field: "demand.balance"
        }
    );
    assert_eq!(running.save().unwrap(), checkpoint);
    let report = running.advance(1).unwrap();
    assert_eq!(report.executed_ticks, 1);
    assert_eq!(report, control.advance(1).unwrap());
    assert_eq!(
        running.fingerprint().unwrap(),
        control.fingerprint().unwrap()
    );
}

#[test]
fn checkpoint_envelope_must_be_an_object() {
    let document = document();
    let positional = json!([
        document["format_version"],
        document["rules_version"],
        document["checksum"],
        document["payload"],
    ]);
    let bytes = serde_json::to_vec(&positional).unwrap();
    assert!(matches!(
        Simulation::load(&bytes),
        Err(LoadError::Json { .. })
    ));
}

#[test]
fn scenario_json_uses_the_same_duplicate_key_check() {
    let scenario = document()["payload"]["scenario"].to_string();
    let original = r#""capacity_mm":30000"#;
    assert!(scenario.contains(original));
    let malformed = scenario.replacen(original, r#""capacity_mm":30000,"capacity_mm":30000"#, 1);
    assert!(matches!(
        Simulation::from_scenario_json(malformed.as_bytes()),
        Err(LoadError::Json { .. })
    ));
}

#[test]
fn newer_format_reports_its_version_before_using_the_v1_schema() {
    let bytes = br#"{"format_version":2,"future_header":"opaque","payload":[1,2,3]}"#;
    assert!(matches!(
        Simulation::load(bytes),
        Err(LoadError::UnsupportedFormat { version: 2 })
    ));
}

#[test]
fn legacy_checkpoint_keeps_its_fingerprint_without_random_streams() {
    let mut legacy = document();
    let current = legacy["payload"]
        .as_object_mut()
        .unwrap()
        .remove("random")
        .unwrap();
    assert_eq!(current["version"], json!("splitmix64_v1"));
    let bytes = with_updated_checksum(legacy);
    let expected: Value = serde_json::from_slice(&bytes).unwrap();
    let mut resumed = Simulation::load(&bytes).unwrap();
    assert_eq!(
        blake3::Hash::from_bytes(resumed.fingerprint().unwrap())
            .to_hex()
            .as_str(),
        expected["checksum"].as_str().unwrap()
    );
    resumed.advance(1).unwrap();
    let saved: Value = serde_json::from_slice(&resumed.save().unwrap()).unwrap();
    assert!(saved["payload"].get("random").is_none());
}

#[test]
fn unsupported_random_stream_version_is_rejected_after_checksum_verification() {
    let mut invalid = document();
    invalid["payload"]["random"]["version"] = json!("splitmix64_v2");
    assert!(matches!(
        Simulation::load(&with_updated_checksum(invalid)),
        Err(LoadError::Json { .. })
    ));
}
