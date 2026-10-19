// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Reading the Bedrock level.dat and the level-wide records of the database into the level model.
//!
//! The level settings are read by the generic [`LevelSettings::from_nbt`]; this module is the Bedrock side of its
//! hook ([`SettingsIo`]): the settings that are not a plain `level.dat` value (the generator, the seed, the flat
//! world version, the permissions level and the "experiment" flags that depend on the version) are decided here the
//! way the stack of Chunker's `LevelReader` classes does. It also parses `~local_player`, the `map_*` records, the
//! `portals` record and the `BiomeIdsTable` of custom biomes (1.21.110+).

use std::sync::Mutex;

use super::data_version::{BedrockVersion, GameVersion};
use crate::convert::level::level_settings::{GeneratorType, SettingKind, SettingValue, SettingsIo};
use crate::convert::level::{Dimension, DimensionRegistry, EncodingType, LevelMap, LevelPlayer, LevelSettings, Portal};
use crate::convert::nbt::{Compound, Tag};

/// The height of the player's eyes that Bedrock adds to the stored y (height + 0.00001 as saved).
pub const PLAYER_HEIGHT: f32 = 1.62001;

/// How a version-dependent flag of the settings is read: from an experiment toggle in level.dat once the version
/// has it, and as "on" for ever once the feature is part of the release.
struct Experiment {
    /// The Chunker field name.
    name: &'static str,
    /// The experiment key in level.dat's `experiments` compound.
    key: Option<&'static str>,
    /// The first version whose reader looks at the experiment.
    since: GameVersion,
    /// The first version whose reader answers `true` without looking.
    always_since: Option<GameVersion>,
}

/// One entry per `readCustomLevelSetting` override in `bedrock/v1_*/reader/LevelReader.java`.
const EXPERIMENTS: [Experiment; 9] = [
    Experiment {
        name: "CavesAndCliffs",
        key: Some("caves_and_cliffs"),
        since: [1, 17, 0],
        always_since: Some([1, 18, 0]),
    },
    Experiment {
        name: "R20Support",
        key: Some("next_major_update"),
        since: [1, 19, 0],
        always_since: Some([1, 20, 0]),
    },
    Experiment {
        name: "R21Support",
        key: Some("updateAnnouncedLive2023"),
        since: [1, 20, 50],
        always_since: Some([1, 21, 0]),
    },
    Experiment { name: "WinterDrop2024", key: None, since: [1, 21, 50], always_since: Some([1, 21, 50]) },
    Experiment {
        name: "SummerDrop2025",
        key: Some("y_2025_drop_2"),
        since: [1, 21, 80],
        always_since: Some([1, 21, 90]),
    },
    Experiment {
        name: "AutumnDrop2025",
        key: Some("y_2025_drop_3"),
        since: [1, 21, 100],
        always_since: Some([1, 21, 110]),
    },
    Experiment {
        name: "SummerDrop2026",
        key: Some("y_2026_drop_2"),
        since: [1, 26, 20],
        always_since: Some([1, 26, 30]),
    },
    Experiment {
        name: "AutumnDrop2026",
        key: Some("y_2026_drop_3"),
        since: [1, 26, 40],
        always_since: Some([1, 26, 50]),
    },
    Experiment { name: "WinterDrop2026", key: Some("y_2026_drop_4"), since: [1, 26, 60], always_since: None },
];

/// The `FlatWorldLayers` value Chunker's own void worlds use.
pub const VOID_WORLD_STRING: &str = "{ \"biome_id\" : 1, \"block_layers\" : [{\"block_data\" : 0, \"block_name\" : \"minecraft:air\", \"count\" : 1 }], \"encoding_version\" : 4,  \"structure_options\" : null}";

/// The Bedrock side of [`LevelSettings::from_nbt`].
pub struct BedrockSettingsIo<'a> {
    /// The reader version (the encoder root decides which experiments exist).
    pub version: &'static BedrockVersion,
    /// The `StorageVersion` of the level (8 means the seed is an int).
    pub storage_version: i32,
    /// Problems met while reading, for the caller to report.
    pub warnings: &'a Mutex<Vec<String>>,
}

