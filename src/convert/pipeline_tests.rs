//! Tests of the whole conversion on a small Bedrock world held in memory: players (owner, ender chest, stats, other
//! profiles, inventory source), map links on every kind of item and the level setting overrides, written for 26.3
//! (components, `players/data/`) and 1.20.4 (`tag`, `Data.Player`, `playerdata/`).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use uuid::Uuid;

use super::*;
use crate::convert::bedrock::db::{BedrockDb, MemDb};
use crate::convert::bedrock::keys::{self, ChunkTag, DimensionId, chunk_key, sub_chunk_key};
use crate::convert::bedrock::level_dat::LevelDat;
use crate::convert::bedrock::nbt_le;
use crate::convert::bedrock::palette::{BlockStateRaw, RawPalette, RawSubChunk, encode_nbt_concat, encode_sub_chunk};
use crate::convert::java::dirs::{JavaDimension, LevelDirs};
use crate::convert::java::format::JavaFormat;
use crate::convert::java::level_dat::read_level_dat;
use crate::convert::java::region::RegionFile;
use crate::convert::java::testutil::TestDir;
use crate::convert::level::{
    Difficulty, EncodingType, InventorySource, LevelOverrides, NullSink, PlayerOptions, TargetVersion, VersionTriple,
};
use crate::convert::nbt::file::{Compression, read_java_file};
use crate::convert::nbt::{Compound, Tag};

const OWNER: &str = "0822296f-ba99-4e86-8126-8d1a3b0403ef";
const GUEST: &str = "874d8b7a-1111-4222-8333-444455556666";

fn item(name: &str, count: i8, slot: i8) -> Compound {
    Compound::new()
        .with("Name", name)
        .with("Count", Tag::Byte(count))
        .with("Damage", Tag::Short(0))
        .with("Slot", Tag::Byte(slot))
}

fn map_item(map_uuid: i64, slot: i8) -> Compound {
    item("minecraft:filled_map", 1, slot).with("tag", Compound::new().with("map_uuid", Tag::Long(map_uuid)))
}

/// A shulker box holding a map.
fn shulker(slot: i8, map_uuid: i64) -> Compound {
    let items = Tag::List(vec![Tag::Compound(map_item(map_uuid, 0))]);
    item("minecraft:white_shulker_box", 1, slot).with("tag", Compound::new().with("Items", items))
}

fn list(items: Vec<Compound>) -> Tag {
    Tag::List(items.into_iter().map(Tag::Compound).collect())
}

fn le(root: Compound) -> Vec<u8> {
    nbt_le::to_vec("", &Tag::Compound(root)).unwrap()
}

fn attribute(name: &str, current: f32) -> Compound {
    Compound::new().with("Name", name).with("Current", current)
}

fn host() -> Compound {
    Compound::new()
        .with("Pos", Tag::List(vec![Tag::Float(20.5), Tag::Float(71.62001), Tag::Float(-30.5)]))
        .with("Rotation", Tag::List(vec![Tag::Float(0.0), Tag::Float(0.0)]))
        .with("DimensionId", 0i32)
        .with("PlayerGameMode", 0i32)
        .with("UniqueID", Tag::Long(-4294967295))
        .with("Inventory", list(vec![map_item(10, 3)]))
        .with("Armor", list(vec![item("minecraft:iron_helmet", 1, 0)]))
        .with("EnderChestInventory", list(vec![map_item(20, 0), shulker(1, 10)]))
        .with("PlayerLevel", 30i32)
        .with("PlayerLevelProgress", 0.5f32)
        .with(
            "Attributes",
            list(vec![
                attribute("minecraft:health", 13.5),
                attribute("minecraft:player.hunger", 17.0),
                attribute("minecraft:player.saturation", 2.5),
                attribute("minecraft:player.exhaustion", 0.5),
            ]),
        )
        .with("SpawnBlockPositionX", 102i32)
        .with("SpawnBlockPositionY", 67i32)
        .with("SpawnBlockPositionZ", -359i32)
        .with("SpawnDimension", 0i32)
        .with(
            "ActiveEffects",
            list(vec![
                Compound::new()
                    .with("Id", Tag::Byte(1))
                    .with("Amplifier", Tag::Byte(1))
                    .with("Duration", 1200i32)
                    .with("Ambient", Tag::Byte(0))
                    .with("ShowParticles", Tag::Byte(1)),
            ]),
        )
        .with("abilities", Compound::new().with("mayfly", Tag::Byte(0)).with("walkSpeed", 0.1f32))
}

