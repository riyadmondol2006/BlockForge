// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The Java on-disk format switches for a data version.
//!
//! Chunker expresses these as a 21-class inheritance chain (`v1_11` ... `v26_3`, each class overriding a few
//! methods of the previous one, see its `JavaEncoders`). Here one [`JavaFormat`] holds the result of walking that
//! chain, so every switch is a plain field. Thresholds are data versions from [`super::version`]; a world's data
//! version is first floored to a registered row, exactly like `JavaDataVersion.getNearestVersion`.
//!
//! The fields describe what a writer produces and what a reader should look for first. Readers also accept the
//! older spellings when the preferred key is missing (Chunker's 1.18 reader falls back to `Palette`, `Biomes`
//! arrays and `TileEntities`); that is up to the reading code.

use super::palette::Layout;
use super::version::*;

/// Where a section keeps its block states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionLayout {
    /// Before 1.13: `Blocks` byte[4096], `Add` and `Data` nibble arrays; numeric block ids.
    LegacyArrays,
    /// 1.13 - 1.17: `Palette` list and `BlockStates` long array in the section itself.
    FlatPalette,
    /// 1.18+: `block_states: {palette, data}` and `biomes: {palette, data}` in the section.
    NestedStates,
}

/// How a column stores its biomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiomeKind {
    /// Before 1.13: `Biomes` byte[256] (one per column).
    ByteArray256,
    /// 1.13 - 1.14: `Biomes` int[256] (one per column).
    IntArray256,
    /// 1.15 - 1.17: `Biomes` int[1024] (4x4x4 cells over the 256-high world).
    IntArray1024,
    /// 1.18+: a string palette per section, `biomes: {palette: [str], data}` (4x4x4 cells, at least 1 bit).
    SectionPalette,
}

/// Which column flag says the lighting is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightFlag {
    /// Before 1.14: `LightPopulated`.
    LightPopulated,
    /// 1.14+: `isLightOn`.
    IsLightOn,
}

/// How item stacks are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemFormat {
    /// Before 1.13: numeric or string `id`, `Damage` short, `Count` byte, `tag` compound.
    Legacy,
    /// 1.13 - 1.20.4: string `id`, `Count` byte, `tag` compound (`Damage` inside it).
    Tag,
    /// 1.20.5+: `id`, `count` int and a `components` compound.
    Components,
}

/// How a feature pack is switched on in `level.dat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureState {
    /// The version does not know the feature.
    Absent,
    /// The feature is an experiment (a data pack in `DataPacks.Enabled` and `enabled_features`).
    Experimental,
    /// Part of the game.
    BuiltIn,
}

/// Caves & Cliffs height (world from -64): a data pack in 1.17, built in from 1.18.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CavesAndCliffs {
    Absent,
    Datapack,
    BuiltIn,
}

