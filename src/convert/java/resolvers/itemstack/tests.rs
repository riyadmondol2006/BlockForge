// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Exact item compounds for 1.16.5 (tag), 1.20.4 (tag) and 1.21.4 (components), plus coverage of the item types.

use serde_json::json;

use crate::convert::java::format::{ItemFormat, JavaFormat};
use crate::convert::model::item::{
    FireworkExplosion, Fireworks, ItemDisplay, Lodestone, MapDecoration, StewEffect, Trim,
};
use crate::convert::model::{
    BlockEntity, BlockEntityData, BlockType, Color, Dimension, EffectType, EnchantmentType, Entity, ExplorerMap,
    FireworkShape, HornInstrument, ItemId, ItemStack, ItemType, PotionType, TrimMaterial, TrimPattern,
    VanillaEntityType,
};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::Version;

use super::{BlockEntityFn, ItemHooks, JavaItems};

fn items(data_version: i32, major: i32, minor: i32, patch: i32) -> JavaItems {
    JavaItems::new(&JavaFormat::for_data_version(data_version), Version::new(major, minor, patch))
}

fn v1_16() -> JavaItems {
    items(2586, 1, 16, 5)
}

fn v1_20() -> JavaItems {
    items(3700, 1, 20, 4)
}

fn v1_21() -> JavaItems {
    items(4189, 1, 21, 4)
}

fn v1_21_5() -> JavaItems {
    items(4325, 1, 21, 5)
}

fn latest() -> JavaItems {
    items(5017, 26, 3, 0)
}

fn list(tags: Vec<Tag>) -> Tag {
    Tag::List(tags)
}

fn c(pairs: Vec<(&str, Tag)>) -> Compound {
    let mut out = Compound::new();
    for (k, v) in pairs {
        out.put(k, v);
    }
    out
}

fn block(ty: BlockType) -> crate::convert::model::BlockRef {
    std::sync::Arc::new(ty.into())
}

fn stick(amount: i32) -> ItemStack {
    ItemStack::with_amount(ItemType::STICK, amount)
}

/// The same stack in the three versions: `(1.16.5, 1.20.4, 1.21.4)`.
fn all(stack: &ItemStack) -> (Compound, Compound, Compound) {
    (v1_16().write(stack).unwrap(), v1_20().write(stack).unwrap(), v1_21().write(stack).unwrap())
}

/// 1.20.4 and 1.21.4, for items that do not exist in 1.16.
fn two(stack: &ItemStack) -> (Compound, Compound) {
    (v1_20().write(stack).unwrap(), v1_21().write(stack).unwrap())
}

#[test]
fn formats_follow_the_data_version() {
    assert_eq!(v1_16().format(), ItemFormat::Tag);
    assert_eq!(v1_20().format(), ItemFormat::Tag);
    assert_eq!(v1_21().format(), ItemFormat::Components);
}

#[test]
fn plain_item() {
    let (a, b, d) = all(&stick(3));
    assert_eq!(a, c(vec![("id", "minecraft:stick".into()), ("Count", 3i8.into())]));
    assert_eq!(b, a);
    assert_eq!(d, c(vec![("id", "minecraft:stick".into()), ("count", 3.into())]));
}

#[test]
fn missing_amount_is_one_in_the_tag_format() {
    let stack = ItemStack::new(ItemType::STICK);
    assert_eq!(v1_20().write(&stack).unwrap().get_i8("Count"), Some(1));
    assert_eq!(v1_21().write(&stack).unwrap(), c(vec![("id", "minecraft:stick".into())]));
}

#[test]
fn block_item_writes_only_the_id() {
    let stack = ItemStack::with_amount(BlockType::OAK_PLANKS, 64);
    let (a, b, d) = all(&stack);
    assert_eq!(a, c(vec![("id", "minecraft:oak_planks".into()), ("Count", 64i8.into())]));
    assert_eq!(b, a);
    assert_eq!(d, c(vec![("id", "minecraft:oak_planks".into()), ("count", 64.into())]));
}

