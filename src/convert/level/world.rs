// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! A world to convert: one dimension and the regions it has (`ChunkerWorld`), plus coordinate pairs.
//!
//! `ChunkPos` and `RegionPos` are local stand-ins for Chunker's `ChunkCoordPair` / `RegionCoordPair`, which belong to
//! the column model; swap them when that lands.

use std::collections::BTreeSet;

use super::dimension::Dimension;

/// A chunk (column) position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub fn new(x: i32, z: i32) -> ChunkPos {
        ChunkPos { x, z }
    }

    /// The region containing the chunk.
    pub fn region(self) -> RegionPos {
        RegionPos::new(self.x >> 5, self.z >> 5)
    }

    /// Whether the block position lies in this chunk.
    pub fn is_inside(self, block_x: i32, block_z: i32) -> bool {
        block_x >> 4 == self.x && block_z >> 4 == self.z
    }

    /// Chunker's `toLong`: `(x << 32) | (z & 0xFFFFFFFF)`.
    pub fn to_long(self) -> i64 {
        ((self.x as i64) << 32) | (self.z as u32 as i64)
    }
}

/// A region (32 x 32 chunks) position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionPos {
    pub x: i32,
    pub z: i32,
}

impl RegionPos {
    pub fn new(x: i32, z: i32) -> RegionPos {
        RegionPos { x, z }
    }

    /// The chunk at a local position (0..32) in the region.
    pub fn chunk(self, local_x: i32, local_z: i32) -> ChunkPos {
        ChunkPos::new((self.x << 5) | local_x, (self.z << 5) | local_z)
    }

    /// Whether a chunk lies in the region.
    pub fn contains(self, chunk: ChunkPos) -> bool {
        chunk.region() == self
    }
}

/// A dimension and the regions to convert in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    pub dimension: Dimension,
    pub regions: BTreeSet<RegionPos>,
}

impl World {
    pub fn new(dimension: Dimension, regions: BTreeSet<RegionPos>) -> World {
        World { dimension, regions }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_region_math() {
        let r = RegionPos::new(-1, 2);
        let c = r.chunk(31, 3);
        assert_eq!(c, ChunkPos::new(-1, 67));
        assert_eq!(c.region(), r);
        assert!(r.contains(c));
        assert!(!r.contains(ChunkPos::new(0, 67)));
        assert!(ChunkPos::new(-1, 0).is_inside(-1, 15));
        assert!(!ChunkPos::new(-1, 0).is_inside(0, 0));
        assert_eq!(ChunkPos::new(1, -1).to_long(), (1i64 << 32) | 0xFFFF_FFFF);
    }

    #[test]
    fn world_holds_sorted_regions() {
        let w = World::new(
            Dimension::overworld(),
            [RegionPos::new(1, 0), RegionPos::new(0, 5), RegionPos::new(0, 1)].into_iter().collect(),
        );
        let order: Vec<_> = w.regions.iter().map(|r| (r.x, r.z)).collect();
        assert_eq!(order, [(0, 1), (0, 5), (1, 0)]);
    }
}