/// All format switches of one data version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaFormat {
    /// The data version as given.
    pub data_version: i32,
    /// The registered row it floors to (the version the format is chosen by).
    pub row_data_version: i32,
    /// The game version of that row.
    pub version: Version,

    // ---- chunks ----
    /// Numeric block ids and `Blocks`/`Data`/`Add` arrays (before 1.13).
    pub legacy_ids: bool,
    /// Block-state storage in a section.
    pub sections: SectionLayout,
    /// The chunk root wraps everything in a `Level` compound (before 1.18).
    pub level_wrapper: bool,
    /// `Sections` (before 1.18) or `sections`.
    pub sections_key: &'static str,
    /// `TileEntities` (before 1.18) or `block_entities`.
    pub block_entities_key: &'static str,
    /// The palette key list of a section: `Palette` (1.13 - 1.17) or `palette` inside `block_states`.
    pub palette_key: &'static str,
    /// The packed index array: `BlockStates` (1.13 - 1.17) or `data` inside `block_states`.
    pub palette_data_key: &'static str,
    /// Packing a writer uses: spanning before 1.16, padded from 1.16. Readers detect it from the array length.
    pub palette_layout: Layout,
    /// Smallest bits per entry of a block palette (4) and of a biome palette (1).
    pub min_bits_blocks: u32,
    pub min_bits_biomes: u32,
    /// Biome storage.
    pub biomes: BiomeKind,
    /// Biomes are identified by namespaced name (1.18+) instead of numeric id.
    pub biome_names: bool,
    /// The `HeightMap` int[256] is read and written (before 1.13); later versions recompute it.
    pub legacy_heightmap: bool,
    /// Which light flag the column carries.
    pub light_flag: LightFlag,
    /// The `Status` a writer puts on a column: none before 1.13, `decorated` in 1.13, `full` from 1.14.
    pub chunk_status: Option<&'static str>,
    /// A writer adds `yPos` (the lowest section y) to columns (1.18+).
    pub writes_y_pos: bool,
    /// Sections outside y 0..15 are valid (1.17+).
    pub extended_height: bool,
    /// Entities live in `entities/r.X.Z.mca` instead of the chunk (1.17+).
    pub entities_separate: bool,
    /// Entity ids are `minecraft:name` (1.11+) instead of CamelCase.
    pub modern_entity_ids: bool,
    /// Palette keys are `{id, properties}` instead of `{Name, Properties}` (26.3+); a bare string is also read.
    pub renamed_block_state_fields: bool,
    /// Beds are block entities (until 26.2 removed them).
    pub bed_block_entity: bool,

    // ---- items ----
    pub items: ItemFormat,
    /// Equipment is split from the inventory into an `equipment` compound with slots -106, 100..103 (1.21.5+).
    pub split_equipment: bool,

    // ---- level.dat ----
    /// `WorldGenSettings` holds the seed and generators (1.16+) instead of `RandomSeed`/`generatorName`.
    pub world_gen_settings: bool,
    /// Flat generator options are a compound (1.13 - 1.15) instead of a string (before 1.13).
    pub generator_options_compound: bool,
    /// The player and map `Dimension` is a string id (1.16+) instead of an int.
    pub dimension_ids_as_strings: bool,
    /// A writer must add an empty `DragonFight` compound (1.20+, the client needs it).
    pub dragon_fight_compound: bool,
    pub caves_and_cliffs: CavesAndCliffs,
    /// 1.20 content (`update_1_20`): experimental in 1.19.3 - 1.19.4, built in from 1.20.
    pub r20_support: FeatureState,
    /// 1.21 content (`update_1_21`): experimental in 1.20.3 - 1.20.6, built in from 1.21.
    pub r21_support: FeatureState,
    /// The winter 2024 drop (`winter_drop`): experimental in 1.21.2 - 1.21.3, built in from 1.21.4.
    pub winter_drop_2024: FeatureState,
    /// Later drops are built in from the version that introduced them and absent before.
    pub summer_drop_2025: FeatureState,
    pub autumn_drop_2025: FeatureState,
    pub summer_drop_2026: FeatureState,
    pub autumn_drop_2026: FeatureState,
    /// Spawn is a `spawn` compound `{pos, pitch, yaw, dimension}` and the border a `world_border` compound (1.21.9+).
    pub spawn_compound: bool,
    /// Game rules use registry keys such as `minecraft:advance_time` with typed values (1.21.11+).
    pub game_rule_registry_keys: bool,
    /// `level.dat` is split into `data/minecraft/*.dat` files and the player into `players/data/<uuid>.dat` (26.1+).
    pub split_level_dat: bool,
    /// Every dimension, the overworld included, lives in `dimensions/<namespace>/<key>/` (26.1+).
    pub namespaced_dimension_dirs: bool,
    /// Maps are `data/minecraft/maps/<id>.dat` with `last_id.dat` (26.1+) instead of `data/map_<id>.dat` with
    /// `idcounts.dat`.
    pub maps_in_registry_dir: bool,
}

