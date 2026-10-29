// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Writing the level model as a Java `level.dat` and the files around it (`JavaLevelWriter` and the `v1_13` ...
//! `v26_3` level writer overrides): settings, world generation, player, maps, data pack switches, the 26.1 split
//! into `data/minecraft/*.dat`, `icon.png`.
//!
//! The steps follow Chunker: the settings table writes every field with a Java name (game rules as strings), the
//! custom ones (generator, seed, experiments) are handled per version in [`JavaSettingsIo::write_custom`], then
//! [`extra_settings`] adds the version block and rewrites names for 1.16, 1.20, 1.21.9, 1.21.11 and 26.1.

use super::dirs::{JavaDimension, LevelDirs};
use super::format::{CavesAndCliffs, FeatureState, JavaFormat};
use super::resolvers::block_entity::{effect_id, effect_name};
use super::resolvers::itemstack::ItemHooks;
use super::writer::writer_biomes::BiomeWriter;
use super::writer::{GuestPlayer, ItemsImpl};
use crate::convert::level::level_settings::{SettingKind, SettingValue, SettingsIo};
use crate::convert::level::{
    ConversionSettings, ConvertError, Dimension, EncodingType, GeneratorType, Level, LevelMap, LevelPlayer,
    LevelSettings,
};
use crate::convert::mapping::{Edition, Version as MapVersion, tables};
use crate::convert::model::{Biome, EffectType, ItemStack, VanillaBiome};
use crate::convert::nbt::file::{Compression, write_java_file};
use crate::convert::nbt::{Compound, Tag};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

/// The name of the data pack that lifts the world height in 1.17.
pub const HEIGHT_PACK_NAME: &str = "CavesAndCliffsHeightPack.zip";

const HEIGHT_PACK_META: &str =
    "{\n  \"pack\": {\n    \"pack_format\": 7,\n    \"description\": \"Caves & Cliffs Height Compatibility\"\n  }\n}\n";

fn height_dimension_type(has_ceiling: bool) -> String {
    format!(
        "{{\n  \"logical_height\": 384,\n  \"infiniburn\": \"minecraft:infiniburn_overworld\",\n  \"effects\": \"minecraft:overworld\",\n  \"ambient_light\": 0.0,\n  \"respawn_anchor_works\": false,\n  \"has_raids\": true,\n  \"min_y\": -64,\n  \"height\": 384,\n  \"natural\": true,\n  \"coordinate_scale\": 1.0,\n  \"piglin_safe\": false,\n  \"bed_works\": true,\n  \"has_skylight\": true,\n  \"has_ceiling\": {has_ceiling},\n  \"ultrawarm\": false\n}}\n"
    )
}

/// What the level writer needs from the world writer.
pub(super) struct LevelFiles<'a> {
    pub format: &'a JavaFormat,
    pub dirs: &'a LevelDirs,
    pub level: &'a Level,
    pub settings: &'a ConversionSettings,
    pub items: &'a ItemsImpl,
    /// Every item is written with these (map ids, shulker box contents).
    pub hooks: &'a dyn ItemHooks,
    pub biomes: &'a BiomeWriter,
    pub player_items: &'a BTreeMap<i8, ItemStack>,
    pub ender_items: &'a BTreeMap<i8, ItemStack>,
    pub guests: &'a [GuestPlayer],
    pub icon: Option<&'a [u8]>,
    pub dimensions: &'a [Dimension],
    pub tall_overworld: bool,
}

fn io_error(e: std::io::Error) -> ConvertError {
    ConvertError::Io(e)
}

fn nbt_error(e: crate::convert::nbt::NbtError) -> ConvertError {
    ConvertError::Internal(format!("could not write NBT: {e}"))
}

