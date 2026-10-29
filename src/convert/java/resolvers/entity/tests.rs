//! Tests of the entity writer: Bedrock actors read by the Bedrock resolver and written for several Java versions.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use super::*;
use crate::convert::bedrock::data_version::BedrockVersion;
use crate::convert::bedrock::resolvers::entity::{HOST_UNIQUE_ID, read_actor};
use crate::convert::bedrock::resolvers::itemstack::BedrockItems;
use crate::convert::java::resolvers::hooks::WorldItemHooks;
use crate::convert::level::{ConversionSettings, EntityFilter};
use crate::convert::model::entity::Tile;
use crate::convert::model::{FacingDirection, ItemType, VanillaEntityType};
use crate::convert::nbt::compound;

static BEDROCK: LazyLock<BedrockItems> =
    LazyLock::new(|| BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap()));

fn v(major: i32, minor: i32, patch: i32) -> Version {
    Version::new(major, minor, patch)
}

/// The versions the switches are tested at, oldest first.
fn versions() -> Vec<Version> {
    vec![
        v(1, 13, 2),
        v(1, 15, 2),
        v(1, 16, 5),
        v(1, 18, 2),
        v(1, 19, 2),
        v(1, 20, 4),
        v(1, 20, 6),
        v(1, 21, 4),
        v(1, 21, 5),
        v(1, 21, 11),
        v(26, 1, 0),
        v(26, 3, 0),
    ]
}

fn floats(values: &[f32]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Float(*v)).collect())
}

fn actor(identifier: &str, unique_id: i64, pos: [f32; 3]) -> Compound {
    compound! {
        "identifier" => identifier,
        "UniqueID" => unique_id,
        "Pos" => floats(&pos),
        "Rotation" => floats(&[90.0, 0.0]),
        "OwnerNew" => -1i64,
        "LeasherID" => -1i64,
    }
}

fn item(name: &str, count: i8) -> Compound {
    compound! { "Name" => name, "Count" => count, "Damage" => 0i16, "WasPickedUp" => 0i8 }
}

fn slot(mut item: Compound, slot: i8) -> Tag {
    item.put("Slot", slot);
    Tag::Compound(item)
}

fn health(current: f32, max: f32) -> Tag {
    Tag::List(vec![Tag::Compound(compound! {
        "Name" => "minecraft:health", "Current" => current, "Max" => max, "Base" => max,
    })])
}

fn definitions(groups: &[&str]) -> Tag {
    Tag::List(groups.iter().map(|g| Tag::String(format!("+{g}"))).collect())
}

fn read(tag: &Compound, settings: &ConversionSettings, knots: &HashMap<i64, [f64; 3]>) -> Option<Entity> {
    read_actor(&BEDROCK, settings, tag, knots)
}

fn write_with(entity: &Entity, version: Version, owner: Option<Uuid>) -> Option<Compound> {
    let format = JavaFormat::for_version(version);
    let items = JavaItems::new(&format, format.version);
    let renamed = format.renamed_block_state_fields;
    let block_state = |b: &BlockIdentifier| {
        let id = items.lookups().block_id(&Arc::new(b.clone()))?;
        Some(Compound::new().with(if renamed { "id" } else { "Name" }, id))
    };
    let hooks = WorldItemHooks::new(&items, &format, 0);
    let ctx = EntityContext {
        items: &items,
        format: &format,
        hooks: &hooks,
        source: Some(&*BEDROCK),
        block_state: &block_state,
        pet_owner: owner,
    };
    write_entity(&ctx, entity)
}

fn convert(tag: &Compound, version: Version) -> Compound {
    let entity = read(tag, &ConversionSettings::default(), &HashMap::new()).expect("kept");
    write_with(&entity, version, None).expect("written")
}

fn s<'a>(c: &'a Compound, key: &str) -> Option<&'a str> {
    c.get(key).and_then(Tag::as_str)
}

fn i(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_integer)
}

fn sub<'a>(c: &'a Compound, key: &str) -> &'a Compound {
    c.get(key).and_then(Tag::as_compound).unwrap_or_else(|| panic!("no {key} in {c:?}"))
}

#[test]
fn uuids_are_derived_from_the_unique_id() {
    let a = uuid_of_unique_id(-4294967295);
    assert_eq!(a, uuid_of_unique_id(-4294967295), "the same id gives the same UUID");
    assert_ne!(a, uuid_of_unique_id(-4294967294));
    assert_eq!(a.get_version_num(), 3);
    let cow = convert(&actor("minecraft:cow", 77, [0.5, 64.0, 0.5]), v(26, 3, 0));
    let ints = common::uuid_ints(uuid_of_unique_id(77));
    assert_eq!(cow.get("UUID").and_then(Tag::as_int_array), Some(&ints));
    let old = convert(&actor("minecraft:cow", 77, [0.5, 64.0, 0.5]), v(1, 15, 2));
    assert!(old.contains_key("UUIDMost") && old.contains_key("UUIDLeast") && !old.contains_key("UUID"));
}

