// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The single-player data of a level (`ChunkerLevelPlayer`).

use std::collections::BTreeMap;

use super::dimension::Dimension;
use crate::convert::nbt::Compound;

/// An inventory item. Chunker uses `ChunkerItemStack` (the column model's item); until that lands the item is carried
/// as its NBT so nothing is lost. Swap the alias when the model's `ItemStack` exists.
pub type PlayerItem = Compound;

/// The player stored in level.dat (Java) or `~local_player` (Bedrock).
#[derive(Clone, Debug, PartialEq)]
pub struct LevelPlayer {
    pub dimension: Dimension,
    pub position: [f64; 3],
    pub motion: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// Items by slot, sorted by slot (Chunker's `Byte2ObjectAVLTreeMap`).
    pub inventory: BTreeMap<i8, PlayerItem>,
    pub game_type: i32,
}

impl LevelPlayer {
    /// A player at the origin of a dimension with no items.
    pub fn new(dimension: Dimension) -> LevelPlayer {
        LevelPlayer {
            dimension,
            position: [0.0; 3],
            motion: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            inventory: BTreeMap::new(),
            game_type: 0,
        }
    }

    pub fn position_x(&self) -> f64 {
        self.position[0]
    }
    pub fn position_y(&self) -> f64 {
        self.position[1]
    }
    pub fn position_z(&self) -> f64 {
        self.position[2]
    }
    pub fn motion_x(&self) -> f64 {
        self.motion[0]
    }
    pub fn motion_y(&self) -> f64 {
        self.motion[1]
    }
    pub fn motion_z(&self) -> f64 {
        self.motion[2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::nbt::Tag;

    #[test]
    fn inventory_is_sorted_by_slot() {
        let mut player = LevelPlayer::new(Dimension::nether());
        for slot in [5i8, -106, 100, 0] {
            let mut item = Compound::new();
            item.insert("Slot", Tag::Byte(slot));
            player.inventory.insert(slot, item);
        }
        assert_eq!(player.inventory.keys().copied().collect::<Vec<_>>(), [-106, 0, 5, 100]);
        player.position = [1.0, 64.5, -3.0];
        assert_eq!((player.position_x(), player.position_y(), player.position_z()), (1.0, 64.5, -3.0));
        assert_eq!(player.dimension, Dimension::nether());
    }
}