/// Write `file` as `{DataVersion, data: {...}}` (when `wrap`), or the compound itself with `DataVersion`, gzip.
fn write_data_file(f: &LevelFiles<'_>, file: &Path, data: Compound, wrap: bool) -> Result<(), ConvertError> {
    if data.is_empty() {
        return Ok(());
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let root = if wrap {
        let mut root = Compound::with_capacity(2);
        root.insert("data", Tag::Compound(data));
        root.insert("DataVersion", Tag::Int(f.format.row_data_version));
        root
    } else {
        let mut root = data;
        root.insert("DataVersion", Tag::Int(f.format.row_data_version));
        root
    };
    write_java_file(file, &root, Compression::Gzip).map_err(nbt_error)
}

/// The version name vanilla writes: `1.21` for 1.21.0, `26.1` for 26.1.0.
fn version_name(v: crate::convert::util::Version) -> String {
    if v.patch == 0 { format!("{}.{}", v.major, v.minor) } else { v.to_string() }
}

/// `Enabled` / `enabled_features` of a data pack switch (`enableExperiments`).
fn enable_experiments(output: &mut Compound, experiments: &[&str]) {
    let packs = output.get_or_create_compound("DataPacks");
    if !matches!(packs.get("Enabled"), Some(Tag::List(_))) {
        packs.insert("Enabled", Tag::List(Vec::new()));
    }
    push_unique(packs, "Enabled", "vanilla");
    for e in experiments {
        push_unique(packs, "Enabled", e);
    }
    if !matches!(output.get("enabled_features"), Some(Tag::List(_))) {
        output.insert("enabled_features", Tag::List(Vec::new()));
    }
    for e in std::iter::once(&"vanilla").chain(experiments.iter()) {
        let list = output.get_list_mut("enabled_features").expect("just created");
        if !list.iter().any(|t| t.as_str() == Some(e)) {
            list.push(Tag::String((*e).to_string()));
        }
    }
}

fn push_unique(compound: &mut Compound, key: &str, value: &str) {
    if let Some(list) = compound.get_list_mut(key) {
        if !list.iter().any(|t| t.as_str() == Some(value)) {
            list.push(Tag::String(value.to_string()));
        }
    }
}

struct JavaSettingsIo<'a, 'b> {
    f: &'a LevelFiles<'b>,
    copying: bool,
    warnings: Mutex<Vec<String>>,
}

impl SettingsIo for JavaSettingsIo<'_, '_> {
    fn encoding_type(&self) -> EncodingType {
        EncodingType::Java
    }

    fn write_custom(&self, settings: &LevelSettings, output: &mut Compound, name: &str, value: &SettingValue) {
        let fmt = self.f.format;
        let on = matches!(value, SettingValue::Bool(true));
        match name {
            // Supported by the game itself or not stored on Java.
            "WinterDrop2026" | "AutumnDrop2026" | "SummerDrop2026" | "AutumnDrop2025" | "SummerDrop2025"
            | "FlatWorldVersion" => {}
            "WinterDrop2024" => {
                if fmt.winter_drop_2024 == FeatureState::Experimental && on && !self.copying {
                    enable_experiments(output, &["winter_drop"]);
                }
            }
            "R21Support" => {
                if fmt.r21_support == FeatureState::Experimental && on && !self.copying {
                    enable_experiments(output, &["update_1_21"]);
                }
            }
            "R20Support" => {
                if fmt.r20_support == FeatureState::Experimental && on && !self.copying {
                    enable_experiments(output, &["update_1_20"]);
                }
            }
            "CavesAndCliffs" => {
                if fmt.caves_and_cliffs == CavesAndCliffs::Datapack && on && !self.copying {
                    let packs = output.get_or_create_compound("DataPacks");
                    if !matches!(packs.get("Enabled"), Some(Tag::List(_))) {
                        packs.insert("Enabled", Tag::List(Vec::new()));
                    }
                    push_unique(packs, "Enabled", "vanilla");
                    push_unique(packs, "Enabled", &format!("file/{HEIGHT_PACK_NAME}"));
                }
            }
            "RandomSeed" => {
                // From 1.16 the seed is part of the generator settings.
                if !fmt.world_gen_settings {
                    output.insert("RandomSeed", Tag::Long(parse_seed(&settings.random_seed)));
                }
            }
            "GeneratorType" => {
                if let SettingValue::Generator(g) = value {
                    self.write_generator(settings, output, *g);
                }
            }
            other => self.log_non_fatal(format!("Writing of {other} is not implemented.")),
        }
    }

    fn log_non_fatal(&self, message: String) {
        if let Ok(mut w) = self.warnings.lock() {
            w.push(message);
        }
    }
}

/// `Long.parseLong` of the seed text; text that is not a number is hashed so the same text gives the same seed.
fn parse_seed(text: &str) -> i64 {
    let text = text.trim();
    if let Ok(v) = text.parse::<i64>() {
        return v;
    }
    if let Ok(v) = text.parse::<u64>() {
        return v as i64;
    }
    // Java's String.hashCode, like the game does for seeds typed in the create world screen.
    let mut h: i32 = 0;
    for unit in text.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h as i64
}

impl JavaSettingsIo<'_, '_> {
    fn plains(&self) -> String {
        self.f.biomes.name(&Biome::Vanilla(VanillaBiome::PLAINS), "minecraft:plains").to_string()
    }

    fn write_generator(&self, settings: &LevelSettings, output: &mut Compound, generator: GeneratorType) {
        let fmt = self.f.format;
        let mut generator = generator;
        if generator == GeneratorType::Custom && !self.copying {
            generator = GeneratorType::Void;
        }
        if generator == GeneratorType::Custom {
            return;
        }
        if fmt.world_gen_settings {
            let seed = parse_seed(&settings.random_seed);
            let mut dimensions = Compound::with_capacity(3);
            dimensions.insert(
                "minecraft:overworld",
                Tag::Compound(self.create_dimension("minecraft:overworld", generator, seed)),
            );
            // The client complains when the end and the nether are not generated.
            dimensions.insert(
                "minecraft:the_end",
                Tag::Compound(self.create_dimension("minecraft:the_end", GeneratorType::Normal, seed)),
            );
            dimensions.insert(
                "minecraft:the_nether",
                Tag::Compound(self.create_dimension("minecraft:the_nether", GeneratorType::Normal, seed)),
            );
            let mut gen_settings = Compound::with_capacity(4);
            gen_settings.insert("bonus_chest", Tag::Byte(settings.bonus_chest_enabled as i8));
            gen_settings.insert("generate_features", Tag::Byte(settings.map_features as i8));
            gen_settings.insert("seed", Tag::Long(seed));
            gen_settings.insert("dimensions", Tag::Compound(dimensions));
            output.insert("WorldGenSettings", Tag::Compound(gen_settings));
            return;
        }
        // 1.13 - 1.15: flat options are a compound.
        match generator {
            GeneratorType::Normal => {
                output.insert("generatorName", Tag::String("default".into()));
                output.insert("generatorVersion", Tag::Int(0));
            }
            GeneratorType::Flat => {
                output.insert("generatorName", Tag::String("flat".into()));
                output.insert("generatorVersion", Tag::Int(0));
            }
            _ => {
                output.insert("generatorName", Tag::String("flat".into()));
                output.insert("generatorVersion", Tag::Int(0));
                let mut options = Compound::with_capacity(2);
                options.insert("biome", Tag::String(self.plains()));
                let mut air = Compound::with_capacity(2);
                air.insert("block", Tag::String("minecraft:air".into()));
                air.insert("height", Tag::Byte(1));
                options.insert("layers", Tag::List(vec![Tag::Compound(air)]));
                output.insert("generatorOptions", Tag::Compound(options));
            }
        }
    }

    fn create_dimension(&self, dimension: &str, generator: GeneratorType, seed: i64) -> Compound {
        let mut tag = Compound::with_capacity(2);
        let mut gen_tag = Compound::with_capacity(4);
        gen_tag.insert(
            "type",
            Tag::String(if generator == GeneratorType::Normal { "minecraft:noise" } else { "minecraft:flat" }.into()),
        );
        if generator == GeneratorType::Normal {
            gen_tag.insert("settings", Tag::String(dimension.replace("the_", "")));
            gen_tag.insert("seed", Tag::Long(seed));
            gen_tag.insert("biome_source", Tag::Compound(self.biome_source(dimension, seed)));
        } else {
            let mut settings = Compound::with_capacity(3);
            let mut structures = Compound::with_capacity(1);
            structures.insert("structures", Tag::Compound(Compound::new()));
            settings.insert("structures", Tag::Compound(structures));
            settings.insert("biome", Tag::String(self.plains()));
            let layer = |block: &str, height: i32| {
                let mut l = Compound::with_capacity(2);
                l.insert("block", Tag::String(block.into()));
                l.insert("height", Tag::Int(height));
                Tag::Compound(l)
            };
            let layers = if generator == GeneratorType::Flat {
                vec![layer("minecraft:bedrock", 1), layer("minecraft:dirt", 2), layer("minecraft:grass_block", 1)]
            } else {
                vec![layer("minecraft:air", 1)]
            };
            settings.insert("layers", Tag::List(layers));
            gen_tag.insert("settings", Tag::Compound(settings));
        }
        tag.insert("generator", Tag::Compound(gen_tag));
        tag.insert("type", Tag::String(dimension.into()));
        tag
    }

    fn biome_source(&self, dimension: &str, seed: i64) -> Compound {
        let modern = self.f.format.sections == super::format::SectionLayout::NestedStates;
        let mut biomes = Compound::with_capacity(3);
        match dimension {
            "minecraft:overworld" => {
                if modern {
                    biomes.insert("preset", Tag::String("minecraft:overworld".into()));
                    biomes.insert("type", Tag::String("minecraft:multi_noise".into()));
                } else {
                    biomes.insert("seed", Tag::Long(seed));
                    biomes.insert("type", Tag::String("minecraft:vanilla_layered".into()));
                    biomes.insert("large_biomes", Tag::Byte(0));
                }
            }
            "minecraft:the_end" => {
                biomes.insert("type", Tag::String("minecraft:the_end".into()));
                biomes.insert("seed", Tag::Long(seed));
            }
            _ => {
                if !modern {
                    biomes.insert("seed", Tag::Long(seed));
                }
                biomes.insert("preset", Tag::String("minecraft:nether".into()));
                biomes.insert("type", Tag::String("minecraft:multi_noise".into()));
            }
        }
        biomes
    }
}

/// The single player: `Data.Player`, moved to its own file by [`split_level_dat`] from 26.1. With an owner account
/// the player carries its `UUID`, and 26.1+ level.dat names it in `singleplayer_uuid`.
fn write_player(f: &LevelFiles<'_>, level: &mut Compound) {
    let Some(player) = &f.level.player else { return };
    let owner = f.settings.player.owner_uuid;
    let tag = level.get_or_create_compound("Player");
    fill_player(f, tag, player, f.player_items, f.ender_items, owner);
    if let Some(uuid) = owner
        && f.format.split_level_dat
    {
        level.insert("singleplayer_uuid", Tag::IntArray(uuid_ints(uuid).to_vec()));
    }
}

/// A uuid as Java's four ints (most significant first).
fn uuid_ints(uuid: uuid::Uuid) -> [i32; 4] {
    let v = uuid.as_u128();
    [(v >> 96) as u32 as i32, (v >> 64) as u32 as i32, (v >> 32) as u32 as i32, v as u32 as i32]
}

/// Write a player compound of the target version into `tag`.
fn fill_player(
    f: &LevelFiles<'_>,
    tag: &mut Compound,
    player: &LevelPlayer,
    items: &BTreeMap<i8, ItemStack>,
    ender_items: &BTreeMap<i8, ItemStack>,
    uuid: Option<uuid::Uuid>,
) {
    let fmt = f.format;
    let v = fmt.version;
    tag.insert("DataVersion", Tag::Int(fmt.row_data_version));
    tag.insert("Pos", Tag::List(player.position.iter().map(|v| Tag::Double(*v)).collect()));
    tag.insert("Motion", Tag::List(player.motion.iter().map(|v| Tag::Double(*v)).collect()));
    tag.insert("Rotation", Tag::List(vec![Tag::Float(player.yaw), Tag::Float(player.pitch)]));
    if fmt.dimension_ids_as_strings {
        tag.insert("Dimension", Tag::String(player.dimension.identifier().to_string()));
    } else {
        tag.insert("Dimension", Tag::Int(player.dimension.java_id().unwrap_or(0)));
    }
    let mut game_type = player.game_type;
    if game_type == 5 {
        game_type = f.level.settings.game_type;
    }
    // A forced game mode is the player's too.
    if let Some(forced) = f.settings.level_overrides.game_type {
        game_type = forced.id();
    }
    tag.insert("playerGameType", Tag::Int(game_type.clamp(0, 3)));
    if let Some(uuid) = uuid {
        if v.is_greater_than_or_equal(1, 16, 0) {
            tag.insert("UUID", Tag::IntArray(uuid_ints(uuid).to_vec()));
        } else {
            let (most, least) = uuid.as_u64_pair();
            tag.insert("UUIDMost", Tag::Long(most as i64));
            tag.insert("UUIDLeast", Tag::Long(least as i64));
        }
    }

    // A player that was never saved (or whose stats are left out) starts with full health and food.
    let stats = &player.stats;
    tag.insert("Health", Tag::Float(stats.health.unwrap_or(20.0)));
    tag.insert("foodLevel", Tag::Int(stats.food_level.unwrap_or(20)));
    if let Some(saturation) = stats.saturation {
        tag.insert("foodSaturationLevel", Tag::Float(saturation));
    }
    if let Some(exhaustion) = stats.exhaustion {
        tag.insert("foodExhaustionLevel", Tag::Float(exhaustion));
    }
    tag.insert("Air", Tag::Short(300));
    if stats.xp_level > 0 || stats.xp_progress > 0.0 {
        let total = stats.xp_total();
        tag.insert("XpLevel", Tag::Int(stats.xp_level));
        tag.insert("XpP", Tag::Float(stats.xp_progress));
        tag.insert("XpTotal", Tag::Int(total));
        tag.insert("Score", Tag::Int(total));
    }
    if let Some(spawn) = &stats.spawn {
        write_respawn(f, tag, spawn);
    }
    write_effects(f, tag, &stats.effects);
    if let Some(a) = &stats.abilities {
        let mut abilities = Compound::with_capacity(7);
        abilities.insert("flying", Tag::Byte(a.flying as i8));
        abilities.insert("flySpeed", Tag::Float(a.fly_speed));
        abilities.insert("instabuild", Tag::Byte(a.instabuild as i8));
        abilities.insert("invulnerable", Tag::Byte(a.invulnerable as i8));
        abilities.insert("mayBuild", Tag::Byte(a.may_build as i8));
        abilities.insert("mayfly", Tag::Byte(a.may_fly as i8));
        abilities.insert("walkSpeed", Tag::Float(a.walk_speed));
        tag.insert("abilities", Tag::Compound(abilities));
    }

    const EQUIPMENT: [(i8, &str); 5] = [(-106, "offhand"), (100, "feet"), (101, "legs"), (102, "chest"), (103, "head")];
    let mut inventory = Vec::new();
    for (&slot, stack) in items {
        if stack.is_air() || (fmt.split_equipment && EQUIPMENT.iter().any(|(s, _)| *s == slot)) {
            continue;
        }
        let Some(mut item) = f.items.write_with(stack, f.hooks) else { continue };
        item.insert("Slot", Tag::Byte(slot));
        inventory.push(Tag::Compound(item));
    }
    tag.insert("Inventory", Tag::List(inventory));
    if fmt.split_equipment {
        let mut equipment = Compound::new();
        for (slot, name) in EQUIPMENT {
            let Some(stack) = items.get(&slot).filter(|s| !s.is_air()) else { continue };
            if let Some(item) = f.items.write_with(stack, f.hooks) {
                equipment.insert(name, Tag::Compound(item));
            }
        }
        tag.insert("equipment", Tag::Compound(equipment));
    }
    let mut ender = Vec::new();
    for (&slot, stack) in ender_items.iter().filter(|(slot, _)| (0..27).contains(*slot)) {
        if stack.is_air() {
            continue;
        }
        let Some(mut item) = f.items.write_with(stack, f.hooks) else { continue };
        item.insert("Slot", Tag::Byte(slot));
        ender.push(Tag::Compound(item));
    }
    tag.insert("EnderItems", Tag::List(ender));
}

/// The bed or anchor the player respawns at: `SpawnX/Y/Z` before 1.21.5 (with `SpawnDimension` and `SpawnAngle` from
/// 1.16; only the overworld before), `respawn{pos, angle, dimension, forced}` in 1.21.5 and
/// `respawn{pos, yaw, pitch, dimension, forced}` from 1.21.9 (the game's `PlayerRespawnDataFix`).
fn write_respawn(f: &LevelFiles<'_>, tag: &mut Compound, spawn: &crate::convert::level::SpawnPoint) {
    let v = f.format.version;
    let dimension = spawn.dimension.identifier();
    if v.is_greater_than_or_equal(1, 21, 5) {
        let mut respawn = Compound::with_capacity(5);
        respawn.insert("pos", Tag::IntArray(vec![spawn.x, spawn.y, spawn.z]));
        if v.is_greater_than_or_equal(1, 21, 9) {
            respawn.insert("yaw", Tag::Float(0.0));
            respawn.insert("pitch", Tag::Float(0.0));
        } else {
            respawn.insert("angle", Tag::Float(0.0));
        }
        respawn.insert("dimension", Tag::String(dimension.to_string()));
        respawn.insert("forced", Tag::Byte(0));
        tag.insert("respawn", Tag::Compound(respawn));
        return;
    }
    let dimensions_by_name = v.is_greater_than_or_equal(1, 16, 0);
    if !dimensions_by_name && dimension != "minecraft:overworld" {
        return;
    }
    tag.insert("SpawnX", Tag::Int(spawn.x));
    tag.insert("SpawnY", Tag::Int(spawn.y));
    tag.insert("SpawnZ", Tag::Int(spawn.z));
    tag.insert("SpawnForced", Tag::Byte(0));
    if dimensions_by_name {
        tag.insert("SpawnAngle", Tag::Float(0.0));
        tag.insert("SpawnDimension", Tag::String(dimension.to_string()));
    }
}

/// Status effects: `active_effects` with names from 1.20.2, `ActiveEffects` with numeric ids before. Effects the
/// version does not know are left out.
fn write_effects(f: &LevelFiles<'_>, tag: &mut Compound, effects: &[crate::convert::level::PlayerEffect]) {
    let v = f.format.version;
    let named = v.is_greater_than_or_equal(1, 20, 2);
    let mut list = Vec::new();
    for e in effects {
        // Bedrock's fatal poison is poison that can kill.
        let effect = if e.effect == EffectType::FATAL_POISON { EffectType::POISON } else { e.effect };
        let mut c = Compound::with_capacity(6);
        if named {
            let name = effect_name(effect, v);
            if name == "minecraft:empty" {
                continue;
            }
            c.insert("id", Tag::String(name.into()));
            c.insert("amplifier", Tag::Byte(e.amplifier));
            c.insert("duration", Tag::Int(e.duration));
            c.insert("ambient", Tag::Byte(e.ambient as i8));
            c.insert("show_particles", Tag::Byte(e.show_particles as i8));
            c.insert("show_icon", Tag::Byte(1));
        } else {
            let id = effect_id(effect, v);
            if id == 0 {
                continue;
            }
            c.insert("Id", Tag::Int(id));
            c.insert("Amplifier", Tag::Byte(e.amplifier));
            c.insert("Duration", Tag::Int(e.duration));
            c.insert("Ambient", Tag::Byte(e.ambient as i8));
            c.insert("ShowParticles", Tag::Byte(e.show_particles as i8));
            c.insert("ShowIcon", Tag::Byte(1));
        }
        list.push(Tag::Compound(c));
    }
    if !list.is_empty() {
        tag.insert(if named { "active_effects" } else { "ActiveEffects" }, Tag::List(list));
    }
}

/// The player files beside level.dat: before 26.1 the owner's (`playerdata/<uuid>.dat`, the same data as
/// `Data.Player`, which a server or LAN game reads), and the other players' profiles in every version.
fn write_player_files(f: &LevelFiles<'_>, data: &Compound) -> Result<(), ConvertError> {
    let owner = f.settings.player.owner_uuid;
    if !f.format.split_level_dat
        && let (Some(uuid), Some(Tag::Compound(player))) = (owner, data.get("Player"))
    {
        write_player_file(f, uuid, player.clone())?;
    }
    for guest in f.guests.iter().filter(|g| Some(g.uuid) != owner) {
        let mut tag = Compound::new();
        fill_player(f, &mut tag, &guest.player, &guest.items, &guest.ender_items, Some(guest.uuid));
        write_player_file(f, guest.uuid, tag)?;
    }
    Ok(())
}

fn write_player_file(f: &LevelFiles<'_>, uuid: uuid::Uuid, player: Compound) -> Result<(), ConvertError> {
    let Some(file) = f.dirs.player_file(&uuid.hyphenated().to_string()) else { return Ok(()) };
    write_data_file(f, &file, player, false)
}

fn get_f64(c: &Compound, key: &str) -> Option<f64> {
    c.get(key).and_then(Tag::as_number)
}

/// `writeExtraLevelSettings` of every version.
fn extra_settings(f: &LevelFiles<'_>, data: &mut Compound) {
    let fmt = f.format;
    if !data.contains_key("Version") {
        let mut version = Compound::with_capacity(4);
        version.insert("Id", Tag::Int(fmt.row_data_version));
        version.insert("Name", Tag::String(version_name(fmt.version)));
        if fmt.writes_y_pos {
            version.insert("Series", Tag::String("main".into()));
        }
        version.insert("Snapshot", Tag::Byte(0));
        data.insert("Version", Tag::Compound(version));
    }
    if !data.contains_key("version") {
        data.insert("version", Tag::Int(super::version::LAST_ANVIL_FILE_VERSION));
    }
    if !data.contains_key("DataVersion") {
        data.insert("DataVersion", Tag::Int(fmt.row_data_version));
    }
    if !data.contains_key("initialized") {
        data.insert("initialized", Tag::Byte(1));
    }
    data.insert("WasModded", Tag::Byte(0));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    data.insert("LastPlayed", Tag::Long(now));
    if !data.contains_key("DataPacks") {
        let mut packs = Compound::with_capacity(2);
        packs.insert("Enabled", Tag::List(vec![Tag::String("vanilla".into())]));
        packs.insert("Disabled", Tag::List(Vec::new()));
        data.insert("DataPacks", Tag::Compound(packs));
    }
    // An unset spawn height (Bedrock stores 32767) becomes -1 so the game finds the surface.
    if data.get("SpawnY").and_then(Tag::as_integer) == Some(32767) {
        data.insert("SpawnY", Tag::Int(-1));
    }
    if data.get("GameType").and_then(Tag::as_integer).is_some_and(|t| t > 3) {
        data.insert("GameType", Tag::Int(3));
    }

    if fmt.world_gen_settings {
        // The seed and the structure switches live in the generator settings.
        data.remove("MapFeatures");
        data.remove("BonusChest");
    }
    if fmt.dragon_fight_compound {
        data.get_or_create_compound("DragonFight");
    }
    if fmt.spawn_compound {
        spawn_and_border_compounds(data);
    }
    if fmt.game_rule_registry_keys {
        game_rules_to_registry(data);
    }
}

/// 1.21.9: `spawn{pos, pitch, yaw, dimension}` and `world_border{...}`.
fn spawn_and_border_compounds(data: &mut Compound) {
    let mut spawn = match data.remove("spawn") {
        Some(Tag::Compound(c)) => c,
        _ => Compound::new(),
    };
    if let (Some(x), Some(y), Some(z)) = (
        data.remove("SpawnX").and_then(|t| t.as_integer()),
        data.remove("SpawnY").and_then(|t| t.as_integer()),
        data.remove("SpawnZ").and_then(|t| t.as_integer()),
    ) {
        spawn.insert("pos", Tag::IntArray(vec![x as i32, y as i32, z as i32]));
    } else if !spawn.contains_key("pos") {
        spawn.insert("pos", Tag::IntArray(vec![0, 0, 0]));
    }
    for (old, new, default) in [("SpawnPitch", "pitch", Tag::Float(0.0)), ("SpawnYaw", "yaw", Tag::Float(0.0))] {
        match data.remove(old) {
            Some(v) => {
                spawn.insert(new, v);
            }
            None if !spawn.contains_key(new) => {
                spawn.insert(new, default);
            }
            None => {}
        }
    }
    match data.remove("SpawnDimension") {
        Some(v) => {
            spawn.insert("dimension", v);
        }
        None if !spawn.contains_key("dimension") => {
            spawn.insert("dimension", Tag::String("minecraft:overworld".into()));
        }
        None => {}
    }
    data.insert("spawn", Tag::Compound(spawn));

    let mut border = match data.remove("world_border") {
        Some(Tag::Compound(c)) => c,
        _ => Compound::new(),
    };
    let rename: [(&str, &str, Tag); 9] = [
        ("BorderCenterX", "center_x", Tag::Double(0.0)),
        ("BorderCenterZ", "center_z", Tag::Double(0.0)),
        ("BorderSize", "size", Tag::Double(5.999997E7)),
        ("BorderSizeLerpTime", "lerp_time", Tag::Long(0)),
        ("BorderSizeLerpTarget", "lerp_target", Tag::Double(0.0)),
        ("BorderSafeZone", "safe_zone", Tag::Double(5.0)),
        ("BorderDamagePerBlock", "damage_per_block", Tag::Double(0.2)),
        ("BorderWarningBlocks", "warning_blocks", Tag::Int(5)),
        ("BorderWarningTime", "warning_time", Tag::Int(15)),
    ];
    for (old, new, default) in rename {
        match data.remove(old) {
            Some(v) => {
                // The warning distances became ints.
                let v = if matches!(new, "warning_blocks" | "warning_time") {
                    Tag::Int(v.as_number().unwrap_or(0.0) as i32)
                } else {
                    v
                };
                border.insert(new, v);
            }
            None if !border.contains_key(new) => {
                border.insert(new, default);
            }
            None => {}
        }
    }
    data.insert("world_border", Tag::Compound(border));
}

/// 1.21.11: `GameRules` (strings) become `game_rules` with registry keys and typed values.
fn game_rules_to_registry(data: &mut Compound) {
    let old = match data.remove("GameRules") {
        Some(Tag::Compound(c)) => c,
        _ => Compound::new(),
    };
    let mut new = match data.remove("game_rules") {
        Some(Tag::Compound(c)) => c,
        _ => Compound::new(),
    };
    let text = |name: &str| old.get(name).and_then(|t| t.as_str()).map(str::to_string);
    let flag = |s: &str| s.eq_ignore_ascii_case("true");
    for (name, key) in [
        ("announceAdvancements", "minecraft:show_advancement_messages"),
        ("commandBlockOutput", "minecraft:command_block_output"),
        ("doDaylightCycle", "minecraft:advance_time"),
        ("doEntityDrops", "minecraft:entity_drops"),
        ("doLimitedCrafting", "minecraft:limited_crafting"),
        ("doMobLoot", "minecraft:mob_drops"),
        ("doMobSpawning", "minecraft:spawn_mobs"),
        ("doTileDrops", "minecraft:block_drops"),
        ("doWeatherCycle", "minecraft:advance_weather"),
        ("keepInventory", "minecraft:keep_inventory"),
        ("logAdminCommands", "minecraft:log_admin_commands"),
        ("mobGriefing", "minecraft:mob_griefing"),
        ("naturalRegeneration", "minecraft:natural_health_regeneration"),
        ("reducedDebugInfo", "minecraft:reduced_debug_info"),
        ("sendCommandFeedback", "minecraft:send_command_feedback"),
        ("showDeathMessages", "minecraft:show_death_messages"),
        ("spectatorsGenerateChunks", "minecraft:spectators_generate_chunks"),
    ] {
        if let Some(v) = text(name) {
            new.insert(key, Tag::Byte(flag(&v) as i8));
        }
    }
    if let Some(v) = text("disableElytraMovementCheck") {
        new.insert("minecraft:elytra_movement_check", Tag::Byte(!flag(&v) as i8));
    }
    // `doFireTick` is only written when fire is switched off.
    if text("doFireTick").is_some_and(|v| v == "false") {
        new.insert("minecraft:fire_spread_radius_around_player", Tag::Int(0));
    }
    for (name, key) in [
        ("maxEntityCramming", "minecraft:max_entity_cramming"),
        ("maxCommandChainLength", "minecraft:max_command_sequence_length"),
        ("randomTickSpeed", "minecraft:random_tick_speed"),
        ("spawnRadius", "minecraft:respawn_radius"),
    ] {
        if let Some(v) = text(name).and_then(|v| v.trim().parse::<i32>().ok()) {
            new.insert(key, Tag::Int(v));
        }
    }
    data.insert("game_rules", Tag::Compound(new));
}

/// 26.1: split the level data into the files the game reads (`v26_1` `writeExtraLevelSettings`).
fn split_level_dat(f: &LevelFiles<'_>, data: &mut Compound) -> Result<(), ConvertError> {
    let difficulty = match data.get("Difficulty").and_then(Tag::as_integer).unwrap_or(2) {
        0 => "peaceful",
        1 => "easy",
        3 => "hard",
        _ => "normal",
    };
    let mut ds = Compound::with_capacity(3);
    ds.insert("difficulty", Tag::String(difficulty.into()));
    ds.insert("locked", Tag::Byte(data.get("DifficultyLocked").and_then(Tag::as_integer).unwrap_or(0) as i8));
    ds.insert("hardcore", Tag::Byte(data.get("hardcore").and_then(Tag::as_integer).unwrap_or(0) as i8));
    data.insert("difficulty_settings", Tag::Compound(ds));
    data.remove("Difficulty");
    data.remove("DifficultyLocked");
    data.remove("hardcore");

    if let Some(v) = data.remove("DragonFight") {
        data.insert("dragon_fight", v);
    }
    if let Some(Tag::Compound(mut wgs)) = data.remove("WorldGenSettings") {
        if let Some(v) = wgs.remove("generate_features") {
            wgs.insert("generate_structures", v);
        }
        data.insert("world_gen_settings", Tag::Compound(wgs));
    }

    let mut weather = Compound::with_capacity(5);
    let int = |data: &Compound, key: &str| data.get(key).and_then(Tag::as_integer).unwrap_or(0);
    weather.insert("clear_weather_time", Tag::Int(int(data, "clearWeatherTime") as i32));
    weather.insert("rain_time", Tag::Int(int(data, "rainTime") as i32));
    weather.insert("thunder_time", Tag::Int(int(data, "thunderTime") as i32));
    weather.insert("raining", Tag::Byte(int(data, "raining") as i8));
    weather.insert("thundering", Tag::Byte(int(data, "thundering") as i8));
    for k in ["clearWeatherTime", "rainTime", "thunderTime", "raining", "thundering"] {
        data.remove(k);
    }

    let day_time = data.get("DayTime").and_then(Tag::as_integer).or_else(|| data.get("Time").and_then(Tag::as_integer));
    let mut clock = Compound::with_capacity(2);
    clock.insert("total_ticks", Tag::Long(day_time.unwrap_or(0)));
    clock.insert("paused", Tag::Byte(0));
    let mut clocks = Compound::with_capacity(1);
    clocks.insert("minecraft:overworld", Tag::Compound(clock));
    data.remove("DayTime");

    let dirs = f.dirs;
    let global = dirs.minecraft_data_dir();
    write_data_file(f, &global.join("weather.dat"), weather, true)?;
    if let Some(Tag::Compound(c)) = data.remove("dragon_fight") {
        write_data_file(f, &dirs.dimension_data_dir(&JavaDimension::End).join("ender_dragon_fight.dat"), c, true)?;
    }
    if let Some(Tag::Compound(c)) = data.remove("game_rules") {
        write_data_file(f, &global.join("game_rules.dat"), c, true)?;
    }
    if let Some(Tag::Compound(c)) = data.remove("world_gen_settings") {
        write_data_file(f, &global.join("world_gen_settings.dat"), c, true)?;
    }
    write_data_file(f, &global.join("world_clocks.dat"), clocks, true)?;

    if let Some(Tag::Compound(border)) = data.remove("world_border") {
        let mut dims: Vec<JavaDimension> = vec![JavaDimension::Overworld, JavaDimension::Nether, JavaDimension::End];
        for d in f.settings.dimension_registry.dimensions().chain(f.dimensions.iter()) {
            if let Some(j) = JavaDimension::from_identifier(d.identifier()) {
                if !dims.contains(&j) {
                    dims.push(j);
                }
            }
        }
        for dim in dims {
            let mut b = border.clone();
            if dim == JavaDimension::Nether {
                for key in ["center_x", "center_z"] {
                    if let Some(v) = get_f64(&border, key) {
                        b.insert(key, Tag::Double(v / 8.0));
                    }
                }
            }
            write_data_file(f, &dirs.dimension_data_dir(&dim).join("world_border.dat"), b, true)?;
        }
    }

    if let Some(Tag::Compound(player)) = data.remove("Player") {
        if !player.is_empty() {
            let uuid = match data.get("singleplayer_uuid") {
                Some(Tag::IntArray(ints)) if ints.len() == 4 => ints.clone(),
                _ => vec![0, 0, 0, 0],
            };
            let name =
                LevelDirs::uuid_from_ints(&uuid).unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".into());
            write_data_file(f, &dirs.players_dir().join(format!("{name}.dat")), player, false)?;
            data.insert("singleplayer_uuid", Tag::IntArray(uuid));
        }
    }
    Ok(())
}

/// Nearest-colour conversion of a map's RGBA pixels to Java colour ids (`JavaMapColorsResolver`).
fn map_colors(f: &LevelFiles<'_>, rgba: &[u8]) -> Vec<i8> {
    const SHADES: [u32; 4] = [180, 220, 255, 135];
    let v = f.format.version;
    let base = tables::map_colors(
        Edition::Java,
        MapVersion::new(v.major.max(0) as u32, v.minor.max(0) as u32, v.patch.max(0) as u32),
    );
    let mut shaded: Vec<[i64; 4]> = Vec::with_capacity(base.len() * 4);
    for c in &base {
        for s in SHADES {
            shaded.push([
                (c[0] as u32 * s / 255) as i64,
                (c[1] as u32 * s / 255) as i64,
                (c[2] as u32 * s / 255) as i64,
                c[3] as i64,
            ]);
        }
    }
    let mut cache: std::collections::HashMap<[u8; 4], i8> = std::collections::HashMap::new();
    rgba.chunks_exact(4)
        .map(|px| {
            let key = [px[0], px[1], px[2], px[3]];
            *cache.entry(key).or_insert_with(|| {
                let (mut best, mut best_score) = (0usize, i64::MAX);
                for (i, c) in shaded.iter().enumerate() {
                    let score = (0..4).map(|k| (c[k] - key[k] as i64).pow(2)).sum::<i64>();
                    if score < best_score {
                        (best, best_score) = (i, score);
                    }
                }
                best as u8 as i8
            })
        })
        .collect()
}

fn write_maps(f: &LevelFiles<'_>) -> Result<(), ConvertError> {
    let maps: &[LevelMap] = &f.level.maps;
    if maps.is_empty() {
        return Ok(());
    }
    let fmt = f.format;
    fs::create_dir_all(f.dirs.maps_dir()).map_err(io_error)?;
    // Java ids are numbers from 0 in order.
    let max_id = maps.len() as i64 - 1;
    let mut counter = Compound::with_capacity(1);
    if fmt.maps_in_registry_dir {
        counter.insert("data", Tag::Compound(Compound::new()));
        if let Some(Tag::Compound(inner)) = counter.get_mut("data") {
            inner.insert("map", Tag::Int(max_id as i32));
        }
        write_java_file(
            f.dirs.map_counter_file(),
            &{
                let mut root = counter;
                root.insert("DataVersion", Tag::Int(fmt.row_data_version));
                root
            },
            Compression::Gzip,
        )
        .map_err(nbt_error)?;
    } else {
        counter.insert("map", Tag::Short(max_id.clamp(0, i16::MAX as i64) as i16));
        write_java_file(f.dirs.map_counter_file(), &counter, Compression::None).map_err(nbt_error)?;
    }
    for (id, map) in maps.iter().enumerate() {
        let mut data = match (&map.original_nbt, f.settings.allow_nbt_copying) {
            (Some(original), true) => original.clone(),
            _ => Compound::with_capacity(11),
        };
        if fmt.dimension_ids_as_strings {
            data.insert("dimension", Tag::String(map.dimension.identifier().to_string()));
        } else {
            data.insert("dimension", Tag::Byte(map.dimension.java_id().unwrap_or(0) as i8));
        }
        data.insert("width", Tag::Short(map.width as i16));
        data.insert("height", Tag::Short(map.height as i16));
        data.insert("xCenter", Tag::Int(map.x_center));
        data.insert("zCenter", Tag::Int(map.z_center));
        data.insert("trackingPosition", Tag::Byte(map.unlimited_tracking as i8));
        data.insert("unlimitedTracking", Tag::Byte(map.unlimited_tracking as i8));
        data.insert("locked", Tag::Byte(map.locked as i8));
        data.insert("scale", Tag::Byte(map.scale));
        data.insert("banners", Tag::List(Vec::new()));
        if let Some(bytes) = &map.bytes {
            data.insert("colors", Tag::ByteArray(map_colors(f, bytes)));
        }
        let mut root = Compound::with_capacity(2);
        root.insert("data", Tag::Compound(data));
        root.insert("DataVersion", Tag::Int(fmt.row_data_version));
        write_java_file(f.dirs.map_file(id as i64), &root, Compression::Gzip).map_err(nbt_error)?;
    }
    Ok(())
}

/// The 1.17 data pack that lifts the overworld to 384 blocks.
fn write_height_pack(f: &LevelFiles<'_>) -> Result<(), ConvertError> {
    use std::io::Write;
    let dir = f.dirs.datapacks_dir();
    fs::create_dir_all(&dir).map_err(io_error)?;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name: &str, text: &str| -> Result<(), zip::result::ZipError> {
        zip.start_file(name, options)?;
        zip.write_all(text.as_bytes())?;
        Ok(())
    };
    let zip_error = |e: zip::result::ZipError| ConvertError::Internal(format!("could not build the height pack: {e}"));
    add("pack.mcmeta", HEIGHT_PACK_META).map_err(zip_error)?;
    add("data/minecraft/dimension_type/overworld.json", &height_dimension_type(false)).map_err(zip_error)?;
    add("data/minecraft/dimension_type/overworld_caves.json", &height_dimension_type(true)).map_err(zip_error)?;
    let bytes = zip.finish().map_err(zip_error)?.into_inner();
    fs::write(dir.join(HEIGHT_PACK_NAME), bytes).map_err(io_error)
}

/// `icon.png`: any picture the image decoders know, as the 64x64 PNG Java wants.
fn write_icon(f: &LevelFiles<'_>, image_bytes: &[u8]) -> Result<(), ConvertError> {
    let Ok(image) = image::load_from_memory(image_bytes) else { return Ok(()) };
    let small = image.resize_exact(64, 64, image::imageops::FilterType::CatmullRom);
    let mut png = std::io::Cursor::new(Vec::new());
    if small.write_to(&mut png, image::ImageFormat::Png).is_err() {
        return Ok(());
    }
    fs::write(f.dirs.root().join("icon.png"), png.into_inner()).map_err(io_error)
}

/// Write `level.dat` (last, so an unfinished world has none) and everything beside it.
pub(super) fn write_level_files(f: &LevelFiles<'_>) -> Result<(), ConvertError> {
    let fmt = f.format;
    fs::create_dir_all(f.dirs.root()).map_err(io_error)?;
    let copying = f.settings.allow_nbt_copying && f.level.original_level_data.is_some();
    let mut data = match (&f.level.original_level_data, copying) {
        (Some(original), true) => original.clone(),
        _ => Compound::with_capacity(100),
    };
    let io = JavaSettingsIo { f, copying, warnings: Mutex::new(Vec::new()) };
    f.level.settings.to_nbt(&mut data, &io);
    write_player(f, &mut data);
    write_player_files(f, &data)?;
    extra_settings(f, &mut data);
    if fmt.split_level_dat {
        split_level_dat(f, &mut data)?;
    }
    if fmt.caves_and_cliffs == CavesAndCliffs::Datapack && f.tall_overworld && !copying {
        write_height_pack(f)?;
    }
    write_maps(f)?;
    if let Some(icon) = f.icon {
        write_icon(f, icon)?;
    }
    // Modern worlds keep an `output` of the settings in `Data`.
    let mut root = Compound::with_capacity(1);
    root.insert("Data", Tag::Compound(data));
    write_java_file(f.dirs.level_dat(), &root, Compression::Gzip).map_err(nbt_error)
}

#[allow(dead_code)]
fn _kind_is_used(_: SettingKind) {}