#[test]
fn legacy_and_read_are_not_implemented() {
    let legacy = JavaItems::new(&JavaFormat::for_data_version(1343), Version::new(1, 12, 2));
    assert_eq!(legacy.format(), ItemFormat::Legacy);
    assert!(legacy.write(&stick(1)).is_none());
    assert!(v1_21().read(&Compound::new()).is_none());
}

#[test]
fn damage_and_repair_cost() {
    let mut stack = ItemStack::with_amount(ItemType::DIAMOND_PICKAXE, 1);
    stack.props.durability = Some(12);
    stack.props.repair_cost = Some(7);
    let (a, b, d) = all(&stack);
    let tag = c(vec![("Damage", 12.into()), ("RepairCost", 7.into())]);
    assert_eq!(a, c(vec![("id", "minecraft:diamond_pickaxe".into()), ("Count", 1i8.into()), ("tag", tag.into())]));
    assert_eq!(b, a);
    let components = c(vec![("minecraft:damage", 12.into()), ("minecraft:repair_cost", 7.into())]);
    assert_eq!(
        d,
        c(vec![("id", "minecraft:diamond_pickaxe".into()), ("count", 1.into()), ("components", components.into())])
    );
}

fn named() -> ItemStack {
    let mut stack = ItemStack::with_amount(ItemType::LEATHER_CHESTPLATE, 1);
    stack.props.display = Some(Box::new(ItemDisplay {
        display_name: Some(json!({"text": "Hi"})),
        lore: Some(vec![json!({"text": "a"}), json!({"text": "b"})]),
        color: Some(Color::from_rgb(0xff0000)),
    }));
    stack
}

#[test]
fn display_in_the_tag_format() {
    let (a, b, _) = all(&named());
    let display = c(vec![
        ("Name", "{\"text\":\"Hi\"}".into()),
        ("Lore", list(vec!["{\"text\":\"a\"}".into(), "{\"text\":\"b\"}".into()])),
        ("color", (0xffff0000u32 as i32).into()),
    ]);
    let expected = c(vec![
        ("id", "minecraft:leather_chestplate".into()),
        ("Count", 1i8.into()),
        ("tag", c(vec![("display", display.into())]).into()),
    ]);
    assert_eq!(a, expected);
    assert_eq!(b, expected);
}

#[test]
fn display_in_components() {
    let name = "{\"text\":\"Hi\"}";
    let lore = list(vec!["{\"text\":\"a\"}".into(), "{\"text\":\"b\"}".into()]);
    // 1.21.4: JSON strings, `dyed_color` wraps the colour
    let got = v1_21().write(&named()).unwrap();
    let components = c(vec![
        ("minecraft:custom_name", name.into()),
        ("minecraft:lore", lore),
        ("minecraft:dyed_color", c(vec![("rgb", (0xffff0000u32 as i32).into())]).into()),
    ]);
    assert_eq!(
        got,
        c(vec![("id", "minecraft:leather_chestplate".into()), ("count", 1.into()), ("components", components.into())])
    );
    // 1.21.5: text is NBT and the colour is bare
    let got = v1_21_5().write(&named()).unwrap();
    let components = got.get_compound("components").unwrap();
    assert_eq!(components.get_compound("minecraft:custom_name"), Some(&c(vec![("text", "Hi".into())])));
    assert_eq!(components.get_i32("minecraft:dyed_color"), Some(0xffff0000u32 as i32));
    assert_eq!(components.get_list("minecraft:lore").map(<[Tag]>::len), Some(2));
}

