use crate::{BuildingId, FootEdgeId, LaneId, MovementId, MovementSpec, ZoneId};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    num::NonZeroU32,
};

/// A position in the city's orthogonal grid.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Terrain {
    Land,
    Water,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TileSpec {
    pub cell: Cell,
    pub terrain: Terrain,
}

/// Location and road access of a building serving an existing demand zone.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingSite {
    pub id: BuildingId,
    pub zone: ZoneId,
    pub cell: Cell,
    pub access: Option<LaneId>,
}

/// The footprint and width allocation of one lane's street profile.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StreetSite {
    pub lane: LaneId,
    /// Cells in traversal order, with each step crossing one cell edge.
    pub cells: Vec<Cell>,
    pub bridge: bool,
    pub road_width_mm: u32,
    pub foot_width_mm: u32,
    pub greenery_width_mm: u32,
    pub width_mm: NonZeroU32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foot_edges: Vec<FootEdgeId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutSpec {
    pub tiles: Vec<TileSpec>,
    pub buildings: Vec<BuildingSite>,
    pub streets: Vec<StreetSite>,
}

#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LayoutError {
    DuplicateTile(Cell),
    DuplicateBuilding(BuildingId),
    DuplicateStreet(LaneId),
    UnknownTile(Cell),
    UnknownBuilding(BuildingId),
    UnknownLane(LaneId),
    UnknownZone(ZoneId),
    DisconnectedMovement(MovementId),
    UnknownFootEdge(FootEdgeId),
    SharedFootEdge(FootEdgeId),
    FootEdgeWithoutWidth(FootEdgeId),
    BuildingAwayFromAccess {
        building: BuildingId,
        lane: LaneId,
    },
    EmptyStreet(LaneId),
    DisconnectedStreet {
        lane: LaneId,
        from: Cell,
        to: Cell,
    },
    StreetOnWater {
        lane: LaneId,
        cell: Cell,
    },
    WidthExceeded {
        lane: LaneId,
        used_mm: u64,
        width_mm: u32,
    },
    BuildingOnWater {
        building: BuildingId,
        cell: Cell,
    },
    BuildingOnStreet {
        building: BuildingId,
        cell: Cell,
    },
    BuildingCollision {
        building: BuildingId,
        occupied_by: BuildingId,
        cell: Cell,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTile(cell) => write!(out, "duplicate tile at {cell:?}"),
            Self::DuplicateBuilding(id) => write!(out, "duplicate building {}", id.get()),
            Self::DuplicateStreet(lane) => write!(out, "duplicate street for lane {}", lane.get()),
            Self::UnknownTile(cell) => write!(out, "unknown tile at {cell:?}"),
            Self::UnknownBuilding(id) => write!(out, "unknown building {}", id.get()),
            Self::UnknownLane(id) => write!(out, "unknown lane {}", id.get()),
            Self::DisconnectedMovement(id) => {
                write!(out, "movement {} crosses a gap in the grid", id.get())
            }
            Self::UnknownFootEdge(id) => write!(out, "unknown foot edge {}", id.get()),
            Self::SharedFootEdge(id) => write!(
                out,
                "foot edge {} belongs to more than one street profile",
                id.get()
            ),
            Self::FootEdgeWithoutWidth(id) => write!(
                out,
                "foot edge {} has no space in its street profile",
                id.get()
            ),
            Self::BuildingAwayFromAccess { building, lane } => write!(
                out,
                "building {} is away from access lane {}",
                building.get(),
                lane.get()
            ),
            Self::UnknownZone(id) => write!(out, "unknown zone {}", id.get()),
            Self::EmptyStreet(lane) => write!(out, "street for lane {} has no cells", lane.get()),
            Self::DisconnectedStreet { lane, from, to } => write!(
                out,
                "street for lane {} has a gap between {from:?} and {to:?}",
                lane.get()
            ),
            Self::StreetOnWater { lane, cell } => {
                write!(
                    out,
                    "street for lane {} needs a bridge at {cell:?}",
                    lane.get()
                )
            }
            Self::WidthExceeded {
                lane,
                used_mm,
                width_mm,
            } => write!(
                out,
                "street for lane {} allocates {used_mm} mm of its {width_mm} mm width",
                lane.get()
            ),
            Self::BuildingOnWater { building, cell } => {
                write!(
                    out,
                    "building {} occupies water at {cell:?}",
                    building.get()
                )
            }
            Self::BuildingOnStreet { building, cell } => {
                write!(
                    out,
                    "building {} occupies street space at {cell:?}",
                    building.get()
                )
            }
            Self::BuildingCollision {
                building,
                occupied_by,
                cell,
            } => write!(
                out,
                "building {} overlaps building {} at {cell:?}",
                building.get(),
                occupied_by.get()
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

impl LayoutSpec {
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty() && self.buildings.is_empty() && self.streets.is_empty()
    }

    /// Canonicalize the order of table rows while preserving street traversal order.
    pub fn sort_tables(&mut self) {
        self.tiles.sort_unstable_by_key(|tile| tile.cell);
        self.buildings.sort_unstable_by_key(|building| building.id);
        self.streets.sort_unstable_by_key(|street| street.lane);
        for street in &mut self.streets {
            street.foot_edges.sort_unstable();
        }
    }

    pub(crate) fn validate_connections(
        &self,
        movements: &[MovementSpec],
        foot_exists: impl Fn(FootEdgeId) -> bool,
    ) -> Result<(), LayoutError> {
        let streets: BTreeMap<_, _> = self
            .streets
            .iter()
            .map(|street| (street.lane, street))
            .collect();
        let distance =
            |a: Cell, b: Cell| u64::from(a.x.abs_diff(b.x)) + u64::from(a.y.abs_diff(b.y));
        for movement in movements {
            if let Some(from) = streets
                .get(&movement.from)
                .and_then(|site| site.cells.last())
                && let Some(to) = streets
                    .get(&movement.to)
                    .and_then(|site| site.cells.first())
                && distance(*from, *to) > 1
            {
                return Err(LayoutError::DisconnectedMovement(movement.id));
            }
        }
        let mut foot_edges = BTreeSet::new();
        for street in &self.streets {
            for &edge in &street.foot_edges {
                if !foot_exists(edge) {
                    return Err(LayoutError::UnknownFootEdge(edge));
                }
                if !foot_edges.insert(edge) {
                    return Err(LayoutError::SharedFootEdge(edge));
                }
                if street.foot_width_mm == 0 {
                    return Err(LayoutError::FootEdgeWithoutWidth(edge));
                }
            }
        }
        for building in &self.buildings {
            if let Some(lane) = building.access
                && let Some(street) = streets.get(&lane)
                && !street
                    .cells
                    .iter()
                    .any(|&cell| distance(cell, building.cell) == 1)
            {
                return Err(LayoutError::BuildingAwayFromAccess {
                    building: building.id,
                    lane,
                });
            }
        }
        Ok(())
    }

    /// Move a building in a proposed layout. Validate the layout after all edits.
    pub fn move_building(&mut self, id: BuildingId, target: Cell) -> Result<(), LayoutError> {
        let building = self
            .buildings
            .iter_mut()
            .find(|building| building.id == id)
            .ok_or(LayoutError::UnknownBuilding(id))?;
        building.cell = target;
        Ok(())
    }

    /// Check geometry and resolve references against the scenario's lanes and zones.
    pub fn validate(
        &self,
        lane_exists: impl Fn(LaneId) -> bool,
        zone_exists: impl Fn(ZoneId) -> bool,
    ) -> Result<(), LayoutError> {
        let mut terrain = BTreeMap::new();
        for tile in &self.tiles {
            if terrain.insert(tile.cell, tile.terrain).is_some() {
                return Err(LayoutError::DuplicateTile(tile.cell));
            }
        }
        let terrain_at = |cell| {
            terrain
                .get(&cell)
                .copied()
                .ok_or(LayoutError::UnknownTile(cell))
        };

        let mut streets = BTreeSet::new();
        let mut street_cells = BTreeSet::new();
        for street in &self.streets {
            let lane = street.lane;
            if !streets.insert(lane) {
                return Err(LayoutError::DuplicateStreet(lane));
            }
            if !lane_exists(lane) {
                return Err(LayoutError::UnknownLane(lane));
            }
            if street.cells.is_empty() {
                return Err(LayoutError::EmptyStreet(lane));
            }
            let used_mm = u64::from(street.road_width_mm)
                + u64::from(street.foot_width_mm)
                + u64::from(street.greenery_width_mm);
            let width_mm = street.width_mm.get();
            if used_mm > u64::from(width_mm) {
                return Err(LayoutError::WidthExceeded {
                    lane,
                    used_mm,
                    width_mm,
                });
            }
            for pair in street.cells.windows(2) {
                let (from, to) = (pair[0], pair[1]);
                let step = (from.x.abs_diff(to.x), from.y.abs_diff(to.y));
                if !matches!(step, (0, 1) | (1, 0)) {
                    return Err(LayoutError::DisconnectedStreet { lane, from, to });
                }
            }
            for &cell in &street.cells {
                if terrain_at(cell)? == Terrain::Water && !street.bridge {
                    return Err(LayoutError::StreetOnWater { lane, cell });
                }
                street_cells.insert(cell);
            }
        }

        let mut buildings = BTreeSet::new();
        let mut building_cells = BTreeMap::new();
        for site in &self.buildings {
            let BuildingSite {
                id: building,
                zone,
                cell,
                access,
            } = *site;
            if !buildings.insert(building) {
                return Err(LayoutError::DuplicateBuilding(building));
            }
            if !zone_exists(zone) {
                return Err(LayoutError::UnknownZone(zone));
            }
            if let Some(lane) = access
                && !lane_exists(lane)
            {
                return Err(LayoutError::UnknownLane(lane));
            }
            if terrain_at(cell)? == Terrain::Water {
                return Err(LayoutError::BuildingOnWater { building, cell });
            }
            if street_cells.contains(&cell) {
                return Err(LayoutError::BuildingOnStreet { building, cell });
            }
            if let Some(occupied_by) = building_cells.insert(cell, building) {
                return Err(LayoutError::BuildingCollision {
                    building,
                    occupied_by,
                    cell,
                });
            }
        }

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const BUILDING: BuildingId = BuildingId::new(12);
    const OTHER_BUILDING: BuildingId = BuildingId::new(15);
    const LANE: LaneId = LaneId::new(1);
    const OTHER_LANE: LaneId = LaneId::new(2);
    const ZONE: ZoneId = ZoneId::new(3);
    const A: Cell = Cell { x: 0, y: 0 };
    const B: Cell = Cell { x: 1, y: 0 };
    const WATER: Cell = Cell { x: 2, y: 0 };

    fn building(id: BuildingId, cell: Cell) -> BuildingSite {
        BuildingSite {
            id,
            zone: ZONE,
            cell,
            access: Some(LANE),
        }
    }

    fn street(lane: LaneId, cells: Vec<Cell>) -> StreetSite {
        StreetSite {
            lane,
            cells,
            bridge: false,
            road_width_mm: 3_500,
            foot_width_mm: 1_500,
            greenery_width_mm: 500,
            width_mm: NonZeroU32::new(5_500).unwrap(),
            foot_edges: Vec::new(),
        }
    }

    fn layout() -> LayoutSpec {
        LayoutSpec {
            tiles: vec![
                TileSpec {
                    cell: A,
                    terrain: Terrain::Land,
                },
                TileSpec {
                    cell: B,
                    terrain: Terrain::Land,
                },
                TileSpec {
                    cell: WATER,
                    terrain: Terrain::Water,
                },
            ],
            buildings: Vec::new(),
            streets: Vec::new(),
        }
    }

    fn validate(layout: &LayoutSpec) -> Result<(), LayoutError> {
        layout.validate(
            |lane| [LANE, OTHER_LANE].contains(&lane),
            |zone| zone == ZONE,
        )
    }

    #[test]
    fn buildings_cannot_share_a_cell() {
        let mut layout = layout();
        layout.buildings = vec![building(BUILDING, A), building(OTHER_BUILDING, A)];
        assert_eq!(
            validate(&layout),
            Err(LayoutError::BuildingCollision {
                building: OTHER_BUILDING,
                occupied_by: BUILDING,
                cell: A,
            })
        );
    }

    #[test]
    fn buildings_cannot_occupy_a_street_cell() {
        let mut layout = layout();
        layout.buildings.push(building(BUILDING, A));
        layout.streets.push(street(LANE, vec![A, B]));
        assert_eq!(
            validate(&layout),
            Err(LayoutError::BuildingOnStreet {
                building: BUILDING,
                cell: A,
            })
        );
    }

    #[test]
    fn buildings_need_dry_land() {
        let mut layout = layout();
        layout.buildings.push(building(BUILDING, WATER));
        assert_eq!(
            validate(&layout),
            Err(LayoutError::BuildingOnWater {
                building: BUILDING,
                cell: WATER,
            })
        );
    }

    #[test]
    fn a_street_needs_a_bridge_to_cross_water() {
        let mut layout = layout();
        layout.streets.push(street(LANE, vec![B, WATER]));
        assert_eq!(
            validate(&layout),
            Err(LayoutError::StreetOnWater {
                lane: LANE,
                cell: WATER
            })
        );

        layout.streets[0].bridge = true;
        assert_eq!(validate(&layout), Ok(()));
    }

    #[test]
    fn the_cross_section_must_fit_its_own_width_without_overflow() {
        let mut layout = layout();
        layout.streets.push(street(LANE, vec![A, B]));
        assert_eq!(validate(&layout), Ok(()));

        layout.streets[0].greenery_width_mm = u32::MAX;
        assert_eq!(
            validate(&layout),
            Err(LayoutError::WidthExceeded {
                lane: LANE,
                used_mm: 4_294_972_295,
                width_mm: 5_500,
            })
        );
    }

    #[test]
    fn different_lanes_can_share_the_same_cells() {
        let mut layout = layout();
        layout.streets = vec![street(LANE, vec![A, B]), street(OTHER_LANE, vec![B, A])];
        assert_eq!(validate(&layout), Ok(()));
    }

    #[test]
    fn moving_a_building_preserves_its_identity_zone_and_access() {
        let mut layout = layout();
        layout.buildings.push(building(BUILDING, A));
        assert_eq!(layout.move_building(BUILDING, B), Ok(()));
        assert_eq!(layout.buildings, [building(BUILDING, B)]);

        assert_eq!(
            layout.move_building(OTHER_BUILDING, A),
            Err(LayoutError::UnknownBuilding(OTHER_BUILDING))
        );
    }

    #[test]
    fn a_street_path_must_step_between_edge_neighbors() {
        let mut layout = layout();
        let diagonal = Cell { x: 1, y: 1 };
        layout.tiles.push(TileSpec {
            cell: diagonal,
            terrain: Terrain::Land,
        });
        layout.streets.push(street(LANE, vec![A, diagonal]));
        assert_eq!(
            validate(&layout),
            Err(LayoutError::DisconnectedStreet {
                lane: LANE,
                from: A,
                to: diagonal
            })
        );

        let far = Cell {
            x: i32::MAX,
            y: i32::MAX,
        };
        let near = Cell {
            x: i32::MIN,
            y: i32::MIN,
        };
        for cell in [far, near] {
            layout.tiles.push(TileSpec {
                cell,
                terrain: Terrain::Land,
            });
        }
        layout.streets[0].cells = vec![far, near];
        assert_eq!(
            validate(&layout),
            Err(LayoutError::DisconnectedStreet {
                lane: LANE,
                from: far,
                to: near
            })
        );
    }

    #[test]
    fn streets_and_buildings_need_a_known_tile() {
        let mut layout = LayoutSpec::default();
        layout.streets.push(street(LANE, vec![A]));
        assert_eq!(validate(&layout), Err(LayoutError::UnknownTile(A)));

        layout.streets.clear();
        layout.buildings.push(building(BUILDING, A));
        assert_eq!(validate(&layout), Err(LayoutError::UnknownTile(A)));
    }

    #[test]
    fn streets_and_buildings_reference_existing_network_and_demand_ids() {
        let mut layout = layout();
        let unknown_lane = LaneId::new(99);
        layout.streets.push(street(unknown_lane, vec![A]));
        assert_eq!(
            validate(&layout),
            Err(LayoutError::UnknownLane(unknown_lane))
        );

        layout.streets.clear();
        layout.buildings.push(building(BUILDING, A));
        let unknown_zone = ZoneId::new(99);
        layout.buildings[0].zone = unknown_zone;
        assert_eq!(
            validate(&layout),
            Err(LayoutError::UnknownZone(unknown_zone))
        );

        layout.buildings[0].zone = ZONE;
        layout.buildings[0].access = Some(unknown_lane);
        assert_eq!(
            validate(&layout),
            Err(LayoutError::UnknownLane(unknown_lane))
        );
    }

    #[test]
    fn validation_rejects_duplicate_table_keys() {
        let mut layout = layout();
        layout.tiles.push(TileSpec {
            cell: A,
            terrain: Terrain::Water,
        });
        assert_eq!(validate(&layout), Err(LayoutError::DuplicateTile(A)));

        layout.tiles.pop();
        layout.buildings = vec![building(BUILDING, A), building(BUILDING, B)];
        assert_eq!(
            validate(&layout),
            Err(LayoutError::DuplicateBuilding(BUILDING))
        );

        layout.buildings.clear();
        layout.streets = vec![street(LANE, vec![A]), street(LANE, vec![B])];
        assert_eq!(validate(&layout), Err(LayoutError::DuplicateStreet(LANE)));
    }

    #[test]
    fn an_empty_street_path_is_invalid() {
        let mut layout = layout();
        layout.streets.push(street(LANE, Vec::new()));
        assert_eq!(validate(&layout), Err(LayoutError::EmptyStreet(LANE)));
    }

    #[test]
    fn sorting_tables_preserves_directed_street_paths() {
        let mut layout = layout();
        layout.tiles.reverse();
        layout.buildings = vec![building(OTHER_BUILDING, B), building(BUILDING, A)];
        layout.streets = vec![street(OTHER_LANE, vec![A, B]), street(LANE, vec![B, A])];
        layout.sort_tables();

        assert_eq!(
            layout
                .tiles
                .iter()
                .map(|tile| tile.cell)
                .collect::<Vec<_>>(),
            [A, B, WATER]
        );
        assert_eq!(
            layout
                .buildings
                .iter()
                .map(|site| site.id)
                .collect::<Vec<_>>(),
            [BUILDING, OTHER_BUILDING]
        );
        assert_eq!(
            layout
                .streets
                .iter()
                .map(|site| site.lane)
                .collect::<Vec<_>>(),
            [LANE, OTHER_LANE]
        );
        assert_eq!(layout.streets[0].cells, [B, A]);
    }

    #[test]
    fn layout_is_empty_only_when_all_tables_are_empty() {
        let mut layout = LayoutSpec::default();
        assert!(layout.is_empty());

        layout.buildings.push(building(BUILDING, A));
        assert!(!layout.is_empty());

        layout.buildings.clear();
        layout.streets.push(street(LANE, vec![A]));
        assert!(!layout.is_empty());

        layout.streets.clear();
        layout.tiles.push(TileSpec {
            cell: A,
            terrain: Terrain::Land,
        });
        assert!(!layout.is_empty());
    }
}
