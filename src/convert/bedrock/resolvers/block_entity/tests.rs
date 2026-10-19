// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Block entity reading with hand built compounds, and on the real test world.

use std::sync::{Arc, LazyLock};

use super::*;
use crate::convert::bedrock::data_version::BedrockVersion;
use crate::convert::model::blockentity::{StructureMirror, StructureRotation};
use crate::convert::model::{
    BlockType, Bool, ChestType, ChunkCoord, Column, EffectType, EntityData, EntityType, FacingDirection,
    FacingDirectionHorizontal, ItemId, ItemType, Note, PotionBottleType, PotionType, Rotation, StructureBlockMode,
    states,
};
use crate::convert::nbt::{Tag, compound};

static ITEMS: LazyLock<BedrockItems> = LazyLock::new(|| BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap()));

fn tag(id: &str, x: i32, y: i32, z: i32) -> Compound {
    compound! { "id" => id, "x" => x, "y" => y, "z" => z, "isMovable" => 1i8 }
}

fn read(c: &Compound) -> BlockEntity {
    read_block_entity(&ITEMS, c, false).unwrap_or_else(|| panic!("unreadable {c:?}"))
}

fn stick(slot: i8, count: i8) -> Tag {
    Tag::Compound(compound! { "Slot" => slot, "Name" => "minecraft:stick", "Count" => count, "Damage" => 0i16 })
}

fn column() -> Column {
    Column::new(ChunkCoord::new(0, 0))
}

fn block(ty: BlockType) -> Arc<BlockIdentifier> {
    Arc::new(BlockIdentifier::new(ty))
}

#[test]
fn common_fields() {
    let mut c = tag("Bell", -3, 70, 12);
    c.put("CustomName", "Ding");
    let be = read(&c);
    assert_eq!((be.x, be.y, be.z), (-3, 70, 12));
    assert!(be.movable);
    assert_eq!(be.custom_name.as_ref().unwrap()["text"], "Ding");
    assert_eq!(be.kind(), BlockEntityKind::Bell);
    assert!(be.original_nbt.is_none());
    let kept = read_block_entity(&ITEMS, &c, true).unwrap();
    assert_eq!(kept.original_nbt.as_ref(), Some(&c));
    assert_eq!(kept, be, "the original tag does not take part in equality");
}

#[test]
fn simple_kinds_by_id() {
    for (id, kind) in [
        ("Bell", BlockEntityKind::Bell),
        ("Conduit", BlockEntityKind::Conduit),
        ("EnchantTable", BlockEntityKind::EnchantmentTable),
        ("DaylightDetector", BlockEntityKind::DaylightDetector),
        ("EndPortal", BlockEntityKind::EndPortal),
        ("Beehive", BlockEntityKind::Beehive),
        ("SculkSensor", BlockEntityKind::SculkSensor),
        ("CalibratedSculkSensor", BlockEntityKind::CalibratedSculkSensor),
        ("SculkCatalyst", BlockEntityKind::SculkCatalyst),
        ("SculkShrieker", BlockEntityKind::SculkShrieker),
        ("TrialSpawner", BlockEntityKind::TrialSpawner),
        ("Vault", BlockEntityKind::Vault),
        ("CreakingHeart", BlockEntityKind::CreakingHeart),
        ("PotentSulfurBlock", BlockEntityKind::PotentSulfur),
        ("EnderChest", BlockEntityKind::EnderChest),
        ("Hopper", BlockEntityKind::Hopper),
        ("Dispenser", BlockEntityKind::Dispenser),
        ("Dropper", BlockEntityKind::Dropper),
        ("Barrel", BlockEntityKind::Barrel),
        ("Smoker", BlockEntityKind::Smoker),
        ("BlastFurnace", BlockEntityKind::BlastFurnace),
        ("HangingSign", BlockEntityKind::HangingSign),
    ] {
        assert_eq!(read(&tag(id, 0, 0, 0)).kind(), kind, "{id}");
    }
}

