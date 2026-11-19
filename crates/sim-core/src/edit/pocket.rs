use std::num::NonZeroU32;

use super::{EditError, TurnPocket};
use crate::Tick;
use crate::network::MovementId;
use crate::save::v1::{LaneV1, QueueItemV1};

pub(super) fn divert(
    pocket: &TurnPocket,
    tick: Tick,
    lanes: &mut Vec<LaneV1>,
) -> Result<u64, EditError> {
    let source = lanes
        .iter_mut()
        .find(|lane| lane.id == pocket.source)
        .ok_or(EditError::MissingLane(pocket.source))?;
    let mut new_lane = LaneV1 {
        id: pocket.lane.id,
        credit: 0,
        wait: None,
        departure: None,
        queue: Vec::new(),
    };

    let mut moved = 0;
    let mut remaining_reach_mm = pocket.lane.capacity_mm.get();
    for mut item in std::mem::take(&mut source.queue) {
        let units = item.entry.units();
        let unit_mm = item.entry.unit_space();
        let fitting_units = units.min(remaining_reach_mm / unit_mm);
        // The reach is measured against the queue before the edit.
        remaining_reach_mm = remaining_reach_mm.saturating_sub(units.saturating_mul(unit_mm));

        let Some(diverted) = NonZeroU32::new(fitting_units).filter(|_| {
            item.ready_at.is_some_and(|ready| ready <= tick)
                && pocket.lane.allowed.contains(&item.entry.kind())
                && item
                    .route
                    .first()
                    .is_some_and(|movement| pocket.turns.contains(movement))
        }) else {
            source.queue.push(item);
            continue;
        };

        if let Some(remaining) = NonZeroU32::new(units - diverted.get()) {
            source.queue.push(QueueItemV1 {
                entry: item.entry.first_units(remaining),
                ..item.clone()
            });
            item.entry = item.entry.first_units(diverted);
        }
        rewrite_route(&mut item.route, pocket, true);
        new_lane.queue.push(item);
        moved += u64::from(diverted.get());
    }

    for item in lanes.iter_mut().flat_map(|lane| &mut lane.queue) {
        rewrite_route(&mut item.route, pocket, false);
    }
    lanes.push(new_lane);
    Ok(moved)
}