impl JavaFormat {
    /// The format for a world's data version (floored to a registered row; unknown newer versions use the latest).
    pub fn for_data_version(data_version: i32) -> JavaFormat {
        let (row, version) = nearest_at_or_below(data_version);
        let at = |t: i32| row >= t;
        let feature = |experimental: Option<(i32, i32)>, built_in: i32| {
            if row >= built_in {
                FeatureState::BuiltIn
            } else if experimental.is_some_and(|(from, _)| row >= from) {
                FeatureState::Experimental
            } else {
                FeatureState::Absent
            }
        };
        JavaFormat {
            data_version,
            row_data_version: row,
            version,

            legacy_ids: !at(DV_1_13),
            sections: if !at(DV_1_13) {
                SectionLayout::LegacyArrays
            } else if !at(DV_1_18) {
                SectionLayout::FlatPalette
            } else {
                SectionLayout::NestedStates
            },
            level_wrapper: !at(DV_1_18),
            sections_key: if at(DV_1_18) { "sections" } else { "Sections" },
            block_entities_key: if at(DV_1_18) { "block_entities" } else { "TileEntities" },
            palette_key: if at(DV_1_18) { "palette" } else { "Palette" },
            palette_data_key: if at(DV_1_18) { "data" } else { "BlockStates" },
            palette_layout: if at(DV_1_16) { Layout::Padded } else { Layout::Spanning },
            min_bits_blocks: 4,
            min_bits_biomes: 1,
            biomes: if at(DV_1_18) {
                BiomeKind::SectionPalette
            } else if at(DV_1_15) {
                BiomeKind::IntArray1024
            } else if at(DV_1_13) {
                BiomeKind::IntArray256
            } else {
                BiomeKind::ByteArray256
            },
            biome_names: at(DV_1_18),
            legacy_heightmap: !at(DV_1_13),
            light_flag: if at(DV_1_14) { LightFlag::IsLightOn } else { LightFlag::LightPopulated },
            chunk_status: if at(DV_1_14) {
                Some("full")
            } else if at(DV_1_13) {
                Some("decorated")
            } else {
                None
            },
            writes_y_pos: at(DV_1_18),
            extended_height: at(DV_1_17),
            entities_separate: at(DV_1_17),
            modern_entity_ids: at(DV_1_11),
            renamed_block_state_fields: at(DV_26_3),
            bed_block_entity: !at(DV_26_2),

            items: if at(DV_1_20_5) {
                ItemFormat::Components
            } else if at(DV_1_13) {
                ItemFormat::Tag
            } else {
                ItemFormat::Legacy
            },
            split_equipment: at(DV_1_21_5),

            world_gen_settings: at(DV_1_16),
            generator_options_compound: at(DV_1_13) && !at(DV_1_16),
            dimension_ids_as_strings: at(DV_1_16),
            dragon_fight_compound: at(DV_1_20),
            caves_and_cliffs: if at(DV_1_18) {
                CavesAndCliffs::BuiltIn
            } else if at(DV_1_17) {
                CavesAndCliffs::Datapack
            } else {
                CavesAndCliffs::Absent
            },
            r20_support: feature(Some((DV_1_19_3, DV_1_20)), DV_1_20),
            r21_support: feature(Some((DV_1_20_3, DV_1_21)), DV_1_21),
            winter_drop_2024: feature(Some((DV_1_21_2, DV_1_21_4)), DV_1_21_4),
            summer_drop_2025: feature(None, DV_1_21_6),
            autumn_drop_2025: feature(None, DV_1_21_9),
            summer_drop_2026: feature(None, DV_26_2),
            autumn_drop_2026: feature(None, DV_26_3),
            spawn_compound: at(DV_1_21_9),
            game_rule_registry_keys: at(DV_1_21_11),
            split_level_dat: at(DV_26_1),
            namespaced_dimension_dirs: at(DV_26_1),
            maps_in_registry_dir: at(DV_26_1),
        }
    }

    /// The format for a game version (rounded down to a registered release).
    pub fn for_version(version: Version) -> JavaFormat {
        JavaFormat::for_data_version(nearest_for_version(version).0)
    }

    /// The latest registered format.
    pub fn latest() -> JavaFormat {
        JavaFormat::for_data_version(latest().0)
    }

    /// Spanning palette arrays (1.13 - 1.15) rather than padded (1.16+).
    pub fn palette_spanning(&self) -> bool {
        self.palette_layout == Layout::Spanning
    }

