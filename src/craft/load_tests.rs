//! Tests of the recipe loader: small in-memory games, and the real jars when they are on this computer.

use std::path::PathBuf;
use std::time::Instant;

use super::*;
use crate::craft::source::MemSource;

/// A game with these items (a model and an English name each) and nothing else; `new` picks the 1.21+ file layout.
fn game(items: &[&str], new: bool) -> MemSource {
    let mut m = MemSource::default();
    let mut lang = serde_json::Map::new();
    for id in items {
        let path = if new {
            format!("assets/minecraft/items/{id}.json")
        } else {
            format!("assets/minecraft/models/item/{id}.json")
        };
        m = m.with(&path, "{}");
        lang.insert(format!("item.minecraft.{id}"), pretty_name(id).into());
    }
    lang.insert("gui.done".into(), "Done".into());
    m.with("assets/minecraft/lang/en_us.json", &Value::Object(lang).to_string())
}

fn recipe(m: MemSource, new: bool, id: &str, json: &str) -> MemSource {
    let dir = if new { "recipe" } else { "recipes" };
    m.with(&format!("data/minecraft/{dir}/{id}.json"), json)
}

fn tag(m: MemSource, new: bool, id: &str, json: &str) -> MemSource {
    let dir = if new { "item" } else { "items" };
    m.with(&format!("data/minecraft/tags/{dir}/{id}.json"), json)
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| format!("minecraft:{s}")).collect()
}

fn load_mem(mut m: MemSource) -> Dataset {
    load("test", &mut m).unwrap()
}

const BASIC: &[&str] = &[
    "oak_planks",
    "birch_planks",
    "stick",
    "iron_ingot",
    "raw_iron",
    "iron_block",
    "iron_nugget",
    "diamond",
    "diamond_block",
    "white_wool",
    "red_wool",
    "oak_log",
    "stripped_oak_log",
    "oak_wood",
    "cake",
    "milk_bucket",
    "wheat",
    "sugar",
    "egg",
    "bucket",
    "stone",
    "stone_slab",
    "cobblestone",
    "netherite_upgrade_smithing_template",
    "diamond_sword",
    "netherite_sword",
    "netherite_ingot",
    "black_dye",
    "ink_sac",
    "bundle",
    "black_bundle",
    "bread",
    "coal",
    "coal_block",
    "charcoal",
    "lever",
    "oak_stairs",
    "stone_bricks",
    "torch",
];

#[test]
fn old_layout_shaped_with_tags_and_optional_entries() {
    let mut m = game(BASIC, false);
    m = tag(
        m,
        false,
        "planks",
        r##"{"values":["minecraft:oak_planks","#minecraft:birch_stuff",{"id":"minecraft:gone","required":false},{"id":"minecraft:oak_planks"}]}"##,
    );
    m = tag(
        m,
        false,
        "birch_stuff",
        r##"{"values":["minecraft:birch_planks",{"id":"minecraft:also_gone","required":false}]}"##,
    );
    m = recipe(
        m,
        false,
        "stick",
        r##"{"type":"minecraft:crafting_shaped","category":"misc","pattern":["#","#"],"key":{"#":{"tag":"minecraft:planks"}},"result":{"item":"minecraft:stick","count":4}}"##,
    );
    let ds = load_mem(m);
    let r = ds.recipe("stick").unwrap();
    assert_eq!((r.kind, r.count, r.small_grid), (Kind::Shaped, 4, true));
    assert_eq!(r.ingredients.len(), 1);
    assert_eq!(r.ingredients[0].key, "#");
    assert_eq!(r.ingredients[0].count, 2);
    assert_eq!(r.ingredients[0].alternatives, ids(&["birch_planks", "oak_planks"]));
    assert!(ds.skipped.is_empty(), "{:?}", ds.skipped);
    assert_eq!(ds.item("minecraft:stick").unwrap().name, "Stick");
    assert_eq!(ds.item("minecraft:stick").unwrap().category, Category::Misc);
    // tag members and the item list: only items with a model and a name, plus what the recipes use
    assert!(ds.item("minecraft:oak_planks").is_some() && ds.item("minecraft:gone").is_none());
}