impl BedrockSettingsIo<'_> {
    fn flag(&self, root: &Compound, exp: &Experiment) -> bool {
        let at = self.version.encoder_root();
        if exp.always_since.is_some_and(|a| at >= a) {
            return true;
        }
        let Some(key) = exp.key.filter(|_| at >= exp.since) else { return false };
        root.get_compound("experiments").and_then(|e| e.get_integer(key)).is_some_and(|v| v == 1)
    }

    fn generator(&self, root: &Compound) -> GeneratorType {
        match root.get_i32_or("Generator", 1) {
            0 => GeneratorType::Custom,
            1 => GeneratorType::Normal,
            2 => {
                let options = root.get_str("generatorOptions").unwrap_or("");
                if !options.is_empty() {
                    return if root.get_str("FlatWorldLayers") == Some(VOID_WORLD_STRING) {
                        GeneratorType::Void
                    } else {
                        GeneratorType::Custom
                    };
                }
                // Deviation from Chunker: a flat world whose layers are "The Void" (current Bedrock writes it as a
                // JSON preset, not as Chunker's string) is a void world, not a grass flat one.
                if root.get_str("FlatWorldLayers").is_some_and(is_void_preset) {
                    GeneratorType::Void
                } else {
                    GeneratorType::Flat
                }
            }
            _ => GeneratorType::Custom,
        }
    }
}

/// Whether a `FlatWorldLayers` JSON is the "The Void" preset or has nothing but air layers.
pub fn is_void_preset(json: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return false };
    if value.get("preset_id").and_then(|p| p.as_str()) == Some("TheVoid") {
        return true;
    }
    match value.get("block_layers").and_then(|l| l.as_array()) {
        Some(layers) if !layers.is_empty() => {
            layers.iter().all(|l| l.get("block_name").and_then(|n| n.as_str()) == Some("minecraft:air"))
        }
        _ => false,
    }
}

impl SettingsIo for BedrockSettingsIo<'_> {
    fn encoding_type(&self) -> EncodingType {
        EncodingType::Bedrock
    }

    fn read_custom(
        &self,
        root: &Compound,
        _settings: &mut LevelSettings,
        name: &str,
        kind: SettingKind,
    ) -> Option<SettingValue> {
        if let Some(exp) = EXPERIMENTS.iter().find(|e| e.name == name) {
            return Some(SettingValue::Bool(self.flag(root, exp)));
        }
        match name {
            "FlatWorldVersion" => Some(SettingValue::Int(root.get_i32_or("WorldVersion", 0))),
            "playerPermissionsLevel" => {
                // Before 1.19.10 the permissions were held inside the abilities compound.
                let source = if root.contains_key(name) { Some(root) } else { root.get_compound("abilities") };
                source?.get_integer(name).map(|v| SettingValue::Int(v as i32))
            }
            "RandomSeed" => {
                let seed = root.get_integer(name)?;
                // After storage version 8 the seed is a long, before it was shown as an int.
                Some(SettingValue::Str(if root.get_i32_or("StorageVersion", 0) > 8 {
                    seed.to_string()
                } else {
                    (seed as i32).to_string()
                }))
            }
            _ if kind == SettingKind::Generator => Some(SettingValue::Generator(self.generator(root))),
            _ => {
                self.log_non_fatal(format!("Type {name} isn't implemented for setting parsing"));
                None
            }
        }
    }

    fn log_non_fatal(&self, message: String) {
        if let Ok(mut w) = self.warnings.lock() {
            w.push(message);
        }
    }
}

/// Read the level settings of a `level.dat` root.
pub fn read_level_settings(
    root: &Compound,
    storage_version: i32,
    version: &'static BedrockVersion,
    warnings: &Mutex<Vec<String>>,
) -> LevelSettings {
    LevelSettings::from_nbt(root, &BedrockSettingsIo { version, storage_version, warnings })
}

fn floats(tag: &Compound, key: &str, default: &[f64]) -> Vec<f64> {
    match tag.get(key) {
        Some(Tag::List(items)) => {
            let v: Vec<f64> = items
                .iter()
                .filter_map(|t| match t {
                    Tag::Float(f) => Some(f64::from(*f)),
                    Tag::Double(d) => Some(*d),
                    _ => None,
                })
                .collect();
            if v.len() >= default.len() { v } else { default.to_vec() }
        }
        _ => default.to_vec(),
    }
}