#[test]
fn enchantments() {
    let mut stack = ItemStack::with_amount(ItemType::DIAMOND_SWORD, 1);
    stack.props.enchantments =
        Some([(EnchantmentType::SHARPNESS, 5), (EnchantmentType::SWEEPING_EDGE, 3)].into_iter().collect());
    let (a, b, d) = all(&stack);
    // `sweeping` was renamed `sweeping_edge` in 1.20.5
    let tag_list = |sweeping: &str| {
        list(vec![
            c(vec![("id", "minecraft:sharpness".into()), ("lvl", 5i16.into())]).into(),
            c(vec![("id", sweeping.into()), ("lvl", 3i16.into())]).into(),
        ])
    };
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("Enchantments", tag_list("minecraft:sweeping"))]));
    assert_eq!(b.get_compound("tag").unwrap(), &c(vec![("Enchantments", tag_list("minecraft:sweeping"))]));
    let levels = c(vec![("minecraft:sharpness", 5.into()), ("minecraft:sweeping_edge", 3.into())]);
    let expected = c(vec![("minecraft:enchantments", c(vec![("levels", levels.clone().into())]).into())]);
    assert_eq!(d.get_compound("components").unwrap(), &expected);
    // 1.21.5 drops the `levels` wrapper
    let flat = v1_21_5().write(&stack).unwrap();
    assert_eq!(flat.get_compound("components").unwrap(), &c(vec![("minecraft:enchantments", levels.into())]));
}

#[test]
fn enchanted_book_stores_its_enchantments() {
    let mut stack = ItemStack::with_amount(ItemType::ENCHANTED_BOOK, 1);
    stack.props.enchantments = Some([(EnchantmentType::MENDING, 1)].into_iter().collect());
    let (a, _, d) = all(&stack);
    let entry = c(vec![("id", "minecraft:mending".into()), ("lvl", 1i16.into())]);
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("StoredEnchantments", list(vec![entry.into()]))]));
    let levels = c(vec![("levels", c(vec![("minecraft:mending", 1.into())]).into())]);
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:stored_enchantments", levels.into())]));
}

#[test]
fn potions_and_tipped_arrows() {
    let mut stack = ItemStack::with_amount(ItemType::SPLASH_POTION, 1);
    stack.props.potion = Some(PotionType::LONG_SWIFTNESS);
    let (a, b, d) = all(&stack);
    assert_eq!(a.get_str("id"), Some("minecraft:splash_potion"));
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("Potion", "minecraft:long_swiftness".into())]));
    assert_eq!(b, a);
    let contents = c(vec![("potion", "minecraft:long_swiftness".into())]);
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:potion_contents", contents.into())]));

    let mut arrow = ItemStack::with_amount(ItemType::TIPPED_ARROW, 8);
    arrow.props.potion = Some(PotionType::STRONG_HEALING);
    let got = v1_20().write(&arrow).unwrap();
    assert_eq!(got.get_str("id"), Some("minecraft:tipped_arrow"));
    assert_eq!(got.get_compound("tag").unwrap().get_str("Potion"), Some("minecraft:strong_healing"));
}

#[test]
fn potion_without_a_java_id_is_water() {
    let mut stack = ItemStack::with_amount(ItemType::POTION, 1);
    stack.props.potion = Some(PotionType::LONG_MUNDANE);
    let got = v1_20().write(&stack).unwrap();
    assert_eq!(got.get_compound("tag").unwrap().get_str("Potion"), Some("minecraft:water"));
}

#[test]
fn written_book() {
    let mut stack = ItemStack::with_amount(ItemType::WRITTEN_BOOK, 1);
    stack.props.book_title = Some("T".into());
    stack.props.book_author = Some("A".into());
    stack.props.book_pages = Some(vec![json!({"text": "one"})]);
    let (a, b, d) = all(&stack);
    let tag =
        c(vec![("title", "T".into()), ("author", "A".into()), ("pages", list(vec!["{\"text\":\"one\"}".into()]))]);
    assert_eq!(a.get_compound("tag").unwrap(), &tag);
    assert_eq!(b, a);
    let content = c(vec![
        ("title", c(vec![("raw", "T".into())]).into()),
        ("author", "A".into()),
        ("pages", list(vec![c(vec![("raw", "{\"text\":\"one\"}".into())]).into()])),
    ]);
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:written_book_content", content.into())]));
}

