// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Exact output per kind for 1.16.5, 1.20.4 and 1.21.4 plus the version boundaries, the "every kind" checks and the
//! NBT round trip.

use super::*;
use crate::convert::model::blockentity::*;
use crate::convert::model::{
    BannerPattern, BlockType, DyeColor, EffectType, EntityType, ItemId, ItemStack, ItemType, StructureBlockMode,
    VanillaEntityType,
};
use crate::convert::nbt::{Flavor, Tag, io};
use crate::convert::util::{Version, json_text};

type V = (i32, i32, i32);
const V113: V = (1, 13, 2);
const V1165: V = (1, 16, 5);
const V1204: V = (1, 20, 4);
const V1214: V = (1, 21, 4);
const THREE: [V; 3] = [V1165, V1204, V1214];

fn fmt(v: V) -> JavaFormat {
    let format = JavaFormat::for_version(Version::new(v.0, v.1, v.2));
    assert_eq!(format.version, Version::new(v.0, v.1, v.2), "registered version");
    format
}

fn items_for(f: &JavaFormat) -> JavaItems {
    JavaItems::new(f, f.version)
}

fn write(be: &BlockEntity, v: V) -> Option<Compound> {
    let f = fmt(v);
    write_block_entity(&items_for(&f), &f, be)
}

fn be(data: BlockEntityData) -> BlockEntity {
    BlockEntity::new(1, 2, 3, data)
}

/// `{id, x: 1, y: 2, z: 3}`.
fn base(id: &str) -> Compound {
    Compound::new().with("id", id).with("x", 1).with("y", 2).with("z", 3)
}

fn stack(item: ItemType, amount: i32) -> ItemStack {
    ItemStack::with_amount(item, amount)
}

/// The item compound the item writer gives, with a `Slot` when asked.
fn item(v: V, s: &ItemStack, slot: Option<i8>) -> Compound {
    let f = fmt(v);
    let mut c = items_for(&f).write(s).expect("item");
    if let Some(slot) = slot {
        c.put("Slot", slot);
    }
    c
}

fn list(items: Vec<Compound>) -> Tag {
    Tag::List(items.into_iter().map(Tag::Compound).collect())
}

/// A text component as a JSON string tag (before 1.21.5).
fn text(s: &str) -> Tag {
    Tag::String(json_text::to_json(&json_text::from_text(s)))
}

/// Check the output for each version against `expected(version)`.
fn check(be: &BlockEntity, versions: &[V], expected: impl Fn(V) -> Option<Compound>) {
    for &v in versions {
        assert_eq!(write(be, v), expected(v), "version {v:?}");
    }
}

fn round_trips(c: &Compound) {
    let tag = Tag::Compound(c.clone());
    let bytes = io::to_vec("", &tag, Flavor::Java).expect("encode");
    let (_, back) = io::from_slice(&bytes, Flavor::Java).expect("decode");
    assert_eq!(back, tag);
}

// ---- containers -------------------------------------------------------------------------------------------------

fn chest_like() -> RandomContainer {
    let mut chest = RandomContainer::default();
    chest.container.items.insert(2, stack(ItemType::DIAMOND, 3));
    chest.container.items.insert(0, stack(ItemType::BRICK, 1));
    chest.container.items.insert(5, ItemStack::new(BlockType::AIR));
    chest
}

#[test]
fn chest_items_name_and_loot_table() {
    let mut chest = chest_like();
    chest.loot_table = Some("minecraft:chests/village/village_weaponsmith".into());
    let mut entity = be(BlockEntityData::Chest(chest));
    entity.custom_name = Some(json_text::from_text("Loot"));
    check(&entity, &THREE, |v| {
        let items =
            list(vec![item(v, &stack(ItemType::BRICK, 1), Some(0)), item(v, &stack(ItemType::DIAMOND, 3), Some(2))]);
        let c = base("minecraft:chest")
            .with("Items", items)
            .with("CustomName", text("Loot"))
            .with("LootTable", "minecraft:chests/village/village_weaponsmith");
        Some(c)
    });
}

#[test]
fn loot_table_rename_before_1_14() {
    let mut chest = chest_like();
    chest.loot_table = Some("minecraft:chests/village/village_weaponsmith".into());
    let c = write(&be(BlockEntityData::Chest(chest.clone())), V113).unwrap();
    assert_eq!(c.get_str("LootTable"), Some("minecraft:chests/village_blacksmith"));
    chest.loot_table = Some("minecraft:chests/monster_room".into());
    let c = write(&be(BlockEntityData::Chest(chest.clone())), V1165).unwrap();
    assert!(!c.contains_key("LootTable"), "unsupported loot table");
    chest.loot_table = Some(String::new());
    assert!(!write(&be(BlockEntityData::Chest(chest)), V1165).unwrap().contains_key("LootTable"));
}

#[test]
fn every_container_kind_writes_its_items() {
    let kinds = [
        ("minecraft:trapped_chest", BlockEntityData::TrappedChest(chest_like())),
        ("minecraft:barrel", BlockEntityData::Barrel(chest_like())),
        ("minecraft:dispenser", BlockEntityData::Dispenser(chest_like())),
        ("minecraft:dropper", BlockEntityData::Dropper(chest_like())),
        ("minecraft:shulker_box", BlockEntityData::ShulkerBox(chest_like())),
        ("minecraft:ender_chest", BlockEntityData::EnderChest(chest_like().container)),
        ("minecraft:hopper", BlockEntityData::Hopper(chest_like().container)),
    ];
    for (id, data) in kinds {
        check(&be(data), &THREE, |v| {
            let items = list(vec![
                item(v, &stack(ItemType::BRICK, 1), Some(0)),
                item(v, &stack(ItemType::DIAMOND, 3), Some(2)),
            ]);
            Some(base(id).with("Items", items))
        });
    }
}