fn guest() -> Compound {
    Compound::new()
        .with("Pos", Tag::List(vec![Tag::Float(-100.5), Tag::Float(65.62001), Tag::Float(4.5)]))
        .with("DimensionId", 0i32)
        .with("Inventory", list(vec![item("minecraft:diamond", 5, 0), item("minecraft:apple", 2, 3)]))
        .with("EnderChestInventory", list(vec![item("minecraft:emerald", 9, 4)]))
        .with("PlayerLevel", 164i32)
}

fn bedrock_map(x_center: i32) -> Vec<u8> {
    le(Compound::new()
        .with("width", Tag::Short(128))
        .with("height", Tag::Short(128))
        .with("scale", Tag::Byte(0))
        .with("dimension", Tag::Byte(0))
        .with("xCenter", x_center)
        .with("zCenter", 0i32))
}

/// One overworld column of stone with a chest holding a map and a shulker box with a map; maps -5, 10 and 20 (Java
/// ids 0, 1 and 2); the host and one other player.
fn world() -> MemDb {
    let mut db = MemDb::new();
    let ow = DimensionId::OVERWORLD;
    let stone = BlockStateRaw::new("minecraft:stone", Compound::new(), 18168865);
    let sub = RawSubChunk { version: 9, y: 0, layers: vec![RawPalette::uniform(stone)] };
    db.put(&sub_chunk_key(ow, 1, -2, 0), &encode_sub_chunk(&sub).unwrap()).unwrap();
    let chest = Compound::new()
        .with("id", "Chest")
        .with("x", 16i32)
        .with("y", 5i32)
        .with("z", -32i32)
        .with("Items", list(vec![map_item(20, 0), shulker(1, 10)]));
    db.put(&chunk_key(ow, 1, -2, ChunkTag::BlockEntity), &encode_nbt_concat(&[Tag::Compound(chest)]).unwrap()).unwrap();
    db.put(&chunk_key(ow, 1, -2, ChunkTag::Version), &[42]).unwrap();
    for (id, x) in [(-5i64, 0), (10, 128), (20, 256)] {
        db.put(format!("map_{id}").as_bytes(), &bedrock_map(x)).unwrap();
    }
    db.put(keys::LOCAL_PLAYER, &le(host())).unwrap();
    db.put(format!("player_server_{GUEST}").as_bytes(), &le(guest())).unwrap();
    let identity = Compound::new().with("MsaId", "msa-hash").with("ServerId", format!("player_server_{GUEST}"));
    db.put(b"player_msa-hash", &le(identity)).unwrap();
    db
}

fn level_dat() -> LevelDat {
    let root = Compound::new()
        .with("LevelName", "Players")
        .with("StorageVersion", 10i32)
        .with("Difficulty", 0i32)
        .with("GameType", 0i32)
        .with("commandsEnabled", Tag::Byte(0));
    LevelDat { storage_version: 10, root_name: String::new(), root }
}

/// Convert the synthetic world to `version` into `out`.
fn convert(settings: ConversionSettings, version: &str, out: &std::path::Path) -> ConvertSummary {
    let reader = BedrockWorldReader::from_memory(world(), level_dat(), [1, 26, 50], &settings).unwrap();
    let req = ConvertRequest {
        input: out.join("unused"),
        output: out.to_path_buf(),
        target: TargetVersion::new(EncodingType::Java, VersionTriple::from_string(version).unwrap()),
        settings,
    };
    let cancel: CancelToken = Arc::new(AtomicBool::new(false));
    convert_opened(&reader, &req, &NullSink, &cancel, Instant::now()).unwrap()
}

fn format_of(version: &str) -> JavaFormat {
    let v = VersionTriple::from_string(version).unwrap();
    JavaFormat::for_version(Version::new(v.major() as i32, v.minor() as i32, v.patch() as i32))
}

fn compound<'a>(c: &'a Compound, key: &str) -> &'a Compound {
    c.get(key).and_then(Tag::as_compound).unwrap_or_else(|| panic!("no {key} in {c:?}"))
}

fn items<'a>(c: &'a Compound, key: &str) -> Vec<&'a Compound> {
    c.get(key)
        .and_then(Tag::as_list)
        .unwrap_or_else(|| panic!("no {key} in {c:?}"))
        .iter()
        .filter_map(Tag::as_compound)
        .collect()
}

fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_integer)
}

