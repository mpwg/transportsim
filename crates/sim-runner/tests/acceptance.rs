use std::{
    collections::BTreeSet,
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use sim_core::{
    BuildingId, Cell, Command as SimCommand, CommandError, DemandObservation, EditAction,
    EditCommand, EditError, GrowthObservation, JourneyMode, LaneId, Observation, Query, QueueEntry,
    Simulation, TaxiStatus, Terrain, VehicleKind, ZoneId,
};

fn scenario(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios")
        .join(name)
}

fn runner() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sim-runner"));
    command.arg("run");
    command
}

fn run(command: &mut Command) -> Value {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn load(path: &Path) -> Simulation {
    Simulation::load(&fs::read(path).unwrap()).unwrap()
}

fn demand(sim: &Simulation) -> DemandObservation {
    let Observation::Demand(demand) = sim.inspect(Query::Demand).unwrap() else {
        panic!("expected a demand observation");
    };
    assert_eq!(
        demand.generated,
        demand.waiting + demand.in_transit + demand.arrived
    );
    demand
}

fn trace(path: &Path) -> impl Iterator<Item = Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect::<Vec<_>>()
        .into_iter()
}

fn assert_lane_invariants(state: &Value) {
    let mut vehicles = BTreeSet::new();
    for lane in state["lanes"].as_array().unwrap() {
        let occupied = lane["occupied_mm"].as_u64().unwrap();
        assert!(occupied <= lane["capacity_mm"].as_u64().unwrap());
        let mut described = 0;
        for item in lane["queue"].as_array().unwrap() {
            if let Some(cars) = item["private_cars"].as_u64() {
                described += cars * 5_000;
            } else {
                let vehicle = &item["vehicle"];
                assert!(vehicles.insert(vehicle["id"].as_u64().unwrap()));
                described += vehicle["length_mm"].as_u64().unwrap();
            }
        }
        assert_eq!(occupied, described);
    }
}