#[test]
fn version_gates_decide_what_is_known() {
    let old = BedrockItems::new(BedrockVersion::exact([1, 16, 0]).unwrap());
    assert!(read_block_entity(&old, &tag("Beehive", 0, 0, 0), false).is_some());
    assert!(read_block_entity(&old, &tag("SculkSensor", 0, 0, 0), false).is_none());
    assert!(read_block_entity(&old, &tag("HangingSign", 0, 0, 0), false).is_none());
    let unknown = read_block_entity(&old, &tag("Vault", 1, 2, 3), true).unwrap();
    assert_eq!(unknown.data, BlockEntityData::Unknown("Vault".into()));
    assert_eq!((unknown.x, unknown.y, unknown.z), (1, 2, 3));
    assert!(unknown.original_nbt.is_some());
}

#[test]
fn unknown_ids() {
    let c = tag("SomeModEntity", 4, 5, 6);
    assert!(read_block_entity(&ITEMS, &c, false).is_none());
    let be = read_block_entity(&ITEMS, &c, true).unwrap();
    assert_eq!(be.data, BlockEntityData::Unknown("SomeModEntity".into()));
    assert_eq!(be.original_nbt, Some(c));
    assert!(read_block_entity(&ITEMS, &compound! { "x" => 1i32 }, true).is_none());
}

#[test]
fn chest_items_slots_and_loot_table() {
    let mut c = tag("Chest", 1, 2, 3);
    c.put("Items", Tag::list([stick(0, 1), stick(26, 64)]));
    c.put("LootTable", "loot_tables/chests/buriedtreasure.json");
    let be = read(&c);
    let BlockEntityData::Chest(chest) = &be.data else { panic!("{be:?}") };
    assert_eq!(chest.container.items.len(), 2);
    assert_eq!(chest.container.items[&26].props.amount, Some(64));
    assert_eq!(chest.loot_table.as_deref(), Some("minecraft:chests/buried_treasure"));
    // The slot defaults to the list index.
    let mut c = tag("Barrel", 0, 0, 0);
    c.put(
        "Items",
        Tag::list([Tag::Compound(compound! { "Name" => "minecraft:stick", "Count" => 2i8, "Damage" => 0i16 })]),
    );
    assert!(read(&c).data.container().unwrap().items.contains_key(&0));
    // Loot tables can be turned off.
    let items = BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap()).with_loot_tables(false);
    let be = read_block_entity(&items, &tag("Chest", 0, 0, 0).with("LootTable", "loot_tables/x.json"), false).unwrap();
    assert_eq!(be.data.loot_table(), Some(&None));
}

#[test]
fn brewing_stand_slots_are_remapped() {
    let mut c = tag("BrewingStand", 0, 0, 0);
    c.put("Items", Tag::list([stick(0, 1), stick(1, 2), stick(4, 3)]));
    c.put("CookTime", 400i16);
    c.put("FuelAmount", 12i16);
    c.put("FuelTotal", 20i16);
    let be = read(&c);
    let BlockEntityData::BrewingStand(b) = &be.data else { panic!() };
    // Bedrock slot 0 is the ingredient (3), slot 1 the first bottle (0).
    assert_eq!(b.container.items[&3].props.amount, Some(1));
    assert_eq!(b.container.items[&0].props.amount, Some(2));
    assert_eq!(b.container.items[&4].props.amount, Some(3));
    assert_eq!((b.brew_time, b.fuel, b.fuel_total), (400, 12, 20));
    assert_eq!(super::read::remap_slot(true, false, 3), 0);
}

#[test]
fn furnaces() {
    let c = tag("BlastFurnace", 0, 0, 0)
        .with("BurnTime", 100i16)
        .with("CookTime", 50i16)
        .with("BurnDuration", 200i16)
        .with("Items", Tag::list([stick(2, 1)]));
    let be = read(&c);
    let BlockEntityData::BlastFurnace(f) = &be.data else { panic!() };
    assert_eq!((f.burn_time, f.cook_time, f.cook_time_total), (100, 50, 200));
    assert!(f.container.items.contains_key(&2));
}

