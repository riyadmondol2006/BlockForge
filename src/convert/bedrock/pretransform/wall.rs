// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Walls: the post (`up`) and, before 1.16, the height of each side (`BedrockWallPreTransformHandler`).

use crate::convert::model::block_groups::{BARS_AND_GLASS_PANES, FENCE_GATES, WALLS, WOODEN_FENCES};
use crate::convert::model::{BlockIdentifier, BlockRef, Bool, State, WallHeight, states};

use super::base::{Ctx, Direction};

fn wall_state(direction: Direction) -> State<WallHeight> {
    match direction {
        Direction::North => states::WALL_NORTH,
        Direction::East => states::WALL_EAST,
        Direction::South => states::WALL_SOUTH,
        _ => states::WALL_WEST,
    }
}

fn is_wall(block: &BlockRef) -> bool {
    block.block_type().is_some_and(|t| WALLS.contains(t))
}

fn wall_height(above: &BlockRef, relative: &BlockRef, relative_above: &BlockRef) -> WallHeight {
    if is_wall(relative) {
        // Tall when there are walls above both this wall and the neighbour.
        return if is_wall(relative_above) && is_wall(above) { WallHeight::TALL } else { WallHeight::LOW };
    }
    match relative.block_type() {
        Some(ty) if ty.is_all_faces_solid() || BARS_AND_GLASS_PANES.contains(ty) || FENCE_GATES.contains(ty) => {
            WallHeight::LOW
        }
        _ => WallHeight::NONE,
    }
}

fn connected(source: &BlockIdentifier, side: Direction) -> u8 {
    (source.get(wall_state(side)) != Some(WallHeight::NONE)) as u8
}

/// Posts show unless the wall runs straight and has no wall, fence or solid block above.
fn can_connect_post(source: &BlockIdentifier, above: &BlockRef) -> bool {
    let north = connected(source, Direction::North);
    let east = connected(source, Direction::East);
    let south = connected(source, Direction::South);
    let west = connected(source, Direction::West);
    if north + east + south + west != 2 {
        return true;
    }
    if !(north == 1 && south == 1 || east == 1 && west == 1) {
        return true;
    }
    above.block_type().is_some_and(|t| WALLS.contains(t) || WOODEN_FENCES.contains(t) || t.is_all_faces_solid())
}

pub(super) fn handle(needs_connections: bool, ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let above = ctx.relative(x, y, z, Direction::Up);
    let mut out = (**block).clone();
    if needs_connections {
        for direction in Direction::ALL_HORIZONTAL {
            let relative = ctx.relative(x, y, z, direction);
            let relative_above = ctx.relative(x, y + 1, z, direction);
            out.set(wall_state(direction), wall_height(above, relative, relative_above));
        }
    }
    let post = can_connect_post(&out, above);
    out.set(states::UP, if post { Bool::TRUE } else { Bool::FALSE });
    out
}