fn lane(state: &Value, id: u64) -> &Value {
    state["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|lane| lane["id"] == id)
        .unwrap()
}

fn checkpoint(name: &str, tick: u64, path: &Path) -> Value {
    run(runner()
        .arg("--scenario")
        .arg(scenario(name))
        .arg("--until-tick")
        .arg(tick.to_string())
        .arg("--save")
        .arg(path))
}

fn assert_vehicle_invariants(sim: &Simulation) {
    let mut locations = BTreeSet::new();
    for id in sim.lane_ids() {
        let Observation::Lane(lane) = sim.inspect(Query::Lane(id)).unwrap() else {
            panic!("expected a lane observation");
        };
        assert!(lane.occupied_mm <= lane.capacity_mm);
        let mut described = 0;
        for item in lane.queue {
            described += match item {
                QueueEntry::PrivateCars(count) => count.get() * 5_000,
                QueueEntry::Vehicle { id, length_mm, .. } => {
                    assert!(locations.insert(id));
                    length_mm.get()
                }
            };
        }
        assert_eq!(lane.occupied_mm, described);
    }
    let Observation::Service(service) = sim.inspect(Query::Service).unwrap() else {
        panic!("expected a service observation");
    };
    for vehicle in service.vehicles {
        assert!(vehicle.onboard <= vehicle.capacity);
    }
    let Observation::Taxi(taxi) = sim.inspect(Query::Taxi).unwrap() else {
        panic!("expected a taxi observation");
    };
    for vehicle in taxi.vehicles {
        if let TaxiStatus::Occupied { assignment } | TaxiStatus::ToPickup { assignment } =
            vehicle.status
        {
            assert!((1..=vehicle.capacity).contains(&assignment.passengers));
        }
    }
}

#[test]
fn bridge_city_starts_on_a_river_and_completes_generated_journeys() {
    let dir = tempfile::tempdir().unwrap();
    let save = dir.path().join("city.json");
    let rows = dir.path().join("city.jsonl");
    let summary = run(runner()
        .arg("--scenario")
        .arg(scenario("bridge-city.json"))
        .args(["--until-tick", "360", "--trace"])
        .arg(&rows)
        .arg("--save")
        .arg(&save));
    assert_eq!(summary["tick"], 360);
    let sim = load(&save);
    let Observation::Layout(layout) = sim.inspect(Query::Layout).unwrap() else {
        panic!("expected a layout observation");
    };
    let water: BTreeSet<_> = layout
        .tiles
        .iter()
        .filter(|tile| tile.terrain == Terrain::Water)
        .map(|tile| tile.cell)
        .collect();
    assert!(water.len() >= 3);
    assert!(
        layout.streets.iter().any(|street| {
            street.bridge && street.cells.iter().any(|cell| water.contains(cell))
        })
    );
    assert!(layout.buildings.len() >= 4);

    let rows: Vec<_> = trace(&rows).collect();
    for row in &rows {
        assert_lane_invariants(row);
    }
    let approach = lane(&rows[4], 1);
    assert_eq!(approach["wait_reason"], "red_signal");
    assert_eq!(approach["queue"][0], json!({"private_cars": 3}));
    assert_eq!(approach["queue"][1]["vehicle"]["id"], 12);
    assert_eq!(approach["queue"][2], json!({"private_cars": 5}));
    assert_eq!(
        lane(&rows[12], 1)["departure"]["entry"]["vehicle"]["id"],
        12
    );
    assert_eq!(lane(&rows[12], 1)["departure"]["to"], 3);
    assert_eq!(
        lane(&rows[52], 11)["departure"]["entry"]["vehicle"]["id"],
        21
    );
    assert_eq!(lane(&rows[52], 11)["departure"]["to"], 12);

    let result = demand(&sim);
    // The first work profile sends half of each neighborhood, rounded down.
    assert_eq!(result.generated, 92 + 27 + 20);
    assert_eq!(result.arrived, result.generated);
    assert_vehicle_invariants(&sim);
}

#[test]
fn loaded_city_keeps_generated_demand_and_a_bridge_queue_in_balance() {
    let dir = tempfile::tempdir().unwrap();
    let save = dir.path().join("load.json");
    let rows = dir.path().join("load.jsonl");
    let summary = run(runner()
        .arg("--scenario")
        .arg(scenario("load-10000.json"))
        .args(["--until-tick", "120", "--trace"])
        .arg(&rows)
        .arg("--save")
        .arg(&save));
    assert_eq!(summary["tick"], 120);
    let sim = load(&save);
    let result = demand(&sim);
    // The first two rush-hour waves sent one eighth each, including carried remainders.
    assert_eq!(result.generated, 2 * (6_200 + 2_400 + 1_400) / 8);
    assert_eq!(result.unassigned, 0);
    assert!(result.arrived > 0);
    assert!(result.waiting > 0);
    assert!(result.active.len() < 100);
    assert_vehicle_invariants(&sim);
    let Observation::Journeys(options) = sim
        .inspect(Query::Journeys {
            from: ZoneId::new(1),
            to: ZoneId::new(5),
        })
        .unwrap()
    else {
        panic!("expected a journey observation");
    };
    for mode in [
        JourneyMode::Walk,
        JourneyMode::Car,
        JourneyMode::Transit,
        JourneyMode::Taxi,
    ] {
        assert!(options.iter().any(|option| option.mode == mode));
    }

    let mut congested = false;
    for row in trace(&rows) {
        assert_lane_invariants(&row);
        congested |= row["lanes"].as_array().unwrap().iter().any(|lane| {
            lane["id"] == 1
                && lane["wait_reason"] == "red_signal"
                && lane["occupied_mm"].as_u64().unwrap() >= 50_000
        });
    }
    assert!(congested);
}

#[test]
fn turn_pocket_frees_the_bus_only_when_all_turning_cars_fit() {
    let dir = tempfile::tempdir().unwrap();
    let save = dir.path().join("red.json");
    let edit = dir.path().join("pocket.json");
    let rows = dir.path().join("pocket.jsonl");
    checkpoint("bridge-city.json", 5, &save);

    for (capacity, first_movement) in [(10_000, 13), (15_000, 6)] {
        let commands = json!([{"tick": 5, "command": {"commit_edit": {
            "expected_revision": 0,
            "actions": [
                {"set_signal": {"id": 4, "phases": []}},
                {"add_turn_pocket": {
                    "source": 1, "turns": [1],
                    "lane": {"id":9,"capacity_mm":capacity,"travel_ticks":1,
                             "discharge":{"millimetres":15000,"ticks":1},"allowed":["car"]},
                    "feeder":{"id":16,"from":1,"to":9,"junction":4,"conflict_zones":[],"priority":0}
                }},
                {"upsert_street": {
                    "lane":9,"cells":[{"x":3,"y":0}],"bridge":false,
                    "road_width_mm":3000,"foot_width_mm":0,"greenery_width_mm":0,"width_mm":3000
                }}
            ]
        }}}]);
        fs::write(&edit, serde_json::to_vec(&commands).unwrap()).unwrap();
        run(runner()
            .arg("--load")
            .arg(&save)
            .args(["--until-tick", "13", "--commands"])
            .arg(&edit)
            .arg("--trace")
            .arg(&rows));

        let rows: Vec<_> = trace(&rows).collect();
        for row in &rows {
            assert_lane_invariants(row);
        }
        assert_eq!(lane(&rows[0], 9)["occupied_mm"], capacity);
        if capacity == 10_000 {
            assert_eq!(lane(&rows[0], 1)["queue"][0], json!({"private_cars": 1}));
            assert_eq!(lane(&rows[0], 1)["wait_reason"], "target_full");
        }
        let crossing = rows
            .iter()
            .find(|row| lane(row, 1)["departure"]["entry"]["vehicle"]["id"] == 12)
            .unwrap();
        assert_eq!(crossing["tick"], first_movement);
        assert_eq!(lane(crossing, 1)["departure"]["to"], 3);
    }
}

#[test]
fn bus_passengers_transfer_to_tram_and_a_taxi_picks_up_its_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let save = dir.path().join("trip.json");
    checkpoint("bridge-city.json", 31, &save);
    let mut sim = load(&save);

    for (offset, id, kind) in [(0, 13, VehicleKind::Bus), (22, 21, VehicleKind::Tram)] {
        sim.advance(offset).unwrap();
        let Observation::Service(service) = sim.inspect(Query::Service).unwrap() else {
            panic!("expected a service observation");
        };
        let carrying = service
            .vehicles
            .iter()
            .find(|vehicle| vehicle.id.get() == id)
            .unwrap();
        assert_eq!(carrying.kind, kind);
        assert_eq!(carrying.onboard, 52);
        demand(&sim);
        assert_vehicle_invariants(&sim);
    }

    checkpoint("bridge-city.json", 41, &save);
    let mut transfer = load(&save);
    assert_eq!(demand(&transfer).transit_transferring, 52);
    let Observation::Taxi(taxi) = transfer.inspect(Query::Taxi).unwrap() else {
        panic!("expected a taxi observation");
    };
    let first_taxi = &taxi.vehicles[0];
    assert_eq!(first_taxi.id.get(), 31);
    let TaxiStatus::ToPickup { assignment } = first_taxi.status else {
        panic!("taxi has not reserved the trip");
    };
    assert_eq!(
        (assignment.origin.get(), assignment.destination.get()),
        (1, 5)
    );
    assert_eq!(assignment.passengers, 3);

    transfer.advance(2).unwrap();
    let Observation::Taxi(taxi) = transfer.inspect(Query::Taxi).unwrap() else {
        panic!("expected a taxi observation");
    };
    assert_eq!(taxi.vehicles[0].status, TaxiStatus::Occupied { assignment });

    let resumed = run(runner()
        .arg("--load")
        .arg(&save)
        .args(["--until-tick", "360"]));
    let direct = checkpoint("bridge-city.json", 360, &save);
    assert_eq!(resumed, direct);
}