#[test]
fn common_fields() {
    let mut tag = actor("minecraft:villager_v2", 5, [-7.7, 34.495, -2.5]);
    tag.put("CustomName", "§6Josim Mama");
    let out = convert(&tag, v(26, 3, 0));
    assert_eq!(s(&out, "id"), Some("minecraft:villager"));
    let pos = out.get("Pos").and_then(Tag::as_list).unwrap();
    assert_eq!(pos.len(), 3);
    assert!((pos[1].as_f64().unwrap() - 34.495).abs() < 1e-4);
    assert_eq!(out.get("Rotation").and_then(Tag::as_list).unwrap()[0], Tag::Float(90.0));
    assert_eq!(sub(&out, "CustomName").get("text").and_then(Tag::as_str), Some("§6Josim Mama"));
    assert_eq!(i(&out, "PersistenceRequired"), Some(1));
    // A JSON string before 1.21.5.
    let old = convert(&tag, v(1, 21, 4));
    assert_eq!(s(&old, "CustomName"), Some(r#"{"text":"§6Josim Mama"}"#));
}

#[test]
fn identifier_renames() {
    for (bedrock, java, version) in [
        ("minecraft:villager_v2", "minecraft:villager", v(26, 3, 0)),
        ("minecraft:zombie_villager_v2", "minecraft:zombie_villager", v(26, 3, 0)),
        ("minecraft:zombie_pigman", "minecraft:zombified_piglin", v(26, 3, 0)),
        ("minecraft:zombie_pigman", "minecraft:zombie_pigman", v(1, 15, 2)),
        ("minecraft:ender_crystal", "minecraft:end_crystal", v(26, 3, 0)),
        ("minecraft:evocation_illager", "minecraft:evoker", v(26, 3, 0)),
        ("minecraft:chest_minecart", "minecraft:chest_minecart", v(26, 3, 0)),
        ("minecraft:snow_golem", "minecraft:snow_golem", v(26, 3, 0)),
        ("minecraft:mooshroom", "minecraft:mooshroom", v(26, 3, 0)),
    ] {
        let out = convert(&actor(bedrock, 1, [0.0, 64.0, 0.0]), version);
        assert_eq!(s(&out, "id"), Some(java), "{bedrock} for {version}");
    }
    // Trader llamas are llamas with MarkVariant 1.
    let mut llama = actor("minecraft:llama", 1, [0.0, 64.0, 0.0]);
    llama.put("MarkVariant", 1);
    assert_eq!(s(&convert(&llama, v(26, 3, 0)), "id"), Some("minecraft:trader_llama"));
    // A type the version does not have is not written.
    let entity =
        read(&actor("minecraft:armadillo", 1, [0.0, 64.0, 0.0]), &ConversionSettings::default(), &HashMap::new());
    assert!(write_with(&entity.unwrap(), v(1, 19, 2), None).is_none());
}

#[test]
fn cat_variants() {
    let names = [
        "white",
        "black",
        "red",
        "siamese",
        "british_shorthair",
        "calico",
        "persian",
        "ragdoll",
        "tabby",
        "all_black",
        "jellie",
    ];
    let cat_types = [8, 1, 2, 3, 4, 5, 6, 7, 0, 10, 9];
    for (variant, name) in names.iter().enumerate() {
        let mut tag = actor("minecraft:cat", 1, [0.0, 64.0, 0.0]);
        tag.put("Variant", variant as i32);
        let out = convert(&tag, v(26, 3, 0));
        assert_eq!(s(&out, "variant"), Some(format!("minecraft:{name}").as_str()));
        let old = convert(&tag, v(1, 18, 2));
        assert_eq!(i(&old, "CatType"), Some(cat_types[variant]), "{name}");
        assert!(!old.contains_key("variant"));
    }
}

#[test]
fn pets_have_their_owner() {
    let mut tag = actor("minecraft:wolf", 9, [91.379, 67.0, -359.7]);
    tag.put("OwnerNew", HOST_UNIQUE_ID);
    tag.put("IsTamed", 1i8);
    tag.put("Sitting", 1i8);
    tag.put("Color", 2i8);
    tag.put("CustomName", "§5Akela ");
    tag.put("Attributes", health(16.0, 20.0));
    let entity = read(&tag, &ConversionSettings::default(), &HashMap::new()).unwrap();
    let account = Uuid::from_u128(0x0822296f_ba99_4e86_8126_8d1a3b0403ef);
    // With an account the host's pets are that account's.
    let out = write_with(&entity, v(26, 3, 0), Some(account)).unwrap();
    assert_eq!(out.get("Owner").and_then(Tag::as_int_array), Some(&common::uuid_ints(account)));
    assert_eq!(i(&out, "Sitting"), Some(1));
    assert_eq!(i(&out, "CollarColor"), Some(2));
    assert_eq!(out.get("Health"), Some(&Tag::Float(16.0)));
    // Without one they stay tamed, owned by the host's derived UUID.
    let out = write_with(&entity, v(26, 3, 0), None).unwrap();
    assert_eq!(
        out.get("Owner").and_then(Tag::as_int_array),
        Some(&common::uuid_ints(uuid_of_unique_id(HOST_UNIQUE_ID)))
    );
    // Before 1.16 the owner is a string.
    let old = write_with(&entity, v(1, 15, 2), Some(account)).unwrap();
    assert_eq!(s(&old, "OwnerUUID"), Some("0822296f-ba99-4e86-8126-8d1a3b0403ef"));
    // Another player's pet gets that player's derived UUID; a wild wolf has no owner and no collar.
    tag.put("OwnerNew", -12884901887i64);
    let out = convert(&tag, v(26, 3, 0));
    assert_eq!(out.get("Owner").and_then(Tag::as_int_array), Some(&common::uuid_ints(uuid_of_unique_id(-12884901887))));
    tag.put("OwnerNew", -1i64);
    let wild = convert(&tag, v(26, 3, 0));
    assert!(!wild.contains_key("Owner") && !wild.contains_key("CollarColor"));
}

fn horse() -> Compound {
    let mut tag = actor("minecraft:horse", 3, [110.676, 67.0, -356.831]);
    tag.put("Variant", 4);
    tag.put("MarkVariant", 4);
    tag.put("IsTamed", 1i8);
    tag.put("OwnerNew", HOST_UNIQUE_ID);
    tag.put(
        "ChestItems",
        Tag::List(vec![slot(item("minecraft:saddle", 1), 0), slot(item("minecraft:diamond_horse_armor", 1), 1)]),
    );
    let mut attributes = vec![
        Tag::Compound(compound! { "Name" => "minecraft:health", "Current" => 26.0f32, "Max" => 26.0f32 }),
        Tag::Compound(compound! { "Name" => "minecraft:movement", "Current" => 0.28f32, "Max" => 1.0f32 }),
    ];
    attributes.push(Tag::Compound(compound! { "Name" => "minecraft:horse.jump_strength", "Current" => 0.599f32 }));
    tag.put("Attributes", Tag::List(attributes));
    tag
}

#[test]
fn horses_by_version() {
    let tag = horse();
    let new = convert(&tag, v(26, 3, 0));
    assert_eq!(i(&new, "Variant"), Some(4 | (4 << 8)));
    assert_eq!(i(&new, "Tame"), Some(1));
    let equipment = sub(&new, "equipment");
    assert_eq!(s(sub(equipment, "saddle"), "id"), Some("minecraft:saddle"));
    assert_eq!(s(sub(equipment, "body"), "id"), Some("minecraft:diamond_horse_armor"));
    assert_eq!(sub(&new, "drop_chances").get("body"), Some(&Tag::Float(2.0)));
    let attributes = new.get("attributes").and_then(Tag::as_list).unwrap();
    let ids: Vec<&str> = attributes.iter().filter_map(|a| a.as_compound()?.get_str("id")).collect();
    assert_eq!(ids, ["minecraft:max_health", "minecraft:movement_speed", "minecraft:jump_strength"]);

    let mid = convert(&tag, v(1, 20, 6));
    assert_eq!(s(sub(&mid, "body_armor_item"), "id"), Some("minecraft:diamond_horse_armor"));
    assert_eq!(s(sub(&mid, "SaddleItem"), "id"), Some("minecraft:saddle"));
    let names: Vec<&str> = mid
        .get("Attributes")
        .and_then(Tag::as_list)
        .unwrap()
        .iter()
        .filter_map(|a| a.as_compound()?.get_str("Name"))
        .collect();
    assert_eq!(names[2], "minecraft:generic.jump_strength");

    let old = convert(&tag, v(1, 19, 2));
    assert_eq!(s(sub(&old, "ArmorItem"), "id"), Some("minecraft:diamond_horse_armor"));
    assert!(!old.contains_key("ArmorItems") && !old.contains_key("equipment"));
    let names: Vec<&str> = old
        .get("Attributes")
        .and_then(Tag::as_list)
        .unwrap()
        .iter()
        .filter_map(|a| a.as_compound()?.get_str("Name"))
        .collect();
    assert_eq!(
        names,
        ["minecraft:generic.max_health", "minecraft:generic.movement_speed", "minecraft:horse.jump_strength"]
    );
    let oldest = convert(&tag, v(1, 15, 2));
    let names: Vec<&str> = oldest
        .get("Attributes")
        .and_then(Tag::as_list)
        .unwrap()
        .iter()
        .filter_map(|a| a.as_compound()?.get_str("Name"))
        .collect();
    assert_eq!(names, ["generic.maxHealth", "generic.movementSpeed", "horse.jumpStrength"]);
}

#[test]
fn chested_llamas_and_donkeys() {
    let mut llama = actor("minecraft:llama", 4, [0.0, 64.0, 0.0]);
    llama.put("Variant", 1);
    llama.put("Strength", 2);
    llama.put("Chested", 1i8);
    let carpet = compound! {
        "Name" => "minecraft:carpet", "Count" => 1i8, "Damage" => 0i16,
        "Block" => compound! { "name" => "minecraft:carpet", "states" => compound! { "color" => "brown" } },
    };
    llama.put("ChestItems", Tag::List(vec![slot(carpet, 0), slot(item("minecraft:stone", 5), 1)]));
    let new = convert(&llama, v(26, 3, 0));
    assert_eq!(i(&new, "Variant"), Some(1));
    assert_eq!(i(&new, "Strength"), Some(2));
    assert_eq!(s(sub(sub(&new, "equipment"), "body"), "id"), Some("minecraft:brown_carpet"));
    let items = new.get("Items").and_then(Tag::as_list).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(i(items[0].as_compound().unwrap(), "Slot"), Some(0));
    let old = convert(&llama, v(1, 19, 2));
    assert_eq!(s(sub(&old, "DecorItem"), "id"), Some("minecraft:brown_carpet"));
    assert_eq!(i(old.get("Items").and_then(Tag::as_list).unwrap()[0].as_compound().unwrap(), "Slot"), Some(2));
}

#[test]
fn pigs_keep_their_saddle() {
    let mut pig = actor("minecraft:pig", 6, [0.0, 64.0, 0.0]);
    pig.put("Saddled", 1i8);
    pig.put("IsBaby", 1i8);
    let new = convert(&pig, v(26, 3, 0));
    assert_eq!(s(sub(sub(&new, "equipment"), "saddle"), "id"), Some("minecraft:saddle"));
    assert_eq!(i(&new, "Age"), Some(-24000));
    let old = convert(&pig, v(1, 21, 4));
    assert_eq!(i(&old, "Saddle"), Some(1));
    assert!(!old.contains_key("equipment"));
}

#[test]
fn villagers() {
    let mut tag = actor("minecraft:villager_v2", 8, [0.0, 64.0, 0.0]);
    tag.put("Variant", 11);
    tag.put("MarkVariant", 6);
    tag.put("TradeTier", 1);
    tag.put("TradeExperience", 12);
    tag.put("definitions", definitions(&["minecraft:villager_v2", "librarian"]));
    let mut buy = item("minecraft:emerald", 1);
    buy.put("Damage", 32767i16);
    let recipe = |tier: i32| {
        Tag::Compound(compound! {
            "buyA" => buy.clone(), "buyCountA" => 24, "buyB" => item("minecraft:book", 1), "buyCountB" => 1,
            "sell" => item("minecraft:bookshelf", 1), "maxUses" => 12, "uses" => 3, "rewardExp" => 1i8,
            "traderExp" => 5, "priceMultiplierA" => 0.2f32, "demand" => 0, "tier" => tier,
        })
    };
    tag.put("Offers", compound! { "Recipes" => Tag::List(vec![recipe(0), recipe(1), recipe(3)]) });
    tag.put("ChestItems", Tag::List(vec![slot(item("minecraft:wheat", 3), 0)]));
    let out = convert(&tag, v(26, 3, 0));
    let data = sub(&out, "VillagerData");
    // The group wins over Variant (11, butcher).
    assert_eq!(s(data, "profession"), Some("minecraft:librarian"));
    assert_eq!(s(data, "type"), Some("minecraft:taiga"));
    assert_eq!(i(data, "level"), Some(2));
    assert_eq!(i(&out, "Xp"), Some(12));
    assert_eq!(i(&out, "VillagerDataFinalized"), Some(1));
    let recipes = sub(&out, "Offers").get("Recipes").and_then(Tag::as_list).unwrap();
    assert_eq!(recipes.len(), 2, "the locked tier is left out");
    let first = recipes[0].as_compound().unwrap();
    let buy = sub(first, "buy");
    assert_eq!((s(buy, "id"), i(buy, "count")), (Some("minecraft:emerald"), Some(24)));
    assert_eq!(s(sub(first, "buyB"), "id"), Some("minecraft:book"));
    assert_eq!(first.get("priceMultiplier"), Some(&Tag::Float(0.2)));
    assert_eq!(i(first, "xp"), Some(5));
    assert_eq!(out.get("Inventory").and_then(Tag::as_list).map(Vec::len), Some(1));
    // Variant when there is no group; no VillagerDataFinalized before 26.1.
    tag.put("definitions", definitions(&["minecraft:villager_v2"]));
    let old = convert(&tag, v(1, 20, 4));
    assert_eq!(s(sub(&old, "VillagerData"), "profession"), Some("minecraft:butcher"));
    assert!(!old.contains_key("VillagerDataFinalized"));
    assert_eq!(
        i(
            sub(sub(&old, "Offers").get("Recipes").and_then(Tag::as_list).unwrap()[0].as_compound().unwrap(), "buy"),
            "Count"
        ),
        Some(24)
    );
}

#[test]
fn painting_position_and_format() {
    let mut tag = actor("minecraft:painting", 11, [-64.969, 51.5, -14.0]);
    tag.put("Motive", "Sunset");
    tag.put("Direction", 3i8);
    let entity = read(&tag, &ConversionSettings::default(), &HashMap::new()).unwrap();
    let EntityData::Painting(p) = &entity.data else { panic!("{:?}", entity.data) };
    assert_eq!(p.tile, Tile { x: -65, y: 51, z: -14 });
    let new = write_with(&entity, v(26, 3, 0), None).unwrap();
    assert_eq!(s(&new, "variant"), Some("minecraft:sunset"));
    assert_eq!(i(&new, "facing"), Some(3));
    assert_eq!(new.get("block_pos").and_then(Tag::as_int_array), Some(&vec![-65, 51, -14]));
    let mid = write_with(&entity, v(1, 19, 2), None).unwrap();
    assert_eq!((i(&mid, "TileX"), i(&mid, "TileY"), i(&mid, "TileZ")), (Some(-65), Some(51), Some(-14)));
    assert_eq!(s(&mid, "variant"), Some("minecraft:sunset"));
    let old = write_with(&entity, v(1, 18, 2), None).unwrap();
    assert_eq!((s(&old, "Motive"), i(&old, "Facing")), (Some("minecraft:sunset"), Some(3)));
    // Odd sizes are centred on their block; tall ones move down half a block.
    for (motive, dir, center, tile) in [
        ("Kebab", 0i8, [10.5, 70.5, 3.03125], (10, 70, 3)),
        ("Wanderer", 2, [-3.5, 65.0, 7.96875], (-4, 64, 7)),
        ("SkullAndRoses", 1, [4.96875, 70.0, 3.0], (4, 69, 2)),
        ("DonkeyKong", 3, [-0.96875, 65.5, 0.0], (-1, 65, 0)),
    ] {
        let mut tag = actor("minecraft:painting", 12, [center[0] as f32, center[1] as f32, center[2] as f32]);
        tag.put("Motive", motive);
        tag.put("Direction", dir);
        let entity = read(&tag, &ConversionSettings::default(), &HashMap::new()).unwrap();
        let EntityData::Painting(p) = &entity.data else { panic!() };
        assert_eq!((p.tile.x, p.tile.y, p.tile.z), tile, "{motive}");
    }
}

#[test]
fn item_frames() {
    let mut frame = ItemFrame {
        tile: Tile { x: 2, y: 1, z: 0 },
        direction: Some(FacingDirection::SOUTH),
        item_rotation: 2,
        item: Some(Box::new(ItemStack::with_amount(ItemType::DIAMOND, 1))),
        ..ItemFrame::default()
    };
    let entity = Entity::new(EntityData::ItemFrame(frame.clone()));
    let new = write_with(&entity, v(26, 3, 0), None).unwrap();
    assert_eq!(s(&new, "id"), Some("minecraft:item_frame"));
    assert_eq!(i(&new, "Facing"), Some(3));
    assert_eq!(new.get("block_pos").and_then(Tag::as_int_array), Some(&vec![2, 1, 0]));
    assert_eq!(i(&new, "ItemRotation"), Some(2));
    assert_eq!(s(sub(&new, "Item"), "id"), Some("minecraft:diamond"));
    let pos = new.get("Pos").and_then(Tag::as_list).unwrap();
    assert_eq!(pos[2], Tag::Double(0.03125));
    let old = write_with(&entity, v(1, 20, 4), None).unwrap();
    assert_eq!((i(&old, "TileX"), i(&old, "TileY"), i(&old, "TileZ")), (Some(2), Some(1), Some(0)));
    frame.glow = true;
    frame.item = None;
    let glow = Entity::new(EntityData::ItemFrame(frame));
    let out = write_with(&glow, v(26, 3, 0), None).unwrap();
    assert_eq!(s(&out, "id"), Some("minecraft:glow_item_frame"));
    assert!(!out.contains_key("Item"));
    // Glow frames came with 1.17.
    assert!(write_with(&glow, v(1, 16, 5), None).is_none());
}

#[test]
fn equipment_by_version() {
    let mut zombie = actor("minecraft:zombie", 20, [0.0, 64.0, 0.0]);
    let mut sword = item("minecraft:iron_sword", 1);
    sword.put("WasPickedUp", 1i8);
    zombie.put("Mainhand", Tag::List(vec![Tag::Compound(sword)]));
    zombie.put("Offhand", Tag::List(vec![Tag::Compound(item("", 0))]));
    zombie.put(
        "Armor",
        Tag::List(vec![
            Tag::Compound(item("minecraft:iron_helmet", 1)),
            Tag::Compound(item("", 0)),
            Tag::Compound(item("", 0)),
            Tag::Compound(item("minecraft:iron_boots", 1)),
        ]),
    );
    let new = convert(&zombie, v(26, 3, 0));
    let equipment = sub(&new, "equipment");
    assert_eq!(s(sub(equipment, "mainhand"), "id"), Some("minecraft:iron_sword"));
    assert_eq!(s(sub(equipment, "head"), "id"), Some("minecraft:iron_helmet"));
    assert_eq!(s(sub(equipment, "feet"), "id"), Some("minecraft:iron_boots"));
    assert!(!equipment.contains_key("offhand"));
    let chances = sub(&new, "drop_chances");
    assert_eq!(chances.len(), 1);
    assert_eq!(chances.get("mainhand"), Some(&Tag::Float(2.0)));
    // A hostile mob holding loot it picked up stays.
    assert_eq!(i(&new, "PersistenceRequired"), Some(1));

    let old = convert(&zombie, v(1, 21, 4));
    let armor = old.get("ArmorItems").and_then(Tag::as_list).unwrap();
    assert_eq!(armor.len(), 4);
    assert_eq!(armor[0].as_compound().and_then(|c| c.get_str("id")), Some("minecraft:iron_boots"));
    assert_eq!(armor[3].as_compound().and_then(|c| c.get_str("id")), Some("minecraft:iron_helmet"));
    assert!(armor[1].as_compound().unwrap().is_empty());
    let hand_chances = old.get("HandDropChances").and_then(Tag::as_list).unwrap();
    assert_eq!(hand_chances, &vec![Tag::Float(2.0), Tag::Float(0.085)]);
    // An ordinary zombie despawns.
    let plain = convert(&actor("minecraft:zombie", 21, [0.0, 64.0, 0.0]), v(26, 3, 0));
    assert_eq!(i(&plain, "PersistenceRequired"), Some(0));
}

#[test]
fn shulker_boxes_keep_their_contents() {
    let mut stand = actor("minecraft:armor_stand", 30, [0.5, 64.0, 0.5]);
    let mut shulker = item("minecraft:undyed_shulker_box", 1);
    shulker.put("tag", compound! { "Items" => Tag::List(vec![slot(item("minecraft:diamond", 5), 0)]) });
    stand.put("Mainhand", Tag::List(vec![Tag::Compound(shulker)]));
    let new = convert(&stand, v(26, 3, 0));
    assert_eq!(i(&new, "ShowArms"), Some(1));
    let held = sub(sub(&new, "equipment"), "mainhand");
    let container = sub(held, "components").get("minecraft:container").and_then(Tag::as_list).expect("contents");
    let first = sub(container[0].as_compound().unwrap(), "item");
    assert_eq!((s(first, "id"), i(first, "count")), (Some("minecraft:diamond"), Some(5)));
    let old = convert(&stand, v(1, 20, 4));
    let held = old.get("HandItems").and_then(Tag::as_list).unwrap()[0].as_compound().unwrap();
    let items = sub(sub(held, "tag"), "BlockEntityTag").get("Items").and_then(Tag::as_list).expect("contents");
    assert_eq!(items.len(), 1);
}

#[test]
fn leashed_mobs_keep_their_knot() {
    let mut cow = actor("minecraft:cow", 40, [106.0, 67.0, -356.0]);
    cow.put("LeasherID", -3040836843111i64);
    let knots = HashMap::from([(-3040836843111i64, [106.5, 67.25, -356.5])]);
    let entity = read(&cow, &ConversionSettings::default(), &knots).unwrap();
    assert_eq!(entity.leash, Some(Tile { x: 106, y: 67, z: -357 }));
    let new = write_with(&entity, v(26, 3, 0), None).unwrap();
    assert_eq!(new.get("leash").and_then(Tag::as_int_array), Some(&vec![106, 67, -357]));
    let old = write_with(&entity, v(1, 20, 4), None).unwrap();
    assert_eq!(i(sub(&old, "Leash"), "Z"), Some(-357));
    // The knot itself is never written.
    let knot = actor("minecraft:leash_knot", -3040836843111, [106.5, 67.25, -356.5]);
    assert!(read(&knot, &ConversionSettings::default(), &HashMap::new()).is_none());
}

#[test]
fn minecarts_and_boats() {
    let mut cart = actor("minecraft:chest_minecart", 50, [-110.0, 16.5, -13509.5]);
    cart.put("LootTable", "loot_tables/chests/abandoned_mineshaft.json");
    cart.put("LootTableSeed", 377723764i64);
    let out = convert(&cart, v(26, 3, 0));
    assert_eq!(s(&out, "LootTable"), Some("minecraft:chests/abandoned_mineshaft"));
    assert_eq!(i(&out, "LootTableSeed"), Some(377723764));
    assert_eq!(out.get("Pos").and_then(Tag::as_list).unwrap()[1], Tag::Double(16.0625));
    let mut hopper = actor("minecraft:hopper_minecart", 51, [0.5, 37.85, 0.5]);
    hopper.put("ChestItems", Tag::List(vec![slot(item("minecraft:rail", 2), 3)]));
    let out = convert(&hopper, v(26, 3, 0));
    let items = out.get("Items").and_then(Tag::as_list).unwrap();
    assert_eq!(i(items[0].as_compound().unwrap(), "Slot"), Some(3));

    let mut boat = actor("minecraft:boat", 52, [0.5, 62.0, 0.5]);
    boat.put("Variant", 2);
    assert_eq!(s(&convert(&boat, v(26, 3, 0)), "id"), Some("minecraft:birch_boat"));
    let old = convert(&boat, v(1, 21, 1));
    assert_eq!((s(&old, "id"), s(&old, "Type")), (Some("minecraft:boat"), Some("birch")));
    let mut chest = actor("minecraft:chest_boat", 53, [0.5, 62.0, 0.5]);
    chest.put("Variant", 6);
    chest.put("ChestItems", Tag::List(vec![slot(item("minecraft:stone", 5), 0)]));
    let out = convert(&chest, v(26, 3, 0));
    assert_eq!(s(&out, "id"), Some("minecraft:mangrove_chest_boat"));
    assert_eq!(out.get("Items").and_then(Tag::as_list).map(Vec::len), Some(1));
    let old = convert(&chest, v(1, 19, 2));
    assert_eq!((s(&old, "id"), s(&old, "Type")), (Some("minecraft:chest_boat"), Some("mangrove")));
}

#[test]
fn small_tables() {
    let one = |id: &str, fields: &[(&str, Tag)], version: Version| {
        let mut tag = actor(id, 60, [0.0, 64.0, 0.0]);
        for (k, value) in fields {
            tag.put(*k, value.clone());
        }
        convert(&tag, version)
    };
    let latest = v(26, 3, 0);
    assert_eq!(i(&one("minecraft:rabbit", &[("Variant", Tag::Int(4))], latest), "RabbitType"), Some(4));
    assert_eq!(i(&one("minecraft:parrot", &[("Variant", Tag::Int(3))], latest), "Variant"), Some(3));
    assert_eq!(s(&one("minecraft:fox", &[("Variant", Tag::Int(1))], latest), "Type"), Some("snow"));
    let sheep = one("minecraft:sheep", &[("Color", Tag::Byte(12)), ("Sheared", Tag::Byte(1))], latest);
    assert_eq!((i(&sheep, "Color"), i(&sheep, "Sheared")), (Some(12), Some(1)));
    assert_eq!(i(&one("minecraft:slime", &[("Variant", Tag::Int(4))], latest), "Size"), Some(3));
    assert_eq!(i(&one("minecraft:shulker", &[("Variant", Tag::Int(16))], latest), "Color"), Some(16));
    assert_eq!(i(&one("minecraft:axolotl", &[("Variant", Tag::Int(1))], latest), "Variant"), Some(3));
    assert_eq!(s(&one("minecraft:frog", &[("Variant", Tag::Int(2))], latest), "variant"), Some("minecraft:warm"));
    assert_eq!(s(&one("minecraft:mooshroom", &[("Variant", Tag::Int(1))], latest), "Type"), Some("brown"));
    let panda = one("minecraft:panda", &[("definitions", definitions(&["minecraft:panda_lazy"]))], latest);
    assert_eq!((s(&panda, "MainGene"), s(&panda, "HiddenGene")), (Some("lazy"), Some("lazy")));
    let golem = one("minecraft:iron_golem", &[("definitions", definitions(&["minecraft:village_created"]))], latest);
    assert_eq!(i(&golem, "PlayerCreated"), Some(0));
    assert_eq!(i(&one("minecraft:snow_golem", &[("Sheared", Tag::Byte(1))], latest), "Pumpkin"), Some(0));
    let fish = one(
        "minecraft:tropicalfish",
        &[("Variant", Tag::Int(1)), ("MarkVariant", Tag::Int(2)), ("Color", Tag::Byte(5)), ("Color2", Tag::Byte(14))],
        latest,
    );
    assert_eq!(i(&fish, "Variant"), Some(1 | (2 << 8) | (5 << 16) | (14 << 24)));
    let home = Tag::List(vec![Tag::Float(8657.0), Tag::Float(66.0), Tag::Float(-12404.0)]);
    let bee = one("minecraft:bee", &[("HomePos", home.clone()), ("definitions", definitions(&["has_nectar"]))], latest);
    assert_eq!(bee.get("hive_pos").and_then(Tag::as_int_array), Some(&vec![8657, 66, -12404]));
    assert_eq!(i(&bee, "HasNectar"), Some(1));
    let old_bee = one("minecraft:bee", &[("HomePos", home.clone())], v(1, 20, 4));
    assert_eq!(i(sub(&old_bee, "HivePos"), "X"), Some(8657));
    let turtle = one("minecraft:turtle", &[("HomePos", home.clone())], latest);
    assert_eq!(turtle.get("home_pos").and_then(Tag::as_int_array), Some(&vec![8657, 66, -12404]));
    let old_turtle = one("minecraft:turtle", &[("HomePos", home)], v(1, 21, 4));
    assert_eq!(i(&old_turtle, "HomePosZ"), Some(-12404));
    let carried = compound! { "name" => "minecraft:grass_block", "states" => Compound::new() };
    let enderman = one("minecraft:enderman", &[("carriedBlock", Tag::Compound(carried.clone()))], latest);
    assert_eq!(s(sub(&enderman, "carriedBlockState"), "id"), Some("minecraft:grass_block"));
    let old = one("minecraft:enderman", &[("carriedBlock", Tag::Compound(carried))], v(26, 2, 0));
    assert_eq!(s(sub(&old, "carriedBlockState"), "Name"), Some("minecraft:grass_block"));
    let air = compound! { "name" => "minecraft:air", "states" => Compound::new() };
    assert!(
        !one("minecraft:enderman", &[("carriedBlock", Tag::Compound(air))], latest).contains_key("carriedBlockState")
    );
}

#[test]
fn filters() {
    let mut named = actor("minecraft:zombie", 70, [0.0, 64.0, 0.0]);
    named.put("CustomName", "Bob");
    let mut tamed_cat = actor("minecraft:cat", 71, [0.0, 64.0, 0.0]);
    tamed_cat.put("IsTamed", 1i8);
    let cases: [(&str, Compound, [bool; 4]); 8] = [
        // [All, NoHostile, PetsAndNamedOnly, None]
        ("cow", actor("minecraft:cow", 72, [0.0, 64.0, 0.0]), [true, true, false, false]),
        ("zombie", actor("minecraft:zombie", 73, [0.0, 64.0, 0.0]), [true, false, false, false]),
        ("named zombie", named, [true, false, true, false]),
        ("tamed cat", tamed_cat, [true, true, true, false]),
        ("villager", actor("minecraft:villager_v2", 74, [0.0, 64.0, 0.0]), [true, true, true, false]),
        (
            "painting",
            {
                let mut p = actor("minecraft:painting", 75, [0.5, 64.5, 0.03125]);
                p.put("Motive", "Kebab");
                p
            },
            [true, true, true, false],
        ),
        ("armor stand", actor("minecraft:armor_stand", 76, [0.0, 64.0, 0.0]), [true, true, true, false]),
        ("creeper", actor("minecraft:creeper", 77, [0.0, 64.0, 0.0]), [true, false, false, false]),
    ];
    let filters = [EntityFilter::All, EntityFilter::NoHostile, EntityFilter::PetsAndNamedOnly, EntityFilter::None];
    for (name, tag, expected) in &cases {
        for (filter, keep) in filters.iter().zip(expected) {
            let settings = ConversionSettings::builder().entity_filter(*filter).build();
            assert_eq!(read(tag, &settings, &HashMap::new()).is_some(), *keep, "{name} with {filter:?}");
        }
    }
}

#[test]
fn transient_entities() {
    let mut dropped = actor("minecraft:item", 80, [0.0, 64.0, 0.0]);
    dropped.put("Item", item("minecraft:stone", 3));
    let mut orb = actor("minecraft:xp_orb", 81, [0.0, 64.0, 0.0]);
    orb.put("experience value", 7);
    let mut falling = actor("minecraft:falling_block", 82, [0.0, 64.0, 0.0]);
    falling.put("FallingBlock", compound! { "name" => "minecraft:sand", "states" => Compound::new() });
    let arrow = actor("minecraft:arrow", 83, [0.0, 64.0, 0.0]);
    let all = [&dropped, &orb, &falling, &arrow];
    let default = ConversionSettings::default();
    for tag in all {
        assert!(read(tag, &default, &HashMap::new()).is_none(), "{tag:?}");
    }
    let keep = ConversionSettings::builder().drop_transient_entities(false).build();
    let written: Vec<Compound> = all
        .iter()
        .map(|tag| write_with(&read(tag, &keep, &HashMap::new()).unwrap(), v(26, 3, 0), None).unwrap())
        .collect();
    assert_eq!(s(sub(&written[0], "Item"), "id"), Some("minecraft:stone"));
    assert_eq!(s(&written[1], "id"), Some("minecraft:experience_orb"));
    assert_eq!(written[1].get("Value"), Some(&Tag::Short(7)));
    assert_eq!(s(sub(&written[2], "BlockState"), "id"), Some("minecraft:sand"));
    assert_eq!(s(&written[3], "id"), Some("minecraft:arrow"));
}

#[test]
fn every_kept_type_has_a_java_id_at_the_latest_version() {
    // The types Bedrock 1.19 worlds hold (the census of a large survival world).
    for id in [
        "armor_stand",
        "bat",
        "bee",
        "blaze",
        "boat",
        "cat",
        "cave_spider",
        "chest_minecart",
        "chicken",
        "cod",
        "cow",
        "creeper",
        "dolphin",
        "donkey",
        "drowned",
        "ender_crystal",
        "ender_dragon",
        "enderman",
        "endermite",
        "evocation_illager",
        "fox",
        "frog",
        "ghast",
        "glow_squid",
        "goat",
        "guardian",
        "hoglin",
        "hopper_minecart",
        "horse",
        "husk",
        "iron_golem",
        "llama",
        "magma_cube",
        "minecart",
        "mooshroom",
        "mule",
        "ocelot",
        "panda",
        "parrot",
        "phantom",
        "pig",
        "piglin",
        "piglin_brute",
        "pillager",
        "polar_bear",
        "pufferfish",
        "rabbit",
        "ravager",
        "salmon",
        "sheep",
        "shulker",
        "silverfish",
        "skeleton",
        "skeleton_horse",
        "slime",
        "snow_golem",
        "spider",
        "squid",
        "stray",
        "strider",
        "tropicalfish",
        "turtle",
        "vex",
        "villager_v2",
        "vindicator",
        "wandering_trader",
        "witch",
        "wither",
        "wither_skeleton",
        "wolf",
        "zoglin",
        "zombie",
        "zombie_horse",
        "zombie_pigman",
        "zombie_villager_v2",
    ] {
        let tag = actor(&format!("minecraft:{id}"), 90, [0.0, 64.0, 0.0]);
        let entity = read(&tag, &ConversionSettings::default(), &HashMap::new());
        let entity = entity.unwrap_or_else(|| panic!("{id} was not read"));
        assert!(write_with(&entity, v(26, 3, 0), None).is_some(), "{id} was not written");
    }
    assert_eq!(VanillaEntityType::from_name("ITEM_FRAME"), Some(VanillaEntityType::ITEM_FRAME));
}

/// Writes one entity chunk per test entity and version for the game's DataFixer harness, when `BF_ENTITY_DUMP`
/// names a folder: `<folder>/<data version>/<name>.nbt`, gzip NBT `{DataVersion, Position, Entities}`. Run with
/// `cargo test dump_entities_for_the_harness -- --ignored`.
#[test]
#[ignore]
fn dump_entities_for_the_harness() {
    let Some(dir) = std::env::var_os("BF_ENTITY_DUMP") else { return };
    let dir = std::path::PathBuf::from(dir);
    let mut samples: Vec<(&str, Compound)> = vec![("horse", horse())];
    let mut wolf = actor("minecraft:wolf", 9, [91.379, 67.0, -359.7]);
    wolf.put("OwnerNew", HOST_UNIQUE_ID);
    wolf.put("IsTamed", 1i8);
    wolf.put("Sitting", 1i8);
    wolf.put("Color", 2i8);
    wolf.put("CustomName", "§5Akela ");
    wolf.put("Attributes", health(16.0, 20.0));
    wolf.put("LeasherID", 1234i64);
    samples.push(("wolf", wolf));
    let mut cat = actor("minecraft:cat", 10, [0.0, 64.0, 0.0]);
    cat.put("Variant", 9);
    cat.put("OwnerNew", HOST_UNIQUE_ID);
    cat.put("Color", 14i8);
    samples.push(("cat", cat));
    let mut llama = actor("minecraft:llama", 4, [0.0, 64.0, 0.0]);
    llama.put("Chested", 1i8);
    llama.put("IsTamed", 1i8);
    let carpet = compound! {
        "Name" => "minecraft:carpet", "Count" => 1i8, "Damage" => 0i16,
        "Block" => compound! { "name" => "minecraft:carpet", "states" => compound! { "color" => "brown" } },
    };
    llama.put("ChestItems", Tag::List(vec![slot(carpet, 0), slot(item("minecraft:stone", 5), 1)]));
    samples.push(("llama", llama));
    let mut donkey = actor("minecraft:donkey", 5, [0.0, 64.0, 0.0]);
    donkey.put("Chested", 1i8);
    donkey
        .put("ChestItems", Tag::List(vec![slot(item("minecraft:saddle", 1), 0), slot(item("minecraft:dirt", 1), 15)]));
    samples.push(("donkey", donkey));
    let mut pig = actor("minecraft:pig", 6, [0.0, 64.0, 0.0]);
    pig.put("Saddled", 1i8);
    samples.push(("pig", pig));
    let mut villager = actor("minecraft:villager_v2", 8, [0.0, 64.0, 0.0]);
    villager.put("Variant", 5);
    villager.put("MarkVariant", 6);
    villager.put("TradeTier", 1);
    villager.put("CustomName", "§l§dMending");
    let mut book = item("minecraft:enchanted_book", 1);
    book.put("tag", compound! { "ench" => Tag::List(vec![Tag::Compound(compound! { "id" => 26i16, "lvl" => 1i16 })]) });
    villager.put(
        "Offers",
        compound! { "Recipes" => Tag::List(vec![Tag::Compound(compound! {
            "buyA" => item("minecraft:emerald", 24), "buyCountA" => 24, "buyB" => item("minecraft:book", 1),
            "sell" => book, "maxUses" => 12, "uses" => 3, "rewardExp" => 1i8, "traderExp" => 5,
            "priceMultiplierA" => 0.2f32, "tier" => 0,
        })]) },
    );
    samples.push(("villager", villager));
    let mut zombie = actor("minecraft:zombie", 20, [0.0, 64.0, 0.0]);
    let mut sword = item("minecraft:iron_sword", 1);
    sword.put("WasPickedUp", 1i8);
    zombie.put("Mainhand", Tag::List(vec![Tag::Compound(sword)]));
    zombie.put("Armor", Tag::List(vec![Tag::Compound(item("minecraft:iron_helmet", 1))]));
    zombie.put("IsBaby", 1i8);
    samples.push(("zombie", zombie));
    let mut stand = actor("minecraft:armor_stand", 30, [0.5, 64.0, 0.5]);
    let mut shulker = item("minecraft:undyed_shulker_box", 1);
    shulker.put("tag", compound! { "Items" => Tag::List(vec![slot(item("minecraft:diamond", 5), 0)]) });
    stand.put("Mainhand", Tag::List(vec![Tag::Compound(shulker)]));
    stand.put("Armor", Tag::List(vec![Tag::Compound(item("minecraft:iron_helmet", 1))]));
    samples.push(("armor_stand", stand));
    let mut painting = actor("minecraft:painting", 11, [-64.969, 51.5, -14.0]);
    painting.put("Motive", "Sunset");
    painting.put("Direction", 3i8);
    samples.push(("painting", painting));
    let mut cart = actor("minecraft:chest_minecart", 50, [-110.0, 16.5, -13509.5]);
    cart.put("LootTable", "loot_tables/chests/abandoned_mineshaft.json");
    cart.put("LootTableSeed", 377723764i64);
    samples.push(("chest_minecart", cart));
    let mut boat = actor("minecraft:chest_boat", 53, [0.5, 62.0, 0.5]);
    boat.put("Variant", 6);
    boat.put("ChestItems", Tag::List(vec![slot(item("minecraft:stone", 5), 0)]));
    samples.push(("chest_boat", boat));
    let home = Tag::List(vec![Tag::Float(8657.0), Tag::Float(66.0), Tag::Float(-12404.0)]);
    let mut bee = actor("minecraft:bee", 61, [0.0, 64.0, 0.0]);
    bee.put("HomePos", home.clone());
    samples.push(("bee", bee));
    let mut turtle = actor("minecraft:turtle", 62, [0.0, 64.0, 0.0]);
    turtle.put("HomePos", home);
    samples.push(("turtle", turtle));
    let mut enderman = actor("minecraft:enderman", 63, [0.0, 64.0, 0.0]);
    enderman.put("carriedBlock", compound! { "name" => "minecraft:sand", "states" => Compound::new() });
    samples.push(("enderman", enderman));

    let knots = HashMap::from([(1234i64, [91.5, 67.25, -359.5])]);
    let account = Uuid::from_u128(0x0822296f_ba99_4e86_8126_8d1a3b0403ef);
    for version in versions().into_iter().filter(|v| v.is_greater_than_or_equal(1, 17, 0)) {
        let format = JavaFormat::for_version(version);
        let out = dir.join(format.row_data_version.to_string());
        std::fs::create_dir_all(&out).unwrap();
        let mut frame_written = false;
        for (name, tag) in &samples {
            let entity = read(tag, &ConversionSettings::default(), &knots).unwrap();
            let Some(compound) = write_with(&entity, version, Some(account)) else { continue };
            let root = crate::convert::java::region::entities_root(
                format.row_data_version,
                0,
                0,
                Tag::List(vec![Tag::Compound(compound)]),
            );
            let bytes = crate::convert::java::nbtio::to_vec("", &Tag::Compound(root));
            std::fs::write(out.join(format!("{name}.nbt")), crate::convert::java::nbtio::gzip(&bytes)).unwrap();
            frame_written = true;
        }
        assert!(frame_written);
        let frame = ItemFrame {
            tile: Tile { x: 2, y: 1, z: 0 },
            direction: Some(FacingDirection::SOUTH),
            item_rotation: 2,
            item: Some(Box::new(ItemStack::with_amount(ItemType::DIAMOND, 1))),
            ..ItemFrame::default()
        };
        let compound = write_with(&Entity::new(EntityData::ItemFrame(frame)), version, None).unwrap();
        let root = crate::convert::java::region::entities_root(
            format.row_data_version,
            0,
            0,
            Tag::List(vec![Tag::Compound(compound)]),
        );
        let bytes = crate::convert::java::nbtio::to_vec("", &Tag::Compound(root));
        std::fs::write(out.join("item_frame.nbt"), crate::convert::java::nbtio::gzip(&bytes)).unwrap();
    }
}