#[test]
fn furnace_family_field_names() {
    let furnace = Furnace { container: chest_like().container, burn_time: 100, cook_time: 50, cook_time_total: 200 };
    for (id, data) in [
        ("minecraft:furnace", BlockEntityData::Furnace(furnace.clone())),
        ("minecraft:blast_furnace", BlockEntityData::BlastFurnace(furnace.clone())),
        ("minecraft:smoker", BlockEntityData::Smoker(furnace.clone())),
    ] {
        let entity = be(data);
        let expected = |v: V| {
            let items = list(vec![
                item(v, &stack(ItemType::BRICK, 1), Some(0)),
                item(v, &stack(ItemType::DIAMOND, 3), Some(2)),
            ]);
            let c = base(id).with("Items", items);
            Some(if v == V1214 {
                c.with("lit_time_remaining", 100_i16).with("cooking_time_spent", 50_i16).with("lit_total_time", 200_i16)
            } else {
                c.with("BurnTime", 100_i16).with("CookTime", 50_i16).with("CookTimeTotal", 200_i16)
            })
        };
        // Blast furnaces and smokers exist from 1.14.
        check(&entity, &THREE, expected);
        assert_eq!(write(&entity, V113).is_some(), id == "minecraft:furnace");
    }
    // 26.3 stores them as ints.
    let c = write(&be(BlockEntityData::Furnace(furnace)), (26, 3, 0)).unwrap();
    assert_eq!(c.get("lit_time_remaining"), Some(&Tag::Int(100)));
    assert_eq!(c.get("lit_total_time"), Some(&Tag::Int(200)));
}

#[test]
fn brewing_stand() {
    let mut stand = BrewingStand::default();
    stand.container.items.insert(3, stack(ItemType::COAL, 1));
    stand.brew_time = 400;
    stand.fuel = 12;
    let entity = be(BlockEntityData::BrewingStand(stand));
    check(&entity, &THREE, |v| {
        Some(
            base("minecraft:brewing_stand")
                .with("Items", list(vec![item(v, &stack(ItemType::COAL, 1), Some(3))]))
                .with("BrewTime", 400_i16)
                .with("Fuel", 12_i8),
        )
    });
    let c = write(&entity, (26, 3, 0)).unwrap();
    assert_eq!(c.get("BrewTime"), Some(&Tag::Int(400)));
    assert_eq!(c.get("Fuel"), Some(&Tag::Int(12)));
    assert_eq!(c.get("total_fuel"), Some(&Tag::Int(20)));
}

#[test]
fn crafter_from_1_20_3() {
    let mut crafter = Crafter::default();
    crafter.disabled_slots = 0b1001;
    crafter.crafting_ticks_remaining = 6;
    crafter.container.container.items.insert(1, stack(ItemType::STICK, 2));
    let entity = be(BlockEntityData::Crafter(Box::new(crafter)));
    check(&entity, &THREE, |v| {
        if v == V1165 {
            return None;
        }
        Some(
            base("minecraft:crafter")
                .with("Items", list(vec![item(v, &stack(ItemType::STICK, 2), Some(1))]))
                .with("disabled_slots", vec![0, 3])
                .with("crafting_ticks_remaining", 6),
        )
    });
}

// ---- signs ------------------------------------------------------------------------------------------------------

fn sample_sign() -> Sign {
    let mut sign = Sign::default();
    sign.front.lines = vec![json_text::from_text("Hello"), json_text::from_text("World")];
    sign.front.color = 0xFFB02E26_u32 as i32;
    sign.front.lit = true;
    sign.back.lines = vec![json_text::from_text("Back")];
    sign.waxed = true;
    sign
}

#[test]
fn sign_before_1_20_uses_text1_to_4() {
    let entity = be(BlockEntityData::Sign(Box::new(sample_sign())));
    let c = write(&entity, V1165).unwrap();
    let expected = base("minecraft:sign")
        .with("Text1", text("Hello"))
        .with("Text2", text("World"))
        .with("Text3", text(""))
        .with("Text4", text(""))
        .with("Color", "red");
    assert_eq!(c, expected);
    // The glow flag appears in 1.17.
    let c = write(&entity, (1, 17, 1)).unwrap();
    assert_eq!(c.get("GlowingText"), Some(&Tag::Byte(1)));
}

#[test]
fn sign_from_1_20_has_two_faces() {
    let entity = be(BlockEntityData::Sign(Box::new(sample_sign())));
    check(&entity, &[V1204, V1214], |_| {
        let messages = |lines: [&str; 4]| Tag::List(lines.iter().map(|l| text(l)).collect());
        let front = Compound::new()
            .with("messages", messages(["Hello", "World", "", ""]))
            .with("has_glowing_text", true)
            .with("color", "red");
        let back = Compound::new()
            .with("messages", messages(["Back", "", "", ""]))
            .with("has_glowing_text", false)
            .with("color", "black");
        Some(base("minecraft:sign").with("is_waxed", true).with("front_text", front).with("back_text", back))
    });
}

#[test]
fn hanging_sign_from_1_20() {
    let entity = be(BlockEntityData::HangingSign(Box::new(sample_sign())));
    assert!(write(&entity, V1165).is_none());
    let c = write(&entity, V1204).unwrap();
    assert_eq!(c.get_str("id"), Some("minecraft:hanging_sign"));
    assert!(c.get_compound("front_text").is_some() && c.get_compound("back_text").is_some());
}

