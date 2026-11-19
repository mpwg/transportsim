use sim_core::{
    AdvanceError, BuildingId, ClockObservation, Command, Query, RuleSet, Scenario, Simulation,
    Tick, ValidationError,
};

fn clock(sim: &Simulation) -> ClockObservation {
    let sim_core::Observation::Clock(clock) = sim.inspect(Query::Clock).unwrap() else {
        panic!("clock query must return a clock");
    };
    clock
}

#[test]
fn pause_and_single_step_preserve_boundary_order() {
    let mut sim =
        Simulation::new_game(Scenario::empty(Tick::new(10)), RuleSet::default(), 7).unwrap();

    let paused = sim.apply(Command::SetPaused(true)).unwrap();
    assert_eq!(paused.sequence, 1);
    assert_eq!(paused.tick, Tick::new(10));
    assert_eq!(sim.advance(7).unwrap().executed_ticks, 0);
    assert_eq!(clock(&sim).traffic, Tick::new(10));

    assert_eq!(sim.step().unwrap().executed_ticks, 1);
    assert_eq!(clock(&sim).traffic, Tick::new(11));
    assert!(clock(&sim).paused);

    let resumed = sim.apply(Command::SetPaused(false)).unwrap();
    assert_eq!(resumed.sequence, 2);
    assert_eq!(resumed.tick, Tick::new(11));
    assert_eq!(sim.advance(3).unwrap().end, Tick::new(14));
}

#[test]
fn growth_pause_freezes_only_development_time() {
    let mut sim =
        Simulation::new_game(Scenario::empty(Tick::new(10)), RuleSet::default(), 7).unwrap();
    sim.advance(2).unwrap();
    sim.apply(Command::SetGrowthPaused(true)).unwrap();
    sim.advance(5).unwrap();
    assert_eq!(clock(&sim).traffic, Tick::new(17));
    assert_eq!(clock(&sim).development_ticks, 2);

    sim.apply(Command::SetGrowthPaused(false)).unwrap();
    sim.advance(1).unwrap();
    assert_eq!(clock(&sim).development_ticks, 3);
}

#[test]
fn duplicate_building_id_identifies_the_scenario_field() {
    let id = BuildingId::new(5);
    let mut scenario = Scenario::empty(Tick::new(0));
    scenario.building_ids = vec![id, id];
    let error = Simulation::new_game(scenario, RuleSet::default(), 7).unwrap_err();
    assert_eq!(error, ValidationError::DuplicateBuildingId { id, index: 1 });
}

#[test]
fn rejected_advance_keeps_the_last_completed_boundary() {
    let start = Tick::new(u64::MAX - 1);
    let mut sim = Simulation::new_game(Scenario::empty(start), RuleSet::default(), 7).unwrap();
    assert_eq!(sim.advance(2), Err(AdvanceError::TrafficClockOverflow));
    assert_eq!(clock(&sim).traffic, start);
    assert_eq!(clock(&sim).development_ticks, 0);
    assert_eq!(sim.advance(1).unwrap().end, Tick::new(u64::MAX));
}

#[test]
fn single_step_requires_a_paused_game() {
    let mut sim =
        Simulation::new_game(Scenario::empty(Tick::new(10)), RuleSet::default(), 7).unwrap();
    assert_eq!(sim.step(), Err(AdvanceError::NotPaused));
    assert_eq!(clock(&sim).traffic, Tick::new(10));
}

#[test]
fn development_period_uses_elapsed_unpaused_ticks() {
    let rules = RuleSet::new(std::num::NonZeroU64::new(3).unwrap());
    let mut sim = Simulation::new_game(Scenario::empty(Tick::new(90)), rules, 7).unwrap();
    sim.advance(2).unwrap();
    assert_eq!(clock(&sim).development_period, 0);

    sim.apply(Command::SetGrowthPaused(true)).unwrap();
    sim.advance(6).unwrap();
    assert_eq!(clock(&sim).development_period, 0);

    sim.apply(Command::SetGrowthPaused(false)).unwrap();
    sim.advance(1).unwrap();
    assert_eq!(clock(&sim).development_period, 1);
}

#[test]
fn scenario_building_is_observable_by_stable_id() {
    let id = BuildingId::new(5);
    let mut scenario = Scenario::empty(Tick::new(0));
    scenario.building_ids.push(id);
    let mut sim = Simulation::new_game(scenario, RuleSet::default(), 7).unwrap();
    sim.advance(1).unwrap();
    assert_eq!(
        sim.inspect(Query::Building(id)),
        Ok(sim_core::Observation::Building(id))
    );
    let absent = BuildingId::new(6);
    assert_eq!(
        sim.inspect(Query::Building(absent)),
        Err(sim_core::QueryError::UnknownBuilding(absent))
    );
}