fn in_slot<'a>(list: &[&'a Compound], slot: i64) -> &'a Compound {
    list.iter().find(|i| int(i, "Slot") == Some(slot)).unwrap_or_else(|| panic!("nothing in slot {slot}: {list:?}"))
}

/// The map id of a `filled_map` in either item format.
fn map_id(item: &Compound) -> Option<i64> {
    assert_eq!(item.get("id").and_then(Tag::as_str), Some("minecraft:filled_map"), "{item:?}");
    match item.get_compound("components") {
        Some(c) => int(c, "minecraft:map_id"),
        None => int(item.get_compound("tag")?, "map"),
    }
}

/// The first item inside a shulker box item, in either format.
fn shulker_content(item: &Compound) -> &Compound {
    match item.get_compound("components") {
        Some(c) => items(c, "minecraft:container")[0].get_compound("item").unwrap(),
        None => items(compound(compound(item, "tag"), "BlockEntityTag"), "Items")[0],
    }
}

fn uuid_tag(uuid: &str) -> Tag {
    let v = Uuid::parse_str(uuid).unwrap().as_u128();
    Tag::IntArray(vec![(v >> 96) as i32, (v >> 64) as i32, (v >> 32) as i32, v as i32])
}

fn read_player(path: std::path::PathBuf) -> Compound {
    read_java_file(&path, Compression::Gzip).unwrap_or_else(|e| panic!("{}: {e}", path.display())).unwrap()
}

fn chest_items(out: &std::path::Path, fmt: &JavaFormat) -> Vec<Compound> {
    let dirs = LevelDirs::new(out, fmt);
    let mut region = RegionFile::open_read(dirs.region_dir(&JavaDimension::Overworld).join("r.0.-1.mca")).unwrap();
    let chunk = region.read_chunk(1, 30).unwrap().unwrap();
    let chunk = if fmt.level_wrapper { compound(&chunk, "Level").clone() } else { chunk };
    let bes = items(&chunk, fmt.block_entities_key);
    assert_eq!(bes.len(), 1, "{bes:?}");
    items(bes[0], "Items").into_iter().cloned().collect()
}

fn owner_settings() -> ConversionSettings {
    let mut s = ConversionSettings::default();
    s.player.owner_uuid = Some(Uuid::parse_str(OWNER).unwrap());
    s.level_overrides =
        LevelOverrides { difficulty: Some(Difficulty::Hard), allow_commands: Some(true), ..LevelOverrides::default() };
    s
}

#[test]
fn maps_keep_their_links_in_containers_and_the_player() {
    for version in ["26.3", "1.20.4", "1.18.2"] {
        let dir = TestDir::new("pipeline-maps");
        let out = dir.path().join("w");
        convert(owner_settings(), version, &out);
        let fmt = format_of(version);
        // Maps -5, 10, 20 are written as 0, 1, 2.
        let chest = chest_items(&out, &fmt);
        let refs: Vec<&Compound> = chest.iter().collect();
        assert_eq!(map_id(in_slot(&refs, 0)), Some(2), "{version}");
        assert_eq!(map_id(shulker_content(in_slot(&refs, 1))), Some(1), "{version}");

        let player = if fmt.split_level_dat {
            read_player(LevelDirs::new(&out, &fmt).players_dir().join(format!("{OWNER}.dat")))
        } else {
            compound(&read_level_dat(&out.join("level.dat")).unwrap().data, "Player").clone()
        };
        assert_eq!(map_id(in_slot(&items(&player, "Inventory"), 3)), Some(1), "{version}");
        let ender = items(&player, "EnderItems");
        assert_eq!(map_id(in_slot(&ender, 0)), Some(2), "{version}");
        assert_eq!(map_id(shulker_content(in_slot(&ender, 1))), Some(1), "{version}");
    }
}

