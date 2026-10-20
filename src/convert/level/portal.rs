// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! A nether portal record (`ChunkerPortal`): the lowest block of a portal frame row, its width and direction.

use super::dimension::Dimension;
use super::world::ChunkPos;

/// A portal: `x, y, z` is the first block, `width` blocks extend along `(xa, za)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Portal {
    pub dimension: Dimension,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub width: i8,
    pub xa: i8,
    pub za: i8,
}

impl Portal {
    /// The portal's blocks that lie in `chunk` (`getHorizontalBlocks`).
    pub fn horizontal_blocks(&self, chunk: ChunkPos) -> Vec<(i32, i32, i32)> {
        let span = self.width as i32 - 1;
        let offset_x = self.x + span * self.xa as i32;
        let offset_z = self.z + span * self.za as i32;
        let (origin_chunk_x, origin_chunk_z) = (self.x >> 4, self.z >> 4);
        let (offset_chunk_x, offset_chunk_z) = (offset_x >> 4, offset_z >> 4);
        if chunk.x < origin_chunk_x.min(offset_chunk_x) || chunk.z < origin_chunk_z.min(offset_chunk_z) {
            return Vec::new();
        }
        if chunk.x > origin_chunk_x.max(offset_chunk_x) || chunk.z > origin_chunk_z.max(offset_chunk_z) {
            return Vec::new();
        }
        let mut blocks = Vec::new();
        for x in self.x.min(offset_x)..=self.x.max(offset_x) {
            for z in self.z.min(offset_z)..=self.z.max(offset_z) {
                if chunk.is_inside(x, z) {
                    blocks.push((x, self.y, z));
                }
            }
        }
        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal(x: i32, z: i32, width: i8, xa: i8, za: i8) -> Portal {
        Portal { dimension: Dimension::overworld(), x, y: 70, z, width, xa, za }
    }

    #[test]
    fn blocks_inside_one_chunk() {
        let p = portal(2, 3, 3, 1, 0);
        assert_eq!(p.horizontal_blocks(ChunkPos::new(0, 0)), [(2, 70, 3), (3, 70, 3), (4, 70, 3)]);
        assert!(p.horizontal_blocks(ChunkPos::new(1, 0)).is_empty());
        assert!(p.horizontal_blocks(ChunkPos::new(0, -1)).is_empty());
    }

    #[test]
    fn blocks_split_over_chunks() {
        // x 14..=17 crosses the chunk border at 16.
        let p = portal(14, -1, 4, 1, 0);
        assert_eq!(p.horizontal_blocks(ChunkPos::new(0, -1)), [(14, 70, -1), (15, 70, -1)]);
        assert_eq!(p.horizontal_blocks(ChunkPos::new(1, -1)), [(16, 70, -1), (17, 70, -1)]);
    }

    #[test]
    fn negative_direction() {
        let p = portal(0, 0, 2, 0, -1);
        assert_eq!(p.horizontal_blocks(ChunkPos::new(0, -1)), [(0, 70, -1)]);
        assert_eq!(p.horizontal_blocks(ChunkPos::new(0, 0)), [(0, 70, 0)]);
    }
}
