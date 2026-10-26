use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh empty base folder under the system temp dir (which honours TMPDIR).
fn temp_dirs(tag: &str) -> Dirs {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("blockforge-craft-saved-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    Dirs::at(base)
}

fn names(dirs: &Dirs) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(craft_dir(dirs))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn request() -> Request {
    Request {
        item: "minecraft:glass".into(),
        quantity: 2500,
        inventory: BTreeMap::from([("minecraft:sand".to_string(), 64)]),
        recipe_choices: BTreeMap::from([("minecraft:glass".to_string(), "glass".to_string())]),
        ingredient_choices: BTreeMap::from([("stick:#0".to_string(), "minecraft:bamboo".to_string())]),
        supplied: ["minecraft:sand".to_string()].into(),
    }
}

fn plan(name: &str) -> SavedPlan {
    SavedPlan {
        id: 0,
        name: name.into(),
        version: "1.21.1".into(),
        request: request(),
        done: ["gather:minecraft:sand".to_string(), "step:glass".to_string()].into(),
        created_ms: 1_700_000_000_000,
        updated_ms: 1_700_000_000_500,
    }
}

#[test]
fn save_and_load_round_trip() {
    let dirs = temp_dirs("round");
    let mut store = Store::default();
    let a = store.add(plan("Glass farm"));
    let b = store.add(plan("Glass farm"));
    store.set_stock("1.21.1", "minecraft:coal", 128);
    save(&dirs, &store).unwrap();
    let back = load(&dirs);
    assert_eq!(back, store);
    assert_ne!(a, b);
    assert_eq!(back.get(b).unwrap().name, "Glass farm (2)");
    // nothing was set aside
    assert_eq!(names(&dirs), ["plans.json", "stock.json"]);
}

#[test]
fn missing_files_give_an_empty_store() {
    let dirs = temp_dirs("missing");
    assert_eq!(load(&dirs), Store::default());
    assert!(!craft_dir(&dirs).exists());
}

#[test]
fn corrupt_file_is_renamed_not_lost() {
    let dirs = temp_dirs("corrupt");
    let dir = craft_dir(&dirs);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plans.json"), b"{ not json").unwrap();
    let mut store = Store::default();
    store.set_stock("1.21.1", "minecraft:coal", 5);
    std::fs::write(
        dir.join("stock.json"),
        serde_json::to_vec(&StockFile { schema: 1, stock: store.stock.clone() }).unwrap(),
    )
    .unwrap();
    let back = load(&dirs);
    assert!(back.plans.is_empty());
    // the good stock file still loads
    assert_eq!(back.stock_for("1.21.1").get("minecraft:coal"), Some(&5));
    let files = names(&dirs);
    assert_eq!(files.len(), 2, "{files:?}");
    let bad = files.iter().find(|n| n.starts_with("plans.json.bad-")).expect("renamed");
    assert_eq!(std::fs::read(dir.join(bad)).unwrap(), b"{ not json");
    // a save afterwards starts fresh and keeps the bad file
    save(&dirs, &back).unwrap();
    assert!(names(&dirs).iter().any(|n| n == bad));
}

#[test]
fn future_schema_is_kept_untouched() {
    let dirs = temp_dirs("future");
    let dir = craft_dir(&dirs);
    std::fs::create_dir_all(&dir).unwrap();
    let text = r#"{"schema":2,"plans":[],"extra":"keep me"}"#;
    std::fs::write(dir.join("plans.json"), text).unwrap();
    assert_eq!(load(&dirs), Store::default());
    assert!(!dir.join("plans.json").exists());
    let bad = names(&dirs).into_iter().find(|n| n.starts_with("plans.json.bad-")).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(bad)).unwrap(), text);
}

