use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    num::NonZeroU32,
};

use serde::{Deserialize, Serialize};

use crate::{FootEdgeId, LineId, StopId, Tick, ZoneId};

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimetableLink {
    pub(crate) line: LineId,
    pub(crate) board: StopId,
    pub(crate) alight: StopId,
    pub(crate) from: ZoneId,
    pub(crate) to: ZoneId,
    pub(crate) first_board: Tick,
    pub(crate) headway: NonZeroU32,
    pub(crate) ride_ticks: u64,
}

impl TimetableLink {
    fn stops(&self) -> (LineId, StopId, StopId) {
        (self.line, self.board, self.alight)
    }

    fn wait_at(&self, ready: u64) -> u64 {
        let first = self.first_board.seconds();
        if ready < first {
            return first - ready;
        }
        let headway = u64::from(self.headway.get());
        (headway - (ready - first) % headway) % headway
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransitPath {
    pub(crate) access: Vec<FootEdgeId>,
    pub(crate) rides: Vec<TransitRide>,
    pub(crate) egress: Vec<FootEdgeId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransitRide {
    pub(crate) line: LineId,
    pub(crate) board: StopId,
    pub(crate) alight: StopId,
    pub(crate) transfer_after: Vec<FootEdgeId>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TransitCost {
    pub access_ticks: u64,
    pub wait_ticks: u64,
    pub ride_ticks: u64,
    pub transfer_ticks: u64,
}

struct Arrival<'a> {
    at: u64,
    rides: usize,
    link: &'a TimetableLink,
    previous: Option<ZoneId>,
    walk_to_board: Vec<FootEdgeId>,
    cost: TransitCost,
}

struct Destination {
    at: u64,
    after_ride: ZoneId,
    walk_ticks: u64,
    walk: Vec<FootEdgeId>,
}

pub(crate) fn itinerary(
    origin: ZoneId,
    destination: ZoneId,
    tick: Tick,
    links: &[TimetableLink],
    walk: impl Fn(ZoneId, ZoneId) -> Option<(u64, Vec<FootEdgeId>)>,
) -> Option<(TransitPath, TransitCost)> {
    let mut boarding_zones = BTreeMap::<_, Vec<_>>::new();
    for link in links {
        boarding_zones.entry(link.from).or_default().push(link);
    }
    for links in boarding_zones.values_mut() {
        links.sort_unstable_by_key(|link| {
            (
                link.stops(),
                link.first_board,
                link.headway,
                link.ride_ticks,
            )
        });
    }

    // Under a periodic timetable, arriving earlier always allows the same onward
    // journeys. The origin gets its own state so an itinerary must contain a ride.
    let mut arrivals = BTreeMap::<_, Arrival<'_>>::new();
    let mut frontier = BinaryHeap::from([Reverse((tick.seconds(), 0, None))]);
    let mut best = None::<Destination>;
    while let Some(Reverse((at, rides, previous))) = frontier.pop() {
        if best.as_ref().is_some_and(|end| at > end.at) {
            break;
        }
        let cost = if let Some(zone) = previous {
            let arrival = &arrivals[&zone];
            if (arrival.at, arrival.rides) != (at, rides) {
                continue;
            }
            if let Some((walk_ticks, walk)) = walk(zone, destination)
                && let Some(end_at) = at.checked_add(walk_ticks)
                && best.as_ref().is_none_or(|end| {
                    let incumbent = &arrivals[&end.after_ride];
                    (end_at, rides, arrival.link.stops(), zone)
                        < (
                            end.at,
                            incumbent.rides,
                            incumbent.link.stops(),
                            end.after_ride,
                        )
                })
            {
                best = Some(Destination {
                    at: end_at,
                    after_ride: zone,
                    walk_ticks,
                    walk,
                });
            }
            arrival.cost
        } else {
            TransitCost::default()
        };

        for (&board_zone, links) in &boarding_zones {
            let Some((walk_ticks, walk_to_board)) = walk(previous.unwrap_or(origin), board_zone)
            else {
                continue;
            };
            let Some(ready) = at.checked_add(walk_ticks) else {
                continue;
            };
            for &link in links {
                let wait_ticks = link.wait_at(ready);
                let Some(alight_at) = ready
                    .checked_add(wait_ticks)
                    .and_then(|board_at| board_at.checked_add(link.ride_ticks))
                else {
                    continue;
                };
                let next_rides = rides + 1;
                let old = arrivals.get(&link.to);
                if old.is_some_and(|old| {
                    (old.at, old.rides, old.link.stops()) <= (alight_at, next_rides, link.stops())
                }) {
                    continue;
                }
                if old.is_none_or(|old| (old.at, old.rides) != (alight_at, next_rides)) {
                    frontier.push(Reverse((alight_at, next_rides, Some(link.to))));
                }
                let mut cost = cost;
                cost.wait_ticks += wait_ticks;
                cost.ride_ticks += link.ride_ticks;
                if previous.is_some() {
                    cost.transfer_ticks += walk_ticks;
                } else {
                    cost.access_ticks += walk_ticks;
                }
                arrivals.insert(
                    link.to,
                    Arrival {
                        at: alight_at,
                        rides: next_rides,
                        link,
                        previous,
                        walk_to_board: walk_to_board.clone(),
                        cost,
                    },
                );
            }
        }
    }

    let best = best?;
    let mut last = best.after_ride;
    let mut cost = arrivals[&last].cost;
    cost.access_ticks += best.walk_ticks;
    let mut rides = Vec::new();
    let mut transfer_after = Vec::new();
    let access = loop {
        let arrival = arrivals.remove(&last)?;
        let link = arrival.link;
        rides.push(TransitRide {
            line: link.line,
            board: link.board,
            alight: link.alight,
            transfer_after,
        });
        let Some(previous) = arrival.previous else {
            break arrival.walk_to_board;
        };
        last = previous;
        transfer_after = arrival.walk_to_board;
    };
    rides.reverse();
    Some((
        TransitPath {
            access,
            rides,
            egress: best.walk,
        },
        cost,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(
        line: u64,
        (from, to): (u64, u64),
        first_board: u64,
        headway: u32,
        ride_ticks: u64,
    ) -> TimetableLink {
        TimetableLink {
            line: LineId::new(line),
            board: StopId::new(10 * from),
            alight: StopId::new(10 * to),
            from: ZoneId::new(from),
            to: ZoneId::new(to),
            first_board: Tick::new(first_board),
            headway: NonZeroU32::new(headway).unwrap(),
            ride_ticks,
        }
    }

    fn walking(from: ZoneId, to: ZoneId) -> Option<(u64, Vec<FootEdgeId>)> {
        match (from.get(), to.get()) {
            (1, 2) => Some((3, vec![FootEdgeId::new(12)])),
            (3, 4) => Some((2, vec![FootEdgeId::new(34)])),
            (5, 6) => Some((4, vec![FootEdgeId::new(56)])),
            (from, to) if from == to => Some((0, vec![])),
            _ => None,
        }
    }

    #[test]
    fn boards_at_the_exact_departure_tick_and_counts_both_access_walks() {
        let links = [link(1, (2, 5), 4, 10, 5)];
        for (tick, wait_ticks) in [(0, 1), (1, 0), (2, 9), (11, 0)] {
            let (path, cost) = itinerary(
                ZoneId::new(1),
                ZoneId::new(6),
                Tick::new(tick),
                &links,
                walking,
            )
            .unwrap();

            assert_eq!(
                path,
                TransitPath {
                    access: vec![FootEdgeId::new(12)],
                    rides: vec![TransitRide {
                        line: LineId::new(1),
                        board: StopId::new(20),
                        alight: StopId::new(50),
                        transfer_after: vec![],
                    }],
                    egress: vec![FootEdgeId::new(56)],
                }
            );
            assert_eq!(
                cost,
                TransitCost {
                    access_ticks: 7,
                    wait_ticks,
                    ride_ticks: 5,
                    transfer_ticks: 0,
                }
            );
        }
    }

    #[test]
    fn transfer_walk_can_miss_a_departure_and_change_the_best_itinerary() {
        let links = [
            link(1, (2, 3), 4, 10, 4),
            link(2, (4, 5), 9, 10, 3),
            link(3, (2, 5), 4, 10, 17),
        ];

        let (path, cost) = itinerary(
            ZoneId::new(1),
            ZoneId::new(6),
            Tick::new(1),
            &links[..2],
            walking,
        )
        .unwrap();
        assert_eq!(path.rides.len(), 2);
        assert_eq!(path.rides[0].line, LineId::new(1));
        assert_eq!(path.rides[0].transfer_after, [FootEdgeId::new(34)]);
        assert_eq!(path.rides[1].line, LineId::new(2));
        assert_eq!(
            cost,
            TransitCost {
                access_ticks: 7,
                wait_ticks: 9,
                ride_ticks: 7,
                transfer_ticks: 2,
            }
        );

        let (path, _) = itinerary(
            ZoneId::new(1),
            ZoneId::new(6),
            Tick::new(1),
            &links,
            walking,
        )
        .unwrap();
        assert_eq!(path.rides.len(), 1);
        assert_eq!(path.rides[0].line, LineId::new(3));
    }

    #[test]
    fn equally_fast_lines_are_ordered_by_stable_id() {
        let mut links = [link(2, (2, 5), 4, 10, 5), link(1, (2, 5), 4, 10, 5)];
        for _ in 0..2 {
            let (path, _) = itinerary(
                ZoneId::new(1),
                ZoneId::new(6),
                Tick::new(1),
                &links,
                walking,
            )
            .unwrap();
            assert_eq!(path.rides[0].line, LineId::new(1));
            links.reverse();
        }
    }

    #[test]
    fn a_walk_alone_is_never_a_transit_itinerary() {
        assert_eq!(
            itinerary(ZoneId::new(1), ZoneId::new(2), Tick::new(0), &[], walking),
            None
        );
    }

    #[test]
    fn overflowing_arrivals_do_not_wrap_to_the_start_of_the_clock() {
        let links = [link(1, (2, 5), 4, 10, 5)];
        assert_eq!(
            itinerary(
                ZoneId::new(1),
                ZoneId::new(6),
                Tick::new(u64::MAX - 5),
                &links,
                walking
            ),
            None
        );
    }
}
