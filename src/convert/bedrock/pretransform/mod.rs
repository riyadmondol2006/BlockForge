// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The Bedrock reader pre-transforms: block states that Bedrock does not store but Java needs (fence, pane and wall
//! connections, stair shapes, door halves, redstone, fire, chorus, chests, snowy blocks, stems, hanging plants and
//! the note block instrument).
//!
//! This is `BedrockReaderPreTransformManager` and the `PreTransformManager.solve` loop of Chunker. A block whose
//! handler needs no block outside its own column is transformed while the column is read; the others are queued
//! and run by the column's pre-transform once the neighbour columns are known. A missing neighbour column reads as
//! air. The only handler not here is the painting relocation (an entity, owned by the entity code).

mod base;
mod connect;
mod misc;
mod noteblock;
mod noteblock_table;
mod redstone;
mod stairs;
mod wall;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::convert::bedrock::data_version::BedrockVersion;
use crate::convert::model::block_groups::{
    BARS_AND_GLASS_PANES, DOORS, DOUBLE_CHESTS, FENCE_GATES, SNOWY_GRASS_BLOCKS, STAIRS, WALLS, WOODEN_FENCES,
};
use crate::convert::model::column::PreTransformHandler;
use crate::convert::model::{
    BlockIdentifier, BlockPalette, BlockRef, BlockType, Chunk, Column, EdgeSet, Neighbours, Palette,
};

pub use base::Direction;
use base::{Ctx, calculate_edges};
use connect::Kind;
use misc::Plant;

/// The pre-transform for a column of a Bedrock version, to be run with the neighbour columns.
///
/// It does the whole of [`solve`]: the blocks that need no neighbour are transformed first, then the others against
/// the given neighbours (a missing one reads as air). Hand it to [`Column::add_pre_transform`] together with
/// [`EdgeSet::ALL`], or use [`solve`] to have the edges worked out per column.
pub fn pretransform(version: &'static BedrockVersion) -> PreTransformHandler {
    let manager = Manager::new(version);
    Box::new(move |column: &mut Column, neighbours: &Neighbours<'_>| {
        let (pending, _) = manager.scan(column, true);
        manager.run(column, neighbours, &pending);
    })
}

/// Transform a column read from a Bedrock world (`PreTransformManager.solve`).
///
/// With `allowed` the blocks that look into another column are queued on the column ([`Column::add_pre_transform`],
/// with exactly the edges they need) and the rest is done now; without it everything is done now with no neighbours.
pub fn solve(version: &'static BedrockVersion, column: &mut Column, allowed: bool) {
    let manager = Manager::new(version);
    let (pending, edges) = manager.scan(column, allowed);
    if pending.is_empty() {
        return;
    }
    column.add_pre_transform(edges, move |column: &mut Column, neighbours: &Neighbours<'_>| {
        manager.run(column, neighbours, &pending);
    });
}

/// Which handler a block type has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handler {
    MelonStem,
    PumpkinStem,
    Connect(Kind),
    Stairs,
    FenceGate,
    Redstone,
    Fire,
    Wall,
    Snowy,
    Door,
    NoteBlock,
    HangingPlant(Plant),
    Chest,
}

/// The handlers of a version (`registerHandlers`).
#[derive(Clone, Copy, Debug)]
struct Manager {
    /// Before 1.16: stems connect.
    stems: bool,
    /// Before 1.26.50: connections and stair shapes are not stored.
    connections: bool,
    /// Before 1.26.60: chorus plant, redstone and fire connections are not stored.
    plants: bool,
    /// Before 1.16: walls need their side heights too.
    wall_connections: bool,
}

impl Manager {
    fn new(version: &BedrockVersion) -> Manager {
        Manager {
            stems: !version.at_least([1, 16, 0]),
            connections: !version.at_least([1, 26, 50]),
            plants: !version.at_least([1, 26, 60]),
            wall_connections: !version.at_least([1, 16, 0]),
        }
    }

