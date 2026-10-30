//! Tests of the planner on small hand-made recipe books.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Instant;

use super::super::stack::storage;
use super::*;

fn full(s: &str) -> String {
    format!("minecraft:{s}")
}

fn ing(key: &str, count: u32, alts: &[&str]) -> Ingredient {
    let mut alternatives: Vec<String> = alts.iter().map(|a| full(a)).collect();
    alternatives.sort();
    Ingredient { key: key.to_string(), count, alternatives }
}

fn rec(id: &str, kind: Kind, out: &str, count: u32, ingredients: Vec<Ingredient>) -> Recipe {
    Recipe {
        id: id.to_string(),
        kind,
        output: full(out),
        count,
        ingredients,
        manual: None,
        ticks: None,
        small_grid: false,
        shape: Vec::new(),
    }
}

fn smelt(id: &str, out: &str, input: &str, ticks: u32) -> Recipe {
    Recipe { ticks: Some(ticks), ..rec(id, Kind::Smelting, out, 1, vec![ing("input", 1, &[input])]) }
}

fn book(items: &[&str], recipes: Vec<Recipe>, raw: &[&str]) -> Dataset {
    let items: BTreeMap<ItemId, Item> = items
        .iter()
        .map(|i| {
            let id = full(i);
            (id.clone(), Item { name: pretty_name(&id), id, stack: 64, category: Category::Misc })
        })
        .collect();
    let raw: HashSet<ItemId> = raw.iter().map(|r| full(r)).collect();
    Dataset::new("test", items, recipes, raw, Vec::new()).unwrap()
}

fn req(item: &str, quantity: u64) -> Request {
    Request { item: full(item), quantity, ..Default::default() }
}

/// A small survival-like book: wood, tools, cooking, stone, a cake, a honey block and a recipe cycle.
fn tools() -> Dataset {
    let items = [
        "oak_log",
        "oak_planks",
        "stick",
        "diamond",
        "diamond_pickaxe",
        "stone",
        "cobblestone",
        "stone_bricks",
        "raw_iron",
        "iron_ingot",
        "iron_nugget",
        "coal",
        "lava_bucket",
        "wheat",
        "cocoa_beans",
        "cookie",
        "sugar",
        "egg",
        "milk_bucket",
        "bucket",
        "cake",
        "honey_bottle",
        "glass_bottle",
        "honey_block",
        "netherite_pickaxe",
        "elytra",
        "ring_a",
        "ring_b",
    ];
    let recipes = vec![
        rec("oak_planks", Kind::Shapeless, "oak_planks", 4, vec![ing("#0", 1, &["oak_log"])]),
        rec("stick", Kind::Shaped, "stick", 4, vec![ing("#", 2, &["oak_planks"])]),
        rec(
            "diamond_pickaxe",
            Kind::Shaped,
            "diamond_pickaxe",
            1,
            vec![ing("#", 2, &["stick"]), ing("X", 3, &["diamond"])],
        ),
        rec("cookie", Kind::Shapeless, "cookie", 8, vec![ing("#0", 2, &["wheat"]), ing("#1", 1, &["cocoa_beans"])]),
        rec("stone_bricks", Kind::Shaped, "stone_bricks", 4, vec![ing("#", 4, &["stone"])]),
        rec(
            "stone_bricks_from_stone_stonecutting",
            Kind::Stonecutting,
            "stone_bricks",
            1,
            vec![ing("input", 1, &["stone"])],
        ),
        smelt("stone", "stone", "cobblestone", 200),
        smelt("iron_ingot_from_smelting_raw_iron", "iron_ingot", "raw_iron", 200),
        rec(
            "cake",
            Kind::Shaped,
            "cake",
            1,
            vec![
                ing("A", 3, &["milk_bucket"]),
                ing("B", 1, &["egg"]),
                ing("C", 2, &["sugar"]),
                ing("E", 3, &["wheat"]),
            ],
        ),
        rec("honey_block", Kind::Shaped, "honey_block", 1, vec![ing("S", 4, &["honey_bottle"])]),
        rec("ring_a", Kind::Shapeless, "ring_a", 1, vec![ing("#0", 1, &["ring_b"])]),
        rec("ring_b", Kind::Shapeless, "ring_b", 1, vec![ing("#0", 1, &["ring_a"])]),
        Recipe {
            manual: Some("Smithing keeps enchantments, upgrade it by hand.".into()),
            ..rec("netherite_pickaxe_smithing", Kind::Smithing, "netherite_pickaxe", 1, vec![])
        },
    ];
    book(&items, recipes, &["oak_log", "diamond", "wheat", "cocoa_beans", "raw_iron", "stone", "cobblestone"])
}

