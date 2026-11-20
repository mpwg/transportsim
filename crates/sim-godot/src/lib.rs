use godot::prelude::*;
use serde_json::{Value, json};
use sim_core::{Command, LaneId, Observation, Query, QueueEntry, RuleSet, Scenario, Simulation};
use sim_core::{VehicleKind, ZoneId};
use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroU64},
};
mod line;

struct Extension;

#[gdextension]
unsafe impl ExtensionLibrary for Extension {}

#[derive(GodotClass)]
#[class(base=RefCounted)]
struct SimBridge {
    sim: Option<Simulation>,
    next_id: u64,
    lines: BTreeMap<u64, line::BuiltLine>,
}

fn dictionary(value: Value) -> VarDictionary {
    variant(&value).try_to().unwrap_or_default()
}

fn variant(value: &Value) -> Variant {
    match value {
        Value::Null => Variant::nil(),
        Value::Bool(value) => value.to_variant(),
        Value::Number(value) => value.as_i64().map_or_else(
            || value.as_f64().map_or_else(Variant::nil, |n| n.to_variant()),
            |n| n.to_variant(),
        ),
        Value::String(value) => GString::from(value).to_variant(),
        Value::Array(values) => values
            .iter()
            .map(variant)
            .collect::<VarArray>()
            .to_variant(),
        Value::Object(values) => {
            let mut result = VarDictionary::new();
            for (key, value) in values {
                result.set(&GString::from(key).to_variant(), &variant(value));
            }
            result.to_variant()
        }
    }
}

#[godot_api]
impl IRefCounted for SimBridge {
    fn init(_base: Base<RefCounted>) -> Self {
        let result = start();
        match result {
            Ok(sim) => Self {
                sim: Some(sim),
                next_id: 1_000,
                lines: BTreeMap::new(),
            },
            Err(error) => {
                godot_error!("Die Brückenstadt konnte nicht geladen werden: {}", error);
                Self {
                    sim: None,
                    next_id: 1_000,
                    lines: BTreeMap::new(),
                }
            }
        }
    }
}

fn start() -> Result<Simulation, Box<dyn std::error::Error>> {
    let mut data: Value =
        serde_json::from_str(include_str!("../../../scenarios/bridge-city.json"))?;
    // The introduction leaves the cross-river bus connection to the player.
    for lane in data["lanes"].as_array_mut().into_iter().flatten() {
        if matches!(lane["id"].as_u64(), Some(4 | 5)) {
            lane["allowed"].as_array_mut().unwrap().push(json!("bus"));
        }
    }
    data["traffic"]
        .as_array_mut()
        .unwrap()
        .retain(|seed| seed["entry"]["vehicle"]["kind"] != "bus");
    for field in ["lines", "fleet"] {
        data["service"][field]
            .as_array_mut()
            .unwrap()
            .retain(|entry| {
                if field == "lines" {
                    entry["kind"] == "tram"
                } else {
                    matches!(entry["line"].as_u64(), Some(2 | 3))
                }
            });
    }
    let mut input = Scenario::empty(serde_json::from_value(data["start"].clone())?);
    input.building_ids = serde_json::from_value(data["building_ids"].clone())?;
    input.lanes = serde_json::from_value(data["lanes"].clone())?;
    input.movements = serde_json::from_value(data["movements"].clone())?;
    input.junctions = serde_json::from_value(data["junctions"].clone())?;
    input.traffic = serde_json::from_value(data["traffic"].clone())?;
    input.demand = serde_json::from_value(data["demand"].clone())?;
    input.service = serde_json::from_value(data["service"].clone())?;
    input.taxi = serde_json::from_value(data["taxi"].clone())?;
    input.layout = serde_json::from_value(data["layout"].clone())?;
    input.growth = serde_json::from_value(data["growth"].clone())?;
    let period: NonZeroU64 = serde_json::from_value(data["development_period_ticks"].clone())?;
    let seed = serde_json::from_value(data["seed"].clone())?;
    Ok(Simulation::new_game(input, RuleSet::new(period), seed)?)
}

