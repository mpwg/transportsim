use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn run(args: &[(&str, &OsStr)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sim-runner"))
        .arg("run")
        .args(
            args.iter()
                .flat_map(|(flag, value)| [flag.as_ref(), *value]),
        )
        .output()
        .unwrap()
}

fn bridge() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/bridge-test.json"
    ))
}

fn summary(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("runner must print a JSON summary")
}

fn trace(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn restart_during_red_preserves_first_legal_crossing() {
    let dir = tempfile::tempdir().unwrap();
    let save = dir.path().join("checkpoint.json");
    let full = dir.path().join("full.jsonl");
    let resumed = dir.path().join("resumed.jsonl");

    let uninterrupted = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--until-tick", "17".as_ref()),
        ("--trace", full.as_os_str()),
    ]));
    assert_eq!(uninterrupted["tick"], 17);
    let fingerprint = uninterrupted["fingerprint"].as_str().unwrap();
    assert_eq!(fingerprint.len(), 64);
    assert!(fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()));

    let checkpoint = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--until-tick", "10".as_ref()),
        ("--save", save.as_os_str()),
    ]));
    assert_eq!(checkpoint["tick"], 10);

    let continuation = summary(run(&[
        ("--load", save.as_os_str()),
        ("--until-tick", "17".as_ref()),
        ("--trace", resumed.as_os_str()),
    ]));
    assert_eq!(continuation, uninterrupted);

    let full = trace(&full);
    assert_eq!(full[9]["lanes"][0]["wait_reason"], "red_signal");
    assert_eq!(
        full[9]["lanes"][0]["queue"][1]["vehicle"],
        serde_json::json!({"id": 12, "kind": "bus", "length_mm": 12000})
    );
    let first_crossing = full
        .iter()
        .find(|row| row["lanes"][0]["departure"]["entry"]["vehicle"]["id"] == 12)
        .expect("the bus never departed from the bridge approach");
    assert_eq!(first_crossing["tick"], 13);
    assert_eq!(first_crossing["lanes"][0]["departure"]["to"], 3);
    assert_eq!(first_crossing["lanes"][2]["queue"][0]["vehicle"]["id"], 12);
    assert_eq!(trace(&resumed), full[10..]);
}

#[test]
fn replay_preserves_same_boundary_command_order_and_trace_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("commands.json");
    fs::write(
        &commands,
        r#"[
            {"tick":10,"command":{"set_paused":true}},
            {"tick":10,"command":{"set_paused":false}},
            {"tick":13,"command":{"set_growth_paused":true}},
            {"tick":20,"command":{"set_growth_paused":false}}
        ]"#,
    )
    .unwrap();

    let a = dir.path().join("a.json");
    let b = dir.path().join("b.json");
    let full = dir.path().join("full.jsonl");
    let suffix = dir.path().join("suffix.jsonl");
    let uninterrupted = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", "37".as_ref()),
        ("--trace", full.as_os_str()),
    ]));

    let direct_b = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", "20".as_ref()),
    ]));
    summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--until-tick", "10".as_ref()),
        ("--save", a.as_os_str()),
    ]));
    let from_a = [
        ("--load", a.as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", "20".as_ref()),
        ("--save", b.as_os_str()),
    ];
    assert_eq!(summary(run(&from_a)), direct_b);
    let b_bytes = fs::read(&b).unwrap();
    let from_b = [("--load", b.as_os_str()), ("--until-tick", "37".as_ref())];
    assert_eq!(summary(run(&from_b)), uninterrupted);
    let from_b_with_trace = [from_b[0], from_b[1], ("--trace", suffix.as_os_str())];
    assert_eq!(summary(run(&from_b_with_trace)), uninterrupted);

    let full = trace(&full);
    assert_eq!(full[13]["clock"]["growth_paused"], true);
    assert_eq!(full[20]["clock"]["development_ticks"], 14);
    let first_suffix = trace(&suffix);
    assert_eq!(first_suffix, full[20..]);
    assert_eq!(summary(run(&from_a)), direct_b);
    assert_eq!(fs::read(&b).unwrap(), b_bytes);
    assert_eq!(summary(run(&from_b_with_trace)), uninterrupted);
    assert_eq!(trace(&suffix), first_suffix);
}