#[test]
fn invalid_entries_are_dropped_and_a_copy_kept() {
    let dirs = temp_dirs("entries");
    let dir = craft_dir(&dirs);
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::default();
    store.add(plan("Good"));
    save(&dirs, &store).unwrap();
    let mut file: Value = serde_json::from_slice(&std::fs::read(dir.join("plans.json")).unwrap()).unwrap();
    file["plans"].as_array_mut().unwrap().push(json!({"id": 5, "name": "Bad", "version": "1.21.1",
        "request": {"item": "Not An Id", "quantity": 3}}));
    file["plans"].as_array_mut().unwrap().push(json!("junk"));
    std::fs::write(dir.join("plans.json"), serde_json::to_vec(&file).unwrap()).unwrap();
    let back = load(&dirs);
    assert_eq!(back.plans.len(), 1);
    assert_eq!(back.plans[0].name, "Good");
    assert!(dir.join("plans.json").exists());
    assert!(names(&dirs).iter().any(|n| n.starts_with("plans.json.bad-")));
}

#[test]
fn atomic_save_leaves_no_temp_files() {
    let dirs = temp_dirs("atomic");
    let mut store = Store::default();
    store.add(plan("One"));
    for _ in 0..3 {
        save(&dirs, &store).unwrap();
    }
    assert_eq!(names(&dirs), ["plans.json", "stock.json"]);
}

#[test]
fn unique_names() {
    let mut store = Store::default();
    assert_eq!(unique_name(&store, "  Base   walls "), "Base walls");
    assert_eq!(unique_name(&store, "   "), "Untitled plan");
    store.add(plan("Base walls"));
    assert_eq!(unique_name(&store, "base walls"), "base walls (2)");
    store.add(plan("Base walls"));
    store.add(plan("Base walls"));
    let names: Vec<_> = store.plans.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Base walls", "Base walls (2)", "Base walls (3)"]);
    // a numbered name is numbered again from its stem
    assert_eq!(unique_name(&store, "Base walls (2)"), "Base walls (4)");
    let long = "x".repeat(300);
    assert_eq!(unique_name(&store, &long).chars().count(), MAX_NAME);
}

#[test]
fn ids_are_unique_even_for_equal_timestamps() {
    let mut store = Store::default();
    let ids: HashSet<u64> = (0..20).map(|i| store.add(plan(&format!("p{i}")))).collect();
    assert_eq!(ids.len(), 20);
}

#[test]
fn rename_duplicate_delete() {
    let mut store = Store::default();
    let a = store.add(plan("A"));
    let b = store.add(plan("B"));
    // renaming to its own name keeps it, to another plan's name numbers it
    assert_eq!(store.rename(a, " A ", 5).unwrap(), "A");
    assert_eq!(store.rename(b, "a", 6).unwrap(), "a (2)");
    assert_eq!(store.get(b).unwrap().updated_ms, 6);
    assert!(store.rename(999, "x", 1).is_none());
    let c = store.duplicate(a, 77).unwrap();
    let copy = store.get(c).unwrap();
    assert_eq!(copy.name, "A (3)");
    assert!(copy.done.is_empty());
    assert_eq!(copy.request, store.get(a).unwrap().request);
    assert_eq!((copy.created_ms, copy.updated_ms), (77, 77));
    assert!(store.delete(a));
    assert!(!store.delete(a));
    assert!(store.get(a).is_none());
}

#[test]
fn stock_add_and_remove() {
    let mut store = Store::default();
    assert!(store.stock_for("1.21.1").is_empty());
    store.set_stock("1.21.1", "minecraft:coal", 10);
    store.set_stock("1.21.1", "minecraft:iron_ingot", 3);
    store.set_stock("1.20.4", "minecraft:coal", 1);
    assert_eq!(store.stock_for("1.21.1").len(), 2);
    store.set_stock("1.21.1", "minecraft:coal", 0);
    assert_eq!(store.stock_for("1.21.1").len(), 1);
    store.set_stock("1.21.1", "minecraft:iron_ingot", 0);
    assert!(!store.stock.contains_key("1.21.1"));
    store.set_stock("1.20.4", "minecraft:coal", u64::MAX);
    assert_eq!(store.stock_for("1.20.4")["minecraft:coal"], MAX_QUANTITY);
    store.set_stock("nothing", "minecraft:coal", 0);
    assert!(!store.stock.contains_key("nothing"));
}

