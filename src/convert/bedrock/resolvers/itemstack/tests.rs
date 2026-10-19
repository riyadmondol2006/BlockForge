// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Item stack reading with hand built compounds.

use std::sync::{Arc, LazyLock, Mutex};

use super::*;
use crate::convert::bedrock::data_version::BedrockVersion;
use crate::convert::model::blockentity::BlockEntityKind;
use crate::convert::model::item::Lodestone;
use crate::convert::model::{
    BlockType, Color, Dimension, DyeColor, EffectType, EnchantmentType, EntityType, ExplorerMap, FireworkShape,
    HornInstrument, ItemType, PotionType, TrimMaterial, TrimPattern, VanillaEntityType,
};
use crate::convert::nbt::{Tag, compound};

static ITEMS: LazyLock<BedrockItems> =
    LazyLock::new(|| BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap()));

fn item(name: &str, damage: i16) -> Compound {
    compound! { "Name" => name, "Count" => 1i8, "Damage" => damage, "WasPickedUp" => false }
}

fn with_tag(mut c: Compound, tag: Compound) -> Compound {
    c.put("tag", tag);
    c
}

fn read(c: &Compound) -> ItemStack {
    ITEMS.read(c).unwrap_or_else(|| panic!("unreadable item {c:?}"))
}

fn is_item(stack: &ItemStack, item: ItemType) -> bool {
    stack.id == ItemId::Item(item)
}

fn is_block(stack: &ItemStack, ty: BlockType) -> bool {
    matches!(&stack.id, ItemId::Block(b) if b.block_type() == Some(ty))
}

#[test]
fn sword_with_enchantments_display_and_costs() {
    let enchantments = Tag::list([
        compound! { "id" => 9i16, "lvl" => 5i16 },
        compound! { "id" => 17i16, "lvl" => 3i16 },
        compound! { "id" => 999i16, "lvl" => 1i16 },
    ]);
    let display = compound! { "Name" => "Excalibur", "Lore" => Tag::list(["first", "second"]) };
    let tag = compound! {
        "Damage" => 12i32, "RepairCost" => 7i32, "ench" => enchantments, "display" => display
    };
    let mut c = with_tag(item("minecraft:diamond_sword", 0), tag);
    c.put("Count", 3i8);
    let stack = read(&c);
    assert!(is_item(&stack, ItemType::DIAMOND_SWORD));
    assert_eq!(stack.props.amount, Some(3));
    assert_eq!(stack.props.durability, Some(12));
    assert_eq!(stack.props.repair_cost, Some(7));
    let ench = stack.props.enchantments.as_ref().unwrap();
    assert_eq!(ench.get(&EnchantmentType::SHARPNESS), Some(&5));
    assert_eq!(ench.get(&EnchantmentType::UNBREAKING), Some(&3));
    assert_eq!(ench.len(), 2, "the unknown id is skipped");
    let display = stack.props.display.as_ref().unwrap();
    assert_eq!(display.display_name.as_ref().unwrap()["text"], "Excalibur");
    assert_eq!(display.lore.as_ref().unwrap().len(), 2);
    assert_eq!(display.color, None);
}

#[test]
fn enchantment_without_level_is_level_one_and_minus_one_is_skipped() {
    let tag = compound! { "ench" => Tag::list([compound! { "id" => 26i16 }, compound! { "lvl" => 4i16 }]) };
    let stack = read(&with_tag(item("minecraft:bow", 0), tag));
    let ench = stack.props.enchantments.unwrap();
    assert_eq!(ench.into_iter().collect::<Vec<_>>(), vec![(EnchantmentType::MENDING, 1)]);
}

#[test]
fn block_item_from_block_compound() {
    let block = compound! {
        "name" => "minecraft:oak_stairs",
        "states" => compound! { "upside_down_bit" => true, "weirdo_direction" => 2i32 },
        "version" => 18168865i32
    };
    let mut c = item("minecraft:oak_stairs", 0);
    c.put("Block", block);
    let stack = read(&c);
    assert!(is_block(&stack, BlockType::OAK_STAIRS), "{stack:?}");
}

#[test]
fn plain_block_by_name() {
    let stack = read(&item("minecraft:stone", 0));
    assert!(is_block(&stack, BlockType::STONE), "{stack:?}");
}