#[test]
fn replay_persists_capacity_edit_and_rejects_stale_revision_after_load() {
    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("commands.json");
    let checkpoint = dir.path().join("checkpoint.json");
    let trace_path = dir.path().join("trace.jsonl");
    fs::write(
        &commands,
        r#"[
            {"tick":10,"command":{"commit_edit":{
                "expected_revision":0,"actions":[{"set_capacity":{"lane":1,"capacity_mm":120000}}]
            }}}
        ]"#,
    )
    .unwrap();

    let edited = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", OsStr::new("10")),
        ("--trace", trace_path.as_os_str()),
        ("--save", checkpoint.as_os_str()),
    ]));
    assert_eq!(edited["tick"], 10);
    assert_eq!(edited["lanes"][0]["id"], 1);
    assert_eq!(edited["lanes"][0]["capacity_mm"], 120_000);
    let rows = trace(&trace_path);
    let before_edit = rows.last().unwrap();
    assert_eq!(before_edit["tick"], 10);
    assert_eq!(before_edit["lanes"][0]["capacity_mm"], 60_000);
    assert_eq!(summary(run(&[("--load", checkpoint.as_os_str())])), edited);

    fs::write(
        &commands,
        r#"[
            {"tick":11,"command":{"commit_edit":{
                "expected_revision":0,"actions":[{"set_capacity":{"lane":1,"capacity_mm":180000}}]
            }}}
        ]"#,
    )
    .unwrap();
    let destination = dir.path().join("destination.json");
    let output = run(&[
        ("--load", checkpoint.as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", OsStr::new("11")),
        ("--save", destination.as_os_str()),
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("cannot apply command at tick 11"), "{error}");
    assert!(error.contains("outdated network revision"), "{error}");
    assert!(!destination.exists());
}

#[test]
fn restart_undoes_bridge_edit_before_replaying_the_bus_departure() {
    let dir = tempfile::tempdir().unwrap();
    let before = dir.path().join("before.json");
    let after = dir.path().join("after.json");
    let restored = dir.path().join("restored.json");
    let edit = dir.path().join("edit.json");
    let undo = dir.path().join("undo.json");
    let full_trace = dir.path().join("full.jsonl");
    let resumed_trace = dir.path().join("resumed.jsonl");

    let original = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--until-tick", OsStr::new("10")),
        ("--save", before.as_os_str()),
    ]));
    let uninterrupted = summary(run(&[
        ("--load", before.as_os_str()),
        ("--until-tick", OsStr::new("13")),
        ("--trace", full_trace.as_os_str()),
    ]));

    fs::write(
        &edit,
        r#"[{"tick":10,"command":{"commit_edit":{
            "expected_revision":0,"actions":[{"set_capacity":{"lane":1,"capacity_mm":120000}}]
        }}}]"#,
    )
    .unwrap();
    let edited = summary(run(&[
        ("--load", before.as_os_str()),
        ("--until-tick", OsStr::new("12")),
        ("--commands", edit.as_os_str()),
        ("--save", after.as_os_str()),
    ]));
    assert_eq!(edited["tick"], 12);
    assert_eq!(edited["lanes"][0]["capacity_mm"], 120_000);

    fs::write(&undo, r#"[{"tick":12,"command":"undo_edit"}]"#).unwrap();
    assert_eq!(
        summary(run(&[
            ("--load", after.as_os_str()),
            ("--commands", undo.as_os_str()),
            ("--save", restored.as_os_str()),
        ])),
        original
    );
    assert_eq!(
        summary(run(&[
            ("--load", restored.as_os_str()),
            ("--until-tick", OsStr::new("13")),
        ])),
        uninterrupted
    );
    assert_eq!(
        summary(run(&[
            ("--load", after.as_os_str()),
            ("--commands", undo.as_os_str()),
            ("--until-tick", OsStr::new("13")),
            ("--trace", resumed_trace.as_os_str()),
        ])),
        uninterrupted
    );
    assert_eq!(trace(&resumed_trace), trace(&full_trace));
    assert_eq!(uninterrupted["lanes"][0]["departure"]["to"], 3);
    assert_eq!(
        uninterrupted["lanes"][0]["departure"]["entry"]["vehicle"]["id"],
        12
    );
}