fn rewrite_route(route: &mut Vec<MovementId>, pocket: &TurnPocket, in_pocket: bool) {
    let Some(first_turn) = route
        .iter()
        .enumerate()
        .skip(usize::from(in_pocket))
        .find_map(|(index, movement)| pocket.turns.contains(movement).then_some(index))
    else {
        return;
    };

    let mut updated = Vec::with_capacity(route.len() + 1);
    updated.extend_from_slice(&route[..first_turn]);
    for &movement in &route[first_turn..] {
        if pocket.turns.contains(&movement) {
            updated.push(pocket.feeder.id);
        }
        updated.push(movement);
    }
    *route = updated;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{DischargeRate, JunctionId, LaneId, LaneSpec, MovementId, MovementSpec};
    use crate::traffic::QueueEntry;
    use crate::{VehicleId, VehicleKind};

    const SOURCE: LaneId = LaneId::new(1);
    const POCKET: LaneId = LaneId::new(2);
    const UPSTREAM: LaneId = LaneId::new(3);
    const FEEDER: MovementId = MovementId::new(1);
    const TURN: MovementId = MovementId::new(2);
    const RETURN: MovementId = MovementId::new(3);
    const APPROACH: MovementId = MovementId::new(4);

    fn nonzero(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap()
    }

    fn pocket(capacity_mm: u32) -> TurnPocket {
        TurnPocket {
            source: SOURCE,
            lane: LaneSpec {
                id: POCKET,
                capacity_mm: nonzero(capacity_mm),
                travel_ticks: nonzero(1),
                discharge: DischargeRate {
                    millimetres: nonzero(5_000),
                    ticks: nonzero(1),
                },
                allowed: vec![VehicleKind::Car],
            },
            feeder: MovementSpec {
                id: FEEDER,
                from: SOURCE,
                to: POCKET,
                junction: JunctionId::new(1),
                conflict_zones: vec![],
                priority: 0,
            },
            turns: vec![TURN],
        }
    }

    fn lane(id: LaneId, queue: Vec<QueueItemV1>) -> LaneV1 {
        LaneV1 {
            id,
            credit: 0,
            wait: None,
            departure: None,
            queue,
        }
    }

    fn cars(count: u32, ready: u64, route: &[MovementId]) -> QueueItemV1 {
        QueueItemV1 {
            entry: QueueEntry::PrivateCars(nonzero(count)),
            ready_at: Some(Tick::new(ready)),
            route: route.to_vec(),
            cohort: Some(17),
            managed: false,
            recovery: None,
        }
    }

    fn bus(route: &[MovementId]) -> QueueItemV1 {
        QueueItemV1 {
            entry: QueueEntry::Vehicle {
                id: VehicleId::new(1),
                kind: VehicleKind::Bus,
                length_mm: nonzero(12_000),
            },
            ready_at: Some(Tick::new(4)),
            route: route.to_vec(),
            cohort: None,
            managed: true,
            recovery: None,
        }
    }

    #[test]
    fn pocket_splits_a_batch_at_the_whole_vehicle_boundary() {
        let mut lanes = vec![lane(
            SOURCE,
            vec![cars(3, 4, &[TURN, RETURN, TURN]), bus(&[])],
        )];

        assert_eq!(
            divert(&pocket(12_000), Tick::new(4), &mut lanes).unwrap(),
            2
        );

        let [source, pocket] = lanes.as_slice() else {
            panic!("expected the source lane and the pocket");
        };
        let [remaining, trailing_bus] = source.queue.as_slice() else {
            panic!("expected one car and the following bus");
        };
        assert_eq!(remaining.entry, QueueEntry::PrivateCars(nonzero(1)));
        assert_eq!(remaining.route, [FEEDER, TURN, RETURN, FEEDER, TURN]);
        assert_eq!(remaining.cohort, Some(17));
        assert_eq!(remaining.ready_at, Some(Tick::new(4)));
        assert_eq!(trailing_bus.entry, bus(&[]).entry);
        assert!(trailing_bus.route.is_empty());
        assert!(trailing_bus.managed);

        let [diverted] = pocket.queue.as_slice() else {
            panic!("expected one diverted batch");
        };
        assert_eq!(pocket.id, POCKET);
        assert_eq!(pocket.credit, 0);
        assert!(pocket.wait.is_none());
        assert!(pocket.departure.is_none());
        assert_eq!(diverted.entry, QueueEntry::PrivateCars(nonzero(2)));
        assert_eq!(diverted.route, [TURN, RETURN, FEEDER, TURN]);
        assert_eq!(diverted.ready_at, Some(Tick::new(4)));
        assert_eq!(diverted.cohort, Some(17));
    }

    #[test]
    fn pocket_keeps_earlier_occupied_space_and_skips_cars_still_travelling() {
        let mut lanes = vec![lane(
            SOURCE,
            vec![cars(1, 4, &[]), cars(1, 5, &[TURN]), cars(2, 4, &[TURN])],
        )];

        assert_eq!(
            divert(&pocket(15_000), Tick::new(4), &mut lanes).unwrap(),
            1
        );
        assert!(lanes[0].queue[0].route.is_empty());
        assert_eq!(lanes[0].queue[1].ready_at, Some(Tick::new(5)));
        assert_eq!(lanes[0].queue[1].route, [FEEDER, TURN]);
        assert_eq!(lanes[0].queue[2].entry, QueueEntry::PrivateCars(nonzero(1)));
        assert_eq!(lanes[1].queue[0].entry, QueueEntry::PrivateCars(nonzero(1)));
    }

    #[test]
    fn vehicles_on_other_lanes_receive_the_feeder_in_their_route() {
        let mut lanes = vec![
            lane(SOURCE, vec![]),
            lane(UPSTREAM, vec![cars(1, 4, &[APPROACH, TURN])]),
        ];

        assert_eq!(
            divert(&pocket(15_000), Tick::new(4), &mut lanes).unwrap(),
            0
        );
        assert_eq!(lanes[1].queue[0].route, [APPROACH, FEEDER, TURN]);
        assert!(lanes[2].queue.is_empty());
    }

    #[test]
    fn pocket_moves_the_bus_only_if_its_kind_is_allowed_and_it_fits() {
        for (capacity, bus_allowed, expected) in
            [(12_000, true, 1), (11_999, true, 0), (12_000, false, 0)]
        {
            let mut lanes = vec![lane(SOURCE, vec![bus(&[TURN])])];
            let mut pocket = pocket(capacity);
            if bus_allowed {
                pocket.lane.allowed.push(VehicleKind::Bus);
            }

            assert_eq!(divert(&pocket, Tick::new(4), &mut lanes).unwrap(), expected);
            let item = if expected == 1 {
                assert!(lanes[0].queue.is_empty());
                assert_eq!(lanes[1].queue[0].route, [TURN]);
                &lanes[1].queue[0]
            } else {
                assert!(lanes[1].queue.is_empty());
                assert_eq!(lanes[0].queue[0].route, [FEEDER, TURN]);
                &lanes[0].queue[0]
            };
            assert_eq!(item.entry, bus(&[]).entry);
            assert_eq!(item.ready_at, Some(Tick::new(4)));
            assert!(item.managed);
        }
    }
}
