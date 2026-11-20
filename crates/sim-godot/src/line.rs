use serde_json::{Value, json};
use sim_core::{
    Command, EditAction, EditCommand, FleetSpec, LaneId, LineId, LineSpec, Observation, Query,
    Simulation, StopId, StopSpec, Tick, VehicleId, VehicleKind, ZoneId,
};
use std::num::NonZeroU32;

use crate::SimBridge;

#[derive(Clone, Copy)]
pub(crate) struct DraftStop {
    pub lane: LaneId,
    pub zone: ZoneId,
}

#[derive(Clone, Copy)]
pub(crate) struct BuiltLine {
    reverse: LineId,
    first_departure: Tick,
    return_departure: Tick,
    fleet: u64,
}

impl SimBridge {
    pub(crate) fn build_line(
        &mut self,
        editing: Option<u64>,
        kind: VehicleKind,
        draft: &[DraftStop],
        legs: &[Vec<LaneId>],
        headway: NonZeroU32,
        commit: bool,
    ) -> Result<Value, String> {
        let sim = self
            .sim
            .as_mut()
            .ok_or("Die Stadt ist noch nicht geladen.")?;
        if draft.len() < 2 || draft.len() > 24 {
            return Err("Setze zwischen zwei und 24 Haltestellen.".into());
        }
        if legs.len() != draft.len() {
            return Err("Die Wegpunkte passen nicht zu den Haltestellen.".into());
        }
        let (length, capacity) = match kind {
            VehicleKind::Bus => (12_000, 52),
            VehicleKind::Tram => (24_000, 88),
            _ => return Err("Wähle Bus oder Straßenbahn.".into()),
        };
        let length = NonZeroU32::new(length).unwrap();
        let previous = editing
            .map(|id| {
                self.lines
                    .get(&id)
                    .copied()
                    .ok_or("Die ausgewählte Linie ist nicht mehr vorhanden.")
            })
            .transpose()?;
        let mut lanes: Vec<_> = draft.iter().map(|stop| stop.lane).collect();
        lanes.push(lanes[0]);
        let routes = lanes
            .windows(2)
            .zip(legs)
            .map(|(pair, via)| {
                let stops: Vec<_> = std::iter::once(pair[0])
                    .chain(via.iter().copied())
                    .chain(std::iter::once(pair[1]))
                    .collect();
                preview_leg(sim, &stops, kind, length).ok_or_else(|| {
                    "Zwischen den Haltestellen fehlt ein befahrbarer Weg.".to_owned()
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let Ok(Observation::Clock(clock)) = sim.inspect(Query::Clock) else {
            return Err("Die Stadt hat keine gültige Uhr.".into());
        };
        let mut actions = Vec::new();
        let mut stop_ids = Vec::new();
        let mut new_stops = Vec::new();
        let mut next = self.next_id;
        let stop_specs = self.scenario["service"]["stops"]
            .as_array()
            .ok_or("Die Stadt hat keine gültige Haltestellenliste.")?;
        for stop in draft {
            if let Some(spec) = stop_specs.iter().find(|spec| {
                spec["lane"].as_u64() == Some(stop.lane.get())
                    && spec["zone"].as_u64() == Some(stop.zone.get())
            }) {
                stop_ids.push(StopId::new(spec["id"].as_u64().unwrap()));
                continue;
            }
            let id = StopId::new(next);
            next += 1;
            let spec = StopSpec {
                id,
                lane: stop.lane,
                zone: stop.zone,
                capacity: NonZeroU32::new(450).unwrap(),
                minimum_dwell: NonZeroU32::new(5).unwrap(),
                passengers_per_tick: NonZeroU32::new(5).unwrap(),
            };
            stop_ids.push(id);
            new_stops.push(json!(spec));
            actions.push(EditAction::UpsertStop(spec));
        }
        let round_trip: u64 = routes
            .iter()
            .flatten()
            .map(|lane| {
                self.scenario["lanes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|spec| spec["id"].as_u64() == Some(lane.get()))
                    .and_then(|spec| spec["travel_ticks"].as_u64())
                    .unwrap_or(0)
            })
            .sum::<u64>()
            + draft.len() as u64 * 10;
        let fleet = round_trip.div_ceil(u64::from(headway.get())).clamp(1, 64);
        let now = clock.traffic.seconds();
        let (line, operation) = if let Some(previous) = previous {
            (LineId::new(editing.unwrap()), previous)
        } else {
            let new = BuiltLine {
                reverse: LineId::new(next + 1),
                first_departure: Tick::new(now.saturating_add(1)),
                return_departure: Tick::new(now.saturating_add(1 + u64::from(headway.get()) / 2)),
                fleet: 0,
            };
            let line = LineId::new(next);
            next += 2;
            (line, new)
        };
        let return_line = operation.reverse;
        actions.push(EditAction::UpsertLine(LineSpec {
            id: line,
            kind,
            stops: stop_ids.clone(),
            headway,
            first_departure: operation.first_departure,
        }));
        add_waypoints(&mut actions, line, legs);
        stop_ids.reverse();
        actions.push(EditAction::UpsertLine(LineSpec {
            id: return_line,
            kind,
            stops: stop_ids,
            headway,
            first_departure: operation.return_departure,
        }));
        let mut return_legs = vec![Vec::new(); legs.len()];
        // The outbound deadhead and the inbound final leg use the same corridor.
        if legs.len() == 2 {
            return_legs[0] = legs[1].clone();
            return_legs[1] = legs[0].clone();
        }
        add_waypoints(&mut actions, return_line, &return_legs);
        for direction in [line, return_line] {
            if previous.is_some() {
                actions.push(EditAction::SetHeadway {
                    line: direction,
                    ticks: headway,
                });
            }
            for _ in operation.fleet..fleet {
                actions.push(EditAction::UpsertLineVehicle(FleetSpec {
                    id: VehicleId::new(next),
                    line: direction,
                    length_mm: length,
                    capacity: NonZeroU32::new(capacity).unwrap(),
                }));
                next += 1;
            }
        }
        let edit = EditCommand {
            expected_revision: sim.network_revision(),
            actions,
        };
        let mut routes = sim
            .preview_line_route(&edit, line)
            .map_err(|error| error.to_string())?
            .ok_or("Der vorbereitete Linienweg fehlt.")?;
        let reverse_routes = sim
            .preview_line_route(&edit, return_line)
            .map_err(|error| error.to_string())?
            .ok_or("Der vorbereitete Rückweg fehlt.")?;
        // The final leg of each one-direction line is a depot return.
        routes.pop();
        routes.extend(reverse_routes.into_iter().take(draft.len() - 1));
        if commit {
            sim.apply(Command::CommitEdit(edit))
                .map_err(|error| error.to_string())?;
            self.next_id = next;
            self.lines.insert(
                line.get(),
                BuiltLine {
                    fleet: fleet.max(operation.fleet),
                    ..operation
                },
            );
            self.scenario["service"]["stops"]
                .as_array_mut()
                .unwrap()
                .extend(new_stops);
        }
        Ok(json!({"ok":true,"routes":routes,"line":line}))
    }
}

fn preview_leg(
    sim: &Simulation,
    stops: &[LaneId],
    kind: VehicleKind,
    length: NonZeroU32,
) -> Option<Vec<LaneId>> {
    sim.preview_vehicle_route_through(stops, kind, length)
}

fn add_waypoints(actions: &mut Vec<EditAction>, line: LineId, legs: &[Vec<LaneId>]) {
    actions.push(EditAction::SetLineWaypoints {
        line,
        legs: legs.to_vec(),
    });
}