#[test]
fn plan_file_round_trips() {
    let original = plan("Glass farm");
    let text = export_plan(&original);
    assert!(text.contains("\"blockforgeCraftPlan\": 1"));
    let back = import_plan(&text, 42).unwrap();
    let expected = SavedPlan { id: 0, created_ms: 42, updated_ms: 42, ..original.clone() };
    assert_eq!(back, expected);
    // and again from the imported one
    assert_eq!(import_plan(&export_plan(&back), 42).unwrap(), back);
    // the store gives it a real id
    let mut store = Store::default();
    let id = store.add(back);
    assert_ne!(id, 0);
}

#[test]
fn import_rejects_junk() {
    let ok = export_plan(&plan("P"));
    assert!(import_plan(&ok, 1).is_ok());
    let edit = |f: &dyn Fn(&mut Value)| {
        let mut v: Value = serde_json::from_str(&ok).unwrap();
        f(&mut v);
        serde_json::to_string(&v).unwrap()
    };
    assert!(import_plan("", 1).is_err());
    assert!(import_plan("not json", 1).is_err());
    assert!(import_plan("{}", 1).unwrap_err().contains("not a BlockForge plan"));
    assert!(import_plan(&edit(&|v| v["blockforgeCraftPlan"] = json!(2)), 1).unwrap_err().contains("newer"));
    assert!(import_plan(&edit(&|v| v["blockforgeCraftPlan"] = json!("1")), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["item"] = json!("Oak Planks")), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["item"] = json!("minecraft:Glass")), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["item"] = json!("glass")), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["quantity"] = json!(0)), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["quantity"] = json!(MAX_QUANTITY + 1)), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["quantity"] = json!(-4)), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["inventory"]["minecraft:sand"] = json!(0)), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["inventory"]["bad id"] = json!(1)), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["supplied"] = json!(["???"])), 1).is_err());
    assert!(import_plan(&edit(&|v| v["request"]["ingredientChoices"]["a"] = json!("nope")), 1).is_err());
    assert!(import_plan(&edit(&|v| v["version"] = json!("")), 1).is_err());
    assert!(import_plan(&edit(&|v| v["version"] = json!("../../x")), 1).is_err());
    assert!(import_plan(&edit(&|v| v["done"] = json!([""])), 1).is_err());
    // a nameless plan gets a name, a long one is cut
    assert_eq!(import_plan(&edit(&|v| v["name"] = json!("  ")), 1).unwrap().name, "Untitled plan");
    let huge = format!("{{\"blockforgeCraftPlan\":1,\"name\":\"{}\"}}", "a".repeat(MAX_PLAN_FILE));
    assert!(import_plan(&huge, 1).unwrap_err().contains("too big"));
}

#[test]
fn number_helpers() {
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(999), "999");
    assert_eq!(thousands(1000), "1,000");
    assert_eq!(thousands(2_500), "2,500");
    assert_eq!(thousands(1_000_000_000), "1,000,000,000");
    assert_eq!(stacks_text(0, 64), "0");
    assert_eq!(stacks_text(5, 64), "5");
    assert_eq!(stacks_text(64, 64), "1 stack");
    assert_eq!(stacks_text(76, 64), "1 stack + 12");
    assert_eq!(stacks_text(140, 64), "2 stacks + 12");
    assert_eq!(stacks_text(2500, 64), "39 stacks + 4");
    assert_eq!(stacks_text(32, 16), "2 stacks");
    assert_eq!(stacks_text(5, 1), "5");
    assert_eq!(stacks_text(5, 0), "5");
    assert_eq!(stacks_text(1_000_000, 64), "15,625 stacks");
}