#[godot_api]
impl SimBridge {
    #[func]
    fn world_data(&self) -> VarDictionary {
        let Some(sim) = self.sim.as_ref() else {
            return VarDictionary::new();
        };
        let Ok(Observation::Layout(layout)) = sim.inspect(Query::Layout) else {
            return VarDictionary::new();
        };
        dictionary(json!({
            "layout": layout, "revision": sim.network_revision(),
            "lanes": sim.lane_specs(),
            "stops": sim.stop_specs(),
            "zones": sim.zone_specs(),
        }))
    }

    #[func]
    fn step(&mut self, ticks: i64) {
        if let Some(sim) = self.sim.as_mut()
            && let Err(error) = sim.advance(ticks.clamp(0, 240) as u64)
        {
            godot_error!("{}", error);
        }
    }

    #[func]
    fn next_morning(&mut self) {
        let Some(sim) = self.sim.as_mut() else { return };
        let Ok(Observation::Clock(clock)) = sim.inspect(Query::Clock) else {
            return;
        };
        if clock.paused && sim.apply(Command::SetPaused(false)).is_err() {
            return;
        }
        let remaining = 86_400 - clock.traffic.seconds() % 86_400;
        if let Err(error) = sim.advance(remaining) {
            godot_error!("{}", error);
        }
        if clock.paused
            && let Err(error) = sim.apply(Command::SetPaused(true))
        {
            godot_error!("{}", error);
        }
    }

    #[func]
    fn pause(&mut self, paused: bool) {
        if let Some(sim) = self.sim.as_mut()
            && let Err(error) = sim.apply(Command::SetPaused(paused))
        {
            godot_error!("{}", error);
        }
    }

    #[func]
    fn line_draft(
        &mut self,
        kind: GString,
        stops: VarArray,
        legs: VarArray,
        minutes: i64,
        commit: bool,
    ) -> VarDictionary {
        self.line_edit(-1, kind, stops, legs, minutes, commit)
    }

    #[func]
    fn line_edit(
        &mut self,
        line: i64,
        kind: GString,
        stops: VarArray,
        legs: VarArray,
        minutes: i64,
        commit: bool,
    ) -> VarDictionary {
        let response = (|| {
            let kind = match kind.to_string().as_str() {
                "bus" => VehicleKind::Bus,
                "tram" => VehicleKind::Tram,
                _ => return Err("Wähle Bus oder Straßenbahn.".into()),
            };
            let headway = u32::try_from(minutes)
                .ok()
                .filter(|minutes| (1..=60).contains(minutes))
                .and_then(|minutes| NonZeroU32::new(minutes * 60))
                .ok_or("Wähle einen Takt zwischen einer und 60 Minuten.")?;
            let stops = stops
                .iter_shared()
                .map(|stop| {
                    let stop = stop.try_to::<VarDictionary>().ok()?;
                    Some(line::DraftStop {
                        lane: LaneId::new(id(&stop.get(&"lane".to_variant())?)?),
                        zone: ZoneId::new(id(&stop.get(&"zone".to_variant())?)?),
                    })
                })
                .collect::<Option<Vec<_>>>()
                .ok_or("Ungültige Haltestelle.")?;
            let legs = legs
                .iter_shared()
                .map(|leg| {
                    let leg = leg.try_to::<VarArray>().ok()?;
                    leg.iter_shared()
                        .map(|lane| id(&lane).map(LaneId::new))
                        .collect::<Option<Vec<_>>>()
                })
                .collect::<Option<Vec<_>>>()
                .ok_or("Ungültiger Wegpunkt.")?;
            self.build_line(
                u64::try_from(line).ok(),
                kind,
                &stops,
                &legs,
                headway,
                commit,
            )
        })();
        dictionary(response.unwrap_or_else(|error| json!({"ok":false,"error":error})))
    }