#[test]
fn written_book_without_pages_gets_an_empty_list_in_the_tag_format() {
    let stack = ItemStack::with_amount(ItemType::WRITTEN_BOOK, 1);
    let got = v1_16().write(&stack).unwrap();
    assert_eq!(got.get_compound("tag").unwrap(), &c(vec![("pages", list(vec![]))]));
    assert!(v1_21().write(&stack).unwrap().get_compound("components").is_none());
}

#[test]
fn writable_book_pages_are_plain_text() {
    let mut stack = ItemStack::with_amount(ItemType::WRITABLE_BOOK, 1);
    stack.props.book_pages = Some(vec![json!({"text": "plain"})]);
    let (a, _, d) = all(&stack);
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("pages", list(vec!["plain".into()]))]));
    let content = c(vec![("pages", list(vec![c(vec![("raw", "plain".into())]).into()]))]);
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:writable_book_content", content.into())]));
}

fn explosion() -> FireworkExplosion {
    FireworkExplosion {
        shape: FireworkShape::STAR,
        colors: vec![Color::from_rgb(0xff0000), Color::from_rgb(0x00ff00)],
        fade_colors: vec![Color::from_rgb(0x0000ff)],
        trail: true,
        twinkle: false,
    }
}

#[test]
fn fireworks() {
    let mut stack = ItemStack::with_amount(ItemType::FIREWORK_ROCKET, 2);
    stack.props.fireworks = Some(Box::new(Fireworks { flight_duration: 2, explosions: vec![explosion()] }));
    let (a, b, d) = all(&stack);
    let tag_explosion = c(vec![
        ("Type", 2i8.into()),
        ("Colors", vec![0xff0000, 0x00ff00].into()),
        ("FadeColors", vec![0x0000ff].into()),
        ("Trail", 1i8.into()),
        ("Flicker", 0i8.into()),
    ]);
    let fireworks = c(vec![("Flight", 2i8.into()), ("Explosions", list(vec![tag_explosion.into()]))]);
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("Fireworks", fireworks.into())]));
    assert_eq!(b, a);
    let comp_explosion = |colors: Tag, fade: Tag| {
        c(vec![
            ("shape", "star".into()),
            ("colors", colors),
            ("fade_colors", fade),
            ("has_trail", 1i8.into()),
            ("has_twinkle", 0i8.into()),
        ])
    };
    let fireworks = |e: Compound| c(vec![("flight_duration", 2i8.into()), ("explosions", list(vec![e.into()]))]);
    let expected = fireworks(comp_explosion(vec![0xff0000, 0x00ff00].into(), vec![0x0000ff].into()));
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:fireworks", expected.into())]));
    // 1.21.5 stores the colours as int lists
    let got = v1_21_5().write(&stack).unwrap();
    let ints = |v: Vec<i32>| list(v.into_iter().map(Tag::Int).collect());
    let expected = fireworks(comp_explosion(ints(vec![0xff0000, 0x00ff00]), ints(vec![0x0000ff])));
    assert_eq!(got.get_compound("components").unwrap(), &c(vec![("minecraft:fireworks", expected.into())]));
}

#[test]
fn default_fireworks_write_nothing() {
    let mut stack = ItemStack::with_amount(ItemType::FIREWORK_ROCKET, 1);
    stack.props.fireworks = Some(Box::default());
    assert!(v1_20().write(&stack).unwrap().get_compound("tag").is_none());
    assert!(v1_21().write(&stack).unwrap().get_compound("components").is_none());
}