#[test]
fn shorter_tram_headway_reduces_end_to_end_travel_for_the_same_departures() {
    let dir = tempfile::tempdir().unwrap();
    let save = dir.path().join("trip.json");
    let commands = dir.path().join("headway.json");
    let on_default_schedule = {
        checkpoint("bridge-city.json", 360, &save);
        demand(&load(&save))
    };
    // The first work wave already chose its destinations and modes at tick 1.
    fs::write(
        &commands,
        r#"[{"tick":1,"command":{"set_line_headway":{"line":2,"ticks":40}}}]"#,
    )
    .unwrap();
    run(runner()
        .arg("--scenario")
        .arg(scenario("bridge-city.json"))
        .args(["--until-tick", "360", "--commands"])
        .arg(&commands)
        .arg("--save")
        .arg(&save));
    let shorter_headway = demand(&load(&save));
    assert_eq!(shorter_headway.generated, shorter_headway.arrived);
    assert_eq!(shorter_headway.arrived, on_default_schedule.arrived);
    assert!(shorter_headway.total_travel_ticks < on_default_schedule.total_travel_ticks);
}

#[test]
fn seeded_edits_and_rejected_commands_preserve_results_across_advance_and_load_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let save = dir.path().join("start.json");
    checkpoint("bridge-city.json", 5, &save);
    let mut one_tick = load(&save);
    let mut batched = load(&save);
    let original = batched.fingerprint().unwrap();
    let mut before_edit = batched.save().unwrap();
    let mut seed = 423_u32;
    let lane_id = LaneId::new(1);

    let too_small = EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::SetCapacity {
            lane: lane_id,
            capacity_mm: NonZeroU32::new(5_000).unwrap(),
        }],
    };
    assert!(matches!(
        batched.apply(SimCommand::CommitEdit(too_small)),
        Err(CommandError::Edit(EditError::Occupied(id))) if id == lane_id
    ));
    assert_eq!(batched.fingerprint().unwrap(), original);

    for revision in 0..12 {
        before_edit = batched.save().unwrap();
        // A fixed sequence keeps capacity and control-boundary choices reproducible.
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let span = 1 + u64::from((seed >> 16) % 21);
        let Observation::Lane(lane) = batched.inspect(Query::Lane(lane_id)).unwrap() else {
            panic!("expected a lane observation");
        };
        let valid = NonZeroU32::new(lane.occupied_mm.max(80_000) + 5_000 * (seed % 8)).unwrap();
        for (sim, invalid_revision) in [(&mut one_tick, revision + 1), (&mut batched, revision + 2)]
        {
            let fingerprint = sim.fingerprint().unwrap();
            let invalid = EditCommand {
                expected_revision: invalid_revision,
                actions: vec![EditAction::SetCapacity {
                    lane: lane_id,
                    capacity_mm: valid,
                }],
            };
            assert!(matches!(
                sim.apply(SimCommand::CommitEdit(invalid)),
                Err(CommandError::Edit(EditError::Outdated { .. }))
            ));
            assert_eq!(fingerprint, sim.fingerprint().unwrap());
            let edit = EditCommand {
                expected_revision: revision,
                actions: vec![EditAction::SetCapacity {
                    lane: lane_id,
                    capacity_mm: valid,
                }],
            };
            sim.apply(SimCommand::CommitEdit(edit)).unwrap();
        }
        for _ in 0..span {
            one_tick.advance(1).unwrap();
            demand(&one_tick);
            assert_vehicle_invariants(&one_tick);
        }
        batched.advance(span).unwrap();
        batched = Simulation::load(&batched.save().unwrap()).unwrap();
        demand(&batched);
        assert_vehicle_invariants(&batched);
        assert_eq!(
            one_tick.fingerprint().unwrap(),
            batched.fingerprint().unwrap()
        );
    }

    batched.apply(SimCommand::UndoEdit).unwrap();
    let before = Simulation::load(&before_edit).unwrap();
    assert_eq!(batched.network_revision(), before.network_revision());
    assert_eq!(batched.inspect(Query::Clock), before.inspect(Query::Clock));
    assert_eq!(
        batched.inspect(Query::Demand),
        before.inspect(Query::Demand)
    );
    assert_eq!(
        batched.inspect(Query::Service),
        before.inspect(Query::Service)
    );
    assert_eq!(batched.inspect(Query::Taxi), before.inspect(Query::Taxi));
    assert_eq!(
        batched.inspect(Query::Growth),
        before.inspect(Query::Growth)
    );
    for id in before.lane_ids() {
        assert_eq!(
            batched.inspect(Query::Lane(id)),
            before.inspect(Query::Lane(id))
        );
    }
    assert!(matches!(
        batched.apply(SimCommand::UndoEdit),
        Err(CommandError::NoUndo)
    ));
}