fn amount(list: &[(ItemId, u64)], item: &str) -> u64 {
    list.iter().find(|(i, _)| *i == full(item)).map(|(_, n)| *n).unwrap_or(0)
}

fn row<'a>(plan: &'a Plan, item: &str) -> &'a LedgerRow {
    plan.ledger.iter().find(|r| r.item == full(item)).unwrap_or_else(|| panic!("no ledger row for {item}"))
}

fn step<'a>(plan: &'a Plan, recipe: &str) -> &'a Step {
    plan.steps.iter().find(|s| s.recipe == recipe).unwrap_or_else(|| panic!("no step {recipe}"))
}

fn fail(ds: &Dataset, r: &Request) -> String {
    calculate(ds, r).expect_err("the request should fail").0
}

#[test]
fn ten_thousand_cookies() {
    let ds = tools();
    let plan = calculate(&ds, &req("cookie", 10_000)).unwrap();
    assert_eq!(plan.operations, 1250);
    assert_eq!(amount(&plan.materials, "wheat"), 2500);
    assert_eq!(amount(&plan.materials, "cocoa_beans"), 1250);
    assert!(plan.intermediates.is_empty());
    assert!(plan.complete && plan.warnings.is_empty());
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].output, 10_000);
    let finished = storage(&ds, &[(full("cookie"), 10_000)]);
    assert_eq!(finished.slots, 157);
    assert_eq!(finished.double_chests.count, 3);
    assert_eq!(finished.double_chests.unused, 5);
    assert_eq!(plan.tree.stop, None);
    assert_eq!(plan.tree.children.len(), 2);
    assert!(plan.tree.children.iter().all(|c| c.stop == Some(Stop::Raw)));
}

#[test]
fn surplus_of_planks_is_reused_after_its_batch() {
    // 3 planks and 1 stick: the planks make a batch of 4 (1 left), the sticks then take that 1 and need 1 more
    let recipes = vec![
        rec("oak_planks", Kind::Shapeless, "oak_planks", 4, vec![ing("#0", 1, &["oak_log"])]),
        rec("stick", Kind::Shaped, "stick", 4, vec![ing("#", 2, &["oak_planks"])]),
        rec("gizmo", Kind::Shaped, "gizmo", 1, vec![ing("P", 3, &["oak_planks"]), ing("S", 1, &["stick"])]),
    ];
    let ds = book(&["oak_log", "oak_planks", "stick", "gizmo"], recipes, &["oak_log"]);
    let plan = calculate(&ds, &req("gizmo", 1)).unwrap();
    let planks = row(&plan, "oak_planks");
    assert_eq!((planks.required, planks.from_surplus, planks.produced, planks.surplus), (5, 1, 8, 3));
    assert_eq!(amount(&plan.materials, "oak_log"), 2);
    let sticks = row(&plan, "stick");
    assert_eq!((sticks.required, sticks.produced, sticks.surplus), (1, 4, 3));
    assert_eq!(step(&plan, "oak_planks").operations, 2);
    // the second plank node took from the surplus of the first
    let stick_node = plan.tree.children.iter().find(|c| c.item == full("stick")).unwrap();
    assert_eq!(stick_node.children[0].from_surplus, 1);
    assert_eq!(stick_node.children[0].to_produce, 1);
}