#[test]
fn firework_star() {
    let mut stack = ItemStack::with_amount(ItemType::FIREWORK_STAR, 1);
    stack.props.firework_explosion = Some(Box::new(explosion()));
    let (a, _, d) = all(&stack);
    let tag = a.get_compound("tag").unwrap().get_compound("Explosion").unwrap();
    assert_eq!(tag.get_i8("Type"), Some(2));
    assert_eq!(tag.get_int_array("Colors"), Some(&[0xff0000, 0x00ff00][..]));
    let comp = d.get_compound("components").unwrap().get_compound("minecraft:firework_explosion").unwrap();
    assert_eq!(comp.get_str("shape"), Some("star"));
    assert_eq!(comp.get_i8("has_trail"), Some(1));
}

#[test]
fn spawn_eggs() {
    let mut stack = ItemStack::with_amount(ItemType::SPAWN_EGG, 1);
    stack.props.spawn_egg_mob = Some(VanillaEntityType::CREEPER.into());
    let (a, b, d) = all(&stack);
    // the item id comes from the mob, the entity id is written as well
    assert_eq!(a.get_str("id"), Some("minecraft:creeper_spawn_egg"));
    assert_eq!(
        a.get_compound("tag").unwrap(),
        &c(vec![("EntityTag", c(vec![("id", "minecraft:creeper".into())]).into())])
    );
    assert_eq!(b, a);
    assert_eq!(
        d.get_compound("components").unwrap(),
        &c(vec![("minecraft:entity_data", c(vec![("id", "minecraft:creeper".into())]).into())])
    );
    // a spawn egg without a mob is a pig egg (a fallback row of Chunker's table)
    let plain = v1_21().write(&ItemStack::new(ItemType::SPAWN_EGG)).unwrap();
    assert_eq!(plain.get_str("id"), Some("minecraft:pig_spawn_egg"));
}

struct Maps;

impl ItemHooks for Maps {
    fn map_id(&self, index: i32) -> Option<i32> {
        Some(index + 100)
    }
}

#[test]
fn maps_use_the_hook_and_the_decoration_table() {
    let mut stack = ItemStack::with_amount(ItemType::FILLED_MAP, 1);
    stack.props.map_index = Some(2);
    stack.props.explorer_map = Some(ExplorerMap::OCEAN_MONUMENT);
    stack.props.map_decoration =
        Some(MapDecoration { structure: ExplorerMap::OCEAN_MONUMENT, x: 1.5, z: -2.0, rotation: 8.0 });
    // without the hook the map index is dropped
    assert!(v1_20().write(&stack).unwrap().get_compound("tag").unwrap().get("map").is_none());

    let a = v1_16().write_with(&stack, &Maps).unwrap();
    let marker =
        c(vec![("id", "+".into()), ("type", 9i8.into()), ("x", 1.5.into()), ("z", (-2.0).into()), ("rot", 8.0.into())]);
    assert_eq!(
        a.get_compound("tag").unwrap(),
        &c(vec![("Decorations", list(vec![marker.into()])), ("map", 102.into())])
    );
    let d = v1_21().write_with(&stack, &Maps).unwrap();
    let marker = c(vec![
        ("type", "minecraft:monument".into()),
        ("x", 1.5.into()),
        ("z", (-2.0).into()),
        ("rotation", 8.0f32.into()),
    ]);
    let expected =
        c(vec![("minecraft:map_decorations", c(vec![("+", marker.into())]).into()), ("minecraft:map_id", 102.into())]);
    assert_eq!(d.get_compound("components").unwrap(), &expected);
}

#[test]
fn explorer_maps_are_items_from_26_3() {
    let mut stack = ItemStack::with_amount(ItemType::FILLED_MAP, 1);
    stack.props.explorer_map = Some(ExplorerMap::ABANDONED_CAMP);
    assert_eq!(latest().write(&stack).unwrap().get_str("id"), Some("minecraft:abandoned_camp_map"));
    assert_eq!(v1_21().write(&stack).unwrap().get_str("id"), Some("minecraft:filled_map"));
}