#[test]
fn serviced_meadow_builds_after_three_development_periods_and_pause_preserves_progress() {
    fn growth(sim: &Simulation) -> GrowthObservation {
        let Observation::Growth(growth) = sim.inspect(Query::Growth).unwrap() else {
            panic!("expected a growth observation");
        };
        growth
    }

    let dir = tempfile::tempdir().unwrap();
    let fast_city = dir.path().join("accelerated.json");
    let start = dir.path().join("under-construction.json");
    let paused = dir.path().join("paused.json");
    let commands = dir.path().join("pause.json");

    // Exercise the delivered parcel with shorter periods in this test.
    let mut city: Value =
        serde_json::from_slice(&fs::read(scenario("bridge-city.json")).unwrap()).unwrap();
    city["development_period_ticks"] = json!(240);
    fs::write(&fast_city, serde_json::to_vec(&city).unwrap()).unwrap();
    run(runner()
        .arg("--scenario")
        .arg(&fast_city)
        .args(["--until-tick", "240", "--save"])
        .arg(&start));

    let building = BuildingId::new(7);
    let developing = load(&start);
    let at_approval = growth(&developing);
    assert_eq!(at_approval.totals.residents, 280);
    assert_eq!(at_approval.sites[0].tiers, 0);
    assert!(at_approval.sites[0].accessibility > 0);
    assert_eq!(at_approval.projects.len(), 1);
    assert_eq!(at_approval.projects[0].building, building);
    assert_eq!(at_approval.projects[0].remaining_periods, 3);
    assert!(developing.inspect(Query::Building(building)).is_err());

    fs::write(
        &commands,
        r#"[{"tick":240,"command":{"set_growth_paused":true}},{"tick":720,"command":{"set_growth_paused":false}}]"#,
    )
    .unwrap();
    let clock = run(runner()
        .arg("--load")
        .arg(&start)
        .args(["--until-tick", "720", "--commands"])
        .arg(&commands)
        .arg("--save")
        .arg(&paused));
    assert_eq!(clock["clock"]["development_ticks"], 240);
    let mut resumed = load(&paused);
    assert_eq!(growth(&resumed), at_approval);
    let Observation::Service(service) = resumed.inspect(Query::Service).unwrap() else {
        panic!("expected a service observation");
    };
    assert!(service.lines[0].last_departure.unwrap().seconds() > 240);

    resumed.advance(719).unwrap();
    let nearly_done = growth(&resumed);
    assert_eq!(nearly_done.projects[0].remaining_periods, 1);
    assert_eq!(nearly_done.totals.residents, 280);
    assert!(resumed.inspect(Query::Building(building)).is_err());

    resumed.advance(1).unwrap();
    let settled = growth(&resumed);
    assert_eq!(settled.sites[0].tiers, 1);
    assert_eq!(settled.totals.residents, 280 + 24);
    assert_eq!(
        resumed.inspect(Query::Building(building)).unwrap(),
        Observation::Building(building)
    );
    let Observation::Layout(layout) = resumed.inspect(Query::Layout).unwrap() else {
        panic!("expected a layout observation");
    };
    let home = layout
        .buildings
        .iter()
        .find(|site| site.id == building)
        .unwrap();
    assert_eq!(home.cell, Cell { x: 0, y: -2 });
    assert_eq!(home.access, Some(LaneId::new(8)));
    assert_vehicle_invariants(&resumed);
    demand(&resumed);
}
