use crate::demand::ZoneId;
use crate::{MovementId, Tick};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::fmt;
use std::num::NonZeroU32;
use std::sync::Arc;

/// The stable identity of a directed pedestrian link.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(transparent)]
pub struct FootEdgeId(u64);

impl FootEdgeId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A pedestrian link with capacity in people and discharge in people per tick.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FootEdgeSpec {
    pub id: FootEdgeId,
    pub from: ZoneId,
    pub to: ZoneId,
    pub travel_ticks: NonZeroU32,
    pub capacity: NonZeroU32,
    pub discharge: NonZeroU32,
    /// Movement that must be allowed before pedestrians can leave this link.
    pub signal: Option<MovementId>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WalkingError {
    DuplicateEdge(FootEdgeId),
    UnknownZone(ZoneId),
    UnknownMovement(MovementId),
    InvalidSnapshot(FootEdgeId),
}

impl fmt::Display for WalkingError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateEdge(id) => write!(out, "duplicate foot edge {}", id.get()),
            Self::UnknownZone(id) => write!(out, "unknown zone {}", id.get()),
            Self::UnknownMovement(id) => write!(out, "unknown crossing movement {}", id.get()),
            Self::InvalidSnapshot(id) => write!(out, "invalid queue for foot edge {}", id.get()),
        }
    }
}

impl std::error::Error for WalkingError {}

#[derive(Debug)]
struct Walkers {
    cohort: u64,
    count: u32,
    entered_at: Tick,
    route: Arc<[FootEdgeId]>,
    next_edge: usize,
}

#[derive(Debug)]
struct FootEdge {
    spec: FootEdgeSpec,
    occupied: u32,
    queue: VecDeque<Walkers>,
}

impl FootEdge {
    fn available(&self) -> u32 {
        self.spec.capacity.get() - self.occupied
    }