#[test]
fn sign_text_is_nbt_from_1_21_5_and_26_3_adds_op_features() {
    let mut sign = sample_sign();
    sign.front.lines = (0..4).map(|i| json_text::from_text(&format!("L{i}"))).collect();
    let c = write(&be(BlockEntityData::Sign(Box::new(sign.clone()))), (1, 21, 5)).unwrap();
    let front = c.get_compound("front_text").unwrap();
    let first = &front.get_list("messages").unwrap()[0];
    assert_eq!(first, &Tag::Compound(Compound::new().with("text", "L0")));
    assert!(!c.contains_key("allow_op_features"));
    sign.allow_op_features = false;
    let c = write(&be(BlockEntityData::Sign(Box::new(sign))), (26, 3, 0)).unwrap();
    assert_eq!(c.get("allow_op_features"), Some(&Tag::Byte(0)));
}

// ---- banner, beacon, skull, spawner -----------------------------------------------------------------------------

fn sample_banner() -> Banner {
    Banner {
        patterns: vec![(DyeColor::RED, BannerPattern::STRIPE_TOP), (DyeColor::BLUE, BannerPattern::CREEPER)],
        base: Some(DyeColor::WHITE),
    }
}

#[test]
fn banner_patterns_by_version() {
    let mut entity = be(BlockEntityData::Banner(sample_banner()));
    entity.custom_name = Some(json_text::from_text("Flag"));
    check(&entity, &[V1165, V1204], |_| {
        let patterns = list(vec![
            Compound::new().with("Color", 14).with("Pattern", "ts"),
            Compound::new().with("Color", 11).with("Pattern", "cre"),
        ]);
        Some(base("minecraft:banner").with("Base", 0).with("Patterns", patterns).with("CustomName", text("Flag")))
    });
    // 1.20.5 renames the list and uses identifiers (so does 1.21.4); patterns newer than the version are dropped.
    let mut banner = sample_banner();
    banner.patterns.push((DyeColor::BLACK, BannerPattern::FLOW));
    let entity = be(BlockEntityData::Banner(banner));
    let c = write(&entity, (1, 20, 5)).unwrap();
    let patterns = list(vec![
        Compound::new().with("color", "red").with("pattern", "minecraft:stripe_top"),
        Compound::new().with("color", "blue").with("pattern", "minecraft:creeper"),
    ]);
    assert_eq!(c.get("patterns"), Some(&patterns));
    assert!(!c.contains_key("Patterns"));
    let c = write(&entity, (1, 21, 0)).unwrap();
    assert_eq!(c.get_list("patterns").unwrap().len(), 3);
    assert_eq!(write(&entity, V1214).unwrap().get_list("patterns").unwrap().len(), 3);
}

#[test]
fn beacon_effects_ids_then_names() {
    let beacon = Beacon { primary_effect: EffectType::SPEED, secondary_effect: EffectType::REGENERATION };
    let entity = be(BlockEntityData::Beacon(beacon));
    assert_eq!(write(&entity, V1165).unwrap(), base("minecraft:beacon").with("Primary", 1).with("Secondary", 10));
    assert_eq!(write(&entity, (1, 20, 1)).unwrap(), base("minecraft:beacon").with("Primary", 1).with("Secondary", 10));
    for v in [(1, 20, 2), V1204, V1214] {
        let expected = base("minecraft:beacon")
            .with("primary_effect", "minecraft:speed")
            .with("secondary_effect", "minecraft:regeneration");
        assert_eq!(write(&entity, v).unwrap(), expected, "{v:?}");
    }
    // No effects: nothing but the position.
    assert_eq!(write(&be(BlockEntityData::Beacon(Beacon::default())), V1204).unwrap(), base("minecraft:beacon"));
}

const UUID: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
const UUID_INTS: [i32; 4] = [110787060, 1156138790, -1514210135, 238594805];

fn sample_skull() -> Skull {
    Skull {
        owner_id: Some(UUID.into()),
        owner_name: Some("Notch".into()),
        texture: Some("dGV4dHVyZQ==".into()),
        texture_signature: Some("c2ln".into()),
    }
}

#[test]
fn skull_profile_by_version() {
    let entity = be(BlockEntityData::Skull(sample_skull()));
    let old = |owner: &str| {
        let textures = list(vec![Compound::new().with("Value", "dGV4dHVyZQ==").with("Signature", "c2ln")]);
        let owner_tag = Compound::new()
            .with("Id", UUID_INTS.to_vec())
            .with("Name", "Notch")
            .with("Properties", Compound::new().with("textures", textures));
        base("minecraft:skull").with(owner, owner_tag)
    };
    // 1.13: owner, uuid as a string.
    let c = write(&entity, V113).unwrap();
    assert_eq!(c.get_compound("Owner").unwrap().get_str("Id"), Some(UUID));
    assert_eq!(write(&entity, V1165).unwrap(), old("SkullOwner"));
    assert_eq!(write(&entity, V1204).unwrap(), old("SkullOwner"));
    // 1.20.5: a lower case profile.
    let textures =
        list(vec![Compound::new().with("name", "textures").with("value", "dGV4dHVyZQ==").with("signature", "c2ln")]);
    let profile = Compound::new().with("id", UUID_INTS.to_vec()).with("name", "Notch").with("properties", textures);
    assert_eq!(write(&entity, (1, 20, 5)).unwrap(), base("minecraft:skull").with("profile", profile.clone()));
    assert_eq!(write(&entity, V1214).unwrap(), base("minecraft:skull").with("profile", profile));
    // Nothing known: no owner at all.
    assert_eq!(write(&be(BlockEntityData::Skull(Skull::default())), V1204).unwrap(), base("minecraft:skull"));
    // Only a name still makes a profile.
    let named = Skull { owner_name: Some("Steve".into()), ..Skull::default() };
    let c = write(&be(BlockEntityData::Skull(named)), V1204).unwrap();
    assert_eq!(c.get_compound("SkullOwner").unwrap(), &Compound::new().with("Name", "Steve"));
}