#[test]
fn leftovers_of_a_short_chain() {
    // a diamond pickaxe: 2 sticks come from one batch of 4 (2 left over), made of 2 of the 4 planks of one log
    let ds = tools();
    let plan = calculate(&ds, &req("diamond_pickaxe", 1)).unwrap();
    assert_eq!(amount(&plan.materials, "diamond"), 3);
    assert_eq!(amount(&plan.materials, "oak_log"), 1);
    assert_eq!(row(&plan, "stick").surplus, 2);
    assert_eq!(row(&plan, "oak_planks").surplus, 2);
}

#[test]
fn crafting_beats_stonecutting_unless_chosen() {
    let ds = tools();
    let plan = calculate(&ds, &req("stone_bricks", 3)).unwrap();
    assert_eq!(plan.tree.recipe.as_deref(), Some("stone_bricks"));
    assert!(plan.tree.default_recipe);
    assert_eq!((plan.tree.operations, plan.tree.output, plan.tree.surplus), (1, 4, 1));
    assert_eq!(amount(&plan.materials, "stone"), 4);

    let mut r = req("stone_bricks", 3);
    r.recipe_choices.insert(full("stone_bricks"), "stone_bricks_from_stone_stonecutting".into());
    let plan = calculate(&ds, &r).unwrap();
    assert!(!plan.tree.default_recipe);
    assert_eq!(plan.steps[0].kind, Kind::Stonecutting);
    assert_eq!(amount(&plan.materials, "stone"), 3);
    assert_eq!(row(&plan, "stone_bricks").surplus, 0);
}

#[test]
fn inventory_is_used_first() {
    let ds = tools();
    let mut r = req("diamond_pickaxe", 1);
    r.inventory.insert(full("diamond"), 1);
    r.inventory.insert(full("stick"), 5);
    let plan = calculate(&ds, &r).unwrap();
    assert_eq!(amount(&plan.materials, "diamond"), 2);
    assert_eq!(row(&plan, "diamond").from_stock, 1);
    assert_eq!(row(&plan, "stick").from_stock, 2);
    // the sticks came from stock: no planks, no logs, only the pickaxe step
    assert_eq!(plan.steps.len(), 1);
    let stick_node = plan.tree.children.iter().find(|c| c.item == full("stick")).unwrap();
    assert_eq!(stick_node.stop, Some(Stop::Inventory));
    assert!(stick_node.children.is_empty());

    // everything in stock: the root itself stops at the inventory
    let mut r = req("diamond_pickaxe", 2);
    r.inventory.insert(full("diamond_pickaxe"), 5);
    let plan = calculate(&ds, &r).unwrap();
    assert_eq!(plan.tree.stop, Some(Stop::Inventory));
    assert!(plan.steps.is_empty() && plan.materials.is_empty() && plan.complete);
}

#[test]
fn supplied_items_stop_the_branch() {
    let ds = tools();
    let mut r = req("diamond_pickaxe", 1);
    r.supplied.insert(full("stick"));
    let plan = calculate(&ds, &r).unwrap();
    assert_eq!(amount(&plan.materials, "stick"), 2);
    assert_eq!(amount(&plan.materials, "oak_log"), 0);
    let stick_node = plan.tree.children.iter().find(|c| c.item == full("stick")).unwrap();
    assert_eq!(stick_node.stop, Some(Stop::Supplied));
    assert!(plan.complete && plan.warnings.is_empty());

    // supplied wins over a recipe choice, and a supplied root is no problem either
    let mut r = req("elytra", 1);
    r.supplied.insert(full("elytra"));
    let plan = calculate(&ds, &r).unwrap();
    assert!(plan.complete && plan.warnings.is_empty());
    assert_eq!(plan.tree.stop, Some(Stop::Supplied));
}

#[test]
fn raw_stops_give_way_to_a_recipe_choice() {
    let ds = tools();
    let plan = calculate(&ds, &req("stone", 5)).unwrap();
    assert_eq!(plan.tree.stop, Some(Stop::Raw));
    assert_eq!(amount(&plan.materials, "stone"), 5);
    assert!(plan.steps.is_empty());

    let mut r = req("stone", 5);
    r.recipe_choices.insert(full("stone"), "stone".into());
    let plan = calculate(&ds, &r).unwrap();
    assert_eq!(plan.tree.stop, None);
    assert_eq!(amount(&plan.materials, "cobblestone"), 5);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.cook_ticks, 1000);
}

