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