#[test]
fn spawner_spawn_data_nests_from_1_18() {
    let spawner = Spawner {
        delay: 20,
        spawn_range: 4,
        max_spawn_delay: 800,
        min_spawn_delay: 200,
        spawn_count: 4,
        required_player_range: 16,
        max_nearby_entities: 6,
        entity_type: Some(EntityType::Vanilla(VanillaEntityType::ZOMBIE)),
    };
    let entity = be(BlockEntityData::Spawner(Box::new(spawner)));
    check(&entity, &THREE, |v| {
        let spawn_data = if v == V1165 {
            Compound::new().with("id", "minecraft:zombie")
        } else {
            Compound::new().with("entity", Compound::new().with("id", "minecraft:zombie"))
        };
        Some(
            base("minecraft:mob_spawner")
                .with("Delay", 20_i16)
                .with("SpawnRange", 4_i16)
                .with("MaxSpawnDelay", 800_i16)
                .with("MinSpawnDelay", 200_i16)
                .with("SpawnCount", 4_i16)
                .with("RequiredPlayerRange", 16_i16)
                .with("MaxNearbyEntities", 6_i16)
                .with("SpawnData", spawn_data),
        )
    });
    // No entity: no SpawnData.
    let c = write(&be(BlockEntityData::Spawner(Box::default())), V1204).unwrap();
    assert!(!c.contains_key("SpawnData"));
    // A custom type keeps its identifier.
    let custom = Spawner { entity_type: Some(EntityType::Custom("mod:thing".into())), ..Spawner::default() };
    let c = write(&be(BlockEntityData::Spawner(Box::new(custom))), V1165).unwrap();
    assert_eq!(c.get_compound("SpawnData").unwrap().get_str("id"), Some("mod:thing"));
}

// ---- the small ones ---------------------------------------------------------------------------------------------

#[test]
fn command_block_and_enchanting_table() {
    let command = CommandBlock { command: Some("say hi".into()), auto: true, track_output: false };
    let mut entity = be(BlockEntityData::CommandBlock(command));
    entity.custom_name = Some(json_text::from_text("Cmd"));
    check(&entity, &THREE, |_| {
        Some(
            base("minecraft:command_block")
                .with("Command", "say hi")
                .with("auto", true)
                .with("TrackOutput", false)
                .with("CustomName", text("Cmd")),
        )
    });
    let mut table = be(BlockEntityData::EnchantmentTable);
    check(&table, &THREE, |_| Some(base("minecraft:enchanting_table")));
    table.custom_name = Some(json_text::from_text("Table"));
    check(&table, &THREE, |_| Some(base("minecraft:enchanting_table").with("CustomName", text("Table"))));
}

#[test]
fn comparator_end_gateway_jukebox_lectern() {
    check(&be(BlockEntityData::Comparator { output_signal: 7 }), &THREE, |_| {
        Some(base("minecraft:comparator").with("OutputSignal", 7))
    });
    let gateway = EndGateway { age: 99, exit_x: 1, exit_y: 2, exit_z: 3 };
    check(&be(BlockEntityData::EndGateway(gateway)), &THREE, |_| {
        Some(base("minecraft:end_gateway").with("Age", 99_i64).with("exit_portal", vec![1, 2, 3]))
    });
    let record = Box::new(stack(ItemType::MUSIC_DISC_13, 1));
    check(&be(BlockEntityData::Jukebox { record: Some(record) }), &THREE, |v| {
        Some(base("minecraft:jukebox").with("RecordItem", item(v, &stack(ItemType::MUSIC_DISC_13, 1), None)))
    });
    // Air and nothing are left out.
    check(&be(BlockEntityData::Jukebox { record: Some(Box::new(ItemStack::new(BlockType::AIR))) }), &THREE, |_| {
        Some(base("minecraft:jukebox"))
    });
    let book = Box::new(stack(ItemType::WRITTEN_BOOK, 1));
    let lectern = be(BlockEntityData::Lectern { book: Some(book), page: 4 });
    check(&lectern, &THREE, |v| {
        Some(base("minecraft:lectern").with("Page", 4).with("Book", item(v, &stack(ItemType::WRITTEN_BOOK, 1), None)))
    });
    assert!(write(&lectern, V113).is_none(), "lecterns are 1.14");
}

#[test]
fn campfire_items_and_cooking_times() {
    let mut campfire = Campfire::default();
    campfire.items[1] = Some(stack(ItemType::BREAD, 1));
    campfire.cooking_times = [5, 100, 7, 9];
    check(&be(BlockEntityData::Campfire(Box::new(campfire))), &THREE, |v| {
        Some(
            base("minecraft:campfire")
                .with("Items", list(vec![item(v, &stack(ItemType::BREAD, 1), Some(1))]))
                .with("CookingTimes", vec![0, 100, 0, 0]),
        )
    });
}