#[test]
fn replay_applies_following_commands_at_their_ticks_after_undo() {
    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("commands.json");
    fs::write(
        &commands,
        r#"[
            {"tick":10,"command":{"commit_edit":{
                "expected_revision":0,"actions":[{"set_capacity":{"lane":1,"capacity_mm":120000}}]
            }}},
            {"tick":12,"command":"undo_edit"},
            {"tick":11,"command":{"set_growth_paused":true}}
        ]"#,
    )
    .unwrap();

    let result = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", OsStr::new("13")),
    ]));
    assert_eq!(result["tick"], 13);
    assert_eq!(result["clock"]["development_ticks"], 11);
    assert_eq!(result["clock"]["growth_paused"], true);
}

#[test]
fn replay_sets_and_removes_a_persistent_marker() {
    use sim_core::{Cell, LaneId, Marker, MarkerAnchor, MarkerId, Observation, Query, Simulation};

    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("commands.json");
    let marked = dir.path().join("marked.json");
    let unmarked = dir.path().join("unmarked.json");
    fs::write(
        &commands,
        r#"[{"tick":0,"command":{"set_marker":{"id":6,"cell":{"x":1,"y":2},"anchor":{"lane":3}}}}]"#,
    )
    .unwrap();
    summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--save", marked.as_os_str()),
    ]));
    let sim = Simulation::load(&fs::read(&marked).unwrap()).unwrap();
    let Observation::Markers(markers) = sim.inspect(Query::Markers).unwrap() else {
        panic!("expected a marker observation");
    };
    assert_eq!(markers.len(), 1);
    assert_eq!(
        markers[0].marker,
        Marker {
            id: MarkerId::new(6),
            cell: Cell { x: 1, y: 2 },
            anchor: Some(MarkerAnchor::Lane(LaneId::new(3))),
        }
    );
    assert!(markers[0].anchor_present);

    fs::write(&commands, r#"[{"tick":0,"command":{"remove_marker":6}}]"#).unwrap();
    summary(run(&[
        ("--load", marked.as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--save", unmarked.as_os_str()),
    ]));
    let sim = Simulation::load(&fs::read(&unmarked).unwrap()).unwrap();
    assert_eq!(
        sim.inspect(Query::Markers).unwrap(),
        Observation::Markers(Vec::new())
    );
}

#[test]
fn replay_rejects_invalid_boundaries_without_creating_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("commands.json");
    let save = dir.path().join("save.json");

    for (plan, message) in [
        (r#"[{"tick":0,"command":{"set_paused":true}}]"#, "paused"),
        (
            r#"[{"tick":1,"command":{"set_paused":false}},{"tick":0,"command":{"set_growth_paused":true}}]"#,
            "out of order",
        ),
    ] {
        fs::write(&commands, plan).unwrap();
        let output = run(&[
            ("--scenario", bridge().as_os_str()),
            ("--commands", commands.as_os_str()),
            ("--until-tick", "1".as_ref()),
            ("--save", save.as_os_str()),
        ]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(message));
        assert!(!save.exists());
    }
}

#[test]
fn replay_rejects_commands_before_loaded_tick() {
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = dir.path().join("checkpoint.json");
    let commands = dir.path().join("commands.json");
    fs::write(&commands, r#"[{"tick":9,"command":{"set_paused":false}}]"#).unwrap();
    summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--until-tick", "10".as_ref()),
        ("--save", checkpoint.as_os_str()),
    ]));

    let output = run(&[
        ("--load", checkpoint.as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", "10".as_ref()),
    ]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("command at tick 9"), "{error}");
    assert!(error.contains("current tick 10"), "{error}");
}