#[test]
fn new_layout_with_string_ingredients_and_id_results() {
    let mut m = game(BASIC, true);
    m = tag(m, true, "planks", r##"{"values":["minecraft:oak_planks","minecraft:birch_planks"]}"##);
    m = recipe(
        m,
        true,
        "stick",
        r##"{"type":"crafting_shaped","pattern":["#","#"],"key":{"#":"#minecraft:planks"},"result":{"id":"minecraft:stick","count":4}}"##,
    );
    m = recipe(
        m,
        true,
        "stick_from_either",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":[["minecraft:oak_planks","minecraft:birch_planks"]],"result":{"id":"stick"}}"##,
    );
    let ds = load_mem(m);
    assert_eq!(ds.recipe("stick").unwrap().ingredients[0].alternatives, ids(&["birch_planks", "oak_planks"]));
    let r = ds.recipe("stick_from_either").unwrap();
    assert_eq!((r.count, r.ingredients[0].key.as_str()), (1, "#0"));
    assert_eq!(r.ingredients[0].alternatives, ids(&["birch_planks", "oak_planks"]));
    assert!(ds.item("minecraft:stick").is_some());
}

#[test]
fn shaped_ingredients_count_symbols_in_first_appearance_order() {
    let mut m = game(BASIC, true);
    m = recipe(
        m,
        true,
        "lever_like",
        r##"{"type":"minecraft:crafting_shaped","pattern":["C C"," S ","CIC"],"key":{"S":"minecraft:stick","I":"minecraft:iron_ingot","C":"minecraft:cobblestone","X":"minecraft:diamond"},"result":{"id":"minecraft:lever"}}"##,
    );
    m = recipe(
        m,
        true,
        "block",
        r##"{"type":"minecraft:crafting_shaped","pattern":["II","II"],"key":{"I":"minecraft:iron_ingot"},"result":{"id":"minecraft:iron_block"}}"##,
    );
    m = recipe(
        m,
        true,
        "wide",
        r##"{"type":"minecraft:crafting_shaped","pattern":["III"],"key":{"I":"minecraft:iron_ingot"},"result":{"id":"minecraft:bucket"}}"##,
    );
    let ds = load_mem(m);
    let r = ds.recipe("lever_like").unwrap();
    let keys: Vec<_> = r.ingredients.iter().map(|i| (i.key.as_str(), i.count)).collect();
    assert_eq!(keys, [("C", 4), ("S", 1), ("I", 1)]);
    // the pattern is kept as the game lays it out, for the grid on the page
    assert_eq!(r.shape, ["C C", " S ", "CIC"]);
    assert!(!r.small_grid);
    assert!(ds.recipe("block").unwrap().small_grid);
    assert!(!ds.recipe("wide").unwrap().small_grid);
    assert_eq!(ds.recipe("wide").unwrap().ingredients[0].count, 3);
}

#[test]
fn shapeless_entries_with_the_same_alternatives_merge() {
    let mut m = game(BASIC, false);
    m = recipe(
        m,
        false,
        "cake_like",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:milk_bucket"},{"item":"minecraft:sugar"},[{"item":"minecraft:egg"}],{"item":"minecraft:sugar"},{"item":"minecraft:wheat"},{"item":"minecraft:wheat"},{"item":"minecraft:wheat"},{"item":"minecraft:milk_bucket"},{"item":"minecraft:milk_bucket"}],"result":{"item":"minecraft:cake"}}"##,
    );
    m = recipe(
        m,
        false,
        "two_dyes",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":[{"item":"minecraft:ink_sac"},{"item":"minecraft:egg"},{"item":"minecraft:ink_sac"},{"item":"minecraft:egg"}],"result":{"item":"minecraft:black_dye","count":2}}"##,
    );
    let ds = load_mem(m);
    let r = ds.recipe("cake_like").unwrap();
    let got: Vec<_> = r.ingredients.iter().map(|i| (i.key.as_str(), i.count, i.alternatives[0].as_str())).collect();
    assert_eq!(
        got,
        [
            ("#0", 3, "minecraft:milk_bucket"),
            ("#1", 2, "minecraft:sugar"),
            ("#2", 1, "minecraft:egg"),
            ("#4", 3, "minecraft:wheat")
        ]
    );
    assert!(!r.small_grid);
    assert!(ds.recipe("two_dyes").unwrap().small_grid);
}

#[test]
fn cooking_and_stonecutting_in_both_formats() {
    let mut m = game(BASIC, false);
    m = recipe(
        m,
        false,
        "iron_ingot_from_raw",
        r##"{"type":"minecraft:smelting","category":"misc","ingredient":{"item":"minecraft:raw_iron"},"result":"minecraft:iron_ingot","cookingtime":150}"##,
    );
    m = recipe(
        m,
        false,
        "iron_ingot_from_blasting_raw",
        r##"{"type":"minecraft:blasting","ingredient":[{"item":"minecraft:raw_iron"},{"item":"minecraft:iron_block"}],"result":"minecraft:iron_ingot"}"##,
    );
    m = recipe(
        m,
        false,
        "bread_smoking",
        r##"{"type":"smoking","category":"food","ingredient":{"item":"minecraft:wheat"},"result":"minecraft:bread"}"##,
    );
    m = recipe(
        m,
        false,
        "bread_campfire",
        r##"{"type":"campfire_cooking","ingredient":{"item":"minecraft:wheat"},"result":"minecraft:bread"}"##,
    );
    m = recipe(
        m,
        false,
        "slabs",
        r##"{"type":"minecraft:stonecutting","count":2,"ingredient":{"item":"minecraft:stone"},"result":"minecraft:stone_slab"}"##,
    );
    let ds = load_mem(m);
    let r = ds.recipe("iron_ingot_from_raw").unwrap();
    assert_eq!((r.kind, r.ticks, r.count), (Kind::Smelting, Some(150), 1));
    assert_eq!((r.ingredients[0].key.as_str(), r.ingredients[0].count), ("input", 1));
    let b = ds.recipe("iron_ingot_from_blasting_raw").unwrap();
    assert_eq!((b.kind, b.ticks), (Kind::Blasting, Some(100)));
    assert_eq!(b.ingredients[0].alternatives, ids(&["iron_block", "raw_iron"]));
    assert_eq!(ds.recipe("bread_smoking").unwrap().ticks, Some(100));
    assert_eq!(ds.recipe("bread_campfire").unwrap().ticks, Some(600));
    assert_eq!(ds.item("minecraft:bread").unwrap().category, Category::Food);
    let s = ds.recipe("slabs").unwrap();
    assert_eq!((s.kind, s.count, s.ingredients[0].key.as_str()), (Kind::Stonecutting, 2, "input"));

    let mut m = game(BASIC, true);
    m = recipe(
        m,
        true,
        "slabs",
        r##"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"count":2,"id":"minecraft:stone_slab"}}"##,
    );
    m = recipe(
        m,
        true,
        "smelt",
        r##"{"type":"minecraft:smelting","ingredient":"minecraft:raw_iron","result":{"id":"minecraft:iron_ingot"}}"##,
    );
    let ds = load_mem(m);
    assert_eq!(ds.recipe("slabs").unwrap().count, 2);
    assert_eq!(ds.recipe("smelt").unwrap().ticks, Some(200));
}

#[test]
fn smithing_and_transmute() {
    let mut m = game(BASIC, true);
    m = recipe(
        m,
        true,
        "netherite_sword_smithing",
        r##"{"type":"minecraft:smithing_transform","template":"minecraft:netherite_upgrade_smithing_template","base":"minecraft:diamond_sword","addition":"minecraft:netherite_ingot","result":{"id":"minecraft:netherite_sword"}}"##,
    );
    m = recipe(
        m,
        true,
        "trim",
        r##"{"type":"minecraft:smithing_trim","template":"minecraft:netherite_upgrade_smithing_template","base":"minecraft:diamond_sword","addition":"minecraft:netherite_ingot"}"##,
    );
    m = tag(m, true, "bundles", r##"{"values":["minecraft:bundle","minecraft:black_bundle"]}"##);
    m = recipe(
        m,
        true,
        "black_bundle",
        r##"{"type":"minecraft:crafting_transmute","category":"equipment","input":"#minecraft:bundles","material":"minecraft:black_dye","result":{"id":"minecraft:black_bundle"}}"##,
    );
    m = recipe(
        m,
        true,
        "weighted",
        r##"{"type":"minecraft:crafting_transmute","input":"minecraft:bread","material":"minecraft:wheat","material_count":3,"add_material_count_to_result":true,"result":{"id":"minecraft:cake"}}"##,
    );
    let ds = load_mem(m);
    let s = ds.recipe("netherite_sword_smithing").unwrap();
    assert_eq!(s.kind, Kind::Smithing);
    assert_eq!(s.manual.as_deref(), Some("Smithing keeps the item's enchantments and name, do this one by hand"));
    let keys: Vec<_> = s.ingredients.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, ["template", "base", "addition"]);
    assert!(ds.recipe("trim").is_none() && ds.skipped.iter().any(|(id, _)| id == "trim"));
    let t = ds.recipe("black_bundle").unwrap();
    assert_eq!((t.kind, t.count), (Kind::Transmute, 1));
    assert_eq!(t.ingredients[0].key, "input");
    assert_eq!(t.ingredients[0].alternatives, ids(&["black_bundle", "bundle"]));
    assert_eq!((t.ingredients[1].key.as_str(), t.ingredients[1].count), ("material", 1));
    assert_eq!(ds.item("minecraft:black_bundle").unwrap().category, Category::Equipment);
    let w = ds.recipe("weighted").unwrap();
    assert_eq!((w.count, w.ingredients[1].count), (4, 3));
}

#[test]
fn odd_recipes_are_skipped_with_a_reason_and_never_fail_the_load() {
    let mut m = game(BASIC, true);
    m = recipe(
        m,
        true,
        "good",
        r##"{"type":"minecraft:smelting","ingredient":"minecraft:raw_iron","result":{"id":"minecraft:iron_ingot"}}"##,
    );
    m = recipe(m, true, "banner", r##"{"type":"minecraft:crafting_special_bannerduplicate","category":"misc"}"##);
    m = recipe(m, true, "dye", r##"{"type":"minecraft:crafting_dye","result":{"id":"minecraft:black_dye"}}"##);
    m = recipe(
        m,
        true,
        "brew",
        r##"{"type":"minecraft:brewing","input":"x","reagent":"y","output":{"id":"minecraft:stick"}}"##,
    );
    m = recipe(m, true, "weird", r##"{"type":"modded:thing","result":{"id":"minecraft:stick"}}"##);
    m = recipe(m, true, "notype", r##"{"result":{"id":"minecraft:stick"}}"##);
    m = recipe(
        m,
        true,
        "bad_item",
        r##"{"type":"minecraft:smelting","ingredient":"minecraft:not_an_item","result":{"id":"minecraft:iron_ingot"}}"##,
    );
    m = recipe(
        m,
        true,
        "bad_tag",
        r##"{"type":"minecraft:smelting","ingredient":"#minecraft:nothing","result":{"id":"minecraft:iron_ingot"}}"##,
    );
    m = recipe(
        m,
        true,
        "no_key",
        r##"{"type":"minecraft:crafting_shaped","pattern":["AB"],"key":{"A":"minecraft:stick"},"result":{"id":"minecraft:lever"}}"##,
    );
    m = recipe(m, true, "bad_json", "{ nope");
    m = recipe(m, true, "no_result", r##"{"type":"minecraft:smelting","ingredient":"minecraft:raw_iron"}"##);
    m = recipe(
        m,
        true,
        "zero",
        r##"{"type":"minecraft:smelting","ingredient":"minecraft:raw_iron","result":{"id":"minecraft:iron_ingot","count":0}}"##,
    );
    m = tag(m, true, "empty", r##"{"values":[{"id":"minecraft:ghost","required":false}]}"##);
    m = recipe(
        m,
        true,
        "empty_tag",
        r##"{"type":"minecraft:smelting","ingredient":"#minecraft:empty","result":{"id":"minecraft:iron_ingot"}}"##,
    );
    m = tag(m, true, "broken", r##"{"values":["#minecraft:missing_parent"]}"##);
    m = recipe(
        m,
        true,
        "broken_tag",
        r##"{"type":"minecraft:smelting","ingredient":"#minecraft:broken","result":{"id":"minecraft:iron_ingot"}}"##,
    );
    let ds = load_mem(m);
    assert_eq!(ds.recipes.len(), 1);
    let why = |id: &str| ds.skipped.iter().find(|(r, _)| r == id).map(|(_, w)| w.clone()).unwrap_or_default();
    assert!(why("banner").contains("dynamic"), "{}", why("banner"));
    assert!(why("dye").contains("dynamic"));
    assert!(why("brew").contains("brewing"));
    assert!(why("weird").contains("modded:thing"));
    assert!(why("notype").contains("no type"));
    assert!(why("bad_item").contains("not_an_item"));
    assert!(why("bad_tag").contains("minecraft:nothing"));
    assert!(why("no_key").contains("no key"));
    assert!(why("bad_json").contains("invalid JSON"));
    assert!(why("no_result").contains("no result"));
    assert!(why("zero").contains("result count"));
    assert!(why("empty_tag").contains("no items"));
    assert!(why("broken_tag").contains("missing_parent"));
    assert_eq!(ds.skipped.len(), 13);
}

#[test]
fn nothing_loadable_is_an_error() {
    let mut m = game(BASIC, true);
    m = recipe(m, true, "banner", r##"{"type":"minecraft:crafting_special_bannerduplicate"}"##);
    assert!(load("t", &mut m).unwrap_err().contains("No usable"));
    // a game with no recipe data at all (before 1.13)
    let mut old = MemSource::default().with("assets/minecraft/lang/en_us.json", "{}");
    let e = load("1.12.2", &mut old).unwrap_err();
    assert!(e.starts_with("Minecraft 1.13 or newer is needed"), "{e}");
}

#[test]
fn tags_nest_and_cycles_are_cut() {
    let mut m = game(BASIC, true);
    m = tag(m, true, "a", r##"{"values":["minecraft:oak_planks","#minecraft:b"]}"##);
    m = tag(m, true, "b", r##"{"values":["minecraft:birch_planks","#minecraft:a"]}"##);
    m = tag(m, true, "sub/c", r##"{"values":["#minecraft:a",{"id":"#minecraft:nothing","required":false}]}"##);
    m = recipe(
        m,
        true,
        "stick",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["#minecraft:sub/c"],"result":{"id":"minecraft:stick"}}"##,
    );
    let ds = load_mem(m);
    assert_eq!(ds.recipe("stick").unwrap().ingredients[0].alternatives, ids(&["birch_planks", "oak_planks"]));
}

#[test]
fn raw_stops_come_from_the_list_the_tags_and_pack_pairs() {
    let mut m = game(BASIC, true);
    m = tag(m, true, "wool", r##"{"values":["minecraft:white_wool","minecraft:red_wool"]}"##);
    m = tag(
        m,
        true,
        "logs",
        r##"{"values":["minecraft:oak_log","minecraft:stripped_oak_log","minecraft:oak_wood","#minecraft:more"]}"##,
    );
    m = tag(m, true, "more", r##"{"values":["minecraft:torch"]}"##);
    // diamond <-> block: diamond is raw, the block is not
    m = recipe(
        m,
        true,
        "diamond",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:diamond_block"],"result":{"id":"minecraft:diamond","count":9}}"##,
    );
    m = recipe(
        m,
        true,
        "diamond_block",
        r##"{"type":"minecraft:crafting_shaped","pattern":["DDD","DDD","DDD"],"key":{"D":"minecraft:diamond"},"result":{"id":"minecraft:diamond_block"}}"##,
    );
    // iron: also smelted, so crafted; its nugget is crafted too
    m = recipe(
        m,
        true,
        "iron_ingot",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:iron_block"],"result":{"id":"minecraft:iron_ingot","count":9}}"##,
    );
    m = recipe(
        m,
        true,
        "iron_block",
        r##"{"type":"minecraft:crafting_shaped","pattern":["DDD","DDD","DDD"],"key":{"D":"minecraft:iron_ingot"},"result":{"id":"minecraft:iron_block"}}"##,
    );
    m = recipe(
        m,
        true,
        "iron_ingot_from_smelting",
        r##"{"type":"minecraft:smelting","ingredient":"minecraft:raw_iron","result":{"id":"minecraft:iron_ingot"}}"##,
    );
    m = recipe(
        m,
        true,
        "iron_nugget",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:iron_ingot"],"result":{"id":"minecraft:iron_nugget","count":9}}"##,
    );
    m = recipe(
        m,
        true,
        "iron_ingot_from_nuggets",
        r##"{"type":"minecraft:crafting_shaped","pattern":["NNN","NNN","NNN"],"key":{"N":"minecraft:iron_nugget"},"result":{"id":"minecraft:iron_ingot"}}"##,
    );
    // coal has no furnace recipe here: raw by the pair rule (it is in the list too); a lone unpack of a mixed block is not
    m = recipe(
        m,
        true,
        "coal",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:coal_block"],"result":{"id":"minecraft:coal","count":9}}"##,
    );
    m = recipe(
        m,
        true,
        "coal_block",
        r##"{"type":"minecraft:crafting_shaped","pattern":["DDD","DDD","DDD"],"key":{"D":"minecraft:coal"},"result":{"id":"minecraft:coal_block"}}"##,
    );
    m = recipe(
        m,
        true,
        "sugar",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:wheat"],"result":{"id":"minecraft:sugar"}}"##,
    );
    let ds = load_mem(m);
    let raw = |s: &str| ds.raw_stops.contains(&format!("minecraft:{s}"));
    assert!(raw("diamond") && raw("white_wool") && raw("red_wool") && raw("oak_log") && raw("coal") && raw("wheat"));
    assert!(raw("torch"), "members of nested tags count");
    assert!(!raw("stripped_oak_log") && !raw("oak_wood"));
    assert!(!raw("diamond_block") && !raw("coal_block") && !raw("iron_block"));
    assert!(!raw("iron_ingot") && !raw("iron_nugget") && !raw("sugar"));
    assert!(raw("raw_iron"));
}

#[test]
fn the_pack_rule_alone_finds_new_gatherables() {
    // `scrap` is in no list: only the pair rule can find it
    let mut m = game(&["scrap", "scrap_block", "plain", "plain_block"], true);
    m = recipe(
        m,
        true,
        "scrap",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:scrap_block"],"result":{"id":"minecraft:scrap","count":9}}"##,
    );
    m = recipe(
        m,
        true,
        "scrap_block",
        r##"{"type":"minecraft:crafting_shaped","pattern":["SSS","SSS","SSS"],"key":{"S":"minecraft:scrap"},"result":{"id":"minecraft:scrap_block"}}"##,
    );
    // one way only: no pair
    m = recipe(
        m,
        true,
        "plain_block",
        r##"{"type":"minecraft:crafting_shaped","pattern":["SS","SS"],"key":{"S":"minecraft:plain"},"result":{"id":"minecraft:plain_block"}}"##,
    );
    let ds = load_mem(m);
    assert!(ds.raw_stops.contains("minecraft:scrap"));
    assert!(!ds.raw_stops.contains("minecraft:scrap_block") && !ds.raw_stops.contains("minecraft:plain"));
}

#[test]
fn categories_and_names() {
    let mut m = game(
        &["iron_sword", "lever", "oak_stairs", "stone_bricks", "stick", "stone_button", "baked_potato", "torch"],
        true,
    );
    // a model without a name, a name without a model, and air
    m = m.with("assets/minecraft/items/nameless.json", "{}");
    m = m.with("assets/minecraft/items/air.json", "{}");
    m = recipe(
        m,
        true,
        "stick_decl",
        r##"{"type":"minecraft:crafting_shapeless","category":"building","ingredients":["minecraft:torch"],"result":{"id":"minecraft:stick"}}"##,
    );
    m = recipe(
        m,
        true,
        "stick_decl2",
        r##"{"type":"minecraft:crafting_shapeless","category":"misc","ingredients":["minecraft:torch"],"result":{"id":"minecraft:stick"}}"##,
    );
    m = recipe(
        m,
        true,
        "torch",
        r##"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stick"],"result":{"id":"minecraft:torch"}}"##,
    );
    let lang = m.0.get_mut("assets/minecraft/lang/en_us.json").unwrap();
    let mut v: serde_json::Map<String, Value> = serde_json::from_slice(lang).unwrap();
    v.insert("block.minecraft.lever".into(), "Block Lever".into());
    v.insert("item.minecraft.lever".into(), "Item Lever".into());
    v.insert("block.minecraft.only_block".into(), "Only Block".into());
    v.insert("item.minecraft.air".into(), "Air".into());
    *lang = serde_json::to_vec(&v).unwrap();
    m = m.with("assets/minecraft/items/only_block.json", "{}");
    let ds = load_mem(m);
    let cat = |s: &str| ds.item(&format!("minecraft:{s}")).unwrap().category;
    assert_eq!(cat("stick"), Category::Building, "the first recipe that declares a category wins (sorted by id)");
    assert_eq!(cat("iron_sword"), Category::Equipment);
    assert_eq!(cat("lever"), Category::Redstone);
    assert_eq!(cat("stone_button"), Category::Redstone);
    assert_eq!(cat("oak_stairs"), Category::Building);
    assert_eq!(cat("stone_bricks"), Category::Building);
    assert_eq!(cat("baked_potato"), Category::Food);
    assert_eq!(cat("torch"), Category::Misc);
    assert_eq!(ds.item("minecraft:lever").unwrap().name, "Item Lever");
    assert_eq!(ds.item("minecraft:only_block").unwrap().name, "Only Block");
    assert_eq!(ds.item("minecraft:stone_bricks").unwrap().name, "Stone Bricks");
    assert!(ds.item("minecraft:nameless").is_none(), "an item without an English name is not listed");
    assert!(ds.item("minecraft:air").is_none());
    assert_eq!(ds.item("minecraft:stick").unwrap().stack, crate::craft::stack::stack_size("minecraft:stick"));
}

#[test]
fn version_order() {
    let key = |v: &str| version_key(v).unwrap_or_else(|| panic!("{v}"));
    assert!(key("26.3") > key("1.21.1"));
    assert!(key("1.21.1") > key("1.20.4"));
    assert!(key("1.20.4") > key("1.20.1"));
    assert!(key("1.20.1") > key("1.13"));
    assert!(key("1.10") > key("1.9"), "numbers, not text");
    assert!(key("1.21") == key("1.21.0"));
    assert!(key("1.21") > key("1.21-rc1") && key("1.21-rc1") > key("1.21-pre4") && key("1.21-pre4") > key("1.20.6"));
    assert!(key("26.1") > key("26.1-snapshot-3") && key("26.1-snapshot-3") > key("1.21.9"));
    assert!(key("1.13") > key("24w14a"), "weekly snapshots come last");
    assert!(key("24w14a") > key("24w13a") && key("24w14b") > key("24w14a") && key("24w01a") > key("23w52a"));
    for bad in [
        "1.20.1-20230612.114412",
        "latest",
        "1",
        "1.x",
        "",
        "1.20.1-forge",
        "1.21.1-pre",
        "24w14",
        "1.14 Pre-Release 1",
    ] {
        assert!(version_key(bad).is_none(), "{bad}");
    }
    assert!(!has_recipe_files(&key("1.12.2")) && !has_recipe_files(&key("1.8.9")));
    assert!(has_recipe_files(&key("1.13")) && has_recipe_files(&key("1.13-pre1")) && has_recipe_files(&key("26.3")));
    assert!(has_recipe_files(&key("17w43a")) && has_recipe_files(&key("24w14a")) && !has_recipe_files(&key("17w31a")));
}

#[test]
fn installed_versions_lists_plain_jars_newest_first() {
    let base = std::env::temp_dir().join(format!("bf-craft-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dirs = Dirs::at(&base);
    assert!(installed_versions(&dirs).is_empty(), "no libraries folder at all");
    let client = dirs.libraries.join("net/minecraft/client");
    let put = |dir: &str, file: &str| {
        std::fs::create_dir_all(client.join(dir)).unwrap();
        std::fs::write(client.join(dir).join(file), b"x").unwrap();
    };
    put("1.20.4", "client-1.20.4.jar");
    put("1.21.1", "client-1.21.1.jar");
    put("26.3", "client-26.3.jar");
    put("1.13", "client-1.13.jar");
    put("1.12.2", "client-1.12.2.jar");
    put("24w14a", "client-24w14a.jar");
    put("1.21.2-pre1", "client-1.21.2-pre1.jar");
    put("1.20.1", "client-1.20.1.jar");
    put("1.20.1-20230612.114412", "client-1.20.1-20230612.114412-srg.jar");
    put("1.20.1-20230612.114412", "client-1.20.1-20230612.114412-slim.jar");
    put("1.21.1-20240808.144430", "client-1.21.1-20240808.144430-extra.jar");
    put("1.19.4", "client-1.19.4-slim.jar");
    put("1.18.2", "other.jar");
    std::fs::create_dir_all(client.join("1.17.1")).unwrap();
    let got: Vec<String> = installed_versions(&dirs).into_iter().map(|v| v.version).collect();
    assert_eq!(got, ["26.3", "1.21.2-pre1", "1.21.1", "1.20.4", "1.20.1", "1.13", "24w14a"]);
    let first = installed_versions(&dirs).remove(0);
    assert_eq!(first.jar, client.join("26.3").join("client-26.3.jar"));
    let _ = std::fs::remove_dir_all(&base);
}

// ---------------------------------------------------------------------------------------------------------
// The real jars

/// Where the client jar of `version` is: the folder named by `BF_CRAFT_JARS` (default `~/bf-craft-work/jars`), or
/// the launcher's own libraries folder.
fn find_jar(version: &str) -> Option<PathBuf> {
    let file = format!("client-{version}.jar");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = std::env::var_os("BF_CRAFT_JARS")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join("bf-craft-work").join("jars")));
    let mut candidates: Vec<PathBuf> = dir.into_iter().map(|d| d.join(&file)).collect();
    candidates.push(Dirs::detect().libraries.join("net/minecraft/client").join(version).join(&file));
    candidates.into_iter().find(|p| p.is_file())
}

/// Load and time one version, or note that its jar is missing.
fn real(version: &str) -> Option<Dataset> {
    let Some(jar) = find_jar(version) else {
        eprintln!("note: no client-{version}.jar on this computer, test skipped");
        return None;
    };
    let t = Instant::now();
    let ds = load_jar(version, &jar).unwrap_or_else(|e| panic!("{version}: {e}"));
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (_, why) in &ds.skipped {
        *kinds.entry(why.split(['(', ':']).next().unwrap_or("").trim().to_string()).or_default() += 1;
    }
    eprintln!(
        "{version}: {ms:.0} ms, {} items, {} recipes, {} skipped {kinds:?}, {} raw stops",
        ds.items.len(),
        ds.recipes.len(),
        ds.skipped.len(),
        ds.raw_stops.len()
    );
    for (id, why) in ds.skipped.iter().filter(|(_, w)| w.starts_with("malformed")) {
        eprintln!("  {id}: {why}");
    }
    Some(ds)
}

/// What the pack rule finds to the listed raw items (for a look with `--nocapture`).
#[test]
fn real_pack_rule_finds() {
    for v in ["1.21.1", "26.3"] {
        if let Some(ds) = real(v) {
            let mut found: Vec<String> = pack_rule(&ds.recipes).into_iter().map(|r| r.replace(NS, "")).collect();
            found.sort();
            eprintln!("{v}: the pack rule finds {found:?}");
        }
    }
}

fn check_modern(ds: &Dataset) {
    let v = &ds.version;
    assert!(ds.items.len() > 1100, "{v}: {} items", ds.items.len());
    assert!(ds.recipes.len() > 1100, "{v}: {} recipes", ds.recipes.len());
    let stick = ds.recipe("stick").expect("stick");
    assert_eq!((stick.kind, stick.count), (Kind::Shaped, 4));
    assert_eq!(stick.ingredients.len(), 1);
    assert_eq!(stick.ingredients[0].count, 2);
    assert!(stick.ingredients[0].alternatives.contains(&"minecraft:oak_planks".to_string()));
    assert!(stick.ingredients[0].alternatives.iter().all(|a| a.ends_with("_planks")));
    assert!(stick.small_grid);
    let iron = ds.recipes_for("minecraft:iron_ingot").next().expect("iron ingot recipe");
    assert_eq!(iron.kind, Kind::Smelting, "{v}: {}", iron.id);
    assert_eq!(iron.ingredients[0].alternatives, ["minecraft:raw_iron"]);
    let cake = ds.recipes_for("minecraft:cake").next().expect("cake");
    let milk = cake.ingredients.iter().find(|i| i.alternatives == ["minecraft:milk_bucket"]).expect("milk in the cake");
    assert_eq!(milk.count, 3);
    for raw in ["diamond", "coal", "white_wool", "oak_log", "raw_iron", "redstone", "slime_ball", "bamboo"] {
        assert!(ds.raw_stops.contains(&format!("minecraft:{raw}")), "{v}: {raw} is a raw stop");
    }
    for crafted in ["diamond_block", "iron_ingot", "stick", "oak_planks", "iron_nugget", "stripped_oak_log", "oak_wood"]
    {
        assert!(!ds.raw_stops.contains(&format!("minecraft:{crafted}")), "{v}: {crafted} is crafted");
    }
    // the netherite upgrade is the manual smithing recipe, without a fixed ingredient list
    let sword = ds.recipes_for("minecraft:netherite_sword").next().expect("netherite sword");
    assert_eq!(sword.kind, Kind::Smithing);
    assert!(sword.manual.is_some());
    assert_eq!(ds.item("minecraft:diamond_sword").unwrap().category, Category::Equipment);
    assert_eq!(ds.item("minecraft:oak_planks").unwrap().name, "Oak Planks");
    assert_eq!(ds.item("minecraft:cake").unwrap().name, "Cake");
    assert!(ds.item("minecraft:air").is_none());
    // no reference to an unknown item (Dataset::new checks this too), and unique recipe ids
    for r in &ds.recipes {
        assert!(ds.item(&r.output).is_some());
        for i in &r.ingredients {
            assert!(!i.alternatives.is_empty() && i.count > 0);
            assert!(i.alternatives.windows(2).all(|w| w[0] < w[1]), "{}: sorted, no duplicates", r.id);
            assert!(i.alternatives.iter().all(|a| ds.item(a).is_some()), "{}", r.id);
        }
    }
    assert!(ds.raw_stops.iter().all(|r| ds.item(r).is_some()));
}

#[test]
fn real_1_20_4() {
    if let Some(ds) = real("1.20.4") {
        check_modern(&ds);
        // 1.20.4: 30 dynamic recipes and 16 trims are the ones left out
        assert!(ds.skipped.len() < 60, "{:?}", ds.skipped);
    }
}

#[test]
fn real_1_21_1() {
    if let Some(ds) = real("1.21.1") {
        check_modern(&ds);
        assert!(ds.skipped.len() < 60, "{:?}", ds.skipped);
    }
}

#[test]
fn real_every_modern_version_loads() {
    for v in ["1.20.1", "1.20.4", "1.21.1", "26.3"] {
        if let Some(ds) = real(v) {
            check_modern(&ds);
            assert!(ds.skipped.iter().all(|(id, why)| !id.is_empty() && !why.is_empty()));
        }
    }
}

#[test]
fn real_old_versions_say_what_is_needed() {
    for v in ["1.12.2", "1.8.9"] {
        let Some(jar) = find_jar(v) else {
            eprintln!("note: no client-{v}.jar on this computer, test skipped");
            continue;
        };
        let e = load_jar(v, &jar).unwrap_err();
        assert!(e.contains("Minecraft 1.13 or newer is needed"), "{v}: {e}");
    }
}

#[test]
fn real_installed_versions_of_this_computer() {
    let list = installed_versions(&Dirs::detect());
    eprintln!("installed: {:?}", list.iter().map(|v| v.version.as_str()).collect::<Vec<_>>());
    for v in &list {
        assert!(v.jar.is_file());
        assert!(v.jar.ends_with(format!("client-{}.jar", v.version)));
    }
}