#[test]
fn cycles_are_cut_with_a_warning() {
    let ds = tools();
    let plan = calculate(&ds, &req("ring_a", 2)).unwrap();
    assert!(!plan.complete);
    assert_eq!(
        plan.warnings,
        vec!["Recipe cycle at Ring A. Supply this ingredient externally or choose another recipe."]
    );
    let b = &plan.tree.children[0];
    assert_eq!(b.item, full("ring_b"));
    assert_eq!(b.children[0].stop, Some(Stop::Cycle));
    assert_eq!(amount(&plan.materials, "ring_a"), 2);
    // the steps still come out in some order, each once
    assert_eq!(plan.steps.len(), 2);
}

#[test]
fn manual_and_missing_recipes_are_warnings() {
    let ds = tools();
    let plan = calculate(&ds, &req("netherite_pickaxe", 1)).unwrap();
    assert!(!plan.complete);
    assert_eq!(plan.tree.stop, Some(Stop::Manual));
    assert_eq!(plan.tree.recipe.as_deref(), Some("netherite_pickaxe_smithing"));
    assert_eq!(plan.warnings, vec!["Netherite Pickaxe: Smithing keeps enchantments, upgrade it by hand."]);
    assert_eq!(amount(&plan.materials, "netherite_pickaxe"), 1);

    let plan = calculate(&ds, &req("elytra", 1)).unwrap();
    assert!(!plan.complete);
    assert_eq!(plan.tree.stop, Some(Stop::NoRecipe));
    assert_eq!(plan.warnings, vec!["Elytra: no normal survival recipe found. Supply or gather this item directly."]);

    // a missing recipe deeper in the tree is only a leaf: no warning, still complete
    let mut r = req("cake", 1);
    r.inventory.insert(full("egg"), 1);
    let plan = calculate(&ds, &r).unwrap();
    assert!(plan.complete && plan.warnings.is_empty());
    assert_eq!(plan.tree.children.iter().find(|c| c.item == full("milk_bucket")).unwrap().stop, Some(Stop::NoRecipe));
}

#[test]
fn bad_requests_are_refused() {
    let ds = tools();
    let quantity =
        "Enter a positive whole quantity, up to 1,000,000,000. Inventory must contain non-negative whole quantities.";
    assert_eq!(fail(&ds, &req("stick", 0)), quantity);
    assert_eq!(fail(&ds, &req("stick", MAX_QUANTITY + 1)), quantity);
    assert!(calculate(&ds, &req("stick", MAX_QUANTITY)).is_ok());
    assert_eq!(fail(&ds, &req("nope", 1)), "Item unavailable in this game version: minecraft:nope");
    let mut r = req("stick", 1);
    r.inventory.insert(full("ghost"), 1);
    assert_eq!(fail(&ds, &r), "Item unavailable in this game version: minecraft:ghost");
    let mut r = req("stick", 1);
    r.supplied.insert(full("ghost"));
    assert_eq!(fail(&ds, &r), "Item unavailable in this game version: minecraft:ghost");

    // a recipe choice must be a recipe of that very item
    let mut r = req("stick", 1);
    r.recipe_choices.insert(full("stick"), "cookie".into());
    assert_eq!(fail(&ds, &r), "Invalid selected recipe for minecraft:stick.");
    r.recipe_choices.insert(full("stick"), "no_such_recipe".into());
    assert_eq!(fail(&ds, &r), "Invalid selected recipe for minecraft:stick.");

    // an ingredient choice must name a real ingredient and one of its alternatives
    let mut r = req("stick", 1);
    r.ingredient_choices.insert("stick:#".into(), full("oak_log"));
    assert_eq!(fail(&ds, &r), "Invalid ingredient alternative: stick:#.");
    r.ingredient_choices.clear();
    r.ingredient_choices.insert("stick:zzz".into(), full("oak_planks"));
    assert_eq!(fail(&ds, &r), "Invalid ingredient alternative: stick:zzz.");
    r.ingredient_choices.clear();
    r.ingredient_choices.insert("nothing:#".into(), full("oak_planks"));
    assert_eq!(fail(&ds, &r), "Invalid ingredient alternative: nothing:#.");
    r.ingredient_choices.clear();
    r.ingredient_choices.insert("stick".into(), full("oak_planks"));
    assert_eq!(fail(&ds, &r), "Invalid ingredient alternative: stick.");
    r.ingredient_choices.clear();
    r.ingredient_choices.insert("stick:#".into(), full("oak_planks"));
    assert!(calculate(&ds, &r).is_ok());
}