    #[func]
    fn walking_access(&self, zone: i64, toward: bool) -> VarArray {
        let Some(sim) = self.sim.as_ref() else {
            return VarArray::new();
        };
        let Ok(zone) = u64::try_from(zone) else {
            return VarArray::new();
        };
        let zone = ZoneId::new(zone);
        let paths = if toward {
            sim.walking_access_to(zone, 300)
        } else {
            sim.walking_access_from(zone, 300)
        };
        paths
            .into_iter()
            .map(|(zone, edges)| {
                let legs: Vec<_> = edges
                    .into_iter()
                    .filter_map(|edge| {
                        if let Ok(Observation::FootEdge(spec)) = sim.inspect(Query::FootEdge(edge))
                        {
                            Some(json!([spec.from, spec.to, edge]))
                        } else {
                            None
                        }
                    })
                    .collect();
                variant(&json!({"zone": zone, "legs": legs}))
            })
            .collect()
    }

    #[func]
    fn private_flows(&self, lane: i64) -> VarArray {
        let Some(sim) = self.sim.as_ref() else {
            return VarArray::new();
        };
        let Ok(id) = u64::try_from(lane) else {
            return VarArray::new();
        };
        sim.private_journeys_via(LaneId::new(id))
            .iter()
            .map(|flow| {
                variant(&json!({
                    "from": flow.from, "to": flow.to, "count": flow.count, "lanes": flow.lanes,
                }))
            })
            .collect()
    }

    #[func]
    fn snapshot(&self) -> VarDictionary {
        let Some(sim) = self.sim.as_ref() else {
            return VarDictionary::new();
        };
        let Ok(Observation::Clock(clock)) = sim.inspect(Query::Clock) else {
            return VarDictionary::new();
        };
        let mut lanes = Vec::new();
        let mut signals = Vec::new();
        {
            for spec in sim.lane_specs() {
                let id = spec.id.get();
                let Ok(Observation::Lane(lane)) = sim.inspect(Query::Lane(spec.id)) else {
                    continue;
                };
                if let Some(green) = sim.lane_signal(LaneId::new(id)) {
                    signals.push(json!({"lane":id,"green":green}));
                }
                let cars: u64 = lane
                    .queue
                    .iter()
                    .filter_map(|entry| match entry {
                        QueueEntry::PrivateCars(count) => Some(u64::from(count.get())),
                        _ => None,
                    })
                    .sum();
                let vehicles: Vec<_> = lane.vehicles.iter().filter_map(|vehicle| {
                    let kind = lane.queue.iter().find_map(|entry| match entry {
                        QueueEntry::Vehicle { id, kind, .. } if *id == vehicle.id => Some(kind),
                        _ => None,
                    })?;
                    Some(json!({
                        "id": vehicle.id.get(), "kind": kind,
                        "progress": 1.0 - f64::from(vehicle.remaining_travel_ticks) / f64::from(vehicle.travel_ticks),
                    }))
                }).collect();
                lanes.push(json!({
                    "id": id, "cars": cars, "ready_cars": cars.min(u64::from(lane.ready_queue_mm / 5000)),
                    "vehicles": vehicles,
                }));
            }
        }
        let stops: Vec<_> = match sim.inspect(Query::Service) {
            Ok(Observation::Service(service)) => service
                .stops
                .iter()
                .filter_map(|stop| {
                    let spec = sim.stop_specs().iter().find(|spec| spec.id == stop.id)?;
                    Some(json!({
                        "id": stop.id.get(), "lane": spec.lane, "waiting": stop.waiting_total,
                    }))
                })
                .collect(),
            _ => Vec::new(),
        };
        let walkers: Vec<_> = sim
            .pedestrians()
            .into_iter()
            .map(|edge| {
                let groups: Vec<_> = edge
                    .groups
                    .into_iter()
                    .map(|group| json!({"people":group.people,"progress":group.progress}))
                    .collect();
                json!({"edge":edge.edge,"from":edge.from,"to":edge.to,"groups":groups})
            })
            .collect();
        dictionary(
            json!({ "tick": clock.traffic.seconds(), "revision": sim.network_revision(), "lanes": lanes, "stops": stops, "signals": signals, "walkers": walkers }),
        )
    }
}

fn id(value: &Variant) -> Option<u64> {
    u64::try_from(value.try_to::<i64>().ok()?).ok()
}
