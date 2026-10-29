//! The version-neutral world: columns, chunk sections, palettes, block types and states, block entities, items.
//!
//! Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//!
//! Chunker's own model is the pivot of the whole converter: a block is a [`BlockType`] plus [`StateId`] values, a
//! column holds [`Palette`]s of shared [`BlockRef`]s. `generated.rs` is produced from Chunker's Java enums by
//! `tools/convert/gen_model.py`; its enum ordinals equal the Java ordinals.

#[macro_use]
mod macros;

pub mod biome;
pub mod block;
pub mod blockentity;
pub mod color;
pub mod column;
pub mod coords;
pub mod dimension;
pub mod entity;
#[rustfmt::skip]
pub mod generated;
pub mod identifier;
pub mod item;
pub mod palette;
pub mod state;
mod state_types;
#[cfg(test)]
mod tests;

use std::fmt;

pub use biome::{Biome, BiomePalette, Biomes, CellPalette};
pub use block::{BlockGroup, BlockIdentifier, BlockKind, BlockRef, PreservedIdentifier, air};
pub use blockentity::{BlockEntity, BlockEntityData, BlockEntityKind};
pub use color::Color;
pub use column::{BlockPalette, Chunk, Column, HeightMap, Neighbours};
pub use coords::{ChunkCoord, Edge, EdgeSet, RegionCoord};
pub use dimension::Dimension;
pub use entity::{Entity, EntityData, EntitySource, EntityType};
pub use generated::*;
pub use identifier::{Identifier, StateValue};
pub use item::{ItemId, ItemProps, ItemStack};
pub use palette::{PackedIndices, Palette};
pub use state::{State, StateEnum, StateMask, StateSet};

/// An error from decoding model values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelError(pub String);

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ModelError {}

/// Result of decoding model values.
pub type Result<T> = std::result::Result<T, ModelError>;