#[test]
fn paused_boundary_without_an_advance_produces_no_trace_row() {
    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("commands.json");
    let trace_path = dir.path().join("trace.jsonl");
    fs::write(&commands, r#"[{"tick":0,"command":{"set_paused":true}}]"#).unwrap();

    let output = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--trace", trace_path.as_os_str()),
    ]));
    assert_eq!(output["tick"], 0);
    assert_eq!(output["clock"]["paused"], true);
    assert!(trace(&trace_path).is_empty());
}

#[test]
fn trace_cannot_replace_a_checkpoint_through_a_path_alias() {
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = dir.path().join("checkpoint.json");
    summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--until-tick", "10".as_ref()),
        ("--save", checkpoint.as_os_str()),
    ]));
    let original = fs::read(&checkpoint).unwrap();
    fs::create_dir(dir.path().join("subdirectory")).unwrap();
    let indirect_path = dir.path().join("subdirectory/../checkpoint.json");
    let hard_link = dir.path().join("trace-hard-link");
    fs::hard_link(&checkpoint, &hard_link).unwrap();
    let mut aliases = vec![indirect_path, hard_link];
    #[cfg(unix)]
    {
        let symlink = dir.path().join("trace-symlink");
        std::os::unix::fs::symlink(&checkpoint, &symlink).unwrap();
        aliases.push(symlink);
    }

    for alias in aliases {
        let output = run(&[
            ("--load", checkpoint.as_os_str()),
            ("--until-tick", "11".as_ref()),
            ("--trace", alias.as_os_str()),
        ]);
        assert!(!output.status.success(), "trace alias {}", alias.display());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--trace"),
            "trace alias {}",
            alias.display()
        );
        assert_eq!(fs::read(&checkpoint).unwrap(), original);
    }
}

#[test]
fn paused_single_step_is_traced_and_leaves_the_clock_paused() {
    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("commands.json");
    let trace_path = dir.path().join("trace.jsonl");
    fs::write(
        &commands,
        r#"[{"tick":0,"command":{"set_paused":true}},{"tick":0,"command":"step"}]"#,
    )
    .unwrap();
    let result = summary(run(&[
        ("--scenario", bridge().as_os_str()),
        ("--commands", commands.as_os_str()),
        ("--until-tick", OsStr::new("1")),
        ("--trace", trace_path.as_os_str()),
    ]));
    assert_eq!(result["tick"], 1);
    assert_eq!(result["clock"]["paused"], true);
    let rows = trace(&trace_path);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["tick"], 1);
}

