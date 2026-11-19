use std::num::NonZeroU32;

use serde_json::{Value, json};
use sim_core::{
    DischargeRate, LaneId, LaneSpec, LoadError, QueueEntry, RoadValidationError, RuleSet, Scenario,
    Simulation, Tick, TrafficSeed, ValidationError, VehicleKind,
};

const LIMIT: usize = 128 * 1024 * 1024;

fn simulation() -> Simulation {
    let mut scenario = Scenario::empty(Tick::new(10));
    scenario.lanes = vec![LaneSpec {
        id: LaneId::new(7),
        capacity_mm: NonZeroU32::new(30_000).unwrap(),
        travel_ticks: NonZeroU32::new(1).unwrap(),
        discharge: DischargeRate {
            millimetres: NonZeroU32::new(5_000).unwrap(),
            ticks: NonZeroU32::new(1).unwrap(),
        },
        allowed: vec![VehicleKind::Car],
    }];
    scenario.traffic = vec![TrafficSeed {
        lane: LaneId::new(7),
        entry: QueueEntry::PrivateCars(NonZeroU32::new(3).unwrap()),
        ready_at: Tick::new(11),
        route: vec![],
    }];
    Simulation::new_game(scenario, RuleSet::default(), 0).unwrap()
}

fn document() -> Value {
    serde_json::from_slice(&simulation().save().unwrap()).unwrap()
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