/// Parse `~local_player` (`parseLocalPlayer`). Items stay Bedrock item compounds (`PlayerItem`), keyed by the slot
/// Java would use: the inventory as stored, armor at `100 + (3 - slot)` and the off hand at `150 + slot`. Empty
/// stacks are left out.
pub fn parse_player(player: &Compound, registry: &DimensionRegistry, keep_items: bool) -> LevelPlayer {
    let pos = floats(player, "Pos", &[0.0; 3]);
    let motion = floats(player, "Motion", &[0.0; 3]);
    let rotation = floats(player, "Rotation", &[0.0; 2]);
    let mut result = LevelPlayer::new(registry.from_bedrock_nbt(player.get("DimensionId"), &Dimension::overworld()));
    result.position = [pos[0], pos[1] - f64::from(PLAYER_HEIGHT), pos[2]];
    result.motion = [motion[0], motion[1], motion[2]];
    result.yaw = rotation[0] as f32;
    result.pitch = rotation[1] as f32;
    result.game_type = player.get_i32_or("PlayerGameMode", 0);
    if keep_items {
        let mut add = |list: &str, slot_of: &dyn Fn(i8) -> i8| {
            if let Some(Tag::List(items)) = player.get(list) {
                for (index, item) in items.iter().enumerate() {
                    let Tag::Compound(item) = item else { continue };
                    let name = item.get_str("Name").unwrap_or("");
                    if name.is_empty() || name == "minecraft:air" {
                        continue;
                    }
                    let slot = item.get_i8("Slot").unwrap_or(index as i8);
                    result.inventory.insert(slot_of(slot), item.clone());
                }
            }
        };
        add("Inventory", &|slot| slot);
        add("Armor", &|slot| (100i32 + (3 - i32::from(slot))) as i8);
        add("Offhand", &|slot| (150i32 + i32::from(slot)) as i8);
    }
    result
}

/// Parse the `map_<id>` records (`parseMaps`), sorted by id. Records that fail to parse are reported in `warnings`.
pub fn parse_maps(
    entries: &[(Vec<u8>, Vec<u8>)],
    registry: &DimensionRegistry,
    keep_nbt: bool,
    warnings: &mut Vec<String>,
) -> Vec<LevelMap> {
    let mut maps = Vec::new();
    for (key, value) in entries {
        let Some(suffix) = key.strip_prefix(super::keys::MAP_PREFIX) else { continue };
        let Some(id) = std::str::from_utf8(suffix).ok().and_then(|s| s.parse::<i64>().ok()) else {
            warnings.push(format!("map key {:?} has no numeric id", String::from_utf8_lossy(key)));
            continue;
        };
        let root = match super::nbt_le::from_slice(value) {
            Ok((_, Tag::Compound(c))) => c,
            Ok(_) => {
                warnings.push(format!("map {id} is not a compound"));
                continue;
            }
            Err(e) => {
                warnings.push(format!("map {id}: {e}"));
                continue;
            }
        };
        let mut map = LevelMap::new(id, registry.from_bedrock_nbt(root.get("dimension"), &Dimension::overworld()));
        map.original_id = Some(id);
        map.height = i32::from(root.get_i16("height").unwrap_or(128));
        map.width = i32::from(root.get_i16("width").unwrap_or(128));
        map.scale = root.get_i8("scale").unwrap_or(0);
        map.x_center = root.get_i32("xCenter").unwrap_or(0);
        map.z_center = root.get_i32("zCenter").unwrap_or(0);
        map.unlimited_tracking = root.get_i8("unlimitedTracking").unwrap_or(0) != 0;
        map.locked = root.get_i8("mapLocked").unwrap_or(0) != 0;
        map.bytes = root.get_byte_array("colors").map(|b| b.iter().map(|v| *v as u8).collect());
        if keep_nbt {
            map.original_nbt = Some(root);
        }
        maps.push(map);
    }
    maps.sort_by(LevelMap::cmp_by_id);
    maps
}