#[test]
fn air_forms() {
    assert!(read(&compound! {}).is_air());
    assert!(read(&compound! { "Name" => "", "Count" => 0i8 }).is_air());
    assert!(read(&item("minecraft:air", 0)).is_air());
    // Something that is not an item at all.
    assert!(ITEMS.read(&compound! { "Foo" => 1i8 }).is_none());
}

#[test]
fn potions_and_tipped_arrows() {
    let stack = read(&item("minecraft:potion", 5));
    assert!(is_item(&stack, ItemType::POTION));
    assert_eq!(stack.props.potion, Some(PotionType::NIGHT_VISION));
    let splash = read(&item("minecraft:splash_potion", 21));
    assert!(is_item(&splash, ItemType::SPLASH_POTION));
    assert_eq!(splash.props.potion, Some(PotionType::HEALING));
    // Tipped arrows are potion id + 1.
    let arrow = read(&item("minecraft:arrow", 6));
    assert!(is_item(&arrow, ItemType::TIPPED_ARROW));
    assert_eq!(arrow.props.potion, Some(PotionType::NIGHT_VISION));
    assert!(is_item(&read(&item("minecraft:arrow", 0)), ItemType::ARROW));
}

#[test]
fn spawn_eggs() {
    let stack = read(&item("minecraft:allay_spawn_egg", 0));
    assert!(is_item(&stack, ItemType::SPAWN_EGG));
    assert_eq!(stack.props.spawn_egg_mob, Some(EntityType::Vanilla(VanillaEntityType::ALLAY)));
    // The identifier tag wins and an unknown one invalidates a spawn egg.
    let mut c = item("minecraft:allay_spawn_egg", 0);
    c.put("ItemIdentifier", "minecraft:wolf");
    assert_eq!(read(&c).props.spawn_egg_mob, Some(EntityType::Vanilla(VanillaEntityType::WOLF)));
    c.put("ItemIdentifier", "minecraft:nonexistent_mob");
    assert!(ITEMS.read(&c).is_none());
    c.put("ItemIdentifier", "mymod:thing");
    assert_eq!(read(&c).props.spawn_egg_mob, Some(EntityType::Custom("mymod:thing".into())));
}

#[test]
fn books() {
    let pages = Tag::list([
        compound! { "text" => "Page one", "photoname" => "" },
        compound! { "text" => "Page two", "photoname" => "" },
    ]);
    let tag = compound! { "title" => "Title", "author" => "Me", "pages" => pages };
    let stack = read(&with_tag(item("minecraft:written_book", 0), tag));
    assert!(is_item(&stack, ItemType::WRITTEN_BOOK));
    assert_eq!(stack.props.book_title.as_deref(), Some("Title"));
    assert_eq!(stack.props.book_author.as_deref(), Some("Me"));
    let pages = stack.props.book_pages.unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1]["text"], "Page two");
}

#[test]
fn fireworks_and_stars() {
    let explosion = compound! {
        "FireworkType" => 1i8,
        "FireworkColor" => Tag::ByteArray(vec![14]),
        "FireworkFade" => Tag::ByteArray(vec![0, 15]),
        "FireworkTrail" => 1i8,
        "FireworkFlicker" => 0i8
    };
    let tag = compound! { "Fireworks" => compound! { "Flight" => 2i8, "Explosions" => Tag::list([explosion.clone()]) } };
    let stack = read(&with_tag(item("minecraft:firework_rocket", 0), tag));
    let fireworks = stack.props.fireworks.unwrap();
    assert_eq!(fireworks.flight_duration, 2);
    assert_eq!(fireworks.explosions.len(), 1);
    let e = &fireworks.explosions[0];
    assert_eq!(e.shape, FireworkShape::LARGE_BALL);
    assert_eq!(e.colors, vec![Color::from_rgb(DyeColor::ORANGE.java_rgb())]);
    assert_eq!(e.fade_colors.len(), 2);
    assert!(e.trail && !e.twinkle);

    let star = read(&with_tag(item("minecraft:firework_star", 0), compound! { "FireworksItem" => explosion }));
    assert!(is_item(&star, ItemType::FIREWORK_STAR));
    assert_eq!(star.props.firework_explosion.unwrap().shape, FireworkShape::LARGE_BALL);
}