#[test]
fn new_process_preserves_rotating_priority_and_fractional_discharge() {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let scenario_path = dir.path().join("junction.json");
    let checkpoint = dir.path().join("checkpoint.json");
    let full_trace = dir.path().join("full.jsonl");
    let resumed_trace = dir.path().join("resumed.jsonl");
    let lanes: Vec<_> = (1..=3)
        .map(|id| {
            json!({
                "id": id, "capacity_mm": 30000, "travel_ticks": 1,
                "discharge": {"millimetres": 5000, "ticks": 2}, "allowed": ["car"]
            })
        })
        .collect();
    let movements: Vec<_> = (1..=2)
        .map(|id| {
            json!({
                "id": id, "from": id, "to": 3, "junction": 1,
                "conflict_zones": [1], "priority": 0
            })
        })
        .collect();
    let traffic: Vec<_> = (1..=2)
        .map(|id| {
            json!({
                "lane": id, "entry": {"private_cars": 4}, "ready_at": 1, "route": [id]
            })
        })
        .collect();
    fs::write(
        &scenario_path,
        serde_json::to_vec(&json!({
            "seed": 42, "start": 0, "development_period_ticks": 86400, "building_ids": [],
            "lanes": lanes, "movements": movements, "junctions": [{"id": 1, "phases": []}],
            "traffic": traffic
        }))
        .unwrap(),
    )
    .unwrap();
    let full = summary(run(&[
        ("--scenario", scenario_path.as_os_str()),
        ("--until-tick", OsStr::new("5")),
        ("--trace", full_trace.as_os_str()),
    ]));
    summary(run(&[
        ("--scenario", scenario_path.as_os_str()),
        ("--until-tick", OsStr::new("2")),
        ("--save", checkpoint.as_os_str()),
    ]));
    fs::remove_file(scenario_path).unwrap();
    let resumed = summary(run(&[
        ("--load", checkpoint.as_os_str()),
        ("--until-tick", OsStr::new("5")),
        ("--trace", resumed_trace.as_os_str()),
    ]));
    let rows = trace(&resumed_trace);
    assert_eq!(rows.len(), 3);
    let winners: Vec<_> = rows
        .iter()
        .map(|row| {
            let lanes = row["lanes"].as_array().unwrap();
            let winning: Vec<_> = lanes[..2]
                .iter()
                .filter(|lane| lane["departure"]["to"] == 3)
                .map(|lane| lane["id"].as_u64().unwrap())
                .collect();
            assert_eq!(winning.len(), 1);
            winning[0]
        })
        .collect();
    assert_eq!(winners, [2, 1, 2]);
    assert_eq!(rows, trace(&full_trace)[2..]);
    assert_eq!(resumed, full);
}

#[test]
fn checkpoint_can_be_updated_in_place_but_cannot_replace_its_scenario() {
    let dir = tempfile::tempdir().unwrap();
    let scenario = dir.path().join("bridge.json");
    let checkpoint = dir.path().join("checkpoint.json");
    let initial = fs::read(bridge()).unwrap();
    fs::write(&scenario, &initial).unwrap();
    let output = run(&[
        ("--scenario", scenario.as_os_str()),
        ("--save", scenario.as_os_str()),
    ]);
    assert!(!output.status.success());
    assert_eq!(fs::read(&scenario).unwrap(), initial);

    summary(run(&[
        ("--scenario", scenario.as_os_str()),
        ("--until-tick", OsStr::new("10")),
        ("--save", checkpoint.as_os_str()),
    ]));
    let advanced = summary(run(&[
        ("--load", checkpoint.as_os_str()),
        ("--until-tick", OsStr::new("13")),
        ("--save", checkpoint.as_os_str()),
    ]));
    let reopened = summary(run(&[("--load", checkpoint.as_os_str())]));
    assert_eq!(reopened, advanced);
}

