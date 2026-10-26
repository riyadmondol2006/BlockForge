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

/// The widest portal the game can make (a 23 by 23 frame).
pub const MAX_PORTAL_WIDTH: i8 = 23;

impl Portal {
    /// The portal's blocks that lie in `chunk` (`getHorizontalBlocks`). A record no game could have written (a
    /// width beyond [`MAX_PORTAL_WIDTH`], a direction other than -1, 0 or 1) gives no blocks: from a damaged or
    /// crafted world it used to make the conversion walk billions of positions per chunk, and never finish.
    pub fn horizontal_blocks(&self, chunk: ChunkPos) -> Vec<(i32, i32, i32)> {
        if !(0..=MAX_PORTAL_WIDTH).contains(&self.width) || !(-1..=1).contains(&self.xa) || !(-1..=1).contains(&self.za)
        {
            return Vec::new();
        }
        // in i64, so a position near the edge of the number range can't wrap round to the other side
        let span = self.width as i64 - 1;
        let (x0, z0) = (self.x as i64, self.z as i64);
        let (x1, z1) = (x0 + span * self.xa as i64, z0 + span * self.za as i64);
        // only the part inside this chunk is looked at
        let (cx, cz) = (chunk.x as i64 * 16, chunk.z as i64 * 16);
        let xs = x0.min(x1).max(cx)..=x0.max(x1).min(cx + 15);
        let zs = z0.min(z1).max(cz)..=z0.max(z1).min(cz + 15);
        let mut blocks = Vec::new();
        for x in xs {
            for z in zs.clone() {
                blocks.push((x as i32, self.y, z as i32));
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
    fn records_no_game_could_write_give_no_blocks_and_finish_at_once() {
        let started = std::time::Instant::now();
        for (x, z, width, xa, za) in [
            (0, 0, -128, -128, -128),
            (i32::MAX - 3, i32::MAX - 3, 23, 1, 1),
            (i32::MIN, i32::MIN, 23, -1, -1),
            (0, 0, 127, 1, 0),
            (0, 0, 5, 3, 0),
        ] {
            let p = portal(x, z, width, xa, za);
            for chunk in [ChunkPos::new(0, 0), ChunkPos::new(-1, -1), ChunkPos::new(x >> 4, z >> 4)] {
                assert!(p.horizontal_blocks(chunk).len() <= 23 * 23);
            }
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        // an invalid record gives nothing at all
        assert!(portal(0, 0, 127, 1, 0).horizontal_blocks(ChunkPos::new(0, 0)).is_empty());
        assert!(portal(0, 0, 3, 2, 0).horizontal_blocks(ChunkPos::new(0, 0)).is_empty());
    }

    #[test]
    fn negative_direction() {
        let p = portal(0, 0, 2, 0, -1);
        assert_eq!(p.horizontal_blocks(ChunkPos::new(0, -1)), [(0, 70, -1)]);
        assert_eq!(p.horizontal_blocks(ChunkPos::new(0, 0)), [(0, 70, 0)]);
    }
}