#[test]
fn owner_player_and_settings_on_26_3() {
    let dir = TestDir::new("pipeline-owner-26");
    let out = dir.path().join("w");
    let summary = convert(owner_settings(), "26.3", &out);
    assert!(summary.warnings.iter().all(|w| !w.contains("profile")), "{:?}", summary.warnings);
    let fmt = format_of("26.3");
    let dirs = LevelDirs::new(&out, &fmt);
    let data = read_level_dat(&out.join("level.dat")).unwrap().data;
    // The owner names the file, level.dat points at it, the player carries it.
    assert_eq!(data.get("singleplayer_uuid"), Some(&uuid_tag(OWNER)));
    assert!(!dirs.players_dir().join("00000000-0000-0000-0000-000000000000.dat").exists());
    let player = read_player(dirs.players_dir().join(format!("{OWNER}.dat")));
    assert_eq!(player.get("UUID"), Some(&uuid_tag(OWNER)));
    // Stats.
    assert_eq!(int(&player, "XpLevel"), Some(30));
    assert_eq!(int(&player, "XpTotal"), Some(1395 + 56));
    assert_eq!(player.get("Health"), Some(&Tag::Float(13.5)));
    assert_eq!(int(&player, "foodLevel"), Some(17));
    assert_eq!(player.get("foodSaturationLevel"), Some(&Tag::Float(2.5)));
    let respawn = compound(&player, "respawn");
    assert_eq!(respawn.get("pos"), Some(&Tag::IntArray(vec![102, 67, -359])));
    assert_eq!(respawn.get_str("dimension"), Some("minecraft:overworld"));
    assert!(respawn.contains_key("yaw") && respawn.contains_key("pitch") && !respawn.contains_key("angle"));
    let effects = items(&player, "active_effects");
    assert_eq!(effects[0].get_str("id"), Some("minecraft:speed"));
    assert_eq!((int(effects[0], "amplifier"), int(effects[0], "duration")), (Some(1), Some(1200)));
    assert_eq!(compound(&player, "abilities").get("walkSpeed"), Some(&Tag::Float(0.1)));
    // Armor sits in `equipment` since 1.21.5.
    assert_eq!(compound(compound(&player, "equipment"), "head").get_str("id"), Some("minecraft:iron_helmet"));
    // The level overrides, difficulty in its 26.1 place.
    let ds = compound(&data, "difficulty_settings");
    assert_eq!(ds.get_str("difficulty"), Some("hard"));
    assert_eq!(int(ds, "hardcore"), Some(0));
    assert_eq!(int(&data, "allowCommands"), Some(1));
    assert_eq!(int(&data, "GameType"), Some(0));
    // The other player keeps its own uuid, inventory, ender chest and level.
    let guest = read_player(dirs.players_dir().join(format!("{GUEST}.dat")));
    assert_eq!(guest.get("UUID"), Some(&uuid_tag(GUEST)));
    assert_eq!(int(&guest, "XpLevel"), Some(164));
    assert_eq!(items(&guest, "Inventory").len(), 2);
    assert_eq!(in_slot(&items(&guest, "EnderItems"), 4).get_str("id"), Some("minecraft:emerald"));
    assert_eq!(std::fs::read_dir(dirs.players_dir()).unwrap().count(), 2);
}

#[test]
fn owner_player_and_settings_before_26_1() {
    let dir = TestDir::new("pipeline-owner-1204");
    let out = dir.path().join("w");
    let mut settings = owner_settings();
    settings.level_overrides.hardcore = Some(true);
    settings.level_overrides.game_type = Some(crate::convert::level::GameType::Creative);
    convert(settings, "1.20.4", &out);
    let fmt = format_of("1.20.4");
    let dirs = LevelDirs::new(&out, &fmt);
    let data = read_level_dat(&out.join("level.dat")).unwrap().data;
    assert!(!data.contains_key("singleplayer_uuid"));
    assert_eq!((int(&data, "Difficulty"), int(&data, "hardcore")), (Some(3), Some(1)));
    assert_eq!((int(&data, "allowCommands"), int(&data, "GameType")), (Some(1), Some(1)));
    // The host lives in level.dat and in the owner's player file, which carry the same data.
    let player = compound(&data, "Player");
    assert_eq!(player.get("UUID"), Some(&uuid_tag(OWNER)));
    assert_eq!(int(player, "playerGameType"), Some(1));
    let file = read_player(dirs.players_dir().join(format!("{OWNER}.dat")));
    assert_eq!(file.get("EnderItems"), player.get("EnderItems"));
    assert_eq!(file.get("UUID"), Some(&uuid_tag(OWNER)));
    // Spawn as SpawnX/Y/Z, effects by name (1.20.2+), armor in slot 103.
    assert_eq!(
        (int(player, "SpawnX"), int(player, "SpawnY"), int(player, "SpawnZ")),
        (Some(102), Some(67), Some(-359))
    );
    assert_eq!(player.get_str("SpawnDimension"), Some("minecraft:overworld"));
    assert!(!player.contains_key("respawn"));
    assert_eq!(items(player, "active_effects")[0].get_str("id"), Some("minecraft:speed"));
    assert_eq!(in_slot(&items(player, "Inventory"), 103).get_str("id"), Some("minecraft:iron_helmet"));
    assert!(dirs.players_dir().join(format!("{GUEST}.dat")).is_file());
}

