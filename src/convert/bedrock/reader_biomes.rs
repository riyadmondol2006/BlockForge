// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Biomes and height maps of a column: `Data3D` (1.18+) and `Data2D` (before) as model [`Biomes`] and [`HeightMap`].
//!
//! Biome ids go through the world version's id table ([`IdTable`] `biome_id` of the Bedrock edition) to Chunker's
//! constants. An id the table does not know becomes the fallback biome of the dimension and is counted; ids of
//! 30000 and up are custom biomes named by the `BiomeIdsTable` record (1.21.110+).
//!
//! `Data3D` stores one palette per 16 blocks of height, lowest first; a palette whose header says 127 means "the
//! same as the section below", which Bedrock itself resolves when it reads. Chunker keeps such a palette empty and
//! only drops trailing empty ones, so a 127 in the middle of a column would read as "no biome"; here the section
//! below is copied instead, and trailing 127 sections are dropped (every consumer clamps to the last palette).

use std::collections::HashMap;
use std::sync::Mutex;

use crate::convert::bedrock::palette::{BiomePalette as RawBiomes, Data2D, Data3D};
use crate::convert::mapping::tables::{IdTable, names};
use crate::convert::mapping::{Edition, StateValue, Version};
use crate::convert::model::{Biome, BiomePalette, Biomes, HeightMap, Palette, VanillaBiome};

/// The first id Bedrock gives to custom biomes (`MINIMUM_CUSTOM_BIOME_ID`).
pub(super) const MINIMUM_CUSTOM_BIOME_ID: i32 = 30000;

/// Biome id resolution of one world.
pub(super) struct BiomeReader {
    table: IdTable,
    /// Custom biome ids from `BiomeIdsTable`.
    custom: HashMap<i32, String>,
    /// Ids met that no table knew, with their counts.
    unmapped: Mutex<HashMap<i32, u64>>,
}

impl BiomeReader {
    pub fn new(version: Version) -> BiomeReader {
        BiomeReader {
            table: IdTable::load(Edition::Bedrock, names::BIOME_ID, version),
            custom: HashMap::new(),
            unmapped: Mutex::default(),
        }
    }

    /// Register the custom biomes of a world (`BedrockBiomeIDResolver.loadCustom`).
    pub fn load_custom(&mut self, biomes: impl IntoIterator<Item = (i32, String)>) {
        for (id, name) in biomes {
            if id >= MINIMUM_CUSTOM_BIOME_ID {
                self.custom.insert(id, name);
            }
        }
    }

    /// The biome of an id, or `fallback` (and a count) when nothing knows it (`readBiome`).
    pub fn biome(&self, id: i32, fallback: &Biome) -> Biome {
        if id >= MINIMUM_CUSTOM_BIOME_ID {
            if let Some(name) = self.custom.get(&id) {
                return Biome::Custom(name.as_str().into());
            }
        } else if let Some(vanilla) = self.table.to_pivot(&StateValue::Int(id)).and_then(VanillaBiome::from_name) {
            return Biome::Vanilla(vanilla);
        }
        if let Ok(mut unmapped) = self.unmapped.lock() {
            *unmapped.entry(id).or_insert(0) += 1;
        }
        fallback.clone()
    }

