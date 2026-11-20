use std::{collections::BTreeMap, num::NonZeroU32, ops::Range};

use crate::Tick;
use crate::network::{JunctionId, LaneId, MovementId};

/// A green interval for the listed movements followed by an all-red interval.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignalPhase {
    pub green_ticks: NonZeroU32,
    pub clearance_ticks: u32,
    pub movements: Vec<MovementId>,
}

impl SignalPhase {
    fn duration(&self) -> u64 {
        u64::from(self.green_ticks.get()) + u64::from(self.clearance_ticks)
    }
}

/// A junction's ordered, repeating signal plan. An empty plan is unsignalized.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct JunctionSpec {
    pub id: JunctionId,
    pub phases: Vec<SignalPhase>,
}

#[derive(Clone, Debug)]
pub(crate) struct JunctionState {
    pub(crate) spec: JunctionSpec,
    pub(crate) last_granted: BTreeMap<u16, LaneId>,
    cycle_ticks: u64,
}

impl JunctionState {
    pub(crate) fn new(spec: JunctionSpec) -> Self {
        let cycle_ticks = spec.phases.iter().map(SignalPhase::duration).sum();
        Self {
            spec,
            last_granted: BTreeMap::new(),
            cycle_ticks,
        }
    }

    pub(crate) fn allows(&self, movement: MovementId, tick: Tick) -> bool {
        if self.cycle_ticks == 0 {
            return true;
        }

        // Tick zero is the initial boundary. The first green tick is tick one.
        let mut offset = tick.seconds().saturating_sub(1) % self.cycle_ticks;
        for phase in &self.spec.phases {
            let duration = phase.duration();
            if offset < duration {
                return offset < u64::from(phase.green_ticks.get())
                    && phase.movements.contains(&movement);
            }
            offset -= duration;
        }
        false
    }

    pub(crate) fn next_green(&self, movement: MovementId, tick: Tick) -> Option<Tick> {
        if self.cycle_ticks == 0 {
            return Some(tick);
        }
        let offset = tick.seconds().saturating_sub(1) % self.cycle_ticks;
        let mut windows = self.green_windows(movement).peekable();
        let first = windows.peek()?.start;
        let wait = windows.find(|window| offset < window.end).map_or_else(
            || self.cycle_ticks - offset + first,
            |window| window.start.saturating_sub(offset),
        );
        if wait == 0 {
            return Some(tick);
        }
        tick.seconds().max(1).checked_add(wait).map(Tick::new)
    }

    pub(crate) fn previous_green(&self, movement: MovementId, tick: Tick) -> Option<Tick> {
        if self.cycle_ticks == 0 {
            return Some(tick);
        }
        let offset = tick.seconds().saturating_sub(1) % self.cycle_ticks;
        let mut latest_in_cycle = None;
        let mut end_of_last_green = None;
        for window in self.green_windows(movement) {
            let end = window.end - 1;
            if window.start <= offset {
                latest_in_cycle = Some(end.min(offset));
            }
            end_of_last_green = Some(end);
        }
        let wait = if let Some(latest) = latest_in_cycle {
            offset - latest
        } else {
            self.cycle_ticks - end_of_last_green? + offset
        };
        if wait == 0 {
            return Some(tick);
        }
        // The initial boundary repeats tick one's phase, not the end of an earlier cycle.
        let previous = tick.seconds().checked_sub(wait)?;
        (previous != 0).then_some(Tick::new(previous))
    }

    fn green_windows(&self, movement: MovementId) -> impl Iterator<Item = Range<u64>> + '_ {
        self.spec
            .phases
            .iter()
            .scan(0, |offset, phase| {
                let green = *offset..*offset + u64::from(phase.green_ticks.get());
                *offset += phase.duration();
                Some((phase, green))
            })
            .filter_map(move |(phase, green)| phase.movements.contains(&movement).then_some(green))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_repeats_the_green_and_clearance_sequence() {
        let straight = MovementId::new(1);
        let left = MovementId::new(2);
        let unlisted = MovementId::new(3);
        let junction = JunctionState::new(JunctionSpec {
            id: JunctionId::new(1),
            phases: vec![
                SignalPhase {
                    green_ticks: NonZeroU32::new(2).unwrap(),
                    clearance_ticks: 1,
                    movements: vec![straight],
                },
                SignalPhase {
                    green_ticks: NonZeroU32::MIN,
                    clearance_ticks: 2,
                    movements: vec![left],
                },
            ],
        });

        for (seconds, straight_green, left_green) in [
            (0, true, false),
            (1, true, false),
            (2, true, false),
            (3, false, false),
            (4, false, true),
            (5, false, false),
            (6, false, false),
            (7, true, false),
        ] {
            let tick = Tick::new(seconds);
            assert_eq!(
                junction.allows(straight, tick),
                straight_green,
                "tick {tick:?}"
            );
            assert_eq!(junction.allows(left, tick), left_green, "tick {tick:?}");
            assert!(!junction.allows(unlisted, tick), "tick {tick:?}");
        }

        for movement in [straight, left, unlisted] {
            for seconds in 0..24 {
                let tick = Tick::new(seconds);
                let next = (seconds..=seconds + 6)
                    .map(Tick::new)
                    .find(|&candidate| junction.allows(movement, candidate));
                let previous = (0..=seconds)
                    .rev()
                    .map(Tick::new)
                    .find(|&candidate| junction.allows(movement, candidate));
                assert_eq!(junction.next_green(movement, tick), next);
                assert_eq!(junction.previous_green(movement, tick), previous);
            }
        }
    }

    #[test]
    fn junction_without_phases_allows_movement_at_any_tick() {
        let junction = JunctionState::new(JunctionSpec {
            id: JunctionId::new(1),
            phases: Vec::new(),
        });

        for seconds in [0, 1, u64::MAX] {
            let tick = Tick::new(seconds);
            assert!(junction.allows(MovementId::new(1), tick));
            assert_eq!(junction.next_green(MovementId::new(1), tick), Some(tick));
            assert_eq!(
                junction.previous_green(MovementId::new(1), tick),
                Some(tick)
            );
        }
    }
}