/// `l0` is the base, each next level needs 9 of the one below.
fn tower(levels: usize, per: u32) -> Dataset {
    let names: Vec<String> = (0..levels).map(|i| format!("l{i}")).collect();
    let items: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let recipes = (1..levels)
        .map(|i| rec(&format!("l{i}"), Kind::Shapeless, &names[i], 1, vec![ing("#0", per, &[&names[i - 1]])]))
        .collect();
    book(&items, recipes, &[])
}

#[test]
fn overflow_is_an_error_not_a_wrong_number() {
    let ds = tower(6, 9);
    let limit = "Calculation exceeds the safe quantity limit (1 trillion items).";
    assert_eq!(fail(&ds, &req("l5", 1_000_000_000)), limit);
    // 9^3 * 1e9 = 7.29e11 fits, 9^4 * 1e9 does not
    let plan = calculate(&ds, &req("l3", 1_000_000_000)).unwrap();
    assert_eq!(amount(&plan.materials, "l0"), 729_000_000_000);
    assert_eq!(fail(&ds, &req("l4", 1_000_000_000)), limit);
    // a huge inventory is refused the same way
    let mut r = req("l1", 1);
    r.inventory.insert(full("l0"), u64::MAX);
    assert_eq!(fail(&ds, &r), limit);
}

#[test]
fn depth_and_node_limits() {
    let ds = tower(70, 1);
    assert_eq!(
        fail(&ds, &req("l69", 1)),
        "Recipe depth limit exceeded. Supply an intermediate item to shorten the plan."
    );
    assert!(calculate(&ds, &req("l60", 1)).is_ok());
    // depth 64 is the deepest allowed: l64 has 64 levels below it
    assert!(calculate(&ds, &req("l64", 1)).is_ok());
    assert!(calculate(&ds, &req("l65", 1)).is_err());

    let ds = wide(120);
    assert_eq!(
        fail(&ds, &req("root", 1)),
        "Calculation complexity limit exceeded. Use externally supplied ingredients to shorten the plan."
    );
    // supplying the middle layer shortens the plan again
    let mut r = req("root", 1);
    r.supplied = (0..120).map(|i| full(&format!("m{i}"))).collect::<BTreeSet<_>>();
    assert_eq!(calculate(&ds, &r).unwrap().tree.children.len(), 120);
}

/// A root that needs `n` different items, each needing `n` different raw ones: 1 + n + n*n nodes.
fn wide(n: usize) -> Dataset {
    let mut items = vec!["root".to_string()];
    let mut recipes = Vec::new();
    let mut root_in = Vec::new();
    for i in 0..n {
        items.push(format!("m{i}"));
        root_in.push(ing(&format!("#{i}"), 1, &[&format!("m{i}")]));
        let mut leaves = Vec::new();
        for j in 0..n {
            items.push(format!("r{i}_{j}"));
            leaves.push(ing(&format!("#{j}"), 1, &[&format!("r{i}_{j}")]));
        }
        recipes.push(rec(&format!("m{i}"), Kind::Shapeless, &format!("m{i}"), 1, leaves));
    }
    recipes.push(rec("root", Kind::Shapeless, "root", 1, root_in));
    let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
    book(&refs, recipes, &[])
}