#[test]
fn signs_read_text_color_and_glow() {
    let front =
        compound! { "Text" => "Hello\nWorld", "IgnoreLighting" => 1i8, "SignTextColor" => 0xff00_ff00_u32 as i32 };
    let back = compound! { "Text" => "Back" };
    let c = tag("Sign", 0, 0, 0).with("FrontText", front).with("BackText", back).with("IsWaxed", 1i8);
    let be = read(&c);
    let sign = be.data.sign().unwrap();
    assert!(sign.waxed);
    assert_eq!(sign.front.lines.len(), 4);
    assert_eq!(sign.front.lines[0]["text"], "Hello");
    assert_eq!(sign.front.lines[1]["text"], "World");
    assert_eq!(sign.front.lines[3]["text"], "");
    assert!(sign.front.lit);
    assert_eq!(sign.front.color, 0xff00_ff00_u32 as i32);
    assert_eq!(sign.back.lines[0]["text"], "Back");
    assert_eq!(sign.back.color, 0xff00_0000_u32 as i32);
}

#[test]
fn old_signs_use_the_root_and_split_long_lines() {
    let long = "the quick brown fox jumps over the lazy dog";
    let be = read(&tag("Sign", 0, 0, 0).with("Text", long));
    let sign = be.data.sign().unwrap();
    let text: Vec<_> = sign.front.lines.iter().map(|l| l["text"].as_str().unwrap().to_string()).collect();
    assert!(text[0].len() < long.len(), "{text:?}");
    assert_eq!(text.join(" ").trim(), long);
    // Text1..Text4 of even older signs.
    let be = read(&tag("Sign", 0, 0, 0).with("Text1", "a").with("Text3", "c"));
    let sign = be.data.sign().unwrap();
    assert_eq!(sign.front.lines[0]["text"], "a");
    assert_eq!(sign.front.lines[1]["text"], "");
    assert_eq!(sign.front.lines[2]["text"], "c");
    // Hanging signs split narrower.
    let line = "aaaaa bbbbb ccccc ddddd";
    let hanging = read(&tag("HangingSign", 0, 0, 0).with("Text", line));
    let normal = read(&tag("Sign", 0, 0, 0).with("Text", line));
    let count = |be: &BlockEntity| be.data.sign().unwrap().front.lines.iter().filter(|l| l["text"] != "").count();
    assert!(count(&hanging) > count(&normal));
    assert_eq!(hanging.kind(), BlockEntityKind::HangingSign);
}

#[test]
fn command_block() {
    let be = read(&tag("CommandBlock", 0, 0, 0).with("Command", "say hi").with("auto", 1i8).with("TrackOutput", 0i8));
    let BlockEntityData::CommandBlock(c) = &be.data else { panic!() };
    assert_eq!(c.command.as_deref(), Some("say hi"));
    assert!(c.auto && !c.track_output);
    let default = read(&tag("CommandBlock", 0, 0, 0));
    let BlockEntityData::CommandBlock(c) = &default.data else { panic!() };
    assert!(c.track_output && !c.auto);
}

#[test]
fn beacon_effects_and_comparator() {
    let be = read(&tag("Beacon", 0, 0, 0).with("primary", 1i32).with("secondary", 10i32));
    let BlockEntityData::Beacon(b) = &be.data else { panic!() };
    assert_eq!(b.primary_effect, EffectType::SPEED);
    assert_eq!(b.secondary_effect, EffectType::REGENERATION);
    let none = read(&tag("Beacon", 0, 0, 0));
    let BlockEntityData::Beacon(b) = &none.data else { panic!() };
    assert_eq!(b.primary_effect, EffectType::EMPTY);
    let be = read(&tag("Comparator", 0, 0, 0).with("OutputSignal", 9i32));
    assert_eq!(be.data, BlockEntityData::Comparator { output_signal: 9 });
}

#[test]
fn spawner() {
    let be = read(&tag("MobSpawner", 0, 0, 0).with("EntityIdentifier", "minecraft:zombie").with("Delay", 33i16));
    let BlockEntityData::Spawner(s) = &be.data else { panic!() };
    assert_eq!(s.entity_type, Some(EntityType::Vanilla(crate::convert::model::VanillaEntityType::ZOMBIE)));
    assert_eq!((s.delay, s.spawn_range, s.max_spawn_delay, s.min_spawn_delay), (33, 4, 800, 200));
    assert_eq!((s.spawn_count, s.required_player_range, s.max_nearby_entities), (4, 16, 6));
    let empty = read(&tag("MobSpawner", 0, 0, 0).with("EntityIdentifier", "minecraft:"));
    let BlockEntityData::Spawner(s) = &empty.data else { panic!() };
    assert_eq!(s.entity_type, None);
}