#[test]
fn player_formats_by_version() {
    let player_of = |version: &str| {
        let dir = TestDir::new("pipeline-formats");
        let out = dir.path().join("w");
        convert(owner_settings(), version, &out);
        compound(&read_level_dat(&out.join("level.dat")).unwrap().data, "Player").clone()
    };
    // 1.15: uuid as two longs, the spawn without a dimension, effects by number.
    let p = player_of("1.15.2");
    assert!(int(&p, "UUIDMost").is_some() && int(&p, "UUIDLeast").is_some() && !p.contains_key("UUID"));
    assert_eq!(int(&p, "SpawnX"), Some(102));
    assert!(!p.contains_key("SpawnDimension") && !p.contains_key("SpawnAngle"));
    let effect = items(&p, "ActiveEffects")[0];
    assert_eq!((int(effect, "Id"), int(effect, "Amplifier"), int(effect, "Duration")), (Some(1), Some(1), Some(1200)));
    // 1.16 to 1.21.4: the spawn's dimension and angle.
    let p = player_of("1.19.2");
    assert_eq!(p.get("UUID"), Some(&uuid_tag(OWNER)));
    assert_eq!(p.get_str("SpawnDimension"), Some("minecraft:overworld"));
    assert_eq!(p.get("SpawnAngle"), Some(&Tag::Float(0.0)));
    assert!(p.contains_key("ActiveEffects"));
    // 1.21.5: `respawn` with an angle; 1.21.9 swapped it for yaw and pitch (checked in the 26.3 test).
    let p = player_of("1.21.5");
    let respawn = compound(&p, "respawn");
    assert_eq!(respawn.get("pos"), Some(&Tag::IntArray(vec![102, 67, -359])));
    assert!(respawn.contains_key("angle") && !respawn.contains_key("yaw"));
    assert!(!p.contains_key("SpawnX"));
}

#[test]
fn defaults_keep_the_host_anonymous_with_everything() {
    let dir = TestDir::new("pipeline-defaults");
    let out = dir.path().join("w");
    convert(ConversionSettings::default(), "26.3", &out);
    let fmt = format_of("26.3");
    let dirs = LevelDirs::new(&out, &fmt);
    let data = read_level_dat(&out.join("level.dat")).unwrap().data;
    assert_eq!(data.get("singleplayer_uuid"), Some(&Tag::IntArray(vec![0; 4])));
    let player = read_player(dirs.players_dir().join("00000000-0000-0000-0000-000000000000.dat"));
    assert!(!player.contains_key("UUID"));
    // Ender chest and stats are on by default; the source's settings are kept.
    assert_eq!(items(&player, "EnderItems").len(), 2);
    assert_eq!(int(&player, "XpLevel"), Some(30));
    assert_eq!(compound(&data, "difficulty_settings").get_str("difficulty"), Some("peaceful"));
    assert_eq!(int(&data, "allowCommands"), Some(0));
    assert!(dirs.players_dir().join(format!("{GUEST}.dat")).is_file());
}

#[test]
fn ender_chest_stats_and_guests_can_be_left_out() {
    let dir = TestDir::new("pipeline-left-out");
    let out = dir.path().join("w");
    let mut settings = ConversionSettings::default();
    settings.player = PlayerOptions {
        include_ender_chest: false,
        include_stats: false,
        export_guest_profiles: false,
        ..Default::default()
    };
    convert(settings, "26.3", &out);
    let dirs = LevelDirs::new(&out, &format_of("26.3"));
    let player = read_player(dirs.players_dir().join("00000000-0000-0000-0000-000000000000.dat"));
    assert!(items(&player, "EnderItems").is_empty());
    assert!(
        !player.contains_key("XpLevel") && !player.contains_key("respawn") && !player.contains_key("active_effects")
    );
    assert_eq!((player.get("Health"), int(&player, "foodLevel")), (Some(&Tag::Float(20.0)), Some(20)));
    // The inventory is still there.
    assert_eq!(items(&player, "Inventory").len(), 1);
    assert_eq!(std::fs::read_dir(dirs.players_dir()).unwrap().count(), 1);
}

