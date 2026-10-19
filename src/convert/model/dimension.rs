// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Dimensions (`intermediate.world.Dimension`), as far as the column model needs them: an identifier, the ids the
//! two editions use, the biome used when a biome has no equivalent and the vertical size of the biome grid.

use std::sync::Arc;

use super::generated::VanillaBiome;

/// A dimension of a world.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dimension {
    /// Numeric id on Java (legacy), if the dimension has one.
    pub java_id: Option<i32>,
    /// Numeric id on Bedrock.
    pub bedrock_id: i32,
    /// Namespaced identifier (`minecraft:overworld`).
    pub identifier: Arc<str>,
    /// Biome substituted when a biome cannot be written.
    pub fallback_biome: VanillaBiome,
    /// Number of vertical biome cells (24 overworld, 8 nether, 16 end).
    pub biome_height: u8,
}

impl Dimension {
    pub fn overworld() -> Dimension {
        Dimension::new(Some(0), 0, "minecraft:overworld", VanillaBiome::PLAINS, 24)
    }

    pub fn nether() -> Dimension {
        Dimension::new(Some(-1), 1, "minecraft:the_nether", VanillaBiome::NETHER_WASTES, 8)
    }

    pub fn the_end() -> Dimension {
        Dimension::new(Some(1), 2, "minecraft:the_end", VanillaBiome::THE_END, 16)
    }

    pub fn new(
        java_id: Option<i32>,
        bedrock_id: i32,
        identifier: &str,
        fallback_biome: VanillaBiome,
        biome_height: u8,
    ) -> Dimension {
        Dimension { java_id, bedrock_id, identifier: Arc::from(identifier), fallback_biome, biome_height }
    }
}