#[test]
fn chiseled_bookshelf_and_shelf() {
    let mut books: Box<[Option<ItemStack>; 6]> = Box::default();
    books[4] = Some(stack(ItemType::WRITTEN_BOOK, 1));
    let entity = be(BlockEntityData::ChiseledBookshelf(books));
    check(&entity, &THREE, |v| {
        if v == V1165 {
            return None;
        }
        Some(
            base("minecraft:chiseled_bookshelf")
                .with("Items", list(vec![item(v, &stack(ItemType::WRITTEN_BOOK, 1), Some(4))])),
        )
    });
    let mut shelf: Box<[Option<ItemStack>; 3]> = Box::default();
    shelf[2] = Some(stack(ItemType::STICK, 1));
    let entity = be(BlockEntityData::Shelf(shelf));
    assert!(write(&entity, V1214).is_none(), "shelves are 1.21.9");
    let c = write(&entity, (1, 21, 9)).unwrap();
    assert_eq!(c.get_str("id"), Some("minecraft:shelf"));
    assert_eq!(c.get_list("Items").unwrap().len(), 1);
}

#[test]
fn decorated_pot() {
    let pot = DecoratedPot {
        back: ItemId::Item(ItemType::BRICK),
        left: ItemId::Item(ItemType::STICK),
        right: ItemId::Item(ItemType::COAL),
        front: ItemId::Item(ItemType::DIAMOND),
        item: Some(stack(ItemType::BREAD, 1)),
    };
    let entity = be(BlockEntityData::DecoratedPot(Box::new(pot)));
    let sherds = |name: &'static str| {
        let ids = ["brick", "stick", "coal", "diamond"];
        (name, Tag::List(ids.iter().map(|i| Tag::String(format!("minecraft:{i}"))).collect()))
    };
    assert!(write(&entity, V1165).is_none());
    // 1.19.4: still `shards`, no stored item.
    let (key, list19) = sherds("shards");
    assert_eq!(write(&entity, (1, 19, 4)).unwrap(), base("minecraft:decorated_pot").with(key, list19));
    // 1.20 renames the list, 1.20.3 adds the item.
    let (key, list20) = sherds("sherds");
    assert_eq!(write(&entity, (1, 20, 2)).unwrap(), base("minecraft:decorated_pot").with(key, list20.clone()));
    for v in [V1204, V1214] {
        let expected = base("minecraft:decorated_pot")
            .with(key, list20.clone())
            .with("item", item(v, &stack(ItemType::BREAD, 1), None));
        assert_eq!(write(&entity, v).unwrap(), expected, "{v:?}");
    }
    // 26.3 uses a compound of sides.
    let c = write(&entity, (26, 3, 0)).unwrap();
    let sherds = c.get_compound("sherds").unwrap();
    assert_eq!(sherds.get_compound("front").unwrap().get_str("id"), Some("minecraft:diamond"));
}

#[test]
fn brushable_block() {
    let block = Brushable {
        brush_direction: 2,
        brush_count: 3,
        item: Some(stack(ItemType::DIAMOND, 1)),
        loot_table: Some("minecraft:archaeology/desert_pyramid".into()),
    };
    let entity = be(BlockEntityData::Brushable(Box::new(block)));
    check(&entity, &THREE, |v| {
        if v == V1165 {
            return None;
        }
        Some(
            base("minecraft:brushable_block")
                .with("hit_direction", 2_i8)
                .with("item", item(v, &stack(ItemType::DIAMOND, 1), None))
                .with("LootTable", "minecraft:archaeology/desert_pyramid"),
        )
    });
    // 6 is "undefined" and is not written.
    let undefined = Brushable { brush_direction: 6, ..Brushable::default() };
    let c = write(&be(BlockEntityData::Brushable(Box::new(undefined))), V1204).unwrap();
    assert!(!c.contains_key("hit_direction"));
}

#[test]
fn jigsaw_priorities_from_1_20_5() {
    let jigsaw = Jigsaw {
        name: "a:b".into(),
        target: "c:d".into(),
        pool: "e:f".into(),
        final_state: "minecraft:stone".into(),
        joint: Joint::Aligned,
        selection_priority: 2,
        placement_priority: 3,
    };
    let entity = be(BlockEntityData::Jigsaw(Box::new(jigsaw)));
    check(&entity, &[V1165, V1204], |_| {
        Some(
            base("minecraft:jigsaw")
                .with("name", "a:b")
                .with("target", "c:d")
                .with("pool", "e:f")
                .with("final_state", "minecraft:stone")
                .with("joint", "aligned"),
        )
    });
    let c = write(&entity, V1214).unwrap();
    assert_eq!(c.get("selection_priority"), Some(&Tag::Int(2)));
    assert_eq!(c.get("placement_priority"), Some(&Tag::Int(3)));
    assert!(write(&entity, V113).is_none());
}

#[test]
fn structure_block() {
    let structure = Structure {
        name: "n".into(),
        author: "a".into(),
        metadata: "m".into(),
        pos_x: 1,
        pos_y: 2,
        pos_z: 3,
        size_x: 4,
        size_y: 5,
        size_z: 6,
        rotation: StructureRotation::CounterClockwise90,
        mirror: StructureMirror::FrontBack,
        mode: StructureBlockMode::EXPORT,
        strict: true,
        integrity: 0.5,
        seed: 9,
        ..Structure::default()
    };
    let entity = be(BlockEntityData::Structure(Box::new(structure)));
    check(&entity, &[V1165, V1204, V1214], |_| {
        Some(
            base("minecraft:structure_block")
                .with("name", "n")
                .with("author", "a")
                .with("metadata", "m")
                .with("posX", 1)
                .with("posY", 2)
                .with("posZ", 3)
                .with("sizeX", 4)
                .with("sizeY", 5)
                .with("sizeZ", 6)
                .with("rotation", "COUNTERCLOCKWISE_90")
                .with("mirror", "FRONT_BACK")
                .with("mode", "SAVE")
                .with("ignoreEntities", false)
                .with("powered", false)
                .with("showair", false)
                .with("showboundingbox", true)
                .with("integrity", 0.5_f32)
                .with("seed", 9_i64),
        )
    });
    let c = write(&entity, (1, 21, 5)).unwrap();
    assert_eq!(c.get("strict"), Some(&Tag::Byte(1)));
}