/// Parse the `portals` record (`parsePortals`).
pub fn parse_portals(value: &[u8], registry: &DimensionRegistry) -> Result<Vec<Portal>, String> {
    let (_, tag) = super::nbt_le::from_slice(value).map_err(|e| e.to_string())?;
    let Tag::Compound(root) = tag else { return Err("portals is not a compound".into()) };
    let Some(records) = root.get_compound("data").and_then(|d| d.get_list("PortalRecords")) else {
        return Ok(Vec::new());
    };
    let mut portals = Vec::new();
    for record in records {
        let Tag::Compound(r) = record else { continue };
        portals.push(Portal {
            dimension: registry.from_bedrock(r.get_i32("DimId").unwrap_or(0), &Dimension::overworld()),
            x: r.get_i32("TpX").unwrap_or(0),
            y: r.get_i32("TpY").unwrap_or(0),
            z: r.get_i32("TpZ").unwrap_or(0),
            width: r.get_i8("Span").unwrap_or(0),
            xa: r.get_i8("Xa").unwrap_or(0),
            za: r.get_i8("Za").unwrap_or(0),
        });
    }
    Ok(portals)
}

/// Parse the `BiomeIdsTable` record (`{list: [{id: short, name}]}`) into `(id, name)` pairs.
pub fn parse_biome_ids(value: &[u8]) -> Result<Vec<(i32, String)>, String> {
    let (_, tag) = super::nbt_le::from_slice(value).map_err(|e| e.to_string())?;
    let Tag::Compound(root) = tag else { return Err("BiomeIdsTable is not a compound".into()) };
    let Some(list) = root.get_list("list") else { return Ok(Vec::new()) };
    Ok(list
        .iter()
        .filter_map(|t| match t {
            Tag::Compound(c) => Some((i32::from(c.get_i16("id")?), c.get_str("name")?.to_string())),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::bedrock::data_version::BedrockVersion;

    fn version(game: GameVersion) -> &'static BedrockVersion {
        BedrockVersion::nearest_at_or_below(game)
    }

    fn settings(root: &Compound, storage: i32, game: GameVersion) -> (LevelSettings, Vec<String>) {
        let warnings = Mutex::new(Vec::new());
        let s = read_level_settings(root, storage, version(game), &warnings);
        (s, warnings.into_inner().unwrap())
    }

    fn experiments(keys: &[&str]) -> Compound {
        let mut e = Compound::new();
        for k in keys {
            e.insert(*k, Tag::Byte(1));
        }
        let mut root = Compound::new();
        root.insert("experiments", Tag::Compound(e));
        root
    }

    #[test]
    fn basic_fields_and_defaults() {
        let mut root = Compound::new();
        root.insert("LevelName", Tag::String("Hello".into()));
        root.insert("RandomSeed", Tag::Long(-7_000_000_000));
        root.insert("StorageVersion", Tag::Int(10));
        root.insert("SpawnX", Tag::Int(5));
        root.insert("GameType", Tag::Int(1));
        root.insert("keepinventory", Tag::Byte(1));
        root.insert("rainLevel", Tag::Float(0.5));
        let (s, warnings) = settings(&root, 10, [1, 26, 50]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.level_name, "Hello");
        assert_eq!(s.random_seed, "-7000000000");
        assert_eq!((s.spawn_x, s.game_type, s.keepinventory), (5, 1, true));
        assert_eq!(s.rain_level, 0.5);
        // Untouched fields keep Chunker's defaults.
        assert_eq!(s.spawn_y, 32767);
        assert!(s.domobloot);
    }

    #[test]
    fn old_seeds_are_truncated_to_int() {
        let mut root = Compound::new();
        root.insert("RandomSeed", Tag::Long(0x1_0000_0005));
        let (s, _) = settings(&root, 8, [1, 12, 0]);
        assert_eq!(s.random_seed, "5");
    }

    #[test]
    fn generator_types() {
        let mut root = Compound::new();
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.generator_type, GeneratorType::Normal);
        root.insert("Generator", Tag::Int(0));
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.generator_type, GeneratorType::Custom);
        root.insert("Generator", Tag::Int(2));
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.generator_type, GeneratorType::Flat);
        root.insert("generatorOptions", Tag::String("x".into()));
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.generator_type, GeneratorType::Custom);
        root.insert("FlatWorldLayers", Tag::String(VOID_WORLD_STRING.into()));
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.generator_type, GeneratorType::Void);
        root.insert("Generator", Tag::Int(7));
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.generator_type, GeneratorType::Custom);
    }

    #[test]
    fn the_void_preset_is_void() {
        let mut root = Compound::new();
        root.insert("Generator", Tag::Int(2));
        root.insert(
            "FlatWorldLayers",
            Tag::String(
                "{\"biome_id\":1,\"encoding_version\":6,\"preset_id\":\"TheVoid\",\"world_version\":\"version.post_1_18\"}\n"
                    .into(),
            ),
        );
        assert_eq!(settings(&root, 10, [1, 26, 50]).0.generator_type, GeneratorType::Void);
        assert!(is_void_preset("{\"block_layers\":[{\"block_name\":\"minecraft:air\",\"count\":1}]}"));
        assert!(!is_void_preset("{\"block_layers\":[{\"block_name\":\"minecraft:grass\",\"count\":1}]}"));
        assert!(!is_void_preset("not json"));
    }

    #[test]
    fn experiments_follow_the_version_stack() {
        // 1.17: reads the caves & cliffs toggle, nothing newer exists.
        let root = experiments(&["caves_and_cliffs", "next_major_update", "y_2025_drop_2"]);
        let (s, _) = settings(&root, 9, [1, 17, 10]);
        assert!(s.caves_and_cliffs);
        assert!(!s.r20_support && !s.summer_drop2025);
        // 1.19: reads next_major_update, caves & cliffs is built in.
        let (s, _) = settings(&root, 9, [1, 19, 40]);
        assert!(s.caves_and_cliffs && s.r20_support && !s.r21_support);
        // 1.20 and on: R20 is on without a toggle; R21 needs updateAnnouncedLive2023 until 1.21.
        let (s, _) = settings(&Compound::new(), 10, [1, 20, 50]);
        assert!(s.r20_support && !s.r21_support);
        let (s, _) = settings(&experiments(&["updateAnnouncedLive2023"]), 10, [1, 20, 50]);
        assert!(s.r21_support);
        let (s, _) = settings(&Compound::new(), 10, [1, 21, 50]);
        assert!(s.r21_support && s.winter_drop2024 && !s.summer_drop2025);
        // The toggle of a drop is only read from the version that has it.
        let drop2 = experiments(&["y_2025_drop_2"]);
        assert!(!settings(&drop2, 10, [1, 21, 60]).0.summer_drop2025);
        assert!(settings(&drop2, 10, [1, 21, 80]).0.summer_drop2025);
        let (s, _) = settings(&Compound::new(), 10, [1, 26, 50]);
        assert!(s.summer_drop2026 && s.autumn_drop2026 && !s.winter_drop2026);
        let (s, _) = settings(&experiments(&["y_2026_drop_4"]), 10, [1, 26, 60]);
        assert!(s.winter_drop2026);
    }

    #[test]
    fn permissions_level_moves_out_of_abilities() {
        let mut abilities = Compound::new();
        abilities.insert("playerPermissionsLevel", Tag::Byte(3));
        let mut root = Compound::new();
        root.insert("abilities", Tag::Compound(abilities));
        assert_eq!(settings(&root, 9, [1, 19, 0]).0.player_permissions_level, 3);
        root.insert("playerPermissionsLevel", Tag::Int(1));
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.player_permissions_level, 1);
        assert_eq!(settings(&Compound::new(), 10, [1, 20, 0]).0.player_permissions_level, 2);
    }

    #[test]
    fn flat_world_version_comes_from_world_version() {
        let mut root = Compound::new();
        root.insert("WorldVersion", Tag::Int(7));
        assert_eq!(settings(&root, 10, [1, 20, 0]).0.flat_world_version, 7);
        assert_eq!(settings(&Compound::new(), 10, [1, 20, 0]).0.flat_world_version, 0);
    }

    fn item(name: &str, slot: Option<i8>) -> Tag {
        let mut c = Compound::new();
        c.insert("Name", Tag::String(name.into()));
        c.insert("Count", Tag::Byte(1));
        if let Some(s) = slot {
            c.insert("Slot", Tag::Byte(s));
        }
        Tag::Compound(c)
    }

    #[test]
    fn local_player() {
        let mut p = Compound::new();
        p.insert("Pos", Tag::List(vec![Tag::Float(10.5), Tag::Float(65.62001), Tag::Float(-3.0)]));
        p.insert("Rotation", Tag::List(vec![Tag::Float(90.0), Tag::Float(-10.0)]));
        p.insert("DimensionId", Tag::Int(1));
        p.insert("PlayerGameMode", Tag::Int(2));
        p.insert(
            "Inventory",
            Tag::List(vec![item("minecraft:stone", Some(4)), item("", Some(5)), item("minecraft:dirt", None)]),
        );
        p.insert(
            "Armor",
            Tag::List(vec![item("minecraft:iron_helmet", Some(0)), item("minecraft:iron_boots", Some(3))]),
        );
        p.insert("Offhand", Tag::List(vec![item("minecraft:shield", Some(0))]));
        let player = parse_player(&p, &DimensionRegistry::new(), true);
        assert_eq!(player.dimension, Dimension::nether());
        assert!((player.position[1] - 64.0).abs() < 1e-4, "{}", player.position[1]);
        assert_eq!((player.position[0], player.position[2]), (10.5, -3.0));
        assert_eq!((player.yaw, player.pitch, player.game_type), (90.0, -10.0, 2));
        // Slots: stone 4, dirt (no slot: its index 2), helmet 103, boots 100, shield 150.
        let mut slots = player.inventory.keys().copied().collect::<Vec<_>>();
        slots.sort_unstable();
        assert_eq!(slots, [-106, 2, 4, 100, 103]);
        let none = parse_player(&p, &DimensionRegistry::new(), false);
        assert!(none.inventory.is_empty());
    }

    #[test]
    fn maps_and_portals() {
        let mut map = Compound::new();
        map.insert("width", Tag::Short(64));
        map.insert("height", Tag::Short(32));
        map.insert("scale", Tag::Byte(2));
        map.insert("dimension", Tag::Byte(2));
        map.insert("xCenter", Tag::Int(128));
        map.insert("mapLocked", Tag::Byte(1));
        map.insert("colors", Tag::ByteArray(vec![-1, 0, 5]));
        let bytes = super::super::nbt_le::to_vec("", &Tag::Compound(map)).unwrap();
        let entries =
            vec![(b"map_-5".to_vec(), bytes.clone()), (b"map_3".to_vec(), bytes), (b"map_x".to_vec(), vec![])];
        let mut warnings = Vec::new();
        let maps = parse_maps(&entries, &DimensionRegistry::new(), true, &mut warnings);
        assert_eq!(maps.iter().map(|m| m.id).collect::<Vec<_>>(), [-5, 3]);
        assert_eq!(warnings.len(), 1);
        let m = &maps[1];
        assert_eq!((m.width, m.height, m.scale, m.x_center, m.locked), (64, 32, 2, 128, true));
        assert_eq!(m.dimension, Dimension::the_end());
        assert_eq!(m.bytes.as_deref(), Some(&[255u8, 0, 5][..]));
        assert_eq!(m.original_id, Some(3));
        assert!(m.original_nbt.is_some());

        let mut record = Compound::new();
        record.insert("DimId", Tag::Int(1));
        record.insert("TpX", Tag::Int(10));
        record.insert("TpY", Tag::Int(64));
        record.insert("TpZ", Tag::Int(-2));
        record.insert("Span", Tag::Byte(3));
        record.insert("Xa", Tag::Byte(1));
        record.insert("Za", Tag::Byte(0));
        let mut data = Compound::new();
        data.insert("PortalRecords", Tag::List(vec![Tag::Compound(record)]));
        let mut root = Compound::new();
        root.insert("data", Tag::Compound(data));
        let bytes = super::super::nbt_le::to_vec("", &Tag::Compound(root)).unwrap();
        let portals = parse_portals(&bytes, &DimensionRegistry::new()).unwrap();
        assert_eq!(portals.len(), 1);
        assert_eq!((portals[0].x, portals[0].y, portals[0].z, portals[0].width), (10, 64, -2, 3));
        assert_eq!(portals[0].dimension, Dimension::nether());
        assert!(parse_portals(&[1, 2, 3], &DimensionRegistry::new()).is_err());
    }

    #[test]
    fn biome_id_table() {
        let mut a = Compound::new();
        a.insert("id", Tag::Short(30000));
        a.insert("name", Tag::String("mymod:glade".into()));
        let mut root = Compound::new();
        root.insert("list", Tag::List(vec![Tag::Compound(a)]));
        let bytes = super::super::nbt_le::to_vec("", &Tag::Compound(root)).unwrap();
        assert_eq!(parse_biome_ids(&bytes).unwrap(), [(30000, "mymod:glade".to_string())]);
    }
}
