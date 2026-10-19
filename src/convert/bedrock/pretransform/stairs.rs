// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Stairs: the corner shape from the neighbouring stairs (`BedrockStairShapePreTransformHandler`).

use crate::convert::model::block_groups::STAIRS;
use crate::convert::model::{BlockIdentifier, BlockRef, StairShape, states};

use super::base::{Ctx, direction_of};

fn is_stairs(block: &BlockRef) -> bool {
    block.block_type().is_some_and(|t| STAIRS.contains(t))
}

/// Stairs connect to stairs of the same half.
fn can_connect(source: &BlockRef, relative: &BlockRef) -> bool {
    is_stairs(relative) && source.get(states::HALF) == relative.get(states::HALF)
}

/// Whether the stairs can take the shape: not when the block beside it is a stair facing the same way and half.
fn can_take_shape(source: &BlockRef, relative: &BlockRef) -> bool {
    if !is_stairs(relative) {
        return true;
    }
    if source.get(states::FACING_HORIZONTAL) != relative.get(states::FACING_HORIZONTAL) {
        return true;
    }
    source.get(states::HALF) != relative.get(states::HALF)
}

pub(super) fn handle(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let Some(facing) = block.get(states::FACING_HORIZONTAL) else { return (**block).clone() };
    let with_shape = |shape: StairShape| block.with(states::STAIR_SHAPE, shape);

    // A stair in front that turns away from this one makes an outer corner.
    let relative = ctx.relative(x, y, z, direction_of(facing));
    if can_connect(block, relative) {
        if let Some(relative_facing) = relative.get(states::FACING_HORIZONTAL) {
            // Chunker looks beside this block here, not beside the neighbour.
            let relative_opposite = ctx.relative(x, y, z, direction_of(relative_facing));
            if facing.is_adjacent(relative_facing) && can_take_shape(relative, relative_opposite) {
                return with_shape(if relative_facing == facing.rotate_anti_clockwise() {
                    StairShape::OUTER_LEFT
                } else {
                    StairShape::OUTER_RIGHT
                });
            }
        }
    }

    // A stair behind that turns towards this one makes an inner corner.
    let opposite = ctx.relative(x, y, z, direction_of(facing).opposite());
    if can_connect(block, opposite) {
        if let Some(relative_facing) = opposite.get(states::FACING_HORIZONTAL) {
            let relative_opposite = ctx.relative(x, y, z, direction_of(relative_facing).opposite());
            if facing.is_adjacent(relative_facing) && can_take_shape(opposite, relative_opposite) {
                return with_shape(if relative_facing == facing.rotate_anti_clockwise() {
                    StairShape::INNER_LEFT
                } else {
                    StairShape::INNER_RIGHT
                });
            }
        }
    }
    with_shape(StairShape::STRAIGHT)
}