#[test]
fn horn_trim_and_stew() {
    let mut horn = ItemStack::with_amount(ItemType::GOAT_HORN, 1);
    horn.props.horn_instrument = Some(HornInstrument::SEEK_GOAT_HORN);
    let (a, d) = two(&horn);
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("instrument", "minecraft:seek_goat_horn".into())]));
    assert_eq!(
        d.get_compound("components").unwrap(),
        &c(vec![("minecraft:instrument", "minecraft:seek_goat_horn".into())])
    );

    let mut helmet = ItemStack::with_amount(ItemType::IRON_HELMET, 1);
    helmet.props.trim = Some(Trim { material: TrimMaterial::GOLD, pattern: TrimPattern::WILD });
    let trim = c(vec![("material", "minecraft:gold".into()), ("pattern", "minecraft:wild".into())]);
    // the pattern table only starts in 1.19.4, so 1.16.5 has none and writes no trim
    assert!(v1_16().write(&helmet).unwrap().get_compound("tag").is_none());
    let (b, d) = two(&helmet);
    assert_eq!(b.get_compound("tag").unwrap(), &c(vec![("Trim", trim.clone().into())]));
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:trim", trim.into())]));

    let mut stew = ItemStack::with_amount(ItemType::SUSPICIOUS_STEW, 1);
    stew.props.stew_effects = Some(StewEffect {
        effects: [(EffectType::NIGHT_VISION, 100), (EffectType::REGENERATION, 160)].into_iter().collect(),
    });
    // effects are in the order of the model enum
    let effects = list(vec![
        c(vec![("duration", 160.into()), ("id", "minecraft:regeneration".into())]).into(),
        c(vec![("duration", 100.into()), ("id", "minecraft:night_vision".into())]).into(),
    ]);
    let (a, _, d) = all(&stew);
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("effects", effects.clone())]));
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:suspicious_stew_effects", effects)]));
}

#[test]
fn lodestone_compass() {
    let mut stack = ItemStack::with_amount(ItemType::LODESTONE_COMPASS, 1);
    stack.props.lodestone_data =
        Some(Box::new(Lodestone { dimension: Dimension::nether(), x: 1, y: 2, z: 3, tracked: true }));
    let (a, _, d) = all(&stack);
    assert_eq!(a.get_str("id"), Some("minecraft:compass"));
    let tag = c(vec![
        ("LodestoneDimension", "minecraft:the_nether".into()),
        ("LodestonePos", c(vec![("X", 1.into()), ("Y", 2.into()), ("Z", 3.into())]).into()),
        ("LodestoneTracked", 1i8.into()),
    ]);
    assert_eq!(a.get_compound("tag").unwrap(), &tag);
    let target = c(vec![("dimension", "minecraft:the_nether".into()), ("pos", vec![1, 2, 3].into())]);
    let tracker = c(vec![("target", target.into()), ("tracked", 1i8.into())]);
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:lodestone_tracker", tracker.into())]));
}

#[test]
fn can_place_on_and_can_destroy() {
    let mut stack = ItemStack::with_amount(ItemType::STICK, 1);
    stack.props.can_place_on = Some(vec![block(BlockType::STONE), block(BlockType::AIR)]);
    stack.props.can_destroy = Some(vec![block(BlockType::DIRT)]);
    let ItemId::Item(_) = stack.id else { unreachable!() };
    let (a, _, d) = all(&stack);
    let tag = a.get_compound("tag").unwrap();
    assert_eq!(tag.get_list("CanPlaceOn"), Some(&[Tag::String("minecraft:stone".into())][..]));
    assert_eq!(tag.get_list("CanDestroy"), Some(&[Tag::String("minecraft:dirt".into())][..]));
    let pred = |id: &str| list(vec![c(vec![("blocks", id.into())]).into()]);
    let expected = c(vec![
        ("minecraft:can_place_on", c(vec![("predicates", pred("minecraft:stone"))]).into()),
        ("minecraft:can_break", c(vec![("predicates", pred("minecraft:dirt"))]).into()),
    ]);
    assert_eq!(d.get_compound("components").unwrap(), &expected);
}

