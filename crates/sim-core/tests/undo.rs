use std::num::NonZeroU32;

use sim_core::{
    Cell, Command, CommandError, DischargeRate, EditAction, EditCommand, LaneId, LaneSpec, Marker,
    MarkerAnchor, MarkerId, Observation, Query, Simulation, Tick, VehicleKind,
};

fn bridge() -> Simulation {
    Simulation::from_scenario_json(include_bytes!("../../../scenarios/bridge-test.json")).unwrap()
}

fn capacity(revision: u64, lane: u64, value: u32) -> EditCommand {
    EditCommand {
        expected_revision: revision,
        actions: vec![EditAction::SetCapacity {
            lane: LaneId::new(lane),
            capacity_mm: NonZeroU32::new(value).unwrap(),
        }],
    }
}

#[test]
fn undo_after_reload_restores_the_exact_earlier_traffic_state_and_time() {
    let mut sim = bridge();
    sim.advance(10).unwrap();
    let before = sim.save().unwrap();
    let mut baseline = Simulation::load(&before).unwrap();
    sim.apply(Command::CommitEdit(capacity(0, 2, 120_000)))
        .unwrap();
    sim.apply(Command::SetPaused(true)).unwrap();
    assert!(sim.apply(Command::CommitEdit(capacity(1, 1, 100))).is_err());
    sim.step().unwrap();
    sim.step().unwrap();

    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    let Observation::Undo(Some(checkpoint)) = loaded.inspect(Query::Undo).unwrap() else {
        panic!("undo checkpoint should survive");
    };
    assert_eq!(checkpoint.target_tick, Tick::new(10));
    assert_eq!(loaded.apply(Command::UndoEdit).unwrap().tick, Tick::new(10));
    assert_eq!(loaded.save().unwrap(), before);
    assert_eq!(
        loaded.inspect(Query::Undo).unwrap(),
        Observation::Undo(None)
    );
    assert!(matches!(
        loaded.apply(Command::UndoEdit),
        Err(CommandError::NoUndo)
    ));
    baseline.advance(3).unwrap();
    loaded.advance(3).unwrap();
    assert_eq!(
        loaded.fingerprint().unwrap(),
        baseline.fingerprint().unwrap()
    );
}

#[test]
fn a_second_edit_replaces_the_undo_point_without_retaining_a_history() {
    let mut sim = bridge();
    sim.apply(Command::CommitEdit(capacity(0, 2, 90_000)))
        .unwrap();
    sim.advance(8).unwrap();
    sim.apply(Command::CommitEdit(capacity(1, 3, 100_000)))
        .unwrap();
    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    assert_eq!(loaded.apply(Command::UndoEdit).unwrap().tick, Tick::new(8));
    assert_eq!(
        loaded.inspect(Query::Undo).unwrap(),
        Observation::Undo(None)
    );
    let Observation::Lane(restored_first) = loaded.inspect(Query::Lane(LaneId::new(2))).unwrap()
    else {
        panic!("first edit still present");
    };
    let Observation::Lane(restored_second) = loaded.inspect(Query::Lane(LaneId::new(3))).unwrap()
    else {
        panic!("second edit undone");
    };
    assert_eq!(restored_first.capacity_mm, 90_000);
    assert_eq!(restored_second.capacity_mm, 60_000);
}

#[test]
fn a_marker_remains_at_its_coordinates_when_the_anchor_disappears() {
    let mut sim = bridge();
    let lane = LaneId::new(99);
    let marker = Marker {
        id: MarkerId::new(4),
        cell: Cell { x: 17, y: -4 },
        anchor: Some(MarkerAnchor::Lane(lane)),
    };
    sim.apply(Command::CommitEdit(EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::UpsertLane(LaneSpec {
            id: lane,
            capacity_mm: NonZeroU32::new(5000).unwrap(),
            travel_ticks: NonZeroU32::MIN,
            discharge: DischargeRate {
                millimetres: NonZeroU32::new(5000).unwrap(),
                ticks: NonZeroU32::MIN,
            },
            allowed: vec![VehicleKind::Car],
        })],
    }))
    .unwrap();
    sim.apply(Command::SetMarker(marker.clone())).unwrap();
    let Observation::Markers(markers) = sim.inspect(Query::Markers).unwrap() else {
        panic!("markers");
    };
    assert!(markers[0].anchor_present);
    sim.apply(Command::CommitEdit(EditCommand {
        expected_revision: 1,
        actions: vec![EditAction::RemoveLane(lane)],
    }))
    .unwrap();

    let mut loaded = Simulation::load(&sim.save().unwrap()).unwrap();
    let Observation::Markers(markers) = loaded.inspect(Query::Markers).unwrap() else {
        panic!("markers");
    };
    assert_eq!(markers[0].marker, marker);
    assert!(!markers[0].anchor_present);
    loaded.apply(Command::RemoveMarker(marker.id)).unwrap();
    assert_eq!(
        loaded.inspect(Query::Markers).unwrap(),
        Observation::Markers(Vec::new())
    );
}

#[test]
fn a_bad_reference_and_nested_history_are_rejected_inside_the_undo_checkpoint() {
    let mut sim = bridge();
    sim.advance(10).unwrap();
    sim.apply(Command::CommitEdit(capacity(0, 2, 120_000)))
        .unwrap();
    let bytes = sim.save().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut bad_reference = value.clone();
    bad_reference["payload"]["undo"]["lanes"][0]["queue"][0]["route"][0] = 9999.into();
    let mut nested = value.clone();
    nested["payload"]["undo"]["undo"] = value["payload"]["undo"].clone();
    for mut wrong in [bad_reference, nested] {
        let payload = &mut wrong["payload"];
        payload.sort_all_objects();
        wrong["checksum"] = blake3::hash(&serde_json::to_vec(payload).unwrap())
            .to_hex()
            .as_str()
            .into();
        let corrupted = serde_json::to_vec(&wrong).unwrap();
        assert!(matches!(
            Simulation::load(&corrupted),
            Err(sim_core::LoadError::InvalidState { field: "undo" })
        ));
        assert_eq!(sim.save().unwrap(), bytes);
    }
}
