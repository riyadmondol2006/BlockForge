// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The handlers that only set a boolean per direction: trip wire, wooden and nether brick fences, glass panes and
//! iron bars, chorus plants and fire (`HorizontalConnectable...`, `Connectable...` and `BedrockFire...`).

use crate::convert::model::block_groups::{BARS_AND_GLASS_PANES, FENCE_GATES, WALLS, WOODEN_FENCES};
use crate::convert::model::{BlockIdentifier, BlockRef, BlockType, Bool, State, states};

use super::base::{Ctx, Direction};

/// Which connectable a block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Tripwire,
    WoodenFence,
    NetherFence,
    Pane,
    Chorus,
}

fn direction_state(direction: Direction) -> State<Bool> {
    match direction {
        Direction::North => states::NORTH,
        Direction::East => states::EAST,
        Direction::South => states::SOUTH,
        Direction::West => states::WEST,
        Direction::Up => states::UP,
        Direction::Down => states::DOWN,
    }
}

fn bool_of(value: bool) -> Bool {
    if value { Bool::TRUE } else { Bool::FALSE }
}

/// Whether a fence connects to a fence gate: only to the sides of it.
fn fence_gate_connects(relative: &BlockRef, direction: Direction) -> bool {
    match (relative.get(states::FACING_HORIZONTAL), direction.horizontal()) {
        (Some(facing), Some(side)) => facing.is_adjacent(side),
        _ => false,
    }
}

fn can_connect(kind: Kind, source: &BlockIdentifier, direction: Direction, relative: &BlockRef) -> bool {
    let ty = relative.block_type();
    match kind {
        Kind::Tripwire => matches!(ty, Some(BlockType::TRIPWIRE | BlockType::TRIPWIRE_HOOK)),
        Kind::WoodenFence | Kind::NetherFence => {
            let Some(ty) = ty else { return false };
            if FENCE_GATES.contains(ty) {
                return fence_gate_connects(relative, direction);
            }
            let same_fence = if kind == Kind::WoodenFence {
                WOODEN_FENCES.contains(ty)
            } else {
                ty == BlockType::NETHER_BRICK_FENCE
            };
            same_fence || ty.is_all_faces_solid()
        }
        Kind::Pane => {
            ty.is_some_and(|ty| BARS_AND_GLASS_PANES.contains(ty) || WALLS.contains(ty) || ty.is_all_faces_solid())
        }
        Kind::Chorus => {
            ty.is_some() && ty == source.block_type()
                || ty == Some(BlockType::CHORUS_FLOWER)
                || direction == Direction::Down && ty == Some(BlockType::END_STONE)
        }
    }
}

/// The directions the handler looks at.
pub(super) fn directions(kind: Kind) -> &'static [Direction] {
    match kind {
        Kind::Chorus => &Direction::ALL,
        _ => &Direction::ALL_HORIZONTAL,
    }
}

/// Set the connection state of every direction of the handler.
pub(super) fn handle(kind: Kind, ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let mut out = (**block).clone();
    for &direction in directions(kind) {
        let relative = ctx.relative(x, y, z, direction);
        out.set(direction_state(direction), bool_of(can_connect(kind, block, direction, relative)));
    }
    out
}

/// Fire connects to solid blocks next to and above it; a block below clears every side (`BedrockFire...`).
pub(super) fn handle_fire(ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
    let mut out = (**block).clone();
    for direction in Direction::ALL {
        let solid = ctx.relative(x, y, z, direction).block_type().is_some_and(|t| t.is_all_faces_solid());
        if direction == Direction::Down {
            if solid {
                for side in [Direction::North, Direction::East, Direction::South, Direction::West, Direction::Up] {
                    out.set(direction_state(side), Bool::FALSE);
                }
                return out;
            }
        } else {
            out.set(direction_state(direction), bool_of(solid));
        }
    }
    out
}