#[test]
fn armour_trim_and_dyed_leather() {
    let tag = compound! {
        "Trim" => compound! { "Material" => "copper", "Pattern" => "coast" },
        "customColor" => 0xff12_3456_u32 as i32
    };
    let stack = read(&with_tag(item("minecraft:leather_helmet", 0), tag));
    let trim = stack.props.trim.unwrap();
    assert_eq!((trim.material, trim.pattern), (TrimMaterial::COPPER, TrimPattern::COAST));
    assert_eq!(stack.props.display.unwrap().color, Some(Color::from_rgb(0x12_34_56)));
    // An unknown pattern drops the trim only.
    let tag = compound! { "Trim" => compound! { "Material" => "copper", "Pattern" => "nope" } };
    assert!(read(&with_tag(item("minecraft:leather_helmet", 0), tag)).props.trim.is_none());
}

#[test]
fn goat_horn_stew_maps_and_ominous_bottle() {
    assert_eq!(read(&item("minecraft:goat_horn", 7)).props.horn_instrument, Some(HornInstrument::DREAM_GOAT_HORN));
    let stew = read(&item("minecraft:suspicious_stew", 0));
    assert_eq!(stew.props.stew_effects.unwrap().effects.get(&EffectType::NIGHT_VISION), Some(&100));
    assert_eq!(read(&item("minecraft:map", 3)).props.explorer_map, Some(ExplorerMap::OCEAN_MONUMENT));
    assert_eq!(read(&item("minecraft:ominous_bottle", 2)).props.ominous_bottle_amplifier, Some(2));
}

#[test]
fn bundle_contents_are_recursive_and_skip_air() {
    let inner = item("minecraft:stick", 0);
    let tag = compound! { "storage_item_component_content" => Tag::list([inner, item("minecraft:air", 0)]) };
    let stack = read(&with_tag(item("minecraft:bundle", 0), tag));
    let contents = stack.props.bundle_contents.unwrap();
    assert_eq!(contents.len(), 1);
    assert!(is_item(&contents[0], ItemType::STICK));
}

#[test]
fn can_place_on_and_destroy() {
    let mut c = item("minecraft:stick", 0);
    c.put("CanPlaceOn", Tag::list(["minecraft:stone", "minecraft:air", "minecraft:dirt"]));
    c.put("CanDestroy", Tag::list(["minecraft:grass_block"]));
    let stack = read(&c);
    let place = stack.props.can_place_on.unwrap();
    assert_eq!(place.iter().map(|b| b.block_type().unwrap()).collect::<Vec<_>>(), vec![BlockType::STONE, BlockType::DIRT]);
    assert_eq!(stack.props.can_destroy.unwrap().len(), 1);
}

#[test]
fn custom_identifiers_pass_through_when_allowed() {
    let c = item("mymod:ruby", 0);
    let stack = read(&c);
    assert!(matches!(&stack.id, ItemId::Block(b) if matches!(b.kind(), crate::convert::model::BlockKind::Custom(id) if &**id == "mymod:ruby")));
    let strict = BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap()).with_custom_identifiers(false);
    assert!(strict.read(&c).is_none());
    // Unknown vanilla names never pass.
    assert!(ITEMS.read(&item("minecraft:definitely_not_an_item", 0)).is_none());
}

#[test]
fn lodestone_compass_and_map_hooks() {
    let lodestone = Lodestone { dimension: Dimension::overworld(), x: 1, y: 64, z: -3, tracked: true };
    let expected = lodestone.clone();
    let items = BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap())
        .with_lodestones(move |handle| (handle == 4).then(|| lodestone.clone()))
        .with_map_indexes(|uuid| (uuid == 99).then_some(7));
    let compass = with_tag(item("minecraft:compass", 0), compound! { "trackingHandle" => 4i32 });
    let stack = items.read(&compass).unwrap();
    assert!(is_item(&stack, ItemType::LODESTONE_COMPASS));
    assert_eq!(*stack.props.lodestone_data.unwrap(), expected);
    // An unknown handle still turns the compass into a lodestone compass but has no data.
    let compass = with_tag(item("minecraft:compass", 0), compound! { "trackingHandle" => 5i32 });
    let stack = items.read(&compass).unwrap();
    assert!(is_item(&stack, ItemType::LODESTONE_COMPASS) && stack.props.lodestone_data.is_none());
    let map = with_tag(item("minecraft:filled_map", 0), compound! { "map_uuid" => 99i64 });
    assert_eq!(items.read(&map).unwrap().props.map_index, Some(7));
}