    /// Ids without a mapping, with how often they were met.
    pub fn unmapped(&self) -> HashMap<i32, u64> {
        self.unmapped.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// One decoded biome storage as a model palette.
    fn palette(&self, storage: &RawBiomes, fallback: &Biome) -> BiomePalette {
        if storage.is_inherit() || storage.entries.is_empty() {
            return Palette::Empty;
        }
        let keys: Vec<Biome> = storage.entries.iter().map(|id| self.biome(*id, fallback)).collect();
        // Merge entries that resolve to the same biome and drop unused ones.
        let mut unique: Vec<Biome> = Vec::new();
        let mut remap: Vec<Option<u16>> = vec![None; keys.len()];
        let mut out = vec![0u16; crate::convert::bedrock::palette::STORAGE_SIZE];
        for (i, &entry) in storage.indices.iter().enumerate() {
            let entry = entry as usize;
            let key = match remap.get(entry).copied() {
                Some(Some(k)) => k,
                Some(None) => {
                    let biome = &keys[entry];
                    let k = match unique.iter().position(|b| b == biome) {
                        Some(k) => k,
                        None => {
                            unique.push(biome.clone());
                            unique.len() - 1
                        }
                    } as u16;
                    remap[entry] = Some(k);
                    k
                }
                // An index outside the palette: use the fallback.
                None => match unique.iter().position(|b| b == fallback) {
                    Some(k) => k as u16,
                    None => {
                        unique.push(fallback.clone());
                        (unique.len() - 1) as u16
                    }
                },
            };
            let (x, z, y) = (i >> 8, (i >> 4) & 15, i & 15);
            out[(y << 8) | (z << 4) | x] = key;
        }
        if unique.len() == 1 {
            return Palette::Single(unique.remove(0));
        }
        Palette::from_parts(unique, &out)
    }

    /// The biomes of a `Data3D` record: one 16 cube palette per section (`readBiomesExtended`).
    pub fn data3d_biomes(&self, data: &Data3D, fallback: &Biome) -> Biomes {
        let mut palettes: Vec<BiomePalette> = Vec::with_capacity(data.sections.len());
        let mut inherited: Vec<bool> = Vec::with_capacity(data.sections.len());
        for storage in &data.sections {
            if storage.is_inherit() {
                palettes.push(palettes.last().cloned().unwrap_or(Palette::Empty));
                inherited.push(true);
            } else {
                palettes.push(self.palette(storage, fallback));
                inherited.push(false);
            }
        }
        // Prune the trailing sections that only inherit, and trailing empty ones (Chunker's rule).
        while palettes.last().is_some_and(|p| p.is_empty()) || inherited.last() == Some(&true) {
            palettes.pop();
            inherited.pop();
        }
        Biomes::Palette(palettes)
    }

    /// The biomes of a `Data2D` record: 256 entries in file order (`readBiomesColumn`; Chunker stores the bytes as
    /// they come, the model's column layout being `z << 4 | x`).
    pub fn data2d_biomes(&self, data: &Data2D, fallback: &Biome) -> Biomes {
        Biomes::Column(data.biomes.iter().map(|id| self.biome(*id as i32, fallback)).collect())
    }
}

/// The height map of a `Data3D`/`Data2D` record: 256 values at `z * 16 + x`, as `[x][z]` (`readHeightMap`).
pub(super) fn height_map(heights: &[i16]) -> HeightMap {
    let mut map = Box::new([[0i16; 16]; 16]);
    for z in 0..16 {
        for x in 0..16 {
            map[x][z] = heights.get(z * 16 + x).copied().unwrap_or(0);
        }
    }
    HeightMap::Bedrock(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::bedrock::palette::{PaletteData, STORAGE_SIZE};

    fn reader() -> BiomeReader {
        BiomeReader::new(Version::new(1, 26, 50))
    }

    fn fallback() -> Biome {
        Biome::Vanilla(VanillaBiome::PLAINS)
    }

    #[test]
    fn ids_map_to_biomes() {
        let r = reader();
        assert_eq!(r.biome(0, &fallback()), Biome::Vanilla(VanillaBiome::OCEAN));
        assert_eq!(r.biome(1, &fallback()), Biome::Vanilla(VanillaBiome::PLAINS));
        assert_eq!(r.biome(8, &fallback()), Biome::Vanilla(VanillaBiome::NETHER_WASTES));
        assert_eq!(r.biome(9, &fallback()), Biome::Vanilla(VanillaBiome::THE_END));
        assert!(r.unmapped().is_empty());
    }

    #[test]
    fn unknown_ids_use_the_fallback_and_count() {
        let r = reader();
        let nether = Biome::Vanilla(VanillaBiome::NETHER_WASTES);
        assert_eq!(r.biome(9999, &nether), nether);
        assert_eq!(r.biome(9999, &nether), nether);
        assert_eq!(r.unmapped().get(&9999), Some(&2));
        // A custom id with no table entry is unmapped too.
        assert_eq!(r.biome(30001, &nether), nether);
    }

    #[test]
    fn custom_biomes_come_from_the_table() {
        let mut r = reader();
        r.load_custom([(30000, "mymod:glade".to_string()), (12, "ignored:below_30000".to_string())]);
        assert_eq!(r.biome(30000, &fallback()), Biome::Custom("mymod:glade".into()));
        assert_eq!(r.biome(12, &fallback()), Biome::Vanilla(VanillaBiome::SNOWY_PLAINS));
    }

    fn storage(entries: Vec<i32>, f: impl Fn(usize) -> u16) -> RawBiomes {
        PaletteData::from_indices(entries, (0..STORAGE_SIZE).map(f).collect()).unwrap()
    }

    #[test]
    fn height_map_is_x_then_z() {
        let heights: Vec<i16> = (0..256).collect();
        let HeightMap::Bedrock(m) = height_map(&heights) else { panic!() };
        // Index z * 16 + x.
        assert_eq!(m[3][5], 5 * 16 + 3);
        assert_eq!(m[15][0], 15);
        assert_eq!(m[0][15], 240);
        // Short input is padded with zeros.
        let HeightMap::Bedrock(m) = height_map(&[7]) else { panic!() };
        assert_eq!((m[0][0], m[1][0]), (7, 0));
    }

    #[test]
    fn data3d_sections_inherit_and_prune() {
        let r = reader();
        let plains = PaletteData::uniform(1);
        let desert = PaletteData::uniform(2);
        let data = Data3D {
            heights: vec![0; 256],
            sections: vec![
                plains,
                PaletteData::inherit(),
                desert,
                PaletteData::inherit(),
                PaletteData::inherit(),
            ],
        };
        let Biomes::Palette(p) = r.data3d_biomes(&data, &fallback()) else { panic!() };
        // Trailing inherit sections are dropped; the middle one copies the section below.
        assert_eq!(p.len(), 3);
        assert_eq!(p[1], p[0]);
        assert!(matches!(&p[2], Palette::Single(Biome::Vanilla(VanillaBiome::DESERT))));
    }

    #[test]
    fn leading_inherit_is_empty() {
        let r = reader();
        let data = Data3D { heights: vec![0; 256], sections: vec![PaletteData::inherit(), PaletteData::uniform(1)] };
        let Biomes::Palette(p) = r.data3d_biomes(&data, &fallback()) else { panic!() };
        assert!(p[0].is_empty());
        assert!(!p[1].is_empty());
    }

    #[test]
    fn mixed_section_is_transposed() {
        let r = reader();
        // storage index x*256 + z*16 + y: make x=2, z=3, y=4 the desert.
        let target = 2 * 256 + 3 * 16 + 4;
        let s = storage(vec![1, 2], |i| (i == target) as u16);
        let data = Data3D { heights: vec![0; 256], sections: vec![s] };
        let Biomes::Palette(p) = r.data3d_biomes(&data, &fallback()) else { panic!() };
        assert_eq!(p[0].get(2, 4, 3), Some(&Biome::Vanilla(VanillaBiome::DESERT)));
        assert_eq!(p[0].get(2, 3, 4), Some(&Biome::Vanilla(VanillaBiome::PLAINS)));
    }

    #[test]
    fn data2d_is_kept_in_file_order() {
        let r = reader();
        let mut biomes = vec![1u8; 256];
        biomes[17] = 2;
        let Biomes::Column(c) = r.data2d_biomes(&Data2D { heights: vec![0; 256], biomes }, &fallback()) else { panic!() };
        assert_eq!(c.len(), 256);
        assert_eq!(c[17], Biome::Vanilla(VanillaBiome::DESERT));
        assert_eq!(c[16], Biome::Vanilla(VanillaBiome::PLAINS));
    }
}