#[test]
fn ominous_bottle_is_a_component_only() {
    let mut stack = ItemStack::with_amount(ItemType::OMINOUS_BOTTLE, 1);
    stack.props.ominous_bottle_amplifier = Some(3);
    assert!(v1_20().write(&stack).is_none());
    assert_eq!(
        v1_21().write(&stack).unwrap().get_compound("components").unwrap(),
        &c(vec![("minecraft:ominous_bottle_amplifier", 3.into())])
    );
}

#[test]
fn bundles_nest_stacks() {
    let mut stack = ItemStack::with_amount(ItemType::BUNDLE, 1);
    stack.props.bundle_contents =
        Some(vec![stick(5), ItemStack::new(BlockType::AIR), ItemStack::with_amount(BlockType::DIRT, 2)]);
    let (a, d) = two(&stack);
    let inner = list(vec![
        c(vec![("id", "minecraft:stick".into()), ("Count", 5i8.into())]).into(),
        c(vec![("id", "minecraft:dirt".into()), ("Count", 2i8.into())]).into(),
    ]);
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("Items", inner)]));
    let inner = list(vec![
        c(vec![("id", "minecraft:stick".into()), ("count", 5.into())]).into(),
        c(vec![("id", "minecraft:dirt".into()), ("count", 2.into())]).into(),
    ]);
    assert_eq!(d.get_compound("components").unwrap(), &c(vec![("minecraft:bundle_contents", inner)]));
}

fn banner_entity() -> BlockEntity {
    BlockEntity::new(4, 5, 6, BlockEntityData::default_for(crate::convert::model::BlockEntityKind::Banner).unwrap())
}

#[test]
fn block_entity_hook_edits_the_item_and_returns_the_data() {
    let mut stack = ItemStack::with_amount(ItemType::SHIELD, 1);
    stack.props.block_entity = Some(Box::new(banner_entity()));
    let hook = BlockEntityFn(|_stack: &ItemStack, be: &BlockEntity, item: &mut Compound| {
        assert_eq!((be.x, be.y, be.z), (4, 5, 6));
        item.get_or_create_compound("components").put("minecraft:base_color", "red");
        Some(c(vec![
            ("id", "minecraft:banner".into()),
            ("x", 4.into()),
            ("y", 5.into()),
            ("z", 6.into()),
            ("keep", 1.into()),
        ]))
    });
    // the tag format strips the id as well
    let a = v1_20().write_with(&stack, &hook).unwrap();
    assert_eq!(a.get_compound("tag").unwrap(), &c(vec![("BlockEntityTag", c(vec![("keep", 1.into())]).into())]));
    // components keep the id (it is required there)
    let d = v1_21().write_with(&stack, &hook).unwrap();
    let components = c(vec![
        ("minecraft:base_color", "red".into()),
        ("minecraft:block_entity_data", c(vec![("id", "minecraft:banner".into()), ("keep", 1.into())]).into()),
    ]);
    assert_eq!(d.get_compound("components").unwrap(), &components);
    // without hooks the block entity is dropped
    assert!(v1_21().write(&stack).unwrap().get_compound("components").is_none());
}

struct Painter;

impl ItemHooks for Painter {
    fn entity(&self, _entity: &Entity) -> Option<Compound> {
        Some(c(vec![("id", "minecraft:painting".into()), ("variant", "minecraft:kebab".into()), ("Pos", list(vec![]))]))
    }
}