#[test]
fn a_big_tree_is_fast() {
    // 1 + 100 + 10,000 nodes, close to the limit of 12,000. Under 20 ms in a release build; the bound here is loose
    // so a debug build on a busy machine passes.
    let ds = wide(100);
    let r = req("root", 1);
    let start = Instant::now();
    let plan = calculate(&ds, &r).unwrap();
    let took = start.elapsed();
    eprintln!("10,101 nodes in {took:?}");
    assert_eq!(plan.materials.len(), 10_000);
    assert!(took.as_millis() < 1500, "{took:?}");
    let ds = tools();
    let r = req("diamond_pickaxe", 64);
    let start = Instant::now();
    for _ in 0..100 {
        calculate(&ds, &r).unwrap();
    }
    eprintln!("a small plan in {:?}", start.elapsed() / 100);
}

#[test]
fn cakes_give_buckets_back() {
    let ds = tools();
    let mut r = req("cake", 2);
    r.supplied.insert(full("milk_bucket"));
    let plan = calculate(&ds, &r).unwrap();
    let s = step(&plan, "cake");
    assert_eq!(s.byproducts, vec![(full("bucket"), 6)]);
    assert_eq!(s.inputs.iter().find(|(i, _)| *i == full("milk_bucket")).unwrap().1, 6);
    // not credited as stock: the bucket is no ledger row and no material
    assert!(plan.ledger.iter().all(|l| l.item != full("bucket")));
    assert_eq!(amount(&plan.materials, "bucket"), 0);

    let plan = calculate(&ds, &req("honey_block", 3)).unwrap();
    assert_eq!(step(&plan, "honey_block").byproducts, vec![(full("glass_bottle"), 12)]);

    // the container is only listed when the book has it
    let items: Vec<&str> = vec!["honey_bottle", "honey_block"];
    let small = book(
        &items,
        vec![rec("honey_block", Kind::Shaped, "honey_block", 1, vec![ing("S", 4, &["honey_bottle"])])],
        &[],
    );
    let plan = calculate(&small, &req("honey_block", 1)).unwrap();
    assert!(plan.steps[0].byproducts.is_empty());
}

#[test]
fn smelting_ticks_and_fuel() {
    let ds = tools();
    let plan = calculate(&ds, &req("iron_ingot", 64)).unwrap();
    // iron ore is raw here, so the ingots are smelted: 64 * 200 ticks
    assert_eq!(plan.tree.stop, None);
    assert_eq!(step(&plan, "iron_ingot_from_smelting_raw_iron").ticks, Some(12_800));
    assert_eq!(plan.cook_ticks, 12_800);
    let fuel: Vec<(&str, u64)> = plan.fuel.iter().map(|f| (f.item.as_str(), f.count)).collect();
    // only fuels that are in this book: coal 1600 ticks, planks and logs 300, sticks 100, lava bucket 20000
    assert_eq!(
        fuel,
        vec![
            ("minecraft:coal", 8),
            ("minecraft:oak_planks", 43),
            ("minecraft:oak_log", 43),
            ("minecraft:stick", 128),
            ("minecraft:lava_bucket", 1),
        ]
    );
    // a plan that cooks nothing has no fuel and no ticks
    let plan = calculate(&ds, &req("diamond_pickaxe", 1)).unwrap();
    assert_eq!(plan.cook_ticks, 0);
    assert!(plan.fuel.is_empty());
    assert!(plan.steps.iter().all(|s| s.ticks.is_none()));
    // campfire cooking and crafting steps never burn fuel items
    assert!(!Kind::Campfire.burns_fuel() && !Kind::Shaped.burns_fuel());
}

#[test]
fn steps_come_dependency_first() {
    let ds = tools();
    let plan = calculate(&ds, &req("diamond_pickaxe", 1)).unwrap();
    let order: Vec<&str> = plan.steps.iter().map(|s| s.recipe.as_str()).collect();
    assert_eq!(order, vec!["oak_planks", "stick", "diamond_pickaxe"]);
    assert_eq!(plan.operations, 3);
    assert_eq!(plan.intermediates, vec![(full("oak_planks"), 4), (full("stick"), 4)]);
    assert_eq!(plan.materials, vec![(full("diamond"), 3), (full("oak_log"), 1)]);
    assert_eq!(plan.steps[2].choices, vec![("#".to_string(), full("stick")), ("X".to_string(), full("diamond"))]);

    // a later branch can need something an earlier one took from stock: the order still holds
    let recipes = vec![
        rec("a", Kind::Shapeless, "a", 1, vec![ing("#0", 1, &["b"]), ing("#1", 1, &["c"])]),
        rec("b", Kind::Shapeless, "b", 1, vec![ing("#0", 1, &["c"])]),
        rec("c", Kind::Shapeless, "c", 1, vec![ing("#0", 1, &["d"])]),
    ];
    let ds = book(&["a", "b", "c", "d"], recipes, &["d"]);
    let plan = calculate(&ds, &req("a", 1)).unwrap();
    let order: Vec<&str> = plan.steps.iter().map(|s| s.item.as_str()).collect();
    assert_eq!(order, vec!["minecraft:c", "minecraft:b", "minecraft:a"]);
}

