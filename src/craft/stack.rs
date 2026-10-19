//! Stack sizes and what storing items takes.

use std::collections::BTreeMap;

use super::types::*;

/// Names that stack to 1 whatever they end with.
const SINGLE: &[&str] = &[
    "shulker_box",
    "bundle",
    "cake",
    "bow",
    "crossbow",
    "brush",
    "carrot_on_a_stick",
    "warped_fungus_on_a_stick",
    "debug_stick",
    "elytra",
    "enchanted_book",
    "fishing_rod",
    "flint_and_steel",
    "goat_horn",
    "knowledge_book",
    "mace",
    "saddle",
    "shears",
    "shield",
    "spyglass",
    "totem_of_undying",
    "trident",
    "writable_book",
    "potion",
    "splash_potion",
    "lingering_potion",
    "minecart",
    "beetroot_soup",
    "rabbit_stew",
    "mushroom_stew",
    "suspicious_stew",
    "turtle_helmet",
    "wolf_armor",
];
const SINGLE_SUFFIX: &[&str] = &[
    "_sword",
    "_pickaxe",
    "_axe",
    "_shovel",
    "_hoe",
    "_spear",
    "_helmet",
    "_chestplate",
    "_leggings",
    "_boots",
    "_horse_armor",
    "_wolf_armor",
    "_nautilus_armor",
    "_boat",
    "_raft",
    "_chest_boat",
    "_chest_raft",
    "_bed",
    "_shulker_box",
    "_minecart",
    "_banner_pattern",
    "_bundle",
    "_potion",
    "_stew",
    "_soup",
];
/// Names that stack to 16.
const SIXTEEN: &[&str] = &[
    "bucket",
    "armor_stand",
    "egg",
    "blue_egg",
    "brown_egg",
    "ender_pearl",
    "honey_bottle",
    "snowball",
    "written_book",
];
const SIXTEEN_SUFFIX: &[&str] = &["_sign", "_hanging_sign", "_banner"];

/// How many of `id` fit in one slot. The game keeps this in code, not in its data files, so it is worked out from
/// the name (tools, armor, boats, beds, ... stack to 1; signs, banners, buckets, eggs, ender pearls to 16).
pub fn stack_size(id: &str) -> u32 {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    // a filled bucket does not stack; the empty one does, to 16
    if name == "bucket" {
        return 16;
    }
    if name.ends_with("_bucket")
        || name.starts_with("music_disc_")
        || SINGLE.contains(&name)
        || SINGLE_SUFFIX.iter().any(|s| name.ends_with(s))
    {
        return 1;
    }
    if SIXTEEN.contains(&name) || SIXTEEN_SUFFIX.iter().any(|s| name.ends_with(s)) {
        return 16;
    }
    64
}

/// How many containers of `per` slots `slots` need.
pub fn containers(slots: u64, per: u64) -> Containers {
    let count = slots.div_ceil(per);
    Containers { count, capacity: count * per, unused: count * per - slots }
}