#[test]
fn sculk_sensors_write_a_listener() {
    let listener = Compound::new().with("event_delay", 0).with("selector", Compound::new().with("tick", -1_i64));
    for (id, data, since) in [
        ("minecraft:sculk_sensor", BlockEntityData::SculkSensor, V1204),
        ("minecraft:calibrated_sculk_sensor", BlockEntityData::CalibratedSculkSensor, V1204),
    ] {
        let entity = be(data);
        let expected = base(id).with("listener", listener.clone()).with("last_vibration_frequency", 0);
        assert_eq!(write(&entity, since).unwrap(), expected);
        assert!(write(&entity, V1165).is_none());
    }
}

#[test]
fn empty_handlers_and_version_gates() {
    // (data, id, first version that writes it)
    let cases: Vec<(BlockEntityData, &str, V)> = vec![
        (BlockEntityData::Bell, "minecraft:bell", (1, 14, 0)),
        (BlockEntityData::Beehive, "minecraft:beehive", (1, 15, 0)),
        (BlockEntityData::Conduit, "minecraft:conduit", V113),
        (BlockEntityData::DaylightDetector, "minecraft:daylight_detector", V113),
        (BlockEntityData::EndPortal, "minecraft:end_portal", V113),
        (BlockEntityData::SculkCatalyst, "minecraft:sculk_catalyst", (1, 19, 0)),
        (BlockEntityData::SculkShrieker, "minecraft:sculk_shrieker", (1, 19, 0)),
        (BlockEntityData::TrialSpawner, "minecraft:trial_spawner", (1, 20, 3)),
        (BlockEntityData::Vault, "minecraft:vault", (1, 20, 5)),
        (BlockEntityData::CreakingHeart, "minecraft:creaking_heart", (1, 21, 2)),
        (BlockEntityData::CopperGolemStatue, "minecraft:copper_golem_statue", (1, 21, 9)),
        (BlockEntityData::PotentSulfur, "minecraft:potent_sulfur", (26, 2, 0)),
    ];
    let all =
        [V113, (1, 14, 0), (1, 15, 0), V1165, (1, 19, 0), (1, 20, 3), (1, 20, 5), (1, 21, 2), (1, 21, 9), (26, 2, 0)];
    for (data, id, since) in cases {
        let entity = be(data);
        for v in all {
            let expected = (v >= since).then(|| base(id));
            assert_eq!(write(&entity, v), expected, "{id} at {v:?}");
        }
    }
}

#[test]
fn beds_stop_being_block_entities_in_26_2() {
    let entity = be(BlockEntityData::Bed);
    assert!(write(&entity, (26, 1, 0)).is_some());
    assert!(write(&entity, (26, 2, 0)).is_none());
    assert!(write(&entity, (26, 3, 0)).is_none());
}

#[test]
fn cauldron_and_test_blocks_are_not_written() {
    for data in [BlockEntityData::Cauldron(Cauldron::default()), BlockEntityData::Test, BlockEntityData::TestInstance] {
        for v in [V1165, V1204, (26, 3, 0)] {
            assert!(write(&be(data.clone()), v).is_none());
        }
    }
}

// ---- original NBT, unknown ---------------------------------------------------------------------------------------

#[test]
fn original_nbt_is_copied_and_overwritten() {
    let mut entity = be(BlockEntityData::Comparator { output_signal: 9 });
    entity.original_nbt =
        Some(Compound::new().with("id", "Comparator").with("x", 100).with("OutputSignal", 1).with("custom", "kept"));
    let c = write(&entity, V1204).unwrap();
    assert_eq!(c, base("minecraft:comparator").with("OutputSignal", 9).with("custom", "kept"));
}

#[test]
fn unknown_keeps_its_id_and_original_nbt() {
    let mut entity = be(BlockEntityData::Unknown("mod:machine".into()));
    entity.original_nbt = Some(Compound::new().with("id", "mod:machine").with("energy", 5));
    for v in THREE {
        assert_eq!(write(&entity, v).unwrap(), base("mod:machine").with("energy", 5), "{v:?}");
    }
    // Without a copy only the position is written.
    let bare = be(BlockEntityData::Unknown("mod:machine".into()));
    assert_eq!(write(&bare, V1204).unwrap(), base("mod:machine"));
}

// ---- every kind ---------------------------------------------------------------------------------------------------

fn default_entity(kind: BlockEntityKind) -> BlockEntity {
    let data = BlockEntityData::default_for(kind).unwrap_or_else(|| BlockEntityData::Unknown("mod:thing".into()));
    be(data)
}