    fn push(&mut self, group: Walkers) {
        self.occupied += group.count;
        self.queue.push_back(group);
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WalkingSnapshot {
    edges: Vec<FootEdgeSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FootEdgeSnapshot {
    id: FootEdgeId,
    queue: Vec<WalkersSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WalkersSnapshot {
    cohort: u64,
    count: u32,
    entered_at: Tick,
    remaining: Vec<FootEdgeId>,
}

#[derive(Debug)]
pub(crate) struct Walking {
    edges: BTreeMap<FootEdgeId, FootEdge>,
    outgoing: BTreeMap<ZoneId, Vec<FootEdgeId>>,
    incoming: BTreeMap<ZoneId, Vec<FootEdgeId>>,
}

impl Walking {
    pub(crate) fn new(
        specs: Vec<FootEdgeSpec>,
        zones: &BTreeSet<ZoneId>,
        known_signal: impl Fn(MovementId) -> bool,
    ) -> Result<Self, WalkingError> {
        let mut edges = BTreeMap::new();
        for spec in specs {
            for zone in [spec.from, spec.to] {
                if !zones.contains(&zone) {
                    return Err(WalkingError::UnknownZone(zone));
                }
            }
            if let Some(movement) = spec.signal
                && !known_signal(movement)
            {
                return Err(WalkingError::UnknownMovement(movement));
            }
            let id = spec.id;
            let edge = FootEdge {
                spec,
                occupied: 0,
                queue: VecDeque::new(),
            };
            if edges.insert(id, edge).is_some() {
                return Err(WalkingError::DuplicateEdge(id));
            }
        }

        let mut outgoing: BTreeMap<_, Vec<_>> = zones
            .iter()
            .copied()
            .map(|zone| (zone, Vec::new()))
            .collect();
        let mut incoming = outgoing.clone();
        for (&id, edge) in &edges {
            outgoing
                .get_mut(&edge.spec.from)
                .expect("validated zone")
                .push(id);
            incoming
                .get_mut(&edge.spec.to)
                .expect("validated zone")
                .push(id);
        }
        Ok(Self {
            edges,
            outgoing,
            incoming,
        })
    }

    pub(crate) fn route(&self, from: ZoneId, to: ZoneId) -> Option<(u64, Vec<FootEdgeId>)> {
        if !self.outgoing.contains_key(&from) || !self.incoming.contains_key(&to) {
            return None;
        }

        // Compute distances to the destination so a forward walk can break ties by edge ID.
        let mut distances = BTreeMap::from([(to, 0_u64)]);
        let mut frontier = BinaryHeap::from([Reverse((0_u64, to))]);
        while let Some(Reverse((cost, zone))) = frontier.pop() {
            if distances[&zone] != cost {
                continue;
            }
            if zone == from {
                break;
            }
            for id in &self.incoming[&zone] {
                let spec = &self.edges[id].spec;
                let Some(candidate) = cost.checked_add(u64::from(spec.travel_ticks.get())) else {
                    continue;
                };
                if distances.get(&spec.from).is_none_or(|&old| candidate < old) {
                    distances.insert(spec.from, candidate);
                    frontier.push(Reverse((candidate, spec.from)));
                }
            }
        }

        let mut zone = from;
        let total = *distances.get(&from)?;
        let mut path = Vec::new();
        while zone != to {
            let id = *self.outgoing[&zone].iter().find(|id| {
                let spec = &self.edges[id].spec;
                distances[&zone]
                    .checked_sub(u64::from(spec.travel_ticks.get()))
                    .zip(distances.get(&spec.to).copied())
                    .is_some_and(|(remaining, distance)| remaining == distance)
            })?;
            path.push(id);
            zone = self.edges[&id].spec.to;
        }
        Some((total, path))
    }

    fn route_end(&self, mut zone: ZoneId, route: &[FootEdgeId]) -> Option<ZoneId> {
        for id in route {
            let edge = self.edges.get(id)?;
            if edge.spec.from != zone {
                return None;
            }
            zone = edge.spec.to;
        }
        Some(zone)
    }

    pub(crate) fn supports_route(&self, from: ZoneId, to: ZoneId, route: &[FootEdgeId]) -> bool {
        self.route_end(from, route) == Some(to)
    }

    pub(crate) fn enter(
        &mut self,
        route: &[FootEdgeId],
        count: u32,
        tick: Tick,
        cohort: u64,
    ) -> u32 {
        let Some((&first, remaining)) = route.split_first() else {
            return 0;
        };
        let Some(edge) = self.edges.get(&first) else {
            return 0;
        };
        let accepted = count.min(edge.available());
        if accepted == 0 || self.route_end(edge.spec.to, remaining).is_none() {
            return 0;
        }
        self.edges
            .get_mut(&first)
            .expect("validated edge")
            .push(Walkers {
                cohort,
                count: accepted,
                entered_at: tick,
                route: Arc::from(route),
                next_edge: 1,
            });
        accepted
    }

    pub(crate) fn tick(
        &mut self,
        tick: Tick,
        allowed: impl Fn(MovementId, Tick) -> bool,
    ) -> Vec<(u64, u32)> {
        // Reserve destination space from the start of the tick in edge ID order.
        let mut available: BTreeMap<_, _> = self
            .edges
            .iter()
            .map(|(&id, edge)| (id, edge.available()))
            .collect();
        let mut transfers = Vec::new();
        let mut arrivals = Vec::new();

        for edge in self.edges.values_mut() {
            let mut discharge = edge.spec.discharge.get();
            while discharge > 0 {
                let Some(group) = edge.queue.front_mut() else {
                    break;
                };
                let elapsed = tick.seconds().saturating_sub(group.entered_at.seconds());
                if elapsed < u64::from(edge.spec.travel_ticks.get())
                    || edge
                        .spec
                        .signal
                        .is_some_and(|movement| !allowed(movement, tick))
                {
                    break;
                }

                let mut count = discharge.min(group.count);
                if let Some(&next) = group.route.get(group.next_edge) {
                    let space = available.get_mut(&next).expect("validated route");
                    count = count.min(*space);
                    if count == 0 {
                        break;
                    }
                    *space -= count;
                    transfers.push((
                        next,
                        Walkers {
                            cohort: group.cohort,
                            count,
                            entered_at: tick,
                            route: Arc::clone(&group.route),
                            next_edge: group.next_edge + 1,
                        },
                    ));
                } else {
                    arrivals.push((group.cohort, count));
                }
                group.count -= count;
                edge.occupied -= count;
                discharge -= count;
                if group.count == 0 {
                    edge.queue.pop_front();
                }
            }
        }

        for (next, group) in transfers {
            self.edges
                .get_mut(&next)
                .expect("validated route")
                .push(group);
        }
        arrivals
    }

    pub(crate) fn snapshot(&self) -> WalkingSnapshot {
        WalkingSnapshot {
            edges: self
                .edges
                .iter()
                .filter(|(_, edge)| !edge.queue.is_empty())
                .map(|(&id, edge)| FootEdgeSnapshot {
                    id,
                    queue: edge
                        .queue
                        .iter()
                        .map(|group| WalkersSnapshot {
                            cohort: group.cohort,
                            count: group.count,
                            entered_at: group.entered_at,
                            remaining: group.route[group.next_edge..].to_vec(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub(crate) fn restore(
        &mut self,
        snapshot: WalkingSnapshot,
        tick: Tick,
    ) -> Result<(), WalkingError> {
        let mut state = BTreeMap::new();
        for saved in snapshot.edges {
            let error = || WalkingError::InvalidSnapshot(saved.id);
            let edge = self.edges.get(&saved.id).ok_or_else(error)?;
            let mut queue = VecDeque::new();
            let mut occupied = 0_u32;
            let mut previous_entry = Tick::new(0);
            for group in saved.queue {
                if group.count == 0
                    || group.entered_at < previous_entry
                    || group.entered_at > tick
                    || self.route_end(edge.spec.to, &group.remaining).is_none()
                {
                    return Err(error());
                }
                occupied = occupied.checked_add(group.count).ok_or_else(error)?;
                if occupied > edge.spec.capacity.get() {
                    return Err(error());
                }
                previous_entry = group.entered_at;
                queue.push_back(Walkers {
                    cohort: group.cohort,
                    count: group.count,
                    entered_at: group.entered_at,
                    route: Arc::from(group.remaining),
                    next_edge: 0,
                });
            }
            if state.insert(saved.id, (occupied, queue)).is_some() {
                return Err(error());
            }
        }

        for (&id, edge) in &mut self.edges {
            (edge.occupied, edge.queue) = state.remove(&id).unwrap_or_default();
        }
        Ok(())
    }

    pub(crate) fn cohorts(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.edges
            .values()
            .flat_map(|edge| edge.queue.iter().map(|group| (group.cohort, group.count)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zones() -> BTreeSet<ZoneId> {
        [1, 2, 3, 4].map(ZoneId::new).into()
    }

    fn edge(
        id: u64,
        from: u64,
        to: u64,
        travel: u32,
        capacity: u32,
        discharge: u32,
    ) -> FootEdgeSpec {
        FootEdgeSpec {
            id: FootEdgeId::new(id),
            from: ZoneId::new(from),
            to: ZoneId::new(to),
            travel_ticks: NonZeroU32::new(travel).unwrap(),
            capacity: NonZeroU32::new(capacity).unwrap(),
            discharge: NonZeroU32::new(discharge).unwrap(),
            signal: None,
        }
    }

    #[test]
    fn finds_fastest_directed_route_with_edge_ids_breaking_ties() {
        let walking = Walking::new(
            vec![
                edge(10, 1, 4, 7, 1, 1),
                edge(6, 1, 2, 1, 1, 1),
                edge(8, 2, 4, 1, 1, 1),
                edge(5, 1, 3, 1, 1, 1),
                edge(9, 3, 4, 1, 1, 1),
            ],
            &zones(),
            |_| false,
        )
        .unwrap();

        assert_eq!(
            walking.route(ZoneId::new(1), ZoneId::new(4)),
            Some((2, vec![FootEdgeId::new(5), FootEdgeId::new(9)]))
        );
        assert_eq!(walking.route(ZoneId::new(4), ZoneId::new(1)), None);
        assert_eq!(
            walking.route(ZoneId::new(4), ZoneId::new(4)),
            Some((0, vec![]))
        );
        assert_eq!(walking.route(ZoneId::new(0), ZoneId::new(0)), None);
    }

    #[test]
    fn rejects_duplicate_edges_unknown_zones_and_unknown_movements() {
        let mut crossing = edge(1, 1, 2, 1, 1, 1);
        crossing.signal = Some(MovementId::new(91));

        assert!(
            Walking::new(
                vec![edge(1, 1, 2, 1, 1, 1), edge(1, 2, 3, 1, 1, 1)],
                &zones(),
                |_| false,
            )
            .is_err()
        );
        assert!(Walking::new(vec![edge(1, 1, 6, 1, 1, 1)], &zones(), |_| false).is_err());
        assert!(Walking::new(vec![crossing], &zones(), |_| false).is_err());
    }

    #[test]
    fn crossing_waits_for_green_and_room_before_releasing_fifo_groups() {
        let signal = MovementId::new(91);
        let mut crossing = edge(1, 1, 2, 1, 3, 1);
        crossing.signal = Some(signal);
        let mut walking = Walking::new(
            vec![crossing, edge(2, 2, 3, 1, 1, 1)],
            &zones(),
            |movement| movement == signal,
        )
        .unwrap();
        let route = [FootEdgeId::new(1), FootEdgeId::new(2)];
        assert_eq!(walking.enter(&route[1..], 1, Tick::new(0), 8), 1);
        assert_eq!(walking.enter(&route, 2, Tick::new(0), 9), 2);
        assert_eq!(walking.enter(&route, 2, Tick::new(0), 10), 1);
        assert_eq!(walking.enter(&route, 1, Tick::new(0), 11), 0);
        assert!(walking.tick(Tick::new(0), |_, _| true).is_empty());

        assert_eq!(walking.tick(Tick::new(1), |_, _| true), [(8, 1)]);
        assert!(walking.tick(Tick::new(2), |_, _| false).is_empty());
        assert!(
            walking
                .tick(Tick::new(3), |id, tick| id == signal && tick.seconds() == 3)
                .is_empty()
        );
        assert_eq!(walking.tick(Tick::new(4), |_, _| true), [(9, 1)]);
        assert!(walking.tick(Tick::new(5), |_, _| true).is_empty());
        assert_eq!(walking.tick(Tick::new(6), |_, _| true), [(9, 1)]);
        assert!(walking.tick(Tick::new(7), |_, _| true).is_empty());
        assert_eq!(walking.tick(Tick::new(8), |_, _| true), [(10, 1)]);
    }

    #[test]
    fn discharge_splits_groups_and_does_not_serve_later_groups_first() {
        let mut walking = Walking::new(vec![edge(1, 1, 2, 2, 6, 3)], &zones(), |_| false).unwrap();
        let route = [FootEdgeId::new(1)];

        assert_eq!(walking.enter(&route, 4, Tick::new(4), 9), 4);
        assert_eq!(walking.enter(&route, 2, Tick::new(5), 10), 2);
        assert!(walking.tick(Tick::new(5), |_, _| true).is_empty());
        assert_eq!(walking.tick(Tick::new(6), |_, _| true), [(9, 3)]);
        assert_eq!(walking.tick(Tick::new(7), |_, _| true), [(9, 1), (10, 2)]);
    }

    #[test]
    fn restore_keeps_partial_groups_and_rejects_incompatible_topology() {
        let first = || edge(1, 1, 2, 1, 3, 2);
        let second = || edge(2, 2, 3, 2, 2, 1);
        let make = || Walking::new(vec![first(), second()], &zones(), |_| false).unwrap();
        let mut uninterrupted = make();
        let route = [FootEdgeId::new(1), FootEdgeId::new(2)];
        assert_eq!(uninterrupted.enter(&route, 3, Tick::new(0), 9), 3);
        assert!(uninterrupted.tick(Tick::new(1), |_, _| true).is_empty());

        let bytes = serde_json::to_vec(&uninterrupted.snapshot()).unwrap();
        let snapshot = || serde_json::from_slice(&bytes).unwrap();
        let mut restored = make();
        restored.restore(snapshot(), Tick::new(1)).unwrap();
        assert_eq!(restored.cohorts().collect::<Vec<_>>(), [(9, 1), (9, 2)]);
        for tick in 2..=7 {
            assert_eq!(
                restored.tick(Tick::new(tick), |_, _| true),
                uninterrupted.tick(Tick::new(tick), |_, _| true)
            );
        }
        assert_eq!(restored.cohorts().count(), 0);

        let mut incompatible = Walking::new(vec![first()], &zones(), |_| false).unwrap();
        assert!(incompatible.restore(snapshot(), Tick::new(1)).is_err());
        assert_eq!(incompatible.cohorts().count(), 0);
        assert!(make().restore(snapshot(), Tick::new(0)).is_err());
    }
}