/// What storing `rows` takes, each item in its own stacks. The same item listed twice is added up before it is
/// rounded to stacks, different items never share one.
pub fn storage(ds: &Dataset, rows: &[(ItemId, u64)]) -> Storage {
    let mut merged: BTreeMap<&str, u64> = BTreeMap::new();
    for (id, quantity) in rows {
        let total = merged.entry(id.as_str()).or_insert(0);
        *total = total.saturating_add(*quantity);
    }
    let mut out = Vec::new();
    let mut slots = 0u64;
    for (id, quantity) in merged {
        if quantity == 0 {
            continue;
        }
        let stack = ds.item(id).map(|i| i.stack).unwrap_or_else(|| stack_size(id)).max(1);
        let full_stacks = quantity / stack as u64;
        let partial = (quantity % stack as u64) as u32;
        let row_slots = full_stacks + u64::from(partial > 0);
        slots = slots.saturating_add(row_slots);
        out.push(StorageRow { item: id.to_string(), quantity, stack, full_stacks, partial, slots: row_slots });
    }
    Storage {
        rows: out,
        slots,
        single_chests: containers(slots, 27),
        double_chests: containers(slots, 54),
        shulker_boxes: containers(slots, 27),
        player_inventories: containers(slots, 36),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};

    fn ds_with(items: &[(&str, u32)]) -> Dataset {
        let map: BTreeMap<ItemId, Item> = items
            .iter()
            .map(|(id, stack)| {
                let id = format!("minecraft:{id}");
                (id.clone(), Item { name: pretty_name(&id), id, stack: *stack, category: Category::Misc })
            })
            .collect();
        Dataset::new("test", map, Vec::new(), HashSet::new(), Vec::new()).unwrap()
    }

    #[test]
    fn rules() {
        for (id, n) in [
            ("minecraft:stone", 64),
            ("minecraft:diamond_pickaxe", 1),
            ("minecraft:bucket", 16),
            ("minecraft:water_bucket", 1),
            ("minecraft:oak_sign", 16),
            ("minecraft:oak_hanging_sign", 16),
            ("minecraft:white_banner", 16),
            ("minecraft:creeper_banner_pattern", 1),
            ("minecraft:music_disc_13", 1),
            ("minecraft:ender_pearl", 16),
            ("minecraft:red_bundle", 1),
            ("minecraft:oak_chest_boat", 1),
            ("minecraft:cake", 1),
            ("minecraft:pumpkin_pie", 64),
            ("minecraft:sandstone", 64),
            ("oak_planks", 64),
        ] {
            assert_eq!(stack_size(id), n, "{id}");
        }
    }

    #[test]
    fn storage_merges_the_same_item_only() {
        let ds = ds_with(&[("cookie", 64), ("stone", 64), ("bucket", 16)]);
        let rows = vec![
            ("minecraft:cookie".to_string(), 10_000),
            ("minecraft:stone".to_string(), 30),
            ("minecraft:stone".to_string(), 40),
            ("minecraft:bucket".to_string(), 0),
        ];
        let s = storage(&ds, &rows);
        assert_eq!(s.rows.len(), 2);
        let cookie = &s.rows[0];
        assert_eq!((cookie.full_stacks, cookie.partial, cookie.slots), (156, 16, 157));
        let stone = &s.rows[1];
        assert_eq!((stone.quantity, stone.full_stacks, stone.partial, stone.slots), (70, 1, 6, 2));
        assert_eq!(s.slots, 159);
        assert_eq!(s.single_chests.count, 6);
        assert_eq!(s.double_chests.count, 3);
        assert_eq!(s.double_chests.unused, 3);
        assert_eq!(s.player_inventories.count, 5);
        assert_eq!(storage(&ds, &[]).slots, 0);
    }

    /// Every item of the two reference datasets (when they are on this computer) has the stack size the rules give.
    #[test]
    fn rules_match_the_reference_data() {
        let dir = std::env::var("BF_P1_DATA").unwrap_or_else(|_| "/home/riyadm/Downloads/p1/public/data/".into());
        // Items where the reference data and the rules differ on purpose. Empty when the rules are exact.
        let exceptions: &[&str] = &[];
        let mut checked = 0;
        for file in ["1.21.1.json", "1.20.4.json"] {
            let Ok(text) = std::fs::read_to_string(std::path::Path::new(&dir).join(file)) else { continue };
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let mut wrong = Vec::new();
            for item in value["items"].as_array().unwrap() {
                let id = item["id"].as_str().unwrap();
                let want = item["stackSize"].as_u64().unwrap() as u32;
                checked += 1;
                if stack_size(id) != want && !exceptions.contains(&id) {
                    wrong.push(format!("{id}: want {want}, got {}", stack_size(id)));
                }
            }
            assert!(wrong.is_empty(), "{file}: {wrong:?}");
        }
        eprintln!("checked {checked} items");
    }
}