    /// Light is `isLightOn` rather than `LightPopulated`.
    pub fn light_is_light_on(&self) -> bool {
        self.light_flag == LightFlag::IsLightOn
    }

    /// Items carry data components.
    pub fn item_components(&self) -> bool {
        self.items == ItemFormat::Components
    }

    /// The `Version.Name` a writer puts in `level.dat`.
    pub fn version_name(&self) -> String {
        self.version.to_string()
    }

    /// Heightmap entries are packed for the padded layout (1.16+); before that they span longs.
    pub fn heightmap_layout(&self) -> Layout {
        self.palette_layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An independent model of Chunker's override chain: each entry is a version that registers an encoder and the
    /// switches its class overrides (read from `JavaEncoders` and the `v*` packages). Walking the chain up to the
    /// nearest registered encoder gives the expected switches, with no thresholds shared with `for_data_version`.
    #[derive(Clone, Default, Debug)]
    struct Expect {
        legacy_ids: bool,
        entity_ids_modern: bool,
        sections: u8, // 0 legacy arrays, 1 flat palette, 2 nested
        spanning: bool,
        padded: bool,
        biomes: u8, // 0 byte256, 1 int256, 2 int1024, 3 palette
        heightmap: bool,
        light_on: bool,
        status: &'static str,
        y_pos: bool,
        ext_height: bool,
        entities_sep: bool,
        level_wrapper: bool,
        world_gen: bool,
        gen_options_compound: bool,
        dimension_strings: bool,
        dragon_fight: bool,
        c_and_c: u8,
        r20: u8,
        r21: u8,
        winter: u8,
        summer25: u8,
        autumn25: u8,
        summer26: u8,
        autumn26: u8,
        spawn_compound: bool,
        rules: bool,
        split: bool,
        components: bool,
        tag_items: bool,
        split_equipment: bool,
        renamed: bool,
        beds: bool,
    }

    fn chain() -> Vec<(i32, fn(&mut Expect))> {
        vec![
            (0, |e| {
                // Base: Anvil 1.8.8.
                *e = Expect { legacy_ids: true, level_wrapper: true, heightmap: true, beds: true, ..Expect::default() };
            }),
            (819, |e| e.entity_ids_modern = true),
            (1519, |e| {
                e.legacy_ids = false;
                e.sections = 1;
                e.spanning = true;
                e.biomes = 1;
                e.heightmap = false;
                e.status = "decorated";
                e.gen_options_compound = true;
                e.tag_items = true;
            }),
            (1952, |e| {
                e.light_on = true;
                e.status = "full";
            }),
            (2225, |e| e.biomes = 2),
            (2566, |e| {
                e.spanning = false;
                e.padded = true;
                e.world_gen = true;
                e.dimension_strings = true;
                e.gen_options_compound = false;
            }),
            (2724, |e| {
                e.ext_height = true;
                e.entities_sep = true;
                e.c_and_c = 1;
            }),
            (2860, |e| {
                e.sections = 2;
                e.biomes = 3;
                e.y_pos = true;
                e.level_wrapper = false;
                e.c_and_c = 2;
            }),
            (3218, |e| e.r20 = 1),
            (3463, |e| {
                e.r20 = 2;
                e.dragon_fight = true;
            }),
            (3698, |e| e.r21 = 1),
            (3837, |e| {
                e.components = true;
                e.tag_items = false;
            }),
            (3953, |e| e.r21 = 2),
            (4080, |e| e.winter = 1),
            (4189, |e| e.winter = 2),
            (4435, |e| e.summer25 = 2),
            (4554, |e| {
                e.spawn_compound = true;
                e.autumn25 = 2;
            }),
            (4671, |e| e.rules = true),
            (4786, |e| e.split = true),
            (4903, |e| {
                e.summer26 = 2;
                e.beds = false;
            }),
            (5017, |e| {
                e.autumn26 = 2;
                e.renamed = true;
            }),
        ]
    }

    // Equipment is split from 1.21.5 (4325): `JavaLevelWriter` checks `isGreaterThanOrEqual(1, 21, 5)`, not a class.
    fn expected(dv: i32) -> Expect {
        let (row, _) = nearest_at_or_below(dv);
        let mut e = Expect::default();
        for (at, apply) in chain() {
            if at <= row {
                apply(&mut e);
            }
        }
        e.split_equipment = row >= 4325;
        e
    }

    fn feat(s: FeatureState) -> u8 {
        match s {
            FeatureState::Absent => 0,
            FeatureState::Experimental => 1,
            FeatureState::BuiltIn => 2,
        }
    }

    #[test]
    fn every_data_version_matches_the_override_chain() {
        for &(dv, _) in DATA_VERSIONS {
            for probe in [dv, dv + 1, dv.max(1) - 1] {
                let f = JavaFormat::for_data_version(probe);
                let e = expected(probe);
                let ctx = format!("data version {probe} (row {})", f.row_data_version);
                assert_eq!(f.legacy_ids, e.legacy_ids, "{ctx}: legacy_ids");
                assert_eq!(f.modern_entity_ids, e.entity_ids_modern, "{ctx}: entity ids");
                assert_eq!(
                    f.sections,
                    [SectionLayout::LegacyArrays, SectionLayout::FlatPalette, SectionLayout::NestedStates]
                        [e.sections as usize],
                    "{ctx}: sections"
                );
                if e.sections > 0 {
                    assert_eq!(f.palette_spanning(), e.spanning, "{ctx}: spanning");
                    assert_eq!(f.palette_layout == Layout::Padded, e.padded, "{ctx}: padded");
                }
                assert_eq!(
                    f.biomes,
                    [
                        BiomeKind::ByteArray256,
                        BiomeKind::IntArray256,
                        BiomeKind::IntArray1024,
                        BiomeKind::SectionPalette
                    ][e.biomes as usize],
                    "{ctx}: biomes"
                );
                assert_eq!(f.biome_names, e.biomes == 3, "{ctx}: biome names");
                assert_eq!(f.legacy_heightmap, e.heightmap, "{ctx}: heightmap");
                assert_eq!(f.light_is_light_on(), e.light_on, "{ctx}: light flag");
                assert_eq!(f.chunk_status.unwrap_or(""), e.status, "{ctx}: status");
                assert_eq!(f.writes_y_pos, e.y_pos, "{ctx}: yPos");
                assert_eq!(f.extended_height, e.ext_height, "{ctx}: extended height");
                assert_eq!(f.entities_separate, e.entities_sep, "{ctx}: entities");
                assert_eq!(f.level_wrapper, e.level_wrapper, "{ctx}: Level wrapper");
                assert_eq!(f.world_gen_settings, e.world_gen, "{ctx}: WorldGenSettings");
                assert_eq!(f.generator_options_compound, e.gen_options_compound, "{ctx}: generatorOptions");
                assert_eq!(f.dimension_ids_as_strings, e.dimension_strings, "{ctx}: dimension ids");
                assert_eq!(f.dragon_fight_compound, e.dragon_fight, "{ctx}: DragonFight");
                assert_eq!(
                    f.caves_and_cliffs,
                    [CavesAndCliffs::Absent, CavesAndCliffs::Datapack, CavesAndCliffs::BuiltIn][e.c_and_c as usize],
                    "{ctx}: caves and cliffs"
                );
                assert_eq!(feat(f.r20_support), e.r20, "{ctx}: r20");
                assert_eq!(feat(f.r21_support), e.r21, "{ctx}: r21");
                assert_eq!(feat(f.winter_drop_2024), e.winter, "{ctx}: winter drop");
                assert_eq!(feat(f.summer_drop_2025), e.summer25, "{ctx}: summer 2025");
                assert_eq!(feat(f.autumn_drop_2025), e.autumn25, "{ctx}: autumn 2025");
                assert_eq!(feat(f.summer_drop_2026), e.summer26, "{ctx}: summer 2026");
                assert_eq!(feat(f.autumn_drop_2026), e.autumn26, "{ctx}: autumn 2026");
                assert_eq!(f.spawn_compound, e.spawn_compound, "{ctx}: spawn compound");
                assert_eq!(f.game_rule_registry_keys, e.rules, "{ctx}: game rules");
                assert_eq!(f.split_level_dat, e.split, "{ctx}: split level.dat");
                assert_eq!(f.namespaced_dimension_dirs, e.split, "{ctx}: dimension dirs");
                assert_eq!(f.maps_in_registry_dir, e.split, "{ctx}: maps dir");
                assert_eq!(f.item_components(), e.components, "{ctx}: components");
                assert_eq!(f.items == ItemFormat::Tag, e.tag_items, "{ctx}: tag items");
                assert_eq!(f.items == ItemFormat::Legacy, e.legacy_ids, "{ctx}: legacy items");
                assert_eq!(f.split_equipment, e.split_equipment, "{ctx}: equipment");
                assert_eq!(f.renamed_block_state_fields, e.renamed, "{ctx}: palette field names");
                assert_eq!(f.bed_block_entity, e.beds, "{ctx}: bed block entity");
            }
        }
    }

    #[test]
    fn derived_keys_follow_the_layout() {
        let old = JavaFormat::for_data_version(1343);
        assert_eq!((old.sections_key, old.block_entities_key), ("Sections", "TileEntities"));
        assert!(old.level_wrapper && old.legacy_ids && old.legacy_heightmap);
        let mid = JavaFormat::for_data_version(2586);
        assert_eq!((mid.palette_key, mid.palette_data_key), ("Palette", "BlockStates"));
        assert!(!mid.palette_spanning() && mid.level_wrapper && !mid.entities_separate);
        let new = JavaFormat::for_data_version(3465);
        assert_eq!(
            (new.sections_key, new.block_entities_key, new.palette_key, new.palette_data_key),
            ("sections", "block_entities", "palette", "data")
        );
        assert!(new.writes_y_pos && new.extended_height && !new.level_wrapper);
        assert_eq!((new.min_bits_blocks, new.min_bits_biomes), (4, 1));
        assert_eq!(new.version, Version::new(1, 20, 1));
    }

    #[test]
    fn spot_checks_from_the_survey() {
        // 1.13 flattening, 1.14 isLightOn, 1.15 biome cells, 1.16 non-spanning, 1.17 entities, 1.18 sections.
        assert!(JavaFormat::for_data_version(1518).legacy_ids && !JavaFormat::for_data_version(1519).legacy_ids);
        assert!(
            !JavaFormat::for_data_version(1951).light_is_light_on()
                && JavaFormat::for_data_version(1952).light_is_light_on()
        );
        assert_eq!(JavaFormat::for_data_version(2224).biomes, BiomeKind::IntArray256);
        assert_eq!(JavaFormat::for_data_version(2225).biomes, BiomeKind::IntArray1024);
        assert!(
            JavaFormat::for_data_version(2565).palette_spanning()
                && !JavaFormat::for_data_version(2566).palette_spanning()
        );
        assert!(
            !JavaFormat::for_data_version(2723).entities_separate
                && JavaFormat::for_data_version(2724).entities_separate
        );
        assert_eq!(JavaFormat::for_data_version(2859).sections, SectionLayout::FlatPalette);
        assert_eq!(JavaFormat::for_data_version(2860).sections, SectionLayout::NestedStates);
        // Nothing newer than the table is unknown: it uses the latest format.
        assert_eq!(JavaFormat::for_data_version(5023).row_data_version, 5017);
        assert!(JavaFormat::for_data_version(5023).split_level_dat);
        assert_eq!(JavaFormat::latest().version, Version::new(26, 3, 0));
        // Older than anything: the 1.8.8 base.
        assert!(JavaFormat::for_data_version(-5).legacy_ids);
        assert_eq!(JavaFormat::for_version(Version::new(1, 12, 2)).row_data_version, 1343);
        assert_eq!(JavaFormat::for_version(Version::new(1, 20, 4)).row_data_version, 3700);
        assert_eq!(JavaFormat::for_data_version(3465).version_name(), "1.20.1");
    }
}
