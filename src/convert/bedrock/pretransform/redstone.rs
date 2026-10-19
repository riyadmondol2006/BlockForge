// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Redstone wire: the connection of each side (`BedrockRedstonePreTransformHandler`).

use crate::convert::model::block_groups::REDSTONE_CONNECTABLE;
use crate::convert::model::{BlockIdentifier, BlockRef, BlockType, RedstoneConnection, State, states};

use super::base::{Ctx, Direction};

fn side_state(direction: Direction) -> State<RedstoneConnection> {
    match direction {
        Direction::North => states::REDSTONE_NORTH,
        Direction::East => states::REDSTONE_EAST,
        Direction::South => states::REDSTONE_SOUTH,
        _ => states::REDSTONE_WEST,
    }
}

fn is_solid(block: &BlockRef) -> bool {
    block.block_type().is_some_and(|t| t.is_all_faces_solid())
}

fn can_connect(direction: Direction, relative: &BlockRef) -> bool {
    let Some(ty) = relative.block_type() else { return false };
    if !REDSTONE_CONNECTABLE.contains(ty) {
        return false;
    }
    // A repeater only connects along its own axis.
    if ty == BlockType::REPEATER {
        return match (relative.get(states::FACING_HORIZONTAL), direction.horizontal()) {
            (Some(facing), Some(side)) => !facing.is_adjacent(side),
            _ => false,
        };
    }
    // An observer only connects at its output.
    if ty == BlockType::OBSERVER {
        return relative.get(states::FACING_ALL) == Some(direction.facing());
    }
    true
}

fn calculate_connection(ctx: &Ctx, direction: Direction, x: i32, y: i32, z: i32) -> RedstoneConnection {
    let relative = ctx.relative(x, y, z, direction);
    if can_connect(direction, relative) {
        return RedstoneConnection::SIDE;
    }
    // Up the side of a block, which needs a see through block above.
    let target = ctx.relative(x, y + 1, z, direction);
    let above = ctx.relative(x, y, z, Direction::Up);
    if can_connect(direction, target) && !is_solid(above) {
        return RedstoneConnection::UP;
    }
    // Down the side, which needs the side block to be see through (Chunker answers SIDE here, as Java does).
    let target = ctx.relative(x, y - 1, z, direction);
    if can_connect(direction, target) && !is_solid(relative) {
        return RedstoneConnection::SIDE;
    }
    RedstoneConnection::NONE
}

pub(super) fn handle(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let mut out = (**block).clone();
    for direction in Direction::ALL_HORIZONTAL {
        out.set(side_state(direction), calculate_connection(ctx, direction, x, y, z));
    }
    let on = |direction: Direction| out.get(side_state(direction)) != Some(RedstoneConnection::NONE);
    let (north, east, south, west) =
        (on(Direction::North), on(Direction::East), on(Direction::South), on(Direction::West));
    let north_south_none = !north && !south;
    let east_west_none = !east && !west;
    // A wire with no side or a lone line is drawn along the axis, as in Java.
    if !north && east_west_none {
        out.set(states::REDSTONE_NORTH, RedstoneConnection::SIDE);
    }
    if !east && north_south_none {
        out.set(states::REDSTONE_EAST, RedstoneConnection::SIDE);
    }
    if !south && east_west_none {
        out.set(states::REDSTONE_SOUTH, RedstoneConnection::SIDE);
    }
    if !west && north_south_none {
        out.set(states::REDSTONE_WEST, RedstoneConnection::SIDE);
    }
    out
}
