use std::{collections::BTreeMap, num::NonZeroU32};

use crate::Tick;
use crate::network::{JunctionId, LaneId, MovementId};

/// A green interval for the listed movements followed by an all-red interval.
#[derive(Clone, Debug, Eq, PartialEq)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
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
    }

    #[test]
    fn junction_without_phases_allows_movement_at_any_tick() {
        let junction = JunctionState::new(JunctionSpec {
            id: JunctionId::new(1),
            phases: Vec::new(),
        });

        for seconds in [0, 1, u64::MAX] {
            assert!(junction.allows(MovementId::new(1), Tick::new(seconds)));
        }
    }
}