#[test]
fn inventory_from_another_profile() {
    let fmt = format_of("26.3");
    // Instead of the host's: items, ender chest and level are the profile's, the place is the host's.
    let dir = TestDir::new("pipeline-profile");
    let out = dir.path().join("w");
    let mut settings = ConversionSettings::default();
    settings.player.inventory_source = InventorySource::Profile(format!("player_server_{GUEST}"));
    settings.player.export_guest_profiles = false;
    let summary = convert(settings, "26.3", &out);
    assert!(summary.warnings.iter().all(|w| !w.contains("profile")), "{:?}", summary.warnings);
    let player = read_player(LevelDirs::new(&out, &fmt).players_dir().join("00000000-0000-0000-0000-000000000000.dat"));
    let inventory = items(&player, "Inventory");
    let ids: Vec<&str> = inventory.iter().filter_map(|i| i.get_str("id")).collect();
    assert_eq!(ids, ["minecraft:diamond", "minecraft:apple"]);
    assert!(!player.get_compound("equipment").is_some_and(|e| e.contains_key("head")));
    assert_eq!(in_slot(&items(&player, "EnderItems"), 4).get_str("id"), Some("minecraft:emerald"));
    assert_eq!(int(&player, "XpLevel"), Some(164));
    assert_eq!(player.get("Health"), Some(&Tag::Float(13.5)));
    let Some(Tag::List(pos)) = player.get("Pos") else { panic!() };
    assert_eq!(pos[0], Tag::Double(20.5));

    // Merged: the host's items stay, the profile's fill the free slots; the rest stays the host's.
    let dir = TestDir::new("pipeline-merge");
    let out = dir.path().join("w");
    let mut settings = ConversionSettings::default();
    settings.player.inventory_source = InventorySource::MergeHostAnd(GUEST.into());
    convert(settings, "26.3", &out);
    let player = read_player(LevelDirs::new(&out, &fmt).players_dir().join("00000000-0000-0000-0000-000000000000.dat"));
    let inventory = items(&player, "Inventory");
    assert_eq!(in_slot(&inventory, 0).get_str("id"), Some("minecraft:diamond"));
    assert_eq!(map_id(in_slot(&inventory, 3)), Some(1));
    // The apple wanted slot 3, which the host's map holds: it took the first free one.
    assert_eq!(in_slot(&inventory, 1).get_str("id"), Some("minecraft:apple"));
    assert_eq!(items(&player, "EnderItems").len(), 2);
    assert_eq!(int(&player, "XpLevel"), Some(30));

    // A profile that is not there keeps the host's, and says so.
    let dir = TestDir::new("pipeline-missing-profile");
    let out = dir.path().join("w");
    let mut settings = ConversionSettings::default();
    settings.player.inventory_source = InventorySource::Profile("nobody".into());
    let summary = convert(settings, "26.3", &out);
    assert!(summary.warnings.iter().any(|w| w.contains("nobody")), "{:?}", summary.warnings);
    let player = read_player(LevelDirs::new(&out, &fmt).players_dir().join("00000000-0000-0000-0000-000000000000.dat"));
    assert_eq!(map_id(in_slot(&items(&player, "Inventory"), 3)), Some(1));
}

#[test]
fn guests_can_be_given_a_java_account() {
    let dir = TestDir::new("pipeline-guest-uuid");
    let out = dir.path().join("w");
    let account = "11111111-2222-3333-4444-555555555555";
    let mut settings = ConversionSettings::default();
    settings.player.guest_uuids.insert(format!("player_server_{GUEST}"), Uuid::parse_str(account).unwrap());
    convert(settings, "1.21.4", &out);
    let dirs = LevelDirs::new(&out, &format_of("1.21.4"));
    assert!(!dirs.players_dir().join(format!("{GUEST}.dat")).exists());
    let guest = read_player(dirs.players_dir().join(format!("{account}.dat")));
    assert_eq!(guest.get("UUID"), Some(&uuid_tag(account)));
    assert_eq!(int(&guest, "XpLevel"), Some(164));
}

#[test]
fn profiles_are_listed_with_their_identity() {
    let reader =
        BedrockWorldReader::from_memory(world(), level_dat(), [1, 26, 50], &ConversionSettings::default()).unwrap();
    let profiles = reader.player_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, GUEST);
    assert!(profiles[0].signed_in());
    assert_eq!((profiles[0].item_count(), profiles[0].player.stats.xp_level), (2, 164));
}