#[test]
fn new_process_resumes_a_bus_mid_dwell() {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("service.json");
    let checkpoint = dir.path().join("checkpoint.json");
    let full_trace = dir.path().join("full.jsonl");
    let suffix_trace = dir.path().join("suffix.jsonl");
    let lanes: Vec<_> = (1..=2)
        .map(|id| {
            json!({
                "id": id, "capacity_mm": 50000, "travel_ticks": 1,
                "discharge":{"millimetres": 50000, "ticks": 1}, "allowed":["bus"]
            })
        })
        .collect();
    let moves: Vec<_> = [(1, 2), (2, 1)].into_iter().map(|(from, to)| json!({
        "id": from, "from": from, "to": to, "junction": 1, "conflict_zones":[1], "priority": 0
    })).collect();
    let stops: Vec<_> = (1..=2)
        .map(|id| {
            json!({
                "id": id, "zone": id, "lane": id, "capacity": 20,
                "minimum_dwell": 3, "passengers_per_tick": 10
            })
        })
        .collect();
    let zones: Vec<_> = (1..=2)
        .map(|id| {
            json!({
                "id": id, "residents": 0, "jobs": 0, "shops": 0, "road_access": null
            })
        })
        .collect();
    let data = json!({
        "seed": 7, "start": 0, "development_period_ticks": 86400, "building_ids": [],
        "lanes": lanes, "movements": moves, "junctions": [{"id":1, "phases":[]}], "traffic":[],
        "demand": { "zones": zones },
        "service": {
            "stops": stops,
            "lines": [{"id":1, "kind":"bus", "stops":[1,2], "headway":20, "first_departure":1}],
            "fleet": [{"id":12, "line":1, "length_mm":12000, "capacity":8}]
        }
    });
    fs::write(&fixture, serde_json::to_vec(&data).unwrap()).unwrap();
    let baseline = summary(run(&[
        ("--scenario", fixture.as_os_str()),
        ("--until-tick", OsStr::new("7")),
        ("--trace", full_trace.as_os_str()),
    ]));
    let at_halt = summary(run(&[
        ("--scenario", fixture.as_os_str()),
        ("--until-tick", OsStr::new("3")),
        ("--save", checkpoint.as_os_str()),
    ]));
    assert_eq!(at_halt["lanes"][0]["wait_reason"], "vehicle_stop");
    let continued = summary(run(&[
        ("--load", checkpoint.as_os_str()),
        ("--until-tick", OsStr::new("7")),
        ("--trace", suffix_trace.as_os_str()),
    ]));
    let tail = trace(&suffix_trace);
    assert_eq!(tail, trace(&full_trace)[3..]);
    assert_eq!(tail[1]["tick"], 5);
    assert_eq!(
        tail[1]["lanes"][0]["departure"]["entry"]["vehicle"]["id"],
        12
    );
    assert_eq!(continued, baseline);
}

#[test]
fn new_process_resumes_passengers_mid_transfer() {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("transfer.json");
    let checkpoint = dir.path().join("checkpoint.json");
    let destination = dir.path().join("destination.json");
    let full_trace = dir.path().join("full.jsonl");
    let suffix_trace = dir.path().join("suffix.jsonl");
    let lanes: Vec<_> = [(1, "bus"), (2, "bus"), (3, "tram"), (4, "tram")]
        .into_iter()
        .map(|(id, kind)| {
            json!({
                "id": id, "capacity_mm": 60000, "travel_ticks": 1,
                "discharge": {"millimetres": 60000, "ticks": 1}, "allowed": [kind]
            })
        })
        .collect();
    let movements: Vec<_> = [(1, 2, 1), (2, 1, 1), (3, 4, 2), (4, 3, 2)]
        .into_iter()
        .map(|(from, to, conflict_zone)| {
            json!({
                "id": from, "from": from, "to": to, "junction": 1,
                "conflict_zones": [conflict_zone], "priority": 0
            })
        })
        .collect();
    let stops: Vec<_> = (1..=4)
        .map(|id| {
            json!({
                "id": id, "zone": id, "lane": id, "capacity": 20,
                "minimum_dwell": 3, "passengers_per_tick": 16
            })
        })
        .collect();
    let zones: Vec<_> = [(1, 4, 0), (2, 0, 0), (3, 0, 0), (4, 0, 4)]
        .into_iter()
        .map(|(id, residents, jobs)| {
            json!({"id": id, "residents": residents, "jobs": jobs, "shops": 0, "road_access": null})
        })
        .collect();
    let scenario = json!({
        "seed": 42, "start": 0, "development_period_ticks": 86400, "building_ids": [],
        "lanes": lanes, "movements": movements, "junctions": [{"id": 1, "phases": []}], "traffic": [],
        "demand": {
            "zones": zones, "walk_weight": 0, "car_weight": 0,
            "foot_edges": [{"id": 1, "from": 2, "to": 3, "travel_ticks": 2, "capacity": 10, "discharge": 10}],
            "profiles": [{"purpose": "work", "departure_offsets": [0], "return_offset": 1000,
                "rate_numerator": 1, "rate_denominator": 1}]
        },
        "service": {
            "stops": stops,
            "lines": [
                {"id": 1, "kind": "bus", "stops": [1, 2], "headway": 20, "first_departure": 1},
                {"id": 2, "kind": "tram", "stops": [3, 4], "headway": 20, "first_departure": 5}
            ],
            "fleet": [
                {"id": 12, "line": 1, "length_mm": 12000, "capacity": 8},
                {"id": 21, "line": 2, "length_mm": 20000, "capacity": 8}
            ]
        }
    });
    fs::write(&fixture, serde_json::to_vec(&scenario).unwrap()).unwrap();

    let uninterrupted = summary(run(&[
        ("--scenario", fixture.as_os_str()),
        ("--until-tick", OsStr::new("11")),
        ("--trace", full_trace.as_os_str()),
    ]));
    let transfer = summary(run(&[
        ("--scenario", fixture.as_os_str()),
        ("--until-tick", OsStr::new("8")),
        ("--save", checkpoint.as_os_str()),
    ]));
    assert_eq!(transfer["tick"], 8);
    let saved: Value = serde_json::from_slice(&fs::read(&checkpoint).unwrap()).unwrap();
    let walkers = saved["payload"]["demand"]["transit"]["walking"]
        .as_object()
        .expect("the checkpoint must contain the foot transfer");
    assert_eq!(walkers.len(), 1);
    let part = walkers.values().next().unwrap();
    assert_eq!(part["leg"], 1);
    assert_eq!(part["amount"], 4);

    fs::remove_file(fixture).unwrap();
    assert_eq!(
        summary(run(&[("--load", checkpoint.as_os_str())])),
        transfer
    );
    let resumed = summary(run(&[
        ("--load", checkpoint.as_os_str()),
        ("--until-tick", OsStr::new("11")),
        ("--trace", suffix_trace.as_os_str()),
        ("--save", destination.as_os_str()),
    ]));
    let tail = trace(&suffix_trace);
    assert_eq!(tail, trace(&full_trace)[8..]);
    assert_eq!(tail[1]["tick"], 10);
    let tram = json!({"id": 21, "kind": "tram", "length_mm": 20000});
    assert_eq!(tail[1]["lanes"][2]["id"], 3);
    assert_eq!(
        tail[1]["lanes"][2]["departure"],
        json!({"entry": {"vehicle": tram}, "to": 4})
    );
    assert_eq!(tail[1]["lanes"][3]["id"], 4);
    assert_eq!(tail[1]["lanes"][3]["queue"][0]["vehicle"], tram);
    assert_eq!(resumed["tick"], 11);
    assert_eq!(resumed, uninterrupted);

    let arrived: Value = serde_json::from_slice(&fs::read(destination).unwrap()).unwrap();
    assert_eq!(arrived["payload"]["demand"]["arrived"], 4);
}

