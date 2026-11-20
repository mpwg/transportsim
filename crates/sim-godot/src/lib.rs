use godot::prelude::*;
use serde_json::{Value, json};
use sim_core::{Command, LaneId, Observation, Query, QueueEntry, RuleSet, Scenario, Simulation};
use std::num::NonZeroU64;

struct Extension;

#[gdextension]
unsafe impl ExtensionLibrary for Extension {}

#[derive(GodotClass)]
#[class(base=RefCounted)]
struct SimBridge {
    scenario: Value,
    sim: Option<Simulation>,
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
            Ok((scenario, sim)) => Self {
                scenario,
                sim: Some(sim),
            },
            Err(error) => {
                godot_error!("Die Brückenstadt konnte nicht geladen werden: {}", error);
                Self {
                    scenario: Value::Null,
                    sim: None,
                }
            }
        }
    }
}

fn start() -> Result<(Value, Simulation), Box<dyn std::error::Error>> {
    let data: Value = serde_json::from_str(include_str!("../../../scenarios/bridge-city.json"))?;
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
    Ok((
        data,
        Simulation::new_game(input, RuleSet::new(period), seed)?,
    ))
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
            "layout": layout,
            "lanes": self.scenario["lanes"],
            "stops": self.scenario["service"]["stops"],
            "zones": self.scenario["demand"]["zones"],
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
    fn pause(&mut self, paused: bool) {
        if let Some(sim) = self.sim.as_mut()
            && let Err(error) = sim.apply(Command::SetPaused(paused))
        {
            godot_error!("{}", error);
        }
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
        if let Some(specs) = self.scenario["lanes"].as_array() {
            for spec in specs {
                let Some(id) = spec["id"].as_u64() else {
                    continue;
                };
                let Ok(Observation::Lane(lane)) = sim.inspect(Query::Lane(LaneId::new(id))) else {
                    continue;
                };
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
                    let spec = self.scenario["service"]["stops"]
                        .as_array()?
                        .iter()
                        .find(|spec| spec["id"].as_u64() == Some(stop.id.get()))?;
                    Some(json!({
                        "id": stop.id.get(), "lane": spec["lane"], "waiting": stop.waiting_total,
                    }))
                })
                .collect(),
            _ => Vec::new(),
        };
        dictionary(json!({ "tick": clock.traffic.seconds(), "lanes": lanes, "stops": stops }))
    }
}
