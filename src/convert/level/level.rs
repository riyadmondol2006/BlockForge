// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The version-neutral level (`ChunkerLevel`): settings, the player, maps and portals.

use super::dimension::Dimension;
use super::level_settings::LevelSettings;
use super::map::LevelMap;
use super::player::LevelPlayer;
use super::portal::Portal;
use crate::convert::nbt::Compound;

/// Everything about a world except its columns.
#[derive(Clone, Debug, Default)]
pub struct Level {
    pub settings: LevelSettings,
    pub player: Option<LevelPlayer>,
    pub maps: Vec<LevelMap>,
    /// The level.dat of the input, kept when copying NBT between the same edition.
    pub original_level_data: Option<Compound>,
    pub portals: Vec<Portal>,
}

impl Level {
    /// The index of the map with the given id in the input world.
    pub fn find_map_index_by_original_id(&self, original_id: i64) -> Option<usize> {
        self.maps.iter().position(|m| m.original_id == Some(original_id))
    }

    pub fn map_by_index(&self, index: usize) -> Option<&LevelMap> {
        self.maps.get(index)
    }

    /// Maps the dimension of every map and portal through `new_dimension`; those it returns `None` for are dropped
    /// (the level handler's dimension remapping).
    pub fn remap_dimensions(&mut self, new_dimension: impl Fn(&Dimension) -> Option<Dimension>) {
        self.maps.retain_mut(|map| match new_dimension(&map.dimension) {
            Some(dimension) => {
                map.dimension = dimension;
                true
            }
            None => false,
        });
        self.portals.retain_mut(|portal| match new_dimension(&portal.dimension) {
            Some(dimension) => {
                portal.dimension = dimension;
                true
            }
            None => false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal(dim: Dimension) -> Portal {
        Portal { dimension: dim, x: 0, y: 0, z: 0, width: 2, xa: 1, za: 0 }
    }

    #[test]
    fn map_lookup() {
        let mut level = Level::default();
        let mut a = LevelMap::new(0, Dimension::overworld());
        a.original_id = Some(7);
        level.maps.push(a);
        level.maps.push(LevelMap::new(1, Dimension::overworld()));
        assert_eq!(level.find_map_index_by_original_id(7), Some(0));
        assert_eq!(level.find_map_index_by_original_id(1), None);
        assert_eq!(level.map_by_index(1).map(|m| m.id), Some(1));
        assert!(level.map_by_index(2).is_none());
    }

    #[test]
    fn remap_drops_unmapped_dimensions() {
        let mut level = Level::default();
        level.maps.push(LevelMap::new(0, Dimension::overworld()));
        level.maps.push(LevelMap::new(1, Dimension::nether()));
        level.portals.push(portal(Dimension::nether()));
        level.portals.push(portal(Dimension::the_end()));
        level.remap_dimensions(|d| {
            if *d == Dimension::the_end() {
                None
            } else if *d == Dimension::nether() {
                Some(Dimension::overworld())
            } else {
                Some(d.clone())
            }
        });
        assert!(level.maps.iter().all(|m| m.dimension == Dimension::overworld()));
        assert_eq!(level.maps.len(), 2);
        assert_eq!(level.portals.len(), 1);
        assert_eq!(level.portals[0].dimension, Dimension::overworld());
    }

    #[test]
    fn default_level_has_chunker_defaults() {
        let level = Level::default();
        assert_eq!(level.settings.level_name, "Converted World");
        assert!(level.player.is_none());
    }
}