#[test]
fn new_process_resumes_taxi_before_pickup_and_with_passenger() {
    use serde_json::json;
    use sim_core::{DemandObservation, Observation, Query, Simulation};

    fn saved_demand(path: &Path) -> DemandObservation {
        let simulation = Simulation::load(&fs::read(path).unwrap()).unwrap();
        let Observation::Demand(demand) = simulation.inspect(Query::Demand).unwrap() else {
            panic!("expected demand observation");
        };
        demand
    }

    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("taxi.json");
    let full_trace = dir.path().join("full.jsonl");
    let destination = dir.path().join("destination.json");
    let lanes: Vec<_> = [(1, 1), (2, 3), (3, 1)]
        .into_iter()
        .map(|(id, travel_ticks)| {
            json!({
                "id": id, "capacity_mm": 30000, "travel_ticks": travel_ticks,
                "discharge": {"millimetres": 30000, "ticks": 1}, "allowed": ["taxi"]
            })
        })
        .collect();
    let movements: Vec<_> = [(1, 3, 1), (2, 1, 2), (3, 2, 1)]
        .into_iter()
        .map(|(id, from, to)| {
            json!({
                "id": id, "from": from, "to": to, "junction": 1,
                "conflict_zones": [id], "priority": 0
            })
        })
        .collect();
    let scenario = json!({
        "seed": 42, "start": 0, "development_period_ticks": 86400, "building_ids": [],
        "lanes": lanes, "movements": movements, "traffic": [],
        "junctions": [{"id": 1, "phases": [
            {"green_ticks": 4, "clearance_ticks": 0, "movements": [2, 3]},
            {"green_ticks": 60, "clearance_ticks": 0, "movements": [1, 2, 3]}
        ]}],
        "demand": {
            "zones": [
                {"id": 1, "residents": 1, "jobs": 0, "shops": 0, "road_access": 1},
                {"id": 2, "residents": 0, "jobs": 1, "shops": 0, "road_access": 2},
                {"id": 3, "residents": 0, "jobs": 0, "shops": 0, "road_access": 3}
            ],
            "walk_weight": 0, "car_weight": 0,
            "profiles": [{"purpose": "work", "departure_offsets": [0], "return_offset": 1000,
                "rate_numerator": 1, "rate_denominator": 1}]
        },
        "taxi": {"fleet": [
            {"id": 9, "start": 3, "length_mm": 5000, "capacity": 2},
            {"id": 7, "start": 3, "length_mm": 5000, "capacity": 2}
        ]}
    });
    fs::write(&fixture, serde_json::to_vec(&scenario).unwrap()).unwrap();

    let uninterrupted = summary(run(&[
        ("--scenario", fixture.as_os_str()),
        ("--until-tick", OsStr::new("9")),
        ("--trace", full_trace.as_os_str()),
        ("--save", destination.as_os_str()),
    ]));
    assert_eq!(uninterrupted["tick"], 9);
    let arrived = saved_demand(&destination);
    assert_eq!(arrived.arrived, 1);
    assert_eq!(arrived.total_travel_ticks, 8);

    let taxi = json!({"id": 7, "kind": "taxi", "length_mm": 5000});
    let full = trace(&full_trace);
    assert_eq!(full.len(), 9);
    assert_eq!(full[3]["lanes"][2]["wait_reason"], "red_signal");
    assert_eq!(full[3]["lanes"][2]["queue"][0]["vehicle"], taxi);
    assert_eq!(
        full[4]["lanes"][2]["departure"],
        json!({
            "entry": {"vehicle": taxi}, "to": 1
        })
    );
    assert_eq!(full[5]["lanes"][1]["queue"][0]["vehicle"], taxi);
    assert_eq!(full[8]["lanes"][1]["queue"], json!([]));

    let mut checkpoints = Vec::new();
    for (tick, waiting, in_transit) in [(4_usize, 1, 0), (6, 0, 1)] {
        let checkpoint = dir.path().join(format!("checkpoint-{tick}.json"));
        let state = summary(run(&[
            ("--scenario", fixture.as_os_str()),
            ("--until-tick", tick.to_string().as_ref()),
            ("--save", checkpoint.as_os_str()),
        ]));
        assert_eq!(state["tick"], tick);
        let demand = saved_demand(&checkpoint);
        assert_eq!(demand.waiting, waiting, "checkpoint at tick {tick}");
        assert_eq!(demand.in_transit, in_transit, "checkpoint at tick {tick}");
        checkpoints.push((tick, checkpoint, state));
    }

    fs::remove_file(fixture).unwrap();
    let full_jsonl = fs::read_to_string(full_trace).unwrap();
    let suffix_trace = dir.path().join("suffix.jsonl");
    for (tick, checkpoint, state) in checkpoints {
        assert_eq!(
            summary(run(&[("--load", checkpoint.as_os_str())])),
            state,
            "loading checkpoint at tick {tick}"
        );
        let resumed = summary(run(&[
            ("--load", checkpoint.as_os_str()),
            ("--until-tick", OsStr::new("9")),
            ("--trace", suffix_trace.as_os_str()),
        ]));
        assert_eq!(resumed, uninterrupted, "resuming from tick {tick}");
        let expected: String = full_jsonl.split_inclusive('\n').skip(tick).collect();
        assert_eq!(
            fs::read_to_string(&suffix_trace).unwrap(),
            expected,
            "trace after tick {tick}"
        );
    }
}