/// A block entity of the kind with every field populated.
fn sample(kind: BlockEntityKind) -> BlockEntity {
    let mut entity = default_entity(kind);
    entity.custom_name = Some(json_text::from_text("Name"));
    if let Some(container) = entity.data.container_mut() {
        container.items.insert(0, stack(ItemType::DIAMOND, 2));
        container.items.insert(-3, stack(ItemType::BRICK, 1));
    }
    match &mut entity.data {
        BlockEntityData::Sign(s) | BlockEntityData::HangingSign(s) => **s = sample_sign(),
        BlockEntityData::Banner(b) => *b = sample_banner(),
        BlockEntityData::Skull(s) => *s = sample_skull(),
        BlockEntityData::Beacon(b) => {
            b.primary_effect = EffectType::HASTE;
            b.secondary_effect = EffectType::STRENGTH;
        }
        BlockEntityData::Spawner(s) => s.entity_type = Some(EntityType::Vanilla(VanillaEntityType::CREEPER)),
        BlockEntityData::Campfire(c) => c.items[3] = Some(stack(ItemType::BREAD, 1)),
        BlockEntityData::ChiseledBookshelf(b) => b[0] = Some(stack(ItemType::WRITTEN_BOOK, 1)),
        BlockEntityData::Shelf(s) => s[1] = Some(stack(ItemType::STICK, 1)),
        BlockEntityData::Jukebox { record } => *record = Some(Box::new(stack(ItemType::MUSIC_DISC_13, 1))),
        BlockEntityData::Lectern { book, page } => {
            *book = Some(Box::new(stack(ItemType::WRITTEN_BOOK, 1)));
            *page = 2;
        }
        BlockEntityData::DecoratedPot(p) => p.item = Some(stack(ItemType::BREAD, 1)),
        BlockEntityData::Brushable(b) => b.item = Some(stack(ItemType::DIAMOND, 1)),
        BlockEntityData::Crafter(c) => c.disabled_slots = 0b101,
        BlockEntityData::Chest(r) => r.loot_table = Some("minecraft:chests/simple_dungeon".into()),
        _ => {}
    }
    entity
}

const FORMATS: [V; 12] = [
    V113,
    (1, 14, 4),
    V1165,
    (1, 19, 4),
    V1204,
    (1, 20, 5),
    V1214,
    (1, 21, 5),
    (1, 21, 9),
    (26, 1, 0),
    (26, 2, 0),
    (26, 3, 0),
];

#[test]
fn every_kind_writes_or_is_documented_unsupported() {
    for kind in BlockEntityKind::ALL {
        let latest = JavaFormat::latest();
        let written = write_block_entity(&items_for(&latest), &latest, &sample(kind));
        assert_eq!(written.is_some(), !UNSUPPORTED_ON_JAVA.contains(&kind) && is_supported(kind, &latest), "{kind:?}");
        for v in FORMATS {
            let f = fmt(v);
            let c = write_block_entity(&items_for(&f), &f, &sample(kind));
            assert_eq!(c.is_some(), is_supported(kind, &f), "{kind:?} at {v:?}");
            if UNSUPPORTED_ON_JAVA.contains(&kind) {
                assert!(c.is_none(), "{kind:?}");
            }
        }
    }
    // Everything that is documented as unsupported really is, and nothing else is.
    let unsupported: Vec<_> = BlockEntityKind::ALL
        .into_iter()
        .filter(|k| {
            let f = JavaFormat::latest();
            !is_supported(*k, &f)
        })
        .collect();
    assert_eq!(
        unsupported,
        vec![BlockEntityKind::Bed, BlockEntityKind::Cauldron, BlockEntityKind::Test, BlockEntityKind::TestInstance]
    );
}

#[test]
fn ids_are_namespaced_and_used() {
    for kind in BlockEntityKind::ALL {
        let f = JavaFormat::latest();
        let id = java_block_entity_id(kind, &f);
        if kind == BlockEntityKind::Unknown {
            assert_eq!(id, "");
            continue;
        }
        assert!(id.starts_with("minecraft:"), "{kind:?}");
        if let Some(c) = write_block_entity(&items_for(&f), &f, &default_entity(kind)) {
            assert_eq!(c.get_str("id"), Some(id), "{kind:?}");
            assert_eq!(c.get_i32("x"), Some(1));
            assert_eq!(c.get_i32("y"), Some(2));
            assert_eq!(c.get_i32("z"), Some(3));
        }
    }
    // The ids are all different.
    let mut ids: Vec<_> = BlockEntityKind::ALL
        .iter()
        .map(|k| java_block_entity_id(*k, &JavaFormat::latest()))
        .filter(|i| !i.is_empty())
        .collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

#[test]
fn written_compounds_survive_the_nbt_codec() {
    for kind in BlockEntityKind::ALL {
        for v in FORMATS {
            let f = fmt(v);
            if let Some(c) = write_block_entity(&items_for(&f), &f, &sample(kind)) {
                round_trips(&c);
                Tag::Compound(c).validate().expect("lists are homogeneous");
            }
        }
    }
}

// ---- items carrying a block entity ----------------------------------------------------------------------------------

fn item_with(v: V, entity: &BlockEntity) -> (Compound, bool) {
    let f = fmt(v);
    let mut c = Compound::new().with("id", "minecraft:test");
    let wrote = write_block_entity_to_item(&items_for(&f), &f, entity, &mut c);
    (c, wrote)
}

#[test]
fn item_block_entity_tag_before_1_20_5() {
    let mut entity = be(BlockEntityData::Chest(chest_like()));
    entity.custom_name = Some(json_text::from_text("Loot"));
    let (c, wrote) = item_with(V1204, &entity);
    assert!(wrote);
    let tag = c.get_compound("tag").unwrap().get_compound("BlockEntityTag").unwrap();
    assert!(!tag.contains_key("id") && !tag.contains_key("x"));
    assert_eq!(tag.get_list("Items").unwrap().len(), 2);
    assert_eq!(tag.get("CustomName"), Some(&text("Loot")));
    assert!(!c.contains_key("components"));
}

#[test]
fn item_components_from_1_20_5() {
    let v = (1, 21, 0);
    // Containers use the container component and no block entity data.
    let (c, _) = item_with(v, &be(BlockEntityData::ShulkerBox(chest_like())));
    let comps = c.get_compound("components").unwrap();
    let slots = comps.get_list("minecraft:container").unwrap();
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[1].as_compound().unwrap().get_i32("slot"), Some(2));
    assert!(!comps.contains_key("minecraft:block_entity_data"));
    // Banners.
    let (c, _) = item_with(v, &be(BlockEntityData::Banner(sample_banner())));
    let comps = c.get_compound("components").unwrap();
    assert_eq!(comps.get_str("minecraft:base_color"), Some("white"));
    assert_eq!(comps.get_list("minecraft:banner_patterns").unwrap().len(), 2);
    // Skulls.
    let (c, _) = item_with(v, &be(BlockEntityData::Skull(sample_skull())));
    let profile = c.get_compound("components").unwrap().get_compound("minecraft:profile").unwrap();
    assert_eq!(profile.get_str("name"), Some("Notch"));
    // Signs have no hook: block_entity_data keeps the id but not the position.
    let (c, _) = item_with(v, &be(BlockEntityData::Sign(Box::new(sample_sign()))));
    let data = c.get_compound("components").unwrap().get_compound("minecraft:block_entity_data").unwrap();
    assert_eq!(data.get_str("id"), Some("minecraft:sign"));
    assert!(!data.contains_key("x"));
    // A loot table goes to its own component.
    let mut chest = chest_like();
    chest.loot_table = Some("minecraft:chests/simple_dungeon".into());
    let (c, _) = item_with(v, &be(BlockEntityData::Chest(chest)));
    let loot = c.get_compound("components").unwrap().get_compound("minecraft:container_loot").unwrap();
    assert_eq!(loot.get_str("loot_table"), Some("minecraft:chests/simple_dungeon"));
}