#[test]
fn banner_base_and_patterns() {
    let patterns = Tag::list([
        compound! { "Color" => 1i32, "Pattern" => "bs" },
        compound! { "Color" => 2i32, "Pattern" => "nope" },
    ]);
    let be = read(&tag("Banner", 0, 0, 0).with("Base", 4i32).with("Patterns", patterns));
    let BlockEntityData::Banner(b) = &be.data else { panic!() };
    assert_eq!(b.base, Some(DyeColor::by_reversed_id(4).unwrap()));
    assert_eq!(
        b.patterns,
        vec![(DyeColor::by_reversed_id(1).unwrap(), crate::convert::model::BannerPattern::STRIPE_BOTTOM)]
    );
}

#[test]
fn skull_end_gateway_jigsaw_structure() {
    let be = read(&tag("EndGateway", 0, 0, 0).with("Age", 40i32).with("ExitPortal", Tag::list([1i32, 2, 3])));
    let BlockEntityData::EndGateway(g) = &be.data else { panic!() };
    assert_eq!((g.age, g.exit_x, g.exit_y, g.exit_z), (40, 1, 2, 3));

    let be = read(&tag("JigsawBlock", 0, 0, 0).with("name", "a").with("target_pool", "pool").with("joint", "aligned"));
    let BlockEntityData::Jigsaw(j) = &be.data else { panic!() };
    assert_eq!((j.name.as_str(), j.pool.as_str(), j.target.as_str()), ("a", "pool", "minecraft:empty"));
    assert_eq!(j.joint, crate::convert::model::blockentity::Joint::Aligned);

    let c = tag("StructureBlock", 0, 0, 0)
        .with("structureName", "house")
        .with("xStructureSize", 5i32)
        .with("data", 2i32)
        .with("rotation", 1i8)
        .with("mirror", 3i8)
        .with("integrity", 50.0f32)
        .with("showBoundingBox", 0i8)
        .with("animationSeconds", 1.5f32)
        .with("lastTouchedPlayerId", 77i64);
    let be = read(&c);
    let BlockEntityData::Structure(s) = &be.data else { panic!() };
    assert_eq!((s.name.as_str(), s.size_x, s.mode), ("house", 5, StructureBlockMode::LOAD));
    assert_eq!((s.rotation, s.mirror), (StructureRotation::Clockwise90, StructureMirror::Both));
    assert_eq!(s.integrity, 0.5);
    assert!(!s.show_bounding_box);
    assert_eq!((s.animation_seconds, s.last_touched_player_id), (1.5, 77));
}

#[test]
fn cauldron_campfire_bookshelf_pot_crafter_shelf_brushable() {
    let be = read(
        &tag("Cauldron", 0, 0, 0)
            .with("PotionId", 5i16)
            .with("PotionType", 1i16)
            .with("CustomColor", 0x80ff0000_u32 as i32),
    );
    let BlockEntityData::Cauldron(c) = &be.data else { panic!() };
    assert_eq!(c.potion_type, PotionType::NIGHT_VISION);
    assert_eq!(c.potion_bottle_type, PotionBottleType::SPLASH);
    assert_eq!(c.custom_color.unwrap().a, 0x80);
    let splash = read(&tag("Cauldron", 0, 0, 0).with("IsSplash", 1i8));
    let BlockEntityData::Cauldron(c) = &splash.data else { panic!() };
    assert_eq!(c.potion_bottle_type, PotionBottleType::SPLASH);

    let be = read(
        &tag("Campfire", 0, 0, 0).with("Item2", stick(0, 1).as_compound().unwrap().clone()).with("ItemTime2", 600i32),
    );
    let BlockEntityData::Campfire(f) = &be.data else { panic!() };
    assert!(f.items[0].is_none() && f.items[1].is_some());
    assert_eq!(f.cooking_times, [0, 600, 0, 0]);

    let be = read(&tag("ChiseledBookshelf", 0, 0, 0).with("Items", Tag::list([stick(0, 1), stick(1, 1)])));
    let BlockEntityData::ChiseledBookshelf(books) = &be.data else { panic!() };
    assert_eq!(books.iter().flatten().count(), 2);

    let be = read(&tag("Shelf", 0, 0, 0).with("Items", Tag::list([stick(0, 1)])));
    let BlockEntityData::Shelf(shelf) = &be.data else { panic!() };
    assert!(shelf[0].is_some());

    let be = read(&tag("Crafter", 0, 0, 0).with("disabled_slots", 5i16).with("crafting_ticks_remaining", 3i32));
    let BlockEntityData::Crafter(c) = &be.data else { panic!() };
    assert_eq!((c.disabled_slots, c.crafting_ticks_remaining), (5, 3));

    let item = compound! { "Name" => "minecraft:stick", "Count" => 1i8, "Damage" => 0i16 };
    let be =
        read(&tag("BrushableBlock", 0, 0, 0).with("brush_direction", 2i8).with("brush_count", 3i32).with("item", item));
    let BlockEntityData::Brushable(b) = &be.data else { panic!() };
    assert_eq!((b.brush_direction, b.brush_count), (2, 3));
    assert!(b.item.is_some());
}

