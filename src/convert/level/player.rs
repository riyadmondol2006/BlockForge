// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The single-player data of a level (`ChunkerLevelPlayer`).

use std::collections::BTreeMap;

use super::dimension::Dimension;
use crate::convert::model::EffectType;
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
    /// Ender chest items by slot (0 to 26).
    pub ender_items: BTreeMap<i8, PlayerItem>,
    /// Experience, health, food, the spawn point, effects and abilities.
    pub stats: PlayerStats,
}

/// What a player has besides items and a place (all of it left at the default when the conversion drops it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerStats {
    pub xp_level: i32,
    /// How far into the next level, 0 to 1.
    pub xp_progress: f32,
    /// `None` keeps the full health a new player has (a stored 0 or less is a dead player and is not kept either).
    pub health: Option<f32>,
    pub food_level: Option<i32>,
    pub saturation: Option<f32>,
    pub exhaustion: Option<f32>,
    /// The bed or respawn anchor the player respawns at.
    pub spawn: Option<SpawnPoint>,
    pub effects: Vec<PlayerEffect>,
    pub abilities: Option<PlayerAbilities>,
}

impl PlayerStats {
    /// The total experience points of the level and progress (`XpTotal`), as the game counts them.
    pub fn xp_total(&self) -> i32 {
        let level = i64::from(self.xp_level.max(0));
        let points = if level <= 16 {
            level * level + 6 * level
        } else if level <= 31 {
            (5 * level * level - 81 * level + 720) / 2
        } else {
            (9 * level * level - 325 * level + 4440) / 2
        };
        let into_next = (f64::from(self.xp_progress.clamp(0.0, 1.0)) * xp_to_next_level(level) as f64) as i64;
        (points + into_next).min(i64::from(i32::MAX)) as i32
    }
}

/// The experience points from `level` to the next one.
fn xp_to_next_level(level: i64) -> i64 {
    if level >= 30 {
        112 + (level - 30) * 9
    } else if level >= 15 {
        37 + (level - 15) * 5
    } else {
        7 + level * 2
    }
}

/// A respawn point: the block of the bed or anchor.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnPoint {
    pub dimension: Dimension,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// A status effect on the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerEffect {
    pub effect: EffectType,
    pub amplifier: i8,
    /// Ticks left.
    pub duration: i32,
    pub ambient: bool,
    pub show_particles: bool,
}

/// What the player may do (`abilities`); the game mode sets most of these again when the player joins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerAbilities {
    pub flying: bool,
    pub may_fly: bool,
    pub instabuild: bool,
    pub invulnerable: bool,
    pub may_build: bool,
    pub walk_speed: f32,
    pub fly_speed: f32,
}

impl Default for PlayerAbilities {
    fn default() -> Self {
        PlayerAbilities {
            flying: false,
            may_fly: false,
            instabuild: false,
            invulnerable: false,
            may_build: true,
            walk_speed: 0.1,
            fly_speed: 0.05,
        }
    }
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
            ender_items: BTreeMap::new(),
            stats: PlayerStats::default(),
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

    #[test]
    fn experience_total_follows_the_game() {
        let at = |xp_level, xp_progress| PlayerStats { xp_level, xp_progress, ..PlayerStats::default() }.xp_total();
        assert_eq!(at(0, 0.0), 0);
        assert_eq!(at(16, 0.0), 352);
        assert_eq!(at(17, 0.0), 394);
        assert_eq!(at(31, 0.0), 1507);
        assert_eq!(at(32, 0.0), 1628);
        // Half of the 7 points from level 0 to 1, and of the 9 from 1 to 2.
        assert_eq!(at(0, 0.5), 3);
        assert_eq!(at(1, 0.5), 11);
        assert_eq!(at(164, 0.0), 96_602);
    }
}
