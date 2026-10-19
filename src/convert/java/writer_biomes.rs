// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Model biomes to Java biomes: namespaced names (1.18+) or numeric ids (1.13 - 1.17), by the target version's tables
//! (`JavaNamedBiomeResolver`, `JavaBiomeIDResolver`).

use crate::convert::level::ConversionSettings;
use crate::convert::mapping::tables::names;
use crate::convert::mapping::{Edition, IdTable, Version as MapVersion};
use crate::convert::model::{Biome, VanillaBiome};
use std::sync::Arc;

/// Plains: the id used when even the dimension fallback has no id.
const PLAINS_ID: i32 = 1;

pub struct BiomeWriter {
    names: IdTable,
    ids: IdTable,
    biome_mapping: Option<std::collections::BTreeMap<String, String>>,
    custom_identifiers: bool,
}

impl BiomeWriter {
    pub fn new(version: crate::convert::util::Version, settings: &ConversionSettings) -> BiomeWriter {
        let v = MapVersion::new(version.major.max(0) as u32, version.minor.max(0) as u32, version.patch.max(0) as u32);
        BiomeWriter {
            names: IdTable::load(Edition::Java, names::BIOME_NAME, v),
            ids: IdTable::load(Edition::Java, names::BIOME_ID, v),
            biome_mapping: settings.biome_mapping.clone(),
            custom_identifiers: settings.custom_identifiers,
        }
    }

    /// Apply the user's biome mapping (`converter.getNewBiome`).
    fn remap(&self, biome: &Biome) -> Biome {
        let Some(mapping) = &self.biome_mapping else { return biome.clone() };
        let id = match biome {
            Biome::Vanilla(v) => v.java_identifier().map(str::to_string),
            Biome::Custom(id) => Some(id.to_string()),
        };
        match id.as_deref().and_then(|id| mapping.get(id)) {
            Some(new) => Biome::from_identifier(new),
            None => biome.clone(),
        }
    }

    fn vanilla_name(&self, mut biome: VanillaBiome) -> Option<&str> {
        // A removed biome is replaced by its fallback when the target does not know it.
        for _ in 0..6 {
            if let Some(name) = self.names.from_pivot_str(biome.name()) {
                return Some(name);
            }
            biome = biome.fallback()?;
        }
        None
    }

    fn vanilla_id(&self, mut biome: VanillaBiome) -> Option<i32> {
        for _ in 0..6 {
            if let Some(id) = self.ids.from_pivot_int(biome.name()) {
                return Some(id);
            }
            biome = biome.fallback()?;
        }
        None
    }

    /// The biome name for 1.18+; `fallback` (`minecraft:plains` and so on) when the version has no such biome.
    pub fn name(&self, biome: &Biome, fallback: &str) -> Arc<str> {
        let biome = self.remap(biome);
        match &biome {
            Biome::Vanilla(v) => {
                if let Some(name) = self.vanilla_name(*v) {
                    return Arc::from(name);
                }
            }
            Biome::Custom(id) => {
                if self.custom_identifiers && id.contains(':') && !id.starts_with("minecraft:") {
                    return id.clone();
                }
            }
        }
        Arc::from(fallback)
    }

    /// The numeric biome id for 1.13 - 1.17.
    pub fn id(&self, biome: &Biome, fallback: &str) -> i32 {
        let biome = self.remap(biome);
        if let Biome::Vanilla(v) = &biome {
            if let Some(id) = self.vanilla_id(*v) {
                return id;
            }
        }
        VanillaBiome::find(fallback).and_then(|v| self.vanilla_id(v)).unwrap_or(PLAINS_ID)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::util::Version;

    fn writer(v: Version) -> BiomeWriter {
        BiomeWriter::new(v, &ConversionSettings::default())
    }

    #[test]
    fn names_follow_the_version() {
        let w = writer(Version::new(1, 20, 4));
        let plains = Biome::Vanilla(VanillaBiome::PLAINS);
        assert_eq!(&*w.name(&plains, "minecraft:plains"), "minecraft:plains");
        let custom = Biome::from_identifier("mod:glade");
        assert_eq!(&*w.name(&custom, "minecraft:plains"), "mod:glade");
        // Cherry groves came with 1.20: an older table falls back.
        let cherry = Biome::Vanilla(VanillaBiome::CHERRY_GROVE);
        assert_eq!(&*writer(Version::new(1, 20, 4)).name(&cherry, "minecraft:plains"), "minecraft:cherry_grove");
        assert_eq!(&*writer(Version::new(1, 18, 2)).name(&cherry, "minecraft:plains"), "minecraft:plains");
    }

    #[test]
    fn ids_for_old_versions() {
        let w = writer(Version::new(1, 16, 5));
        assert_eq!(w.id(&Biome::Vanilla(VanillaBiome::PLAINS), "minecraft:plains"), 1);
        assert_eq!(w.id(&Biome::Vanilla(VanillaBiome::DESERT), "minecraft:plains"), 2);
        assert_eq!(w.id(&Biome::from_identifier("mod:x"), "minecraft:plains"), 1);
    }
}