#[test]
fn decorated_pot_keeps_the_chunker_early_return() {
    let sherds = Tag::list([
        "minecraft:angler_pottery_sherd",
        "minecraft:brick",
        "minecraft:brick",
        "minecraft:archer_pottery_sherd",
    ]);
    let item = compound! { "Name" => "minecraft:stick", "Count" => 1i8, "Damage" => 0i16 };
    let be = read(&tag("DecoratedPot", 0, 0, 0).with("sherds", sherds).with("item", item.clone()));
    let BlockEntityData::DecoratedPot(p) = &be.data else { panic!() };
    assert_eq!(p.back, ItemId::Item(ItemType::ANGLER_POTTERY_SHERD));
    assert_eq!(p.left, ItemId::Item(ItemType::BRICK));
    assert_eq!(p.front, ItemId::Item(ItemType::ARCHER_POTTERY_SHERD));
    assert!(p.item.is_some());
    // Fewer than four sherds return before the item is read, like Chunker.
    let short = read(
        &tag("DecoratedPot", 0, 0, 0).with("sherds", Tag::list(["minecraft:angler_pottery_sherd"])).with("item", item),
    );
    let BlockEntityData::DecoratedPot(p) = &short.data else { panic!() };
    assert!(p.item.is_none());
    assert_eq!(p.left, ItemId::Item(ItemType::BRICK));
}

#[test]
fn lectern_and_jukebox() {
    let book = compound! { "Name" => "minecraft:writable_book", "Count" => 1i8, "Damage" => 0i16 };
    let be = read(&tag("Lectern", 0, 0, 0).with("page", 3i32).with("book", book));
    let BlockEntityData::Lectern { book, page } = &be.data else { panic!() };
    assert_eq!(*page, 6);
    assert!(book.is_some());
    let record = compound! { "Name" => "minecraft:music_disc_13", "Count" => 1i8, "Damage" => 0i16 };
    let be = read(&tag("Jukebox", 0, 0, 0).with("RecordItem", record));
    let BlockEntityData::Jukebox { record } = &be.data else { panic!() };
    assert!(record.is_some());
}

// The column step.