// ---------------------------------------------------------------------------------------------------------
// Text and JSON on a hand-built plan

fn item(id: &str, name: &str, stack: u32) -> (ItemId, Item) {
    (id.to_string(), Item { id: id.to_string(), name: name.to_string(), stack, category: Category::Misc })
}

fn dataset() -> Dataset {
    let items = BTreeMap::from([
        item("minecraft:sand", "Sand", 64),
        item("minecraft:glass", "Glass", 64),
        item("minecraft:coal", "Coal", 64),
        item("minecraft:glass_bottle", "Glass Bottle", 64),
        item("minecraft:ender_pearl", "Ender Pearl", 16),
    ]);
    Dataset::new("1.21.1", items, Vec::new(), HashSet::new(), Vec::new()).unwrap()
}

fn leaf(item: &str, required: u64) -> Node {
    Node {
        item: item.into(),
        required,
        from_stock: 0,
        from_surplus: 0,
        to_produce: required,
        output: required,
        surplus: 0,
        operations: 0,
        recipe: None,
        default_recipe: true,
        stop: Some(Stop::Raw),
        children: Vec::new(),
    }
}

fn built_plan() -> Plan {
    let mut tree = leaf("minecraft:glass", 2500);
    tree.stop = None;
    tree.operations = 2500;
    tree.recipe = Some("glass".into());
    tree.children = vec![leaf("minecraft:sand", 2500)];
    Plan {
        request: Request { item: "minecraft:glass".into(), quantity: 2500, ..Request::default() },
        tree,
        ledger: vec![
            LedgerRow {
                item: "minecraft:glass".into(),
                required: 2500,
                from_stock: 0,
                from_surplus: 0,
                produced: 2500,
                gather: 0,
                surplus: 0,
            },
            LedgerRow {
                item: "minecraft:sand".into(),
                required: 2500,
                from_stock: 0,
                from_surplus: 0,
                produced: 0,
                gather: 2500,
                surplus: 0,
            },
        ],
        materials: vec![("minecraft:ender_pearl".into(), 20), ("minecraft:sand".into(), 2500)],
        intermediates: Vec::new(),
        steps: vec![Step {
            recipe: "glass".into(),
            item: "minecraft:glass".into(),
            kind: Kind::Smelting,
            operations: 2500,
            output: 2500,
            inputs: vec![("minecraft:sand".into(), 2500)],
            choices: Vec::new(),
            byproducts: Vec::new(),
            ticks: Some(500_000),
        }],
        operations: 2500,
        cook_ticks: 500_000,
        fuel: vec![FuelEstimate { item: "minecraft:coal".into(), count: 313 }],
        warnings: vec!["Sand is slow to gather by hand.".into()],
        complete: true,
    }
}

fn built_storage() -> Storage {
    let c = |count, per: u64| Containers { count, capacity: count * per, unused: 0 };
    Storage {
        rows: vec![StorageRow {
            item: "minecraft:sand".into(),
            quantity: 2500,
            stack: 64,
            full_stacks: 39,
            partial: 4,
            slots: 40,
        }],
        slots: 41,
        single_chests: c(2, 27),
        double_chests: c(1, 54),
        shulker_boxes: c(2, 27),
        player_inventories: c(2, 36),
    }
}

#[test]
fn text_golden() {
    let got = text_with_storage(&dataset(), &built_plan(), &built_storage());
    let want = "\
2,500 x Glass (Minecraft 1.21.1)

Gather
- Ender Pearl x 20 (1 stack + 4)
- Sand x 2,500 (39 stacks + 4)

Craft/Smelt in order
1. Smelt 2,500 times (Furnace): 2,500 Sand -> 2,500 Glass

Fuel: about 313 Coal if smelted in one furnace

Warnings
- Sand is slow to gather by hand.

Storage (41 slots)
- Single chests: 2
- Double chests: 1
- Shulker boxes: 2
- Player inventories: 2

Made with BlockForge
";
    assert_eq!(got, want);
}

