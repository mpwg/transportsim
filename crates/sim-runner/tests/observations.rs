use std::{fs, path::PathBuf, process::Command};

use serde_json::{Value, json};

fn bridge_city() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/bridge-city.json")
}

fn run(args: &mut Command) -> Value {
    let output = args.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn runner() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sim-runner"));
    command.arg("run");
    command
}

fn by_id(rows: &Value, id: u64) -> &Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap()
}

#[test]
fn summary_and_trace_report_demand_and_vehicle_operations_at_tick_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("observations.jsonl");
    let save = dir.path().join("checkpoint.json");
    let mut summary = run(runner()
        .arg("--scenario")
        .arg(bridge_city())
        .args(["--until-tick", "360", "--trace"])
        .arg(&trace)
        .arg("--save")
        .arg(&save));
    assert_eq!(summary, run(runner().arg("--load").arg(&save)));
    assert!(
        summary
            .as_object_mut()
            .unwrap()
            .remove("fingerprint")
            .is_some()
    );

    let rows: Vec<Value> = fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 360);
    assert_eq!(rows.last().unwrap(), &summary);
    assert_eq!(summary["demand"]["generated"], 139);
    assert_eq!(summary["demand"]["arrived"], 139);
    assert_eq!(summary["demand"]["total_travel_ticks"], "9222");
    assert_eq!(summary["demand"]["active_cohorts"], 0);
    assert!(rows[0]["demand"]["active_cohorts"].as_u64().unwrap() > 0);

    for row in &rows {
        let demand = &row["demand"];
        assert!(demand.get("active").is_none());
        assert_eq!(
            demand["generated"].as_u64().unwrap(),
            demand["waiting"].as_u64().unwrap()
                + demand["in_transit"].as_u64().unwrap()
                + demand["arrived"].as_u64().unwrap()
        );
    }
    let before_departure = &rows[9]["service"];
    let stop = by_id(&before_departure["stops"], 1);
    assert_eq!(stop["waiting_total"], 52);
    assert_eq!(stop["oldest_waiting_since"], 1);
    let bus = by_id(&rows[30]["service"]["vehicles"], 13);
    assert_eq!(bus["kind"], "bus");
    assert_eq!(bus["capacity"], 52);
    assert_eq!(bus["onboard"], 52);
    assert_eq!(rows[40]["demand"]["transit_transferring"], 52);
    assert_eq!(by_id(&rows[52]["service"]["vehicles"], 21)["onboard"], 52);
    let tram_line = by_id(&rows[52]["service"]["lines"], 2);
    assert_eq!(tram_line["last_departure"], 53);
    assert_eq!(tram_line["departed"], 1);

    let pickup = by_id(&rows[40]["taxi"]["vehicles"], 31);
    assert_eq!(pickup["status"], "to_pickup");
    assert_eq!(
        pickup["assignment"],
        json!({"origin": 1, "destination": 5, "passengers": 3})
    );
    let occupied = by_id(&rows[42]["taxi"]["vehicles"], 31);
    assert_eq!(occupied["status"], "occupied");
    assert_eq!(occupied["assignment"], pickup["assignment"]);
    assert_eq!(summary["taxi"]["waiting_total"], 0);
}

#[test]
fn trace_reports_ready_queue_space_and_individual_vehicle_progress() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lanes.jsonl");
    run(runner()
        .arg("--scenario")
        .arg(bridge_city())
        .args(["--until-tick", "15", "--trace"])
        .arg(&path));
    let rows: Vec<Value> = fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    let queue = by_id(&rows[4]["lanes"], 1);
    assert!(queue["ready_queue_mm"].as_u64().unwrap() >= 52_000);
    let entered = by_id(&rows[12]["lanes"], 3);
    assert_eq!(
        by_id(&entered["vehicles"], 12),
        &json!({"id": 12, "travel_ticks": 5, "remaining_travel_ticks": 5})
    );
    let underway = by_id(&rows[14]["lanes"], 3);
    assert_eq!(
        by_id(&underway["vehicles"], 12)["remaining_travel_ticks"],
        3
    );
}

#[test]
fn summary_reports_why_the_meadow_is_reserved_and_how_long_construction_takes() {
    let dir = tempfile::tempdir().unwrap();
    let fast_city = dir.path().join("accelerated.json");
    let mut city: Value = serde_json::from_slice(&fs::read(bridge_city()).unwrap()).unwrap();
    city["development_period_ticks"] = json!(240);
    fs::write(&fast_city, serde_json::to_vec(&city).unwrap()).unwrap();

    let app = run(runner()
        .arg("--scenario")
        .arg(&fast_city)
        .args(["--until-tick", "240"]));
    let growth = &app["growth"];
    assert_eq!(growth["totals"]["residents"], 280);
    assert_eq!(growth["sites"][0]["building"], 7);
    assert_eq!(growth["sites"][0]["reason"], "reserved");
    assert!(growth["sites"][0]["accessibility"].as_u64().unwrap() > 0);
    assert_eq!(
        growth["projects"][0],
        json!({"building": 7, "usage": "housing", "amount": 24, "remaining_periods": 3})
    );
}