#[test]
fn chest_pairs_set_the_chest_type_and_trapped_chests_are_converted() {
    let mut col = column();
    let facing_north =
        |ty| Arc::new(BlockIdentifier::new(ty).with(states::FACING_HORIZONTAL, FacingDirectionHorizontal::NORTH));
    col.set_block(1, 64, 1, facing_north(BlockType::CHEST));
    col.set_block(2, 64, 1, facing_north(BlockType::CHEST));
    col.set_block(5, 64, 5, facing_north(BlockType::TRAPPED_CHEST));
    // For a north facing chest, clockwise (east) is the left neighbour.
    let tags = [
        tag("Chest", 1, 64, 1).with("pairx", 2i32).with("pairz", 1i32).with("pairlead", 1i8),
        tag("Chest", 2, 64, 1).with("pairx", 1i32).with("pairz", 1i32),
        tag("Chest", 5, 64, 5),
    ];
    let missing = read_into_column(&ITEMS, &tags, &mut col, false);
    assert!(missing.is_empty());
    assert_eq!(col.block(1, 64, 1).get(states::CHEST_TYPE), Some(ChestType::LEFT));
    assert_eq!(col.block(2, 64, 1).get(states::CHEST_TYPE), Some(ChestType::RIGHT));
    assert_eq!(col.block(5, 64, 5).get(states::CHEST_TYPE), Some(ChestType::SINGLE));
    assert_eq!(col.block_entities.len(), 3);
    assert_eq!(col.block_entities[0].kind(), BlockEntityKind::Chest);
    assert_eq!(col.block_entities[2].kind(), BlockEntityKind::TrappedChest);
}

#[test]
fn beds_banners_skulls_and_shulkers_update_their_blocks() {
    let mut col = column();
    col.set_block(0, 10, 0, block(BlockType::WHITE_BED));
    col.set_block(1, 10, 0, block(BlockType::WHITE_BANNER));
    col.set_block(2, 10, 0, block(BlockType::SKELETON_SKULL));
    col.set_block(3, 10, 0, block(BlockType::SHULKER_BOX));
    let patterns = Tag::list([compound! { "Color" => 1i32, "Pattern" => "bs" }]);
    let tags = [
        tag("Bed", 0, 10, 0).with("color", 14i8),
        tag("Banner", 1, 10, 0).with("Base", 15i32).with("Patterns", patterns),
        // Zombie head, rotation 90 degrees (4 steps), mouth moving.
        tag("Skull", 2, 10, 0).with("SkullType", 2i8).with("Rotation", 90.0f32).with("DoingAnimation", 1i8),
        tag("ShulkerBox", 3, 10, 0).with("facing", 1i8),
    ];
    assert!(read_into_column(&ITEMS, &tags, &mut col, false).is_empty());
    assert_eq!(col.block(0, 10, 0).block_type(), Some(BlockType::RED_BED));
    // Reversed id 15 is white... id 15 reversed is dye 0.
    assert_eq!(col.block(1, 10, 0).block_type(), Some(BlockType::WHITE_BANNER));
    assert_eq!(col.block(2, 10, 0).block_type(), Some(BlockType::ZOMBIE_HEAD));
    assert_eq!(col.block(2, 10, 0).get(states::ROTATION), Some(Rotation::_4));
    assert_eq!(col.block(2, 10, 0).get(states::POWERED), Some(Bool::TRUE));
    assert_eq!(col.block(3, 10, 0).get(states::FACING_ALL), Some(FacingDirection::UP));
    let BlockEntityData::Banner(b) = &col.block_entities[1].data else { panic!() };
    assert_eq!(b.base, None, "the base moved into the block");
    assert_eq!(b.patterns.len(), 1);
}

#[test]
fn wall_banners_and_wall_skulls_keep_their_shape() {
    let mut col = column();
    let wall = |ty| Arc::new(BlockIdentifier::new(ty).with(states::FACING_HORIZONTAL, FacingDirectionHorizontal::EAST));
    col.set_block(0, 1, 0, wall(BlockType::WHITE_WALL_BANNER));
    col.set_block(1, 1, 0, wall(BlockType::SKELETON_WALL_SKULL));
    let tags = [tag("Banner", 0, 1, 0).with("Base", 1i32), tag("Skull", 1, 1, 0).with("SkullType", 3i8)];
    read_into_column(&ITEMS, &tags, &mut col, false);
    assert_eq!(col.block(0, 1, 0).block_type(), Some(BlockType::RED_WALL_BANNER));
    assert_eq!(col.block(0, 1, 0).get(states::FACING_HORIZONTAL), Some(FacingDirectionHorizontal::EAST));
    assert_eq!(col.block(1, 1, 0).block_type(), Some(BlockType::PLAYER_WALL_HEAD));
}