#[test]
fn ordering_survives_a_cycle() {
    let step_of = |item: &str, input: &str| Step {
        recipe: item.into(),
        item: full(item),
        kind: Kind::Shaped,
        operations: 1,
        output: 1,
        inputs: vec![(full(input), 1)],
        choices: Vec::new(),
        byproducts: Vec::new(),
        ticks: None,
    };
    let ordered = order(vec![step_of("a", "b"), step_of("b", "c"), step_of("c", "a"), step_of("d", "d")]);
    let mut names: Vec<&str> = ordered.iter().map(|s| s.recipe.as_str()).collect();
    assert_eq!(names.len(), 4);
    names.sort();
    assert_eq!(names, vec!["a", "b", "c", "d"]);
}

#[test]
fn ingredient_choice_rules() {
    let items = [
        "bundle",
        "red_bundle",
        "blue_bundle",
        "oak_planks",
        "birch_planks",
        "spruce_planks",
        "q",
        "rrr",
        "top",
        "spruce_log",
        "spruce_logs",
        "chest",
        "bundle_box",
    ];
    let recipes = vec![
        // the shortest id: `bundle`
        rec("bundle_box", Kind::Shaped, "bundle_box", 1, vec![ing("B", 1, &["red_bundle", "bundle", "blue_bundle"])]),
        // the fixed preference beats the shortest id
        rec("chest", Kind::Shaped, "chest", 1, vec![ing("P", 8, &["birch_planks", "oak_planks", "spruce_planks"])]),
        // q <- top <- one of [q, rrr]: q is being made above, so rrr
        rec("q", Kind::Shapeless, "q", 1, vec![ing("#0", 1, &["top"])]),
        rec("top", Kind::Shapeless, "top", 1, vec![ing("#0", 1, &["q", "rrr"])]),
        // never the recipe's own output
        rec("rrr", Kind::Shapeless, "rrr", 1, vec![ing("#0", 1, &["rrr", "spruce_log"])]),
        // alphabetical among equal lengths
        rec("spruce_logs", Kind::Shapeless, "spruce_logs", 1, vec![ing("#0", 1, &["spruce_log", "spruce_lap"])]),
    ];
    let mut items = items.to_vec();
    items.push("spruce_lap");
    let ds = book(&items, recipes, &["bundle", "red_bundle", "oak_planks", "rrr"]);
    let chosen = |plan: &Plan, recipe: &str| step(plan, recipe).choices[0].1.clone();

    let plan = calculate(&ds, &req("bundle_box", 1)).unwrap();
    assert_eq!(chosen(&plan, "bundle_box"), full("bundle"));
    let plan = calculate(&ds, &req("chest", 1)).unwrap();
    assert_eq!(chosen(&plan, "chest"), full("oak_planks"));
    let mut r = req("q", 1);
    r.recipe_choices.insert(full("rrr"), "rrr".into());
    let plan = calculate(&ds, &r).unwrap();
    assert_eq!(chosen(&plan, "top"), full("rrr"));
    assert!(plan.complete);
    let mut r = req("rrr", 1);
    r.recipe_choices.insert(full("rrr"), "rrr".into());
    let plan = calculate(&ds, &r).unwrap();
    assert_eq!(chosen(&plan, "rrr"), full("spruce_log"));
    let plan = calculate(&ds, &req("spruce_logs", 1)).unwrap();
    assert_eq!(chosen(&plan, "spruce_logs"), full("spruce_lap"));

    // the player's pick wins over everything, even when it makes a cycle
    let mut r = req("bundle_box", 1);
    r.ingredient_choices.insert("bundle_box:B".into(), full("blue_bundle"));
    assert_eq!(chosen(&calculate(&ds, &r).unwrap(), "bundle_box"), full("blue_bundle"));
    let mut r = req("q", 1);
    r.ingredient_choices.insert("top:#0".into(), full("q"));
    let plan = calculate(&ds, &r).unwrap();
    assert!(!plan.complete);
    assert_eq!(plan.tree.children[0].children[0].stop, Some(Stop::Cycle));
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The tree must agree with itself: every node's numbers add up, and the leaves are what `materials` says.
fn check_tree(node: &Node, ds: &Dataset, gather: &mut BTreeMap<String, u64>) {
    assert_eq!(node.required, node.from_stock + node.from_surplus + node.to_produce, "{}", node.item);
    match node.stop {
        None => {
            let recipe = ds.recipe(node.recipe.as_ref().unwrap()).unwrap();
            assert_eq!(node.output, node.operations * recipe.count as u64);
            assert_eq!(node.surplus, node.output - node.to_produce);
            assert!(node.operations > 0 && !node.children.is_empty());
        }
        Some(Stop::Inventory) => {
            assert_eq!(node.to_produce, 0);
            assert!(node.children.is_empty());
        }
        Some(_) => {
            assert!(node.children.is_empty() && node.operations == 0);
            *gather.entry(node.item.clone()).or_insert(0) += node.to_produce;
        }
    }
    for c in &node.children {
        check_tree(c, ds, gather);
    }
}

#[test]
fn the_books_balance_for_random_requests() {
    let ds = tools();
    let items: Vec<&String> = ds.items.keys().collect();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut ok, mut failed) = (0, 0);
    for _ in 0..3000 {
        let scale = [5, 100, 5000, 1_000_000_000][rng.below(4) as usize];
        let mut r = Request {
            item: items[rng.below(items.len() as u64) as usize].clone(),
            quantity: 1 + rng.below(scale),
            ..Default::default()
        };
        for _ in 0..rng.below(6) {
            let item = items[rng.below(items.len() as u64) as usize].clone();
            match rng.below(3) {
                0 => {
                    r.inventory.insert(item, rng.below(300));
                }
                1 => {
                    r.supplied.insert(item);
                }
                _ => {
                    let options: Vec<&Recipe> = ds.recipes_for(&item).collect();
                    if !options.is_empty() {
                        r.recipe_choices.insert(item, options[rng.below(options.len() as u64) as usize].id.clone());
                    }
                }
            }
        }
        match calculate(&ds, &r) {
            Ok(plan) => {
                ok += 1;
                for l in &plan.ledger {
                    assert_eq!(l.required + l.surplus, l.from_stock + l.gather + l.produced, "{}", l.item);
                    assert!(l.from_stock <= r.inventory.get(&l.item).copied().unwrap_or(0));
                }
                let mut gather = BTreeMap::new();
                check_tree(&plan.tree, &ds, &mut gather);
                let from_tree: Vec<(String, u64)> = gather.into_iter().filter(|(_, n)| *n > 0).collect();
                assert_eq!(from_tree, plan.materials);
                assert_eq!(plan.steps.iter().map(|s| s.operations).sum::<u64>(), plan.operations);
                // dependency order: an input made by another step comes from an earlier one
                for (i, s) in plan.steps.iter().enumerate() {
                    for (input, _) in &s.inputs {
                        if let Some(j) = plan.steps.iter().position(|p| &p.item == input) {
                            assert!(j < i || plan.warnings.iter().any(|w| w.starts_with("Recipe cycle")));
                        }
                    }
                }
            }
            Err(e) => {
                failed += 1;
                assert!(e.0.starts_with("Calculation exceeds"), "{e}");
            }
        }
    }
    assert!(ok > 1000 && failed < ok, "{ok} ok, {failed} over the limit");
}