#[test]
fn text_edge_cases() {
    let ds = dataset();
    let mut plan = built_plan();
    plan.steps[0].operations = 1;
    plan.steps[0].byproducts = vec![("minecraft:glass_bottle".into(), 3)];
    plan.steps.push(Step { byproducts: vec![("minecraft:glass_bottle".into(), 2)], ..plan.steps[0].clone() });
    plan.cook_ticks = 0;
    plan.warnings.clear();
    plan.complete = false;
    let none = Storage { slots: 0, ..built_storage() };
    let text = text_with_storage(&ds, &plan, &none);
    assert!(text.contains("1. Smelt 1 time (Furnace)"));
    assert!(text.contains("By-products\n- Glass Bottle x 5\n"));
    assert!(!text.contains("Fuel:"));
    assert!(text.contains("This list is not complete"));
    assert!(!text.contains("Storage"));
    plan.materials.clear();
    assert!(text_with_storage(&ds, &plan, &none).contains("Gather\n- Nothing"));
}

#[test]
fn clipboard_is_the_gather_list() {
    assert_eq!(clipboard_list(&dataset(), &built_plan()), "Ender Pearl x 20\nSand x 2500\n");
}

#[test]
fn json_has_everything() {
    let text = json_with_storage(&dataset(), &built_plan(), &built_storage());
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["version"], "1.21.1");
    assert_eq!(v["request"]["quantity"], 2500);
    assert_eq!(v["materials"][1]["item"], "minecraft:sand");
    assert_eq!(v["materials"][1]["name"], "Sand");
    assert_eq!(v["steps"][0]["station"], "Furnace");
    assert_eq!(v["steps"][0]["inputs"][0]["quantity"], 2500);
    assert_eq!(v["ledger"][1]["gather"], 2500);
    assert_eq!(v["storage"]["slots"], 41);
    assert_eq!(v["storage"]["doubleChests"]["count"], 1);
    assert_eq!(v["fuel"][0]["count"], 313);
    // the public wrappers work with whatever the storage helper returns today
    let plan = built_plan();
    assert!(!to_text(&dataset(), &plan).is_empty());
    assert!(serde_json::from_str::<Value>(&to_json(&dataset(), &plan)).is_ok());
}

#[test]
fn a_bad_entry_is_copied_once_not_on_every_start() {
    let dirs = temp_dirs("copy-once");
    let dir = craft_dir(&dirs);
    let mut store = Store::default();
    store.add(plan("Good"));
    save(&dirs, &store).unwrap();
    let mut file: Value = serde_json::from_slice(&std::fs::read(dir.join("plans.json")).unwrap()).unwrap();
    file["plans"].as_array_mut().unwrap().push(json!("junk"));
    std::fs::write(dir.join("plans.json"), serde_json::to_vec(&file).unwrap()).unwrap();
    for _ in 0..4 {
        let back = load(&dirs);
        assert_eq!(back.plans.len(), 1);
    }
    let copies = names(&dirs).iter().filter(|n| n.starts_with("plans.json.bad-")).count();
    assert_eq!(copies, 1, "{:?}", names(&dirs));
}

#[test]
fn saves_from_two_threads_at_once_never_corrupt_the_store() {
    let dirs = temp_dirs("two-savers");
    std::thread::scope(|s| {
        for t in 0..8 {
            let dirs = &dirs;
            s.spawn(move || {
                for i in 0..10 {
                    let mut store = Store::default();
                    store.add(plan(&format!("t{t}-{i}")));
                    save(dirs, &store).unwrap();
                }
            });
        }
    });
    let back = load(&dirs);
    assert_eq!(back.plans.len(), 1, "one whole save won");
    assert!(names(&dirs).iter().all(|n| !n.contains(".tmp") && !n.contains(".bad-")), "{:?}", names(&dirs));
}