    /// The handler of a block type. Chunker registers into a map, so when two registrations share a type the later
    /// one wins: the checks run from the last registration to the first.
    fn handler(&self, ty: BlockType) -> Option<Handler> {
        use BlockType as B;
        if DOUBLE_CHESTS.contains(ty) {
            return Some(Handler::Chest);
        }
        match ty {
            B::WEEPING_VINES => return Some(Handler::HangingPlant(Plant::WeepingVines)),
            B::TWISTING_VINES => return Some(Handler::HangingPlant(Plant::TwistingVines)),
            B::CAVE_VINES_HEAD => return Some(Handler::HangingPlant(Plant::CaveVines)),
            B::KELP => return Some(Handler::HangingPlant(Plant::Kelp)),
            B::NOTE_BLOCK => return Some(Handler::NoteBlock),
            _ => {}
        }
        if DOORS.contains(ty) {
            return Some(Handler::Door);
        }
        if SNOWY_GRASS_BLOCKS.contains(ty) {
            return Some(Handler::Snowy);
        }
        if WALLS.contains(ty) {
            return Some(Handler::Wall);
        }
        if self.plants {
            match ty {
                B::FIRE => return Some(Handler::Fire),
                B::REDSTONE_WIRE => return Some(Handler::Redstone),
                B::CHORUS_PLANT => return Some(Handler::Connect(Kind::Chorus)),
                _ => {}
            }
        }
        if FENCE_GATES.contains(ty) {
            return Some(Handler::FenceGate);
        }
        if self.connections {
            if STAIRS.contains(ty) {
                return Some(Handler::Stairs);
            }
            if BARS_AND_GLASS_PANES.contains(ty) {
                return Some(Handler::Connect(Kind::Pane));
            }
            if ty == B::NETHER_BRICK_FENCE {
                return Some(Handler::Connect(Kind::NetherFence));
            }
            if WOODEN_FENCES.contains(ty) {
                return Some(Handler::Connect(Kind::WoodenFence));
            }
            if ty == B::TRIPWIRE {
                return Some(Handler::Connect(Kind::Tripwire));
            }
        }
        if self.stems {
            match ty {
                B::ATTACHED_PUMPKIN_STEM => return Some(Handler::PumpkinStem),
                B::ATTACHED_MELON_STEM => return Some(Handler::MelonStem),
                _ => {}
            }
        }
        None
    }

    fn handler_of(&self, block: &BlockRef) -> Option<Handler> {
        block.block_type().and_then(|ty| self.handler(ty))
    }