#[test]
fn entity_hook_for_paintings() {
    use crate::convert::model::entity::{EntityData, Painting};
    let mut stack = ItemStack::with_amount(ItemType::PAINTING, 1);
    stack.props.entity = Some(Box::new(Entity::new(EntityData::Painting(Painting::default()))));
    let a = v1_20().write_with(&stack, &Painter).unwrap();
    assert_eq!(
        a.get_compound("tag").unwrap(),
        &c(vec![("EntityTag", c(vec![("variant", "minecraft:kebab".into())]).into())])
    );
    let d = v1_21().write_with(&stack, &Painter).unwrap();
    assert_eq!(
        d.get_compound("components").unwrap(),
        &c(vec![(
            "minecraft:entity_data",
            c(vec![("id", "minecraft:painting".into()), ("variant", "minecraft:kebab".into())]).into()
        )])
    );
}

#[test]
fn preserved_identifier_replaces_the_id() {
    use crate::convert::model::{Identifier, PreservedIdentifier};
    use std::sync::Arc;
    let mut stack = stick(1);
    stack.preserved =
        Some(Arc::new(PreservedIdentifier { from_reader: true, identifier: Identifier::new("mod:wand") }));
    assert_eq!(v1_21().write(&stack).unwrap().get_str("id"), Some("mod:wand"));
    // a mapping made by the writer side is not applied again
    stack.preserved =
        Some(Arc::new(PreservedIdentifier { from_reader: false, identifier: Identifier::new("mod:wand") }));
    assert_eq!(v1_21().write(&stack).unwrap().get_str("id"), Some("minecraft:stick"));
}

#[test]
fn custom_blocks_keep_their_identifier() {
    use crate::convert::model::BlockIdentifier;
    let block = BlockIdentifier::custom::<&str, _>("mod:ore", []);
    let stack = ItemStack::with_amount(block, 1);
    assert_eq!(v1_21().write(&stack).unwrap().get_str("id"), Some("mod:ore"));
}

#[test]
fn items_missing_on_old_versions_are_none() {
    // Copper tools do not exist before the newest versions
    let stack = ItemStack::new(ItemType::COPPER_SWORD);
    assert!(v1_21().write(&stack).is_none());
    assert_eq!(latest().write(&stack).unwrap().get_str("id"), Some("minecraft:copper_sword"));
}

#[test]
fn every_item_type_is_mapped_on_the_newest_version() {
    let items = latest();
    let mut missing = Vec::new();
    for ty in ItemType::ALL {
        let mut stack = ItemStack::new(ty);
        if ty == ItemType::SPAWN_EGG {
            stack.props.spawn_egg_mob = Some(VanillaEntityType::ALLAY.into());
        }
        if items.item_id(&stack).is_none() {
            missing.push(ty.name());
        }
    }
    println!("unmapped item types on 26.3: {missing:?}");
    assert!(missing.is_empty(), "unmapped item types: {missing:?}");
}

/// Block types with no Java item id on the newest version; all of them are Bedrock only.
const UNMAPPABLE_BLOCKS: [&str; 4] = ["ITEM_FRAME_BEDROCK", "ICICLE", "ICE_CRYSTAL", "DISC_PRESS"];

#[test]
fn every_block_type_is_mapped_or_listed() {
    let items = latest();
    let missing: Vec<&str> = BlockType::ALL
        .into_iter()
        .filter(|t| items.item_id(&ItemStack::new(*t)).is_none())
        .map(BlockType::name)
        .collect();
    println!("unmapped block types on 26.3: {missing:?}");
    assert_eq!(missing, UNMAPPABLE_BLOCKS);
}

#[test]
fn spawn_eggs_cover_the_entities_that_have_one() {
    let items = latest();
    let mut missing = Vec::new();
    for ty in VanillaEntityType::ALL.into_iter().filter(|t| t.has_spawn_egg()) {
        let mut stack = ItemStack::new(ItemType::SPAWN_EGG);
        stack.props.spawn_egg_mob = Some(ty.into());
        if items.write(&stack).is_none() {
            missing.push(ty.name());
        }
    }
    println!("entities with a spawn egg and no Java egg on 26.3: {missing:?}");
}
