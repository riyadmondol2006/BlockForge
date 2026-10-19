// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The small handlers: doors, fence gates, chests, snowy blocks, stems, hanging plants and note blocks.

use crate::convert::model::block_groups::{DOORS, SNOWY_BLOCKS, WALLS};
use crate::convert::model::{
    BlockEntityData, BlockIdentifier, BlockRef, BlockType, Bool, ChestType, EdgeSet, Half, StateSet, states,
};

use super::base::{Ctx, Direction, calculate_edges, direction_of};
use super::noteblock;

/// Doors: the hinge of the bottom half comes from the top half, the open bit and direction of the top half from the
/// bottom half (`BedrockDoorPreTransformHandler`).
pub(super) fn door(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let half = block.get(states::HALF);
    let other = ctx.relative(x, y, z, if half == Some(Half::TOP) { Direction::Down } else { Direction::Up });
    let mut out = (**block).clone();
    if !other.block_type().is_some_and(|t| DOORS.contains(t)) {
        return out;
    }
    if half == Some(Half::BOTTOM) {
        if let Some(hinge) = other.get(states::DOOR_HINGE) {
            out.set(states::DOOR_HINGE, hinge);
        }
    } else {
        if let Some(open) = other.get(states::OPEN) {
            out.set(states::OPEN, open);
        }
        if let Some(facing) = other.get(states::FACING_HORIZONTAL) {
            out.set(states::FACING_HORIZONTAL, facing);
        }
    }
    out
}

/// The edges a fence gate needs: the two sides.
pub(super) fn gate_edges(x: i32, z: i32, block: &BlockRef) -> EdgeSet {
    match block.get(states::FACING_HORIZONTAL) {
        Some(facing) => calculate_edges(x, z, Direction::adjacent_faces(facing)),
        None => EdgeSet::EMPTY,
    }
}

/// Fence gates: `in_wall` when a wall is beside the gate (`BedrockFenceGatePreTransformHandler`).
pub(super) fn gate(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let mut in_wall = false;
    if let Some(facing) = block.get(states::FACING_HORIZONTAL) {
        in_wall = Direction::adjacent_faces(facing)
            .iter()
            .any(|&side| ctx.relative(x, y, z, side).block_type().is_some_and(|t| WALLS.contains(t)));
    }
    block.with(states::IN_WALL, if in_wall { Bool::TRUE } else { Bool::FALSE })
}

/// Whether the chest still needs its side worked out: only the lead of a pair stores it in older versions.
fn chest_unpaired(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> bool {
    block.get(states::CHEST_TYPE) == Some(ChestType::SINGLE)
        && matches!(
            ctx.column.block_entity(x, y, z).map(|be| &be.data),
            Some(BlockEntityData::Chest(_) | BlockEntityData::TrappedChest(_))
        )
}

pub(super) fn chest_edges(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> EdgeSet {
    if !chest_unpaired(ctx, x, y, z, block) {
        return EdgeSet::EMPTY;
    }
    match block.get(states::FACING_HORIZONTAL) {
        Some(facing) => calculate_edges(x, z, Direction::adjacent_faces(facing)),
        None => EdgeSet::EMPTY,
    }
}

/// Chests: a single chest next to the lead of a pair is the other half (`BedrockChestPreTransformHandler`).
pub(super) fn chest(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    if !chest_unpaired(ctx, x, y, z, block) {
        return (**block).clone();
    }
    let Some(facing) = block.get(states::FACING_HORIZONTAL) else { return (**block).clone() };
    let right = ctx.relative(x, y, z, direction_of(facing.rotate_anti_clockwise()));
    if right.get(states::CHEST_TYPE) == Some(ChestType::LEFT) {
        return block.with(states::CHEST_TYPE, ChestType::RIGHT);
    }
    let left = ctx.relative(x, y, z, direction_of(facing.rotate_clockwise()));
    if left.get(states::CHEST_TYPE) == Some(ChestType::RIGHT) {
        return block.with(states::CHEST_TYPE, ChestType::LEFT);
    }
    block.with(states::CHEST_TYPE, ChestType::SINGLE)
}

/// Grass like blocks are snowy when snow lies above (the handler in `BedrockReaderPreTransformManager`).
pub(super) fn snowy(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let above = ctx.relative(x, y, z, Direction::Up);
    let snow = above.block_type().is_some_and(|t| SNOWY_BLOCKS.contains(t));
    block.with(states::SNOWY, if snow { Bool::TRUE } else { Bool::FALSE })
}

/// Attached stems face the first horizontal neighbour that is their fruit (`BedrockStemPreTransformHandler`).
pub(super) fn stem(ctx: &Ctx, fruit: BlockType, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    for direction in Direction::ALL_HORIZONTAL {
        if ctx.relative(x, y, z, direction).block_type() == Some(fruit) {
            if let Some(facing) = direction.horizontal() {
                return block.with(states::FACING_HORIZONTAL, facing);
            }
        }
    }
    (**block).clone()
}

/// A plant that becomes its body block when another plant of its kind is next to it (`BedrockHangingPlant...`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plant {
    Kelp,
    CaveVines,
    TwistingVines,
    WeepingVines,
}

pub(super) fn hanging_plant(ctx: &Ctx, plant: Plant, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    use BlockType as B;
    // Whether the neighbour is checked below (hanging) or above, the body block and the types that connect.
    let (below, body, connectable): (bool, B, [B; 2]) = match plant {
        Plant::Kelp => (false, B::KELP_PLANT, [B::KELP, B::KELP_PLANT]),
        Plant::CaveVines => (true, B::CAVE_VINES_BODY, [B::CAVE_VINES_BODY, B::CAVE_VINES_HEAD]),
        Plant::TwistingVines => (false, B::TWISTING_VINES_PLANT, [B::TWISTING_VINES, B::TWISTING_VINES_PLANT]),
        Plant::WeepingVines => (true, B::WEEPING_VINES_PLANT, [B::WEEPING_VINES, B::WEEPING_VINES_PLANT]),
    };
    let relative = ctx.relative(x, y, z, if below { Direction::Down } else { Direction::Up });
    if !relative.block_type().is_some_and(|t| connectable.contains(&t)) {
        return (**block).clone();
    }
    if plant == Plant::CaveVines {
        // Keeps every state and the preserved identifier.
        return BlockIdentifier::from_parts(body.into(), block.states().clone(), block.preserved().cloned());
    }
    // The others only keep the water.
    let mut set = StateSet::new();
    set.set(states::WATERLOGGED, block.get(states::WATERLOGGED).unwrap_or(Bool::FALSE));
    BlockIdentifier::with_states(body, set)
}

/// Note blocks: the instrument of the block below (`BedrockNoteBlockPreTransformHandler`).
pub(super) fn note_block(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let below = ctx.relative(x, y, z, Direction::Down);
    block.with(states::NOTE_BLOCK_INSTRUMENT, noteblock::instrument(below.block_type()))
}