#[test]
fn jukebox_and_lectern_set_their_has_states() {
    let mut col = column();
    col.set_block(0, 1, 0, block(BlockType::JUKEBOX));
    col.set_block(1, 1, 0, block(BlockType::LECTERN));
    let record = compound! { "Name" => "minecraft:music_disc_13", "Count" => 1i8, "Damage" => 0i16 };
    let tags = [tag("Jukebox", 0, 1, 0).with("RecordItem", record), tag("Lectern", 1, 1, 0)];
    read_into_column(&ITEMS, &tags, &mut col, false);
    assert_eq!(col.block(0, 1, 0).get(states::HAS_RECORD), Some(Bool::TRUE));
    assert_eq!(col.block(1, 1, 0).get(states::HAS_BOOK), Some(Bool::FALSE));
}

#[test]
fn bedrock_only_block_entities_become_states_or_entities() {
    let mut col = column();
    col.set_block(0, 1, 0, block(BlockType::PISTON));
    col.set_block(1, 1, 0, block(BlockType::NOTE_BLOCK));
    col.set_block(2, 1, 0, block(BlockType::ITEM_FRAME_BEDROCK));
    col.set_block(3, 1, 0, block(BlockType::FLOWER_POT));
    let plant = compound! { "name" => "minecraft:red_flower", "states" => compound! { "flower_type" => "poppy" } };
    let item = compound! { "Name" => "minecraft:stick", "Count" => 1i8, "Damage" => 0i16 };
    let tags = [
        tag("PistonArm", 0, 1, 0).with("State", 2i8).with("Sticky", 1i8),
        tag("Music", 1, 1, 0).with("note", 7i8),
        tag("ItemFrame", 2, 1, 0).with("Item", item).with("ItemRotation", 90.0f32),
        tag("FlowerPot", 3, 1, 0).with("PlantBlock", plant),
        tag("Lodestone", 4, 1, 0),
    ];
    assert!(read_into_column(&ITEMS, &tags, &mut col, false).is_empty());
    assert!(col.block_entities.is_empty());
    assert_eq!(col.block(0, 1, 0).get(states::EXTENDED), Some(Bool::TRUE));
    assert_eq!(col.block(1, 1, 0).get(states::NOTE), Some(Note::_7));
    assert!(col.block(2, 1, 0).is_air(), "the frame block was replaced by an entity");
    let frame = col.entities.iter().find_map(|e| match &e.data {
        EntityData::ItemFrame(f) => Some((e, f)),
        _ => None,
    });
    let (entity, frame) = frame.expect("item frame entity");
    assert_eq!(entity.position, [2.0, 1.0, 0.0]);
    assert_eq!(frame.item_rotation, 2);
    assert!(frame.item.is_some());
    assert_eq!(col.block(3, 1, 0).block_type(), Some(BlockType::POTTED_POPPY));
}

#[test]
fn bedrock_only_reads_without_a_column() {
    let c = tag("PistonArm", 9, 8, 7).with("State", 2i8);
    assert!(read_block_entity(&ITEMS, &c, true).is_none());
    let Some(Read::BedrockOnly(only)) = read_block_entity_full(&ITEMS, &c, true) else { panic!() };
    assert_eq!((only.x, only.y, only.z), (9, 8, 7));
    assert_eq!(only.kind, BedrockOnlyKind::PistonArm { sticky: false, extended: true });
}

#[test]
fn unreadable_entities_are_reported() {
    let mut col = column();
    let tags = [tag("SomeMod", 0, 0, 0), tag("Bell", 1, 0, 0)];
    let missing = read_into_column(&ITEMS, &tags, &mut col, false);
    assert_eq!(missing, vec!["SomeMod".to_string()]);
    assert_eq!(col.block_entities.len(), 1);
    let missing = read_into_column(&ITEMS, &tags, &mut col, true);
    assert!(missing.is_empty());
    assert_eq!(col.block_entities.len(), 3);
}

#[test]
fn copper_golem_statue_pose_goes_into_the_block() {
    let mut col = column();
    col.set_block(0, 1, 0, block(BlockType::COPPER_GOLEM_STATUE));
    read_into_column(&ITEMS, &[tag("CopperGolemStatue", 0, 1, 0).with("Pose", 2i32)], &mut col, false);
    assert_eq!(
        col.block(0, 1, 0).get(states::COPPER_GOLEM_POSE),
        Some(crate::convert::model::CopperGolemPose::RUNNING)
    );
}

