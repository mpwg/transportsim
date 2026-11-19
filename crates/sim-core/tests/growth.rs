use std::num::{NonZeroU32, NonZeroU64};

use sim_core::{
    BuildingId, Cell, Command, DemandProfile, DischargeRate, EditAction, EditCommand, FleetSpec,
    GrowthObservation, GrowthRules, GrowthSite, GrowthUse, JunctionId, JunctionSpec, LaneId,
    LaneSpec, LineId, LineSpec, MovementId, MovementSpec, Observation, Query, RuleSet, Scenario,
    Simulation, StopId, StopSpec, Terrain, Tick, TileSpec, TripPurpose, VehicleId, VehicleKind,
    ZoneId, ZoneSpec,
};

fn nz(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn site() -> GrowthSite {
    GrowthSite {
        building: BuildingId::new(101),
        zone: ZoneId::new(1),
        cell: Cell { x: 0, y: 0 },
        access: Some(LaneId::new(1)),
        usage: GrowthUse::Housing,
        amount: nz(2),
        max_tiers: nz(1),
        initial_tiers: 0,
        local_road: None,
    }
}

fn scenario() -> Scenario {
    let mut spec = Scenario::empty(Tick::new(0));
    spec.demand.walk_weight = 0;
    spec.demand.car_weight = 0;
    spec.demand.zones = vec![
        ZoneSpec {
            id: ZoneId::new(1),
            residents: 0,
            jobs: 0,
            shops: 0,
            road_access: Some(LaneId::new(1)),
        },
        ZoneSpec {
            id: ZoneId::new(2),
            residents: 0,
            jobs: 10,
            shops: 0,
            road_access: Some(LaneId::new(2)),
        },
    ];
    spec.demand.profiles.push(DemandProfile {
        purpose: TripPurpose::Work,
        departure_offsets: vec![0],
        return_offset: 1000,
        rate_numerator: 1,
        rate_denominator: nz(1),
    });
    spec.layout.tiles.push(TileSpec {
        cell: site().cell,
        terrain: Terrain::Land,
    });
    spec.lanes = (1..=2)
        .map(|id| LaneSpec {
            id: LaneId::new(id),
            capacity_mm: nz(30_000),
            travel_ticks: nz(1),
            discharge: DischargeRate {
                millimetres: nz(30_000),
                ticks: nz(1),
            },
            allowed: vec![VehicleKind::Bus],
        })
        .collect();
    let junction = JunctionId::new(1);
    spec.movements = [(1, 2), (2, 1)]
        .into_iter()
        .map(|(from, to)| MovementSpec {
            id: MovementId::new(from),
            from: LaneId::new(from),
            to: LaneId::new(to),
            junction,
            priority: 0,
            conflict_zones: vec![1],
        })
        .collect();
    spec.junctions.push(JunctionSpec {
        id: junction,
        phases: vec![],
    });
    spec.growth.rules = GrowthRules {
        build_periods: nz(3),
        projects_per_period: nz(1),
        max_travel_ticks: nz(60),
        minimum_accessibility: nz(1),
    };
    spec.growth.sites.push(site());
    spec
}

fn add_line(simulation: &mut Simulation) {
    let mut actions: Vec<_> = (1..=2)
        .map(|id| {
            EditAction::UpsertStop(StopSpec {
                id: StopId::new(id),
                zone: ZoneId::new(id),
                lane: LaneId::new(id),
                capacity: nz(20),
                minimum_dwell: nz(1),
                passengers_per_tick: nz(10),
            })
        })
        .collect();
    actions.extend([
        EditAction::UpsertLine(LineSpec {
            id: LineId::new(1),
            kind: VehicleKind::Bus,
            stops: vec![StopId::new(1), StopId::new(2)],
            first_departure: Tick::new(1),
            headway: nz(10),
        }),
        EditAction::UpsertLineVehicle(FleetSpec {
            id: VehicleId::new(1),
            line: LineId::new(1),
            length_mm: nz(10_000),
            capacity: nz(10),
        }),
    ]);
    simulation
        .apply(Command::CommitEdit(EditCommand {
            expected_revision: 0,
            actions,
        }))
        .unwrap();
}

fn growth(sim: &Simulation) -> GrowthObservation {
    let Observation::Growth(view) = sim.inspect(Query::Growth).unwrap() else {
        panic!("growth");
    };
    view
}

fn rules() -> RuleSet {
    RuleSet::new(NonZeroU64::new(2).unwrap())
}

#[test]
fn a_line_unlocks_building_and_new_demand_after_the_next_day_boundary() {
    let mut control = Simulation::new_game(scenario(), rules(), 42).unwrap();
    let mut improved = Simulation::load(&control.save().unwrap()).unwrap();
    add_line(&mut improved);

    control.advance(4).unwrap();
    improved.advance(4).unwrap();
    assert!(growth(&control).projects.is_empty());
    assert_eq!(growth(&control).sites[0].accessibility, 0);
    let under_construction = growth(&improved);
    assert_eq!(under_construction.totals.residents, 0);
    assert!(under_construction.sites[0].accessibility > 0);
    assert_eq!(under_construction.projects[0].remaining_periods, 2);

    let mut restored = Simulation::load(&improved.save().unwrap()).unwrap();
    improved.advance(4).unwrap();
    restored.advance(4).unwrap();
    assert_eq!(
        improved.fingerprint().unwrap(),
        restored.fingerprint().unwrap()
    );
    assert_eq!(growth(&restored).totals.residents, 2);
    assert!(growth(&restored).projects.is_empty());
    assert_eq!(
        restored.inspect(Query::Building(site().building)).unwrap(),
        Observation::Building(site().building)
    );
    let Observation::Demand(before) = restored.inspect(Query::Demand).unwrap() else {
        panic!("demand");
    };
    assert_eq!(before.generated, 0);
    restored.advance(86_401 - 8).unwrap();
    let Observation::Demand(after) = restored.inspect(Query::Demand).unwrap() else {
        panic!("demand");
    };
    assert_eq!(after.generated, 2);
    assert!(
        after
            .active
            .iter()
            .any(|trip| trip.origin == ZoneId::new(1) && trip.destination == Some(ZoneId::new(2)))
    );
}

#[test]
fn growth_pause_does_not_advance_construction_or_freeze_the_bus() {
    let mut sim = Simulation::new_game(scenario(), rules(), 42).unwrap();
    add_line(&mut sim);
    sim.advance(2).unwrap();
    let before = growth(&sim);
    sim.apply(Command::SetGrowthPaused(true)).unwrap();
    sim.advance(8).unwrap();
    let Observation::Service(service) = sim.inspect(Query::Service).unwrap() else {
        panic!("service");
    };
    assert_eq!(service.lines[0].departed, 1);
    assert_eq!(before, growth(&sim));
    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.apply(Command::SetGrowthPaused(false)).unwrap();
    restored.advance(2).unwrap();
    assert_eq!(growth(&restored).projects[0].remaining_periods, 2);
    assert_eq!(growth(&restored).totals.residents, 0);
}

#[test]
fn construction_reserves_only_the_last_available_resident() {
    let mut spec = scenario();
    spec.demand.zones[1].residents = 9_999;
    let mut sim = Simulation::new_game(spec, rules(), 42).unwrap();
    add_line(&mut sim);
    sim.advance(2).unwrap();
    assert_eq!(growth(&sim).projects[0].amount, 1);
    sim.advance(6).unwrap();
    assert_eq!(growth(&sim).totals.residents, 10_000);
}

#[test]
fn local_road_is_reserved_then_built_and_the_building_can_be_densified() {
    use sim_core::{LocalRoad, StreetSite};
    let mut spec = scenario();
    spec.demand.car_weight = 1;
    for lane in &mut spec.lanes {
        lane.allowed.push(VehicleKind::Car);
    }
    spec.growth.rules.build_periods = nz(1);
    let access = LaneId::new(10);
    let home = Cell { x: 1, y: 1 };
    let new_street = Cell { x: 1, y: 0 };
    let existing_street = Cell { x: 0, y: 0 };
    spec.layout.tiles = [home, new_street, existing_street]
        .map(|cell| TileSpec {
            cell,
            terrain: Terrain::Land,
        })
        .into();
    let street = |lane, cell| StreetSite {
        lane,
        cells: vec![cell],
        bridge: false,
        road_width_mm: 3000,
        foot_width_mm: 1000,
        greenery_width_mm: 500,
        width_mm: nz(5000),
        foot_edges: vec![],
    };
    spec.layout.streets = vec![street(LaneId::new(1), existing_street)];
    let zone = ZoneId::new(3);
    spec.demand.zones.push(ZoneSpec {
        id: zone,
        residents: 0,
        jobs: 0,
        shops: 0,
        road_access: None,
    });
    let junction = JunctionId::new(10);
    let mut access_lane = spec.lanes[0].clone();
    access_lane.id = access;
    let road = LocalRoad {
        lane: access_lane,
        street: street(access, new_street),
        movements: [(10, 1, 10), (11, 10, 1)]
            .into_iter()
            .map(|(id, from, to)| MovementSpec {
                id: MovementId::new(id),
                from: LaneId::new(from),
                to: LaneId::new(to),
                junction,
                conflict_zones: vec![1],
                priority: 0,
            })
            .collect(),
        junctions: vec![JunctionSpec {
            id: junction,
            phases: vec![],
        }],
        connects_zone: ZoneId::new(1),
    };
    spec.growth.sites[0] = GrowthSite {
        cell: home,
        zone,
        access: Some(access),
        local_road: Some(road),
        max_tiers: nz(2),
        ..site()
    };

    let mut sim = Simulation::new_game(spec, rules(), 42).unwrap();
    sim.advance(2).unwrap();
    assert_eq!(growth(&sim).projects[0].remaining_periods, 1);
    assert!(sim.inspect(Query::Lane(access)).is_err());
    let later_edit = EditCommand {
        expected_revision: 0,
        actions: vec![EditAction::UpsertStreet(street(LaneId::new(2), new_street))],
    };
    assert!(sim.preview(&later_edit).is_err());
    sim.advance(2).unwrap();
    assert!(sim.inspect(Query::Lane(access)).is_ok());
    assert_eq!(sim.network_revision(), 1);
    assert_eq!(growth(&sim).totals.residents, 2);
    let mut restored = Simulation::load(&sim.save().unwrap()).unwrap();
    restored.advance(2).unwrap();
    assert_eq!(growth(&restored).totals.residents, 4);
    assert_eq!(growth(&restored).sites[0].tiers, 2);
    let Observation::Layout(layout) = restored.inspect(Query::Layout).unwrap() else {
        panic!("layout");
    };
    assert_eq!(layout.buildings.len(), 1);
    assert_eq!(layout.buildings[0].access, Some(access));
    assert_eq!(
        layout
            .streets
            .iter()
            .filter(|street| street.lane == access)
            .count(),
        1
    );
}
