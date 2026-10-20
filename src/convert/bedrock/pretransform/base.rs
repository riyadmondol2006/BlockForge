// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! What every handler shares: the six directions, relative block lookups across the column edge and the edges a
//! block needs (`BlockPreTransformHandler`).

use crate::convert::model::{
    BlockRef, Column, Edge, EdgeSet, FacingDirection, FacingDirectionHorizontal, Neighbours, air,
};

/// A relative direction from one block to another (`BlockPreTransformHandler.Direction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    North,
    East,
    South,
    West,
    Up,
    Down,
}

impl Direction {
    pub const ALL: [Direction; 6] =
        [Direction::North, Direction::East, Direction::South, Direction::West, Direction::Up, Direction::Down];
    pub const ALL_HORIZONTAL: [Direction; 4] = [Direction::North, Direction::East, Direction::South, Direction::West];
    pub const EAST_WEST: [Direction; 2] = [Direction::East, Direction::West];
    pub const NORTH_SOUTH: [Direction; 2] = [Direction::North, Direction::South];

    /// The two faces next to a facing (`getAdjacentFaces`).
    pub fn adjacent_faces(facing: FacingDirectionHorizontal) -> &'static [Direction; 2] {
        match facing {
            FacingDirectionHorizontal::NORTH | FacingDirectionHorizontal::SOUTH => &Direction::EAST_WEST,
            _ => &Direction::NORTH_SOUTH,
        }
    }

    /// The edge the direction crosses, none for up and down.
    pub const fn edge(self) -> Option<Edge> {
        match self {
            Direction::North => Some(Edge::NegativeZ),
            Direction::East => Some(Edge::PositiveX),
            Direction::South => Some(Edge::PositiveZ),
            Direction::West => Some(Edge::NegativeX),
            Direction::Up | Direction::Down => None,
        }
    }

    /// The block offset (x, y, z).
    pub const fn offset(self) -> (i32, i32, i32) {
        match self {
            Direction::North => (0, 0, -1),
            Direction::East => (1, 0, 0),
            Direction::South => (0, 0, 1),
            Direction::West => (-1, 0, 0),
            Direction::Up => (0, 1, 0),
            Direction::Down => (0, -1, 0),
        }
    }

    pub const fn opposite(self) -> Direction {
        match self {
            Direction::North => Direction::South,
            Direction::East => Direction::West,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
        }
    }

    /// The horizontal facing, none for up and down (`asFacingDirectionHorizontal` throws there).
    pub const fn horizontal(self) -> Option<FacingDirectionHorizontal> {
        match self {
            Direction::North => Some(FacingDirectionHorizontal::NORTH),
            Direction::East => Some(FacingDirectionHorizontal::EAST),
            Direction::South => Some(FacingDirectionHorizontal::SOUTH),
            Direction::West => Some(FacingDirectionHorizontal::WEST),
            Direction::Up | Direction::Down => None,
        }
    }

    pub const fn facing(self) -> FacingDirection {
        match self {
            Direction::North => FacingDirection::NORTH,
            Direction::East => FacingDirection::EAST,
            Direction::South => FacingDirection::SOUTH,
            Direction::West => FacingDirection::WEST,
            Direction::Up => FacingDirection::UP,
            Direction::Down => FacingDirection::DOWN,
        }
    }
}

/// `FacingDirectionHorizontal.asDirection`.
pub const fn direction_of(facing: FacingDirectionHorizontal) -> Direction {
    match facing {
        FacingDirectionHorizontal::NORTH => Direction::North,
        FacingDirectionHorizontal::EAST => Direction::East,
        FacingDirectionHorizontal::SOUTH => Direction::South,
        FacingDirectionHorizontal::WEST => Direction::West,
    }
}

/// The edges needed to look in the given directions from a block at world (x, z): an edge only when the neighbouring
/// block lies in another column (`calculateEdges`).
pub fn calculate_edges(x: i32, z: i32, directions: &[Direction]) -> EdgeSet {
    let mut set = EdgeSet::EMPTY;
    for direction in directions {
        let Some(edge) = direction.edge() else { continue };
        let (dx, _, dz) = direction.offset();
        if (x + dx) >> 4 != x >> 4 || (z + dz) >> 4 != z >> 4 {
            set.insert(edge);
        }
    }
    set
}

/// The column being transformed and the neighbours that are present.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub column: &'a Column,
    pub neighbours: &'a Neighbours<'a>,
}

impl<'a> Ctx<'a> {
    /// The column holding the block next to world (x, z) in the direction, `None` when that column is not there.
    fn target(&self, x: i32, z: i32, direction: Direction) -> Option<&'a Column> {
        let Some(edge) = direction.edge() else { return Some(self.column) };
        let (dx, _, dz) = direction.offset();
        let (cx, cz) = ((x + dx) >> 4, (z + dz) >> 4);
        if cx == self.column.position.x && cz == self.column.position.z {
            return Some(self.column);
        }
        let target = self.neighbours.get(edge)?;
        // Chunker throws when the supplied column is not the adjacent one; treat it as absent.
        if target.position.x != cx || target.position.z != cz {
            debug_assert!(false, "relative request is outside of adjacent chunks");
            return None;
        }
        Some(target)
    }

    /// The block next to (x, y, z) in a direction; air when the neighbour column is missing (`getRelative`).
    pub fn relative(&self, x: i32, y: i32, z: i32, direction: Direction) -> &'a BlockRef {
        match self.target(x, z, direction) {
            Some(column) => {
                let (dx, dy, dz) = direction.offset();
                column.block(x + dx, y + dy, z + dz)
            }
            None => air(),
        }
    }
}