// The real world.

#[test]
fn test_world_block_entities() {
    use crate::convert::bedrock::db::{BedrockDb, LevelDb};
    use crate::convert::bedrock::detect::detect_bedrock;
    use crate::convert::bedrock::keys::{self, ChunkTag};
    use crate::convert::bedrock::palette::decode_nbt_concat;

    let Some(path) = crate::convert::bedrock::test_world_path() else {
        eprintln!("test world not present, skipping");
        return;
    };
    let info = detect_bedrock(&path).expect("detect");
    let mut db = LevelDb::open_read(&info.db_path()).expect("open");
    let columns = keys::enumerate_columns(&mut db).unwrap();
    let items = BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap());

    let mut by_id: std::collections::BTreeMap<String, usize> = Default::default();
    let mut model_kinds: std::collections::BTreeMap<BlockEntityKind, usize> = Default::default();
    let mut dropped = Vec::new();
    let mut in_columns = 0;
    for c in &columns {
        let Some(value) = db.get(&keys::chunk_key(c.dim, c.x, c.z, ChunkTag::BlockEntity)).unwrap() else { continue };
        let tags: Vec<Compound> = decode_nbt_concat(&value)
            .unwrap()
            .into_iter()
            .filter_map(|(_, t)| if let Tag::Compound(c) = t { Some(c) } else { None })
            .collect();
        for t in &tags {
            let id = block_entity_id(t).unwrap_or("?").to_string();
            *by_id.entry(id.clone()).or_default() += 1;
            match read_block_entity_full(&items, t, false) {
                Some(Read::Entity { entity, .. }) => *model_kinds.entry(entity.kind()).or_default() += 1,
                Some(Read::BedrockOnly(_)) => {}
                None => dropped.push(id),
            }
        }
        let mut col = Column::new(ChunkCoord::new(c.x, c.z));
        let missing = read_into_column(&items, &tags, &mut col, false);
        assert!(missing.is_empty(), "{missing:?}");
        in_columns += col.block_entities.len();
    }
    eprintln!("block entity ids: {by_id:?}");
    eprintln!("model kinds: {model_kinds:?}");
    assert!(dropped.is_empty(), "dropped {dropped:?}");
    assert_eq!(by_id["CommandBlock"], 24);
    assert_eq!(by_id["HangingSign"], 12);
    assert_eq!(by_id["Chest"], 7);
    assert_eq!(by_id["Sign"], 5);
    assert_eq!(by_id["PistonArm"], 3);
    assert_eq!(model_kinds[&BlockEntityKind::CommandBlock], 24);
    assert_eq!(model_kinds[&BlockEntityKind::HangingSign], 12);
    assert_eq!(model_kinds[&BlockEntityKind::Chest], 7);
    assert_eq!(model_kinds[&BlockEntityKind::Sign], 5);
    // Piston arms are Bedrock only and are applied to the (here empty) blocks instead.
    assert_eq!(in_columns, by_id.values().sum::<usize>() - 3);

    // Print the chest items.
    let mut enchanted = 0;
    for c in &columns {
        let Some(value) = db.get(&keys::chunk_key(c.dim, c.x, c.z, ChunkTag::BlockEntity)).unwrap() else { continue };
        for (_, t) in decode_nbt_concat(&value).unwrap() {
            let Tag::Compound(t) = t else { continue };
            if block_entity_id(&t) != Some("Chest") {
                continue;
            }
            let be = read_block_entity(&items, &t, false).unwrap();
            let chest = be.data.container().unwrap();
            for (slot, stack) in &chest.items {
                eprintln!(
                    "chest ({}, {}, {}) slot {slot}: {:?} x{:?} ench {:?}",
                    be.x, be.y, be.z, stack.id, stack.props.amount, stack.props.enchantments
                );
                if stack.props.enchantments.as_ref().is_some_and(|e| !e.is_empty()) {
                    enchanted += 1;
                }
            }
        }
    }
    assert!(enchanted > 0, "the chests hold enchanted items");
}