// ---- tables ---------------------------------------------------------------------------------------------------------

#[test]
fn name_tables_follow_the_version() {
    use super::tables::*;
    let v = Version::new;
    assert_eq!(banner_pattern_id(BannerPattern::STRIPE_SMALL, v(1, 20, 5)), Some("minecraft:small_stripes"));
    assert_eq!(banner_pattern_id(BannerPattern::GUSTER, v(1, 20, 5)), None);
    assert_eq!(banner_pattern_id(BannerPattern::GUSTER, v(1, 21, 0)), Some("minecraft:guster"));
    assert_eq!(banner_short_name(BannerPattern::HALF_HORIZONTAL_MIRROR), Some("hhb"));
    assert_eq!(banner_short_name(BannerPattern::FLOW), None);
    assert_eq!(effect_name(EffectType::SLOW_FALLING, v(1, 13, 0)), "minecraft:slow_falling");
    assert_eq!(effect_name(EffectType::DARKNESS, v(1, 18, 0)), "minecraft:empty");
    assert_eq!(effect_id(EffectType::DARKNESS, v(1, 19, 0)), 33);
    assert_eq!(effect_id(EffectType::SLOW_FALLING, v(1, 13, 0)), 28);
    assert_eq!(effect_id(EffectType::CONDUIT_POWER, v(1, 13, 0)), 29);
    // The boat is renamed in 1.21.2.
    let boat = EntityType::Vanilla(VanillaEntityType::OAK_BOAT);
    assert_eq!(entity_type_id(&boat, v(1, 21, 1)).as_deref(), Some("minecraft:boat"));
    assert_eq!(entity_type_id(&boat, v(1, 21, 2)).as_deref(), Some("minecraft:oak_boat"));
    assert_eq!(
        entity_type_id(&EntityType::Vanilla(VanillaEntityType::CREEPER), v(1, 13, 0)).as_deref(),
        Some("minecraft:creeper")
    );
    assert_eq!(entity_type_id(&EntityType::Vanilla(VanillaEntityType::ARMADILLO), v(1, 20, 4)), None);
}

#[test]
fn uuid_to_ints() {
    assert_eq!(skull::uuid_ints(UUID), Some(UUID_INTS.to_vec()));
    assert_eq!(skull::uuid_ints("nonsense"), None);
}

/// [`write`] with the hooks the world writer gives every block entity (the block entity data of items, map ids).
fn write_in_world(be: &BlockEntity, v: V) -> Option<Compound> {
    let f = fmt(v);
    let items = items_for(&f);
    let hooks = crate::convert::java::resolvers::hooks::WorldItemHooks::new(&items, &f, 0);
    write_block_entity_with(&items, &f, be, &hooks)
}

#[test]
fn shulker_boxes_in_a_chest_keep_their_contents() {
    let mut shulker = ItemStack::new(BlockType::SHULKER_BOX);
    shulker.props.block_entity = Some(Box::new(be(BlockEntityData::ShulkerBox(chest_like()))));
    let mut chest = RandomContainer::default();
    chest.container.items.insert(4, shulker);
    let entity = be(BlockEntityData::Chest(chest));
    let new = write_in_world(&entity, (26, 3, 0)).unwrap();
    let held = new.get_list("Items").unwrap()[0].as_compound().unwrap();
    let inner = held.get_compound("components").and_then(|c| c.get_list("minecraft:container")).expect("contents");
    assert_eq!(inner.len(), 2);
    let old = write_in_world(&entity, (1, 20, 4)).unwrap();
    let held = old.get_list("Items").unwrap()[0].as_compound().unwrap();
    let tag = held.get_compound("tag").and_then(|t| t.get_compound("BlockEntityTag")).expect("BlockEntityTag");
    assert_eq!(tag.get_list("Items").map(<[Tag]>::len), Some(2));
}
