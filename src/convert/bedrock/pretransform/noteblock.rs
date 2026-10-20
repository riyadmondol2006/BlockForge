// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The instrument lookup of note blocks.

use std::sync::LazyLock;

use crate::convert::model::{BlockType, NoteBlockInstrument};

use super::noteblock_table::INSTRUMENTS;

/// The table indexed by block type ordinal.
static BY_TYPE: LazyLock<Vec<Option<NoteBlockInstrument>>> = LazyLock::new(|| {
    let mut table = vec![None; BlockType::COUNT];
    for &(ty, instrument) in INSTRUMENTS {
        table[ty as usize] = Some(instrument);
    }
    table
});

/// The instrument played above a block; the harp for blocks that are not listed (and custom blocks).
pub(super) fn instrument(below: Option<BlockType>) -> NoteBlockInstrument {
    below.and_then(|ty| BY_TYPE[ty as usize]).unwrap_or(NoteBlockInstrument::HARP)
}