    /// The edges the handler needs for the block at world (x, y, z) (`getRequiredEdges`).
    fn required_edges(&self, handler: Handler, ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> EdgeSet {
        match handler {
            Handler::Connect(kind) => calculate_edges(x, z, connect::directions(kind)),
            Handler::Fire => calculate_edges(x, z, &Direction::ALL),
            Handler::Stairs | Handler::Redstone => EdgeSet::ALL,
            Handler::MelonStem | Handler::PumpkinStem => calculate_edges(x, z, &Direction::ALL_HORIZONTAL),
            Handler::FenceGate => misc::gate_edges(x, z, block),
            Handler::Wall if self.wall_connections => calculate_edges(x, z, &Direction::ALL),
            Handler::Chest => misc::chest_edges(ctx, x, y, z, block),
            Handler::Wall | Handler::Snowy | Handler::Door | Handler::NoteBlock | Handler::HangingPlant(_) => {
                EdgeSet::EMPTY
            }
        }
    }

    /// The new block for the block at world (x, y, z) (`handle`).
    fn handle(&self, handler: Handler, ctx: &Ctx, x: i32, y: i32, z: i32, block: &BlockRef) -> BlockIdentifier {
        match handler {
            Handler::MelonStem => misc::stem(ctx, BlockType::MELON, x, y, z, block),
            Handler::PumpkinStem => misc::stem(ctx, BlockType::PUMPKIN, x, y, z, block),
            Handler::Connect(kind) => connect::handle(kind, ctx, x, y, z, block),
            Handler::Stairs => stairs::handle(ctx, x, y, z, block),
            Handler::FenceGate => misc::gate(ctx, x, y, z, block),
            Handler::Redstone => redstone::handle(ctx, x, y, z, block),
            Handler::Fire => connect::handle_fire(ctx, x, y, z, block),
            Handler::Wall => wall::handle(self.wall_connections, ctx, x, y, z, block),
            Handler::Snowy => misc::snowy(ctx, x, y, z, block),
            Handler::Door => misc::door(ctx, x, y, z, block),
            Handler::NoteBlock => misc::note_block(ctx, x, y, z, block),
            Handler::HangingPlant(plant) => misc::hanging_plant(ctx, plant, x, y, z, block),
            Handler::Chest => misc::chest(ctx, x, y, z, block),
        }
    }

    /// Run the handler on the block at (x, y, z) and store the result when it changed.
    fn apply(&self, handler: Handler, column: &mut Column, neighbours: &Neighbours<'_>, pos: (i32, i32, i32)) {
        let (x, y, z) = pos;
        let block = column.block(x, y, z).clone();
        let new = self.handle(handler, &Ctx { column, neighbours }, x, y, z, &block);
        if new != *block {
            column.set_block(x, y, z, Arc::new(new));
        }
    }

    /// The voxels of a section that have a handler, in Chunker's scan order (y, then x, then z). Only the palette
    /// keys are tested, so a section without a handled key costs nothing and the others cost one pass over the
    /// indices. A handler only ever rewrites its own block, so reading the section up front sees what Chunker sees.
    fn candidates(&self, chunk: &Chunk) -> Vec<(usize, usize, usize, Handler)> {
        let mut found = Vec::new();
        match &chunk.blocks {
            Palette::Empty => {}
            Palette::Single(block) => {
                if let Some(handler) = self.handler_of(block) {
                    for y in 0..16 {
                        for x in 0..16 {
                            for z in 0..16 {
                                found.push((x, y, z, handler));
                            }
                        }
                    }
                }
            }
            Palette::Packed { keys, indices } => {
                let handlers: Vec<Option<Handler>> = keys.iter().map(|k| self.handler_of(k)).collect();
                if handlers.iter().any(Option::is_some) {
                    for y in 0..16 {
                        for x in 0..16 {
                            for z in 0..16 {
                                let index = indices.get(BlockPalette::index(x, y, z)) as usize;
                                if let Some(Some(handler)) = handlers.get(index) {
                                    found.push((x, y, z, *handler));
                                }
                            }
                        }
                    }
                }
            }
        }
        found
    }

    /// Transform what can be done without neighbours and collect the rest with the edges it needs
    /// (the loop of `PreTransformManager.solve`).
    fn scan(&self, column: &mut Column, allowed: bool) -> (Vec<(i32, i32, i32, Handler)>, EdgeSet) {
        let mut pending = Vec::new();
        let mut edges = EdgeSet::EMPTY;
        let none = Neighbours::new();
        let base_x = column.position.x << 4;
        let base_z = column.position.z << 4;
        let sections: Vec<i8> = column.chunks.keys().copied().collect();
        for section in sections {
            let Some(chunk) = column.chunks.get(&section) else { continue };
            for (local_x, local_y, local_z, handler) in self.candidates(chunk) {
                let (x, y, z) =
                    (base_x | local_x as i32, (section as i32) << 4 | local_y as i32, base_z | local_z as i32);
                let required = if allowed {
                    let block = column.block(x, y, z);
                    self.required_edges(handler, &Ctx { column, neighbours: &none }, x, y, z, block)
                } else {
                    EdgeSet::EMPTY
                };
                if required.is_empty() {
                    self.apply(handler, column, &none, (x, y, z));
                } else {
                    edges = edges.union(required);
                    pending.push((x, y, z, handler));
                }
            }
        }
        (pending, edges)
    }

    /// Run the queued blocks with the neighbours (`solveEdges`).
    fn run(&self, column: &mut Column, neighbours: &Neighbours<'_>, pending: &[(i32, i32, i32, Handler)]) {
        for &(x, y, z, handler) in pending {
            self.apply(handler, column, neighbours, (x, y, z));
        }
    }
}