#[test]
fn missing_mappings_are_reported() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let items = BedrockItems::new(BedrockVersion::exact([1, 26, 50]).unwrap())
        .with_missing_logger(move |kind, what| log.lock().unwrap().push((kind, what.to_string())));
    let tag = compound! { "ench" => Tag::list([compound! { "id" => 999i16, "lvl" => 1i16 }]) };
    items.read(&with_tag(item("minecraft:bow", 0), tag)).unwrap();
    assert!(items.read(&item("minecraft:definitely_not_an_item", 0)).is_none());
    let seen = seen.lock().unwrap();
    assert!(seen.contains(&(Missing::Enchantment, "999".to_string())));
    assert!(seen.iter().any(|(k, w)| *k == Missing::Block && w == "minecraft:definitely_not_an_item"));
}

#[test]
fn banner_item_carries_a_banner_block_entity() {
    let patterns = Tag::list([compound! { "Color" => 3i32, "Pattern" => "bs" }]);
    let stack = read(&with_tag(item("minecraft:banner", 1), compound! { "Patterns" => patterns }));
    assert!(is_item(&stack, ItemType::RED_BANNER), "{stack:?}");
    let be = stack.props.block_entity.expect("banner block entity");
    assert_eq!(be.kind(), BlockEntityKind::Banner);
}

#[test]
fn shulker_box_item_holds_its_items() {
    let inner = compound! { "Slot" => 2i8, "Name" => "minecraft:stick", "Count" => 5i8, "Damage" => 0i16 };
    let stack = read(&with_tag(item("minecraft:shulker_box", 0), compound! { "Items" => Tag::list([inner]) }));
    let be = stack.props.block_entity.expect("shulker box block entity");
    let container = be.data.container().unwrap();
    assert_eq!(container.items.get(&2).unwrap().props.amount, Some(5));
}

#[test]
fn trapped_chest_item_carries_a_trapped_chest() {
    let stack = read(&with_tag(item("minecraft:trapped_chest", 0), compound! { "Items" => Tag::list::<Compound>([]) }));
    let be = stack.props.block_entity.expect("block entity");
    assert_eq!(be.kind(), BlockEntityKind::TrappedChest);
}

#[test]
fn old_versions_have_no_bundles_or_trims() {
    let old = BedrockItems::new(BedrockVersion::exact([1, 12, 0]).unwrap());
    let tag = compound! { "Trim" => compound! { "Material" => "copper", "Pattern" => "coast" } };
    let stack = old.read(&with_tag(item("minecraft:stick", 0), tag)).unwrap();
    assert!(stack.props.trim.is_none());
}

#[test]
fn write_round_trips_the_common_properties() {
    let enchantments = Tag::list([compound! { "id" => 9i16, "lvl" => 5i16 }]);
    let display = compound! { "Name" => "Excalibur", "Lore" => Tag::list(["first"]) };
    let tag = compound! { "Damage" => 12i32, "RepairCost" => 7i32, "ench" => enchantments, "display" => display };
    let sword = read(&with_tag(item("minecraft:diamond_sword", 0), tag));
    let written = ITEMS.write(&sword).expect("sword is writable");
    assert_eq!(written.get_str("Name"), Some("minecraft:diamond_sword"));
    assert_eq!(read(&written), sword);

    for c in [
        item("minecraft:potion", 5),
        item("minecraft:arrow", 6),
        item("minecraft:allay_spawn_egg", 0),
        item("minecraft:goat_horn", 3),
        item("minecraft:stone", 0),
        with_tag(item("minecraft:leather_helmet", 0), compound! { "customColor" => 0x12_34_56i32 }),
    ] {
        let stack = read(&c);
        let back = ITEMS.write(&stack).unwrap_or_else(|| panic!("cannot write {stack:?}"));
        assert_eq!(read(&back), stack, "{c:?} -> {back:?}");
    }

    // Blocks are written with their block compound.
    let block = compound! {
        "name" => "minecraft:oak_stairs",
        "states" => compound! { "upside_down_bit" => true, "weirdo_direction" => 2i32 },
        "version" => 1i32
    };
    let mut c = item("minecraft:oak_stairs", 0);
    c.put("Block", block);
    let stairs = read(&c);
    let written = ITEMS.write(&stairs).unwrap();
    assert!(written.get_compound("Block").is_some());
    assert_eq!(read(&written), stairs);
}
