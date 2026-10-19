// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Chunker's own allow-lists for the validation tests, ported verbatim from
//! `BedrockBlockIdentifierValidationTests`, `JavaBlockIdentifierValidationTests` and the item tests: blocks that are
//! lossy on purpose, blocks that are not supported on purpose, and the state normalisation (`removeLossyStates`) that
//! the round-trip comparison applies to both sides.

use crate::convert::mapping::{StateMap, StateValue};

fn int(v: i32) -> StateValue {
    StateValue::Int(v)
}

fn text(v: &str) -> StateValue {
    StateValue::str(v)
}

/// `Map.replace(key, old, new)`: only when the key currently holds `old`.
fn replace(m: &mut StateMap, key: &str, old: StateValue, new: StateValue) {
    if m.get(key) == Some(&old) {
        m.insert(key.into(), new);
    }
}

/// `Map.replace(key, new)`: only when the key is present.
fn replace_any(m: &mut StateMap, key: &str, new: StateValue) {
    if let Some(slot) = m.get_mut(key) {
        *slot = new;
    }
}

/// Bedrock `UNSUPPORTED_BLOCKS`: technical, old, education and element blocks Chunker deliberately does not map.
pub(crate) fn bedrock_unsupported_block(id: &str) -> bool {
    const EXACT: &[&str] = &[
        "minecraft:unknown",
        "minecraft:info_update",
        "minecraft:info_update2",
        "minecraft:reserved6",
        "minecraft:client_request_placeholder_block",
        "minecraft:glowingobsidian",
        "minecraft:netherreactor",
        "minecraft:stonecutter",
        "minecraft:mysterious_frame",
        "minecraft:mysterious_frame_slot",
        "minecraft:chalkboard",
        "minecraft:camera",
        "minecraft:allow",
        "minecraft:deny",
        "minecraft:border_block",
        "minecraft:chemical_heat",
        "minecraft:chemistry_table",
        "minecraft:lab_table",
        "minecraft:material_reducer",
        "minecraft:element_constructor",
        "minecraft:compound_creator",
        "minecraft:colored_torch_bp",
        "minecraft:colored_torch_rg",
        "minecraft:colored_torch_blue",
        "minecraft:colored_torch_green",
        "minecraft:colored_torch_purple",
        "minecraft:colored_torch_red",
        "minecraft:hard_glass",
        "minecraft:hard_glass_pane",
        "minecraft:hard_stained_glass",
        "minecraft:hard_stained_glass_pane",
        "minecraft:underwater_torch",
    ];
    if EXACT.contains(&id) {
        return true;
    }
    const COLORS: &[&str] = &[
        "black",
        "blue",
        "brown",
        "cyan",
        "gray",
        "green",
        "light_blue",
        "light_gray",
        "lime",
        "magenta",
        "orange",
        "pink",
        "purple",
        "red",
        "white",
        "yellow",
    ];
    for c in COLORS {
        if id == format!("minecraft:hard_{c}_stained_glass") || id == format!("minecraft:hard_{c}_stained_glass_pane") {
            return true;
        }
    }
    // minecraft:element_0 ..= minecraft:element_118
    id.strip_prefix("minecraft:element_").and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| n <= 118)
}

/// Bedrock `LOSSY_BLOCKS`: (id, partial states). An empty state list matches every state of the block.
pub(crate) fn bedrock_lossy_block(id: &str, states: &StateMap) -> bool {
    let rows: Vec<(&str, Vec<(&str, StateValue)>)> = vec![
        ("minecraft:cherry_wood", vec![("stripped_bit", StateValue::Bool(true))]),
        ("minecraft:mangrove_wood", vec![("stripped_bit", StateValue::Bool(true))]),
        ("minecraft:double_stone_block_slab", vec![("stone_slab_type", text("wood"))]),
        ("minecraft:stone_block_slab", vec![("stone_slab_type", text("wood"))]),
        ("minecraft:red_mushroom_block", vec![("huge_mushroom_bits", int(10))]),
        ("minecraft:red_mushroom_block", vec![("huge_mushroom_bits", int(15))]),
        ("minecraft:brown_mushroom_block", vec![("huge_mushroom_bits", int(10))]),
        ("minecraft:brown_mushroom_block", vec![("huge_mushroom_bits", int(15))]),
        ("minecraft:lava_cauldron", vec![]),
        ("minecraft:glow_frame", vec![]),
        ("minecraft:frame", vec![]),
        ("minecraft:jigsaw", vec![]),
    ];
    rows.iter().any(|(rid, want)| *rid == id && want.iter().all(|(k, v)| states.get(*k) == Some(v)))
}

/// Bedrock `UNSUPPORTED_ITEMS`.
pub(crate) fn bedrock_unsupported_item(id: &str) -> bool {
    const LIST: &[&str] = &[
        "minecraft:wool",
        "minecraft:carpet",
        "minecraft:fence",
        "minecraft:planks",
        "minecraft:log",
        "minecraft:log2",
        "minecraft:leaves",
        "minecraft:leaves2",
        "minecraft:wood",
        "minecraft:wooden_slab",
        "minecraft:concrete",
        "minecraft:concrete_powder",
        "minecraft:coral",
        "minecraft:hard_stained_glass",
        "minecraft:hard_stained_glass_pane",
        "minecraft:shulker_box",
        "minecraft:stained_glass",
        "minecraft:stained_glass_pane",
        "minecraft:stained_hardened_clay",
        "minecraft:coral_fan",
        "minecraft:coral_fan_dead",
        "minecraft:sapling",
        "minecraft:red_flower",
        "minecraft:coral_block",
        "minecraft:double_plant",
        "minecraft:stone_block_slab",
        "minecraft:tallgrass",
        "minecraft:double_stone_block_slab",
        "minecraft:stone_block_slab2",
        "minecraft:stone_block_slab3",
        "minecraft:stone_block_slab4",
        "minecraft:double_stone_block_slab2",
        "minecraft:double_stone_block_slab3",
        "minecraft:double_stone_block_slab4",
        "minecraft:light_block",
        "minecraft:monster_egg",
        "minecraft:stonebrick",
        "minecraft:colored_torch_bp",
        "minecraft:colored_torch_rg",
        "minecraft:chemistry_table",
        "minecraft:firefly_spawn_egg",
        "minecraft:board",
        "minecraft:agent_spawn_egg",
        "minecraft:npc_spawn_egg",
        "minecraft:glow_stick",
        "minecraft:balloon",
        "minecraft:bleach",
        "minecraft:camera",
        "minecraft:compound",
        "minecraft:ice_bomb",
        "minecraft:medicine",
        "minecraft:photo_item",
        "minecraft:portfolio",
        "minecraft:rapid_fertilizer",
        "minecraft:sparkler",
    ];
    LIST.contains(&id)
}

/// Bedrock `LOSSY_ITEMS`: legacy items that come back as their newer variant.
pub(crate) fn bedrock_lossy_item(id: &str) -> bool {
    matches!(
        id,
        "minecraft:dye" | "minecraft:banner_pattern" | "minecraft:boat" | "minecraft:chest_boat" | "minecraft:skull"
    )
}

/// Java `removeLossyStates`: the torchflower crop looks the same from age 1 on.
pub(crate) fn java_remove_lossy(id: &str, states: &StateMap) -> StateMap {
    let mut cloned = states.clone();
    if id == "minecraft:torchflower_crop" {
        let age = states.get("age").and_then(|v| v.short().parse::<i32>().ok()).unwrap_or(0);
        if age >= 2 {
            replace_any(&mut cloned, "age", text("1"));
        }
    }
    cloned
}

/// Bedrock `removeLossyStates`, returns the normalised (lower-case) id and states.
pub(crate) fn bedrock_remove_lossy(input_id: &str, input_states: &StateMap) -> (String, StateMap) {
    let mut id = input_id.to_string();
    let mut c = input_states.clone();
    let ends = |id: &str, s: &str| id.ends_with(s);
    let eq = |id: &str, s: &str| id == s;

    // Deprecated states
    if [
        "bone_block",
        "hay_block",
        "stripped_crimson_hyphae",
        "stripped_crimson_stem",
        "stripped_warped_hyphae",
        "stripped_warped_stem",
    ]
    .iter()
    .any(|n| id == format!("minecraft:{n}"))
    {
        c.remove("deprecated");
    }
    if id.contains("_pressure_plate") {
        c.remove("redstone_signal");
    }
    if eq(&id, "minecraft:creaking_heart") {
        c.remove("natural");
    }
    if id.starts_with("minecraft:suspicious_") {
        c.remove("hanging");
    }
    if eq(&id, "minecraft:flower_pot") {
        c.remove("update_bit");
    }
    if eq(&id, "minecraft:skull") {
        c.remove("no_drop_bit");
    }
    if eq(&id, "minecraft:bell") {
        c.remove("toggle_bit");
    }
    if eq(&id, "minecraft:snow_layer") {
        c.remove("covered_bit");
    }
    if eq(&id, "minecraft:chiseled_bookshelf") {
        c.remove("last_interacted_slot");
    }
    if ends(&id, "torch") {
        replace(&mut c, "torch_facing_direction", text("unknown"), text("top"));
    }

    // Unused states
    if eq(&id, "minecraft:pumpkin") {
        c.remove("minecraft:cardinal_direction");
        c.remove("direction");
    }
    if eq(&id, "minecraft:portal") {
        replace(&mut c, "portal_axis", text("unknown"), text("x"));
    }
    if id.contains("double") && id.contains("slab") {
        c.remove("minecraft:vertical_half");
        c.remove("top_slot_bit");
    }
    if eq(&id, "minecraft:bamboo_sapling") {
        c.remove("sapling_type");
    }
    if eq(&id, "minecraft:hopper") {
        for from in [1, 6, 7] {
            replace(&mut c, "facing_direction", int(from), int(0));
        }
    }
    if id.contains("_glazed_terracotta")
        || id.contains("ladder")
        || id.contains("wall_")
        || eq(&id, "minecraft:skull")
        || ends(&id, "_skull")
        || ends(&id, "_head")
        || ends(&id, "_stem")
        || ends(&id, "chest")
        || ends(&id, "furnace")
        || ends(&id, "smoker")
        || ends(&id, "stonecutter_block")
        || ends(&id, "_hanging_sign")
        || eq(&id, "minecraft:bee_nest")
        || eq(&id, "minecraft:beehive")
    {
        for from in [0, 1, 6, 7] {
            replace(&mut c, "facing_direction", int(from), int(2));
        }
    }
    if ends(&id, "_rod") || ends(&id, "piston") || ends(&id, "pistonArmCollision") {
        for from in [6, 7] {
            replace(&mut c, "facing_direction", int(from), int(0));
        }
    }
    if eq(&id, "minecraft:dispenser")
        || eq(&id, "minecraft:barrel")
        || ends(&id, "command_block")
        || eq(&id, "minecraft:dropper")
        || eq(&id, "minecraft:observer")
    {
        for from in [6, 7] {
            replace(&mut c, "facing_direction", int(from), int(0));
        }
    }
    if id.contains("_hanging_sign") {
        if input_states.get("hanging") == Some(&StateValue::Bool(false)) {
            c.remove("ground_sign_direction");
        } else {
            c.remove("facing_direction");
        }
    }
    if ends(&id, "_button") {
        for from in [6, 7] {
            replace(&mut c, "facing_direction", int(from), int(0));
        }
    }
    if ends(&id, "_log")
        || eq(&id, "minecraft:log")
        || eq(&id, "minecraft:log2")
        || eq(&id, "minecraft:purpur_block")
        || eq(&id, "minecraft:quartz_block")
        || eq(&id, "minecraft:hay_block")
        || eq(&id, "minecraft:bone_block")
    {
        replace(&mut c, "direction", int(3), int(0));
    }
    if eq(&id, "minecraft:detector_rail") || eq(&id, "minecraft:activator_rail") || eq(&id, "minecraft:golden_rail") {
        for from in [6, 7] {
            replace(&mut c, "rail_direction", int(from), int(0));
        }
    }
    if eq(&id, "minecraft:rail") {
        for from in 10..=15 {
            replace(&mut c, "rail_direction", int(from), int(0));
        }
    }
    if eq(&id, "minecraft:chorus_flower") {
        for from in [6, 7] {
            replace(&mut c, "age", int(from), int(5));
        }
    }
    if eq(&id, "minecraft:cauldron") {
        replace(&mut c, "fill_level", int(7), int(6));
    }
    if eq(&id, "minecraft:composter") {
        for from in 9..=15 {
            replace(&mut c, "composter_fill_level", int(from), int(8));
        }
    }
    if eq(&id, "minecraft:cake") {
        replace(&mut c, "bite_counter", int(7), int(6));
    }
    if eq(&id, "minecraft:cocoa") {
        replace(&mut c, "age", int(3), int(2));
    }

    // Same texture replacements
    if id.contains("minecraft:stonebrick") {
        replace(&mut c, "stone_brick_type", text("smooth"), text("default"));
        if c.values().any(|v| *v == text("default")) {
            c.remove("minecraft:pillar_axis");
        }
    }
    if eq(&id, "minecraft:purpur_block") {
        if c.contains_key("chisel_type") {
            replace(&mut c, "chisel_type", text("smooth"), text("default"));
            replace(&mut c, "chisel_type", text("chiseled"), text("default"));
        }
        if c.get("chisel_type") != Some(&text("lines")) {
            c.remove("pillar_axis");
            c.remove("direction");
        }
    }
    if eq(&id, "minecraft:quartz_block")
        || eq(&id, "minecraft:smooth_quartz")
        || eq(&id, "minecraft:chiseled_quartz_block")
    {
        if c.get("chisel_type") != Some(&text("lines")) {
            c.remove("pillar_axis");
            c.remove("direction");
        }
    }
    if id.contains("minecraft:deprecated_purpur_block_") {
        id = "minecraft:purpur_block".to_string();
        c.remove("pillar_axis");
        c.remove("direction");
    }
    if id.contains("minecraft:deprecated_anvil") {
        id = "minecraft:damaged_anvil".to_string();
    }
    if id.contains("minecraft:anvil") {
        replace(&mut c, "damage", text("broken"), text("very_damaged"));
    }
    if id.contains("minecraft:cauldron") && input_states.get("fill_level") == Some(&int(0)) {
        replace_any(&mut c, "cauldron_liquid", text("water"));
    }
    if id.contains("minecraft:mangrove_propagule") {
        c.remove("facing_direction");
        if matches!(input_states.get("growth"), Some(StateValue::Int(g)) if *g >= 5) {
            replace_any(&mut c, "growth", int(4));
        }
    }
    if id.contains("minecraft:coral_fan_hang3") {
        replace_any(&mut c, "coral_hang_type_bit", StateValue::Bool(false));
    }
    if (eq(&id, "minecraft:melon_stem") || eq(&id, "minecraft:pumpkin_stem"))
        && input_states.get("growth").is_some_and(|g| *g != int(7))
    {
        replace_any(&mut c, "facing_direction", int(2));
    }
    if eq(&id, "minecraft:big_dripleaf") && input_states.get("big_dripleaf_head") == Some(&StateValue::Bool(false)) {
        replace_any(&mut c, "big_dripleaf_tilt", text("none"));
    }
    if ends(&id, "_comparator") {
        c.remove("output_lit_bit");
    }
    if eq(&id, "minecraft:torchflower_crop")
        && matches!(input_states.get("growth"), Some(StateValue::Int(g)) if *g >= 1)
    {
        replace_any(&mut c, "growth", int(7));
    }
    if (eq(&id, "minecraft:pink_petals") || eq(&id, "minecraft:wildflowers") || eq(&id, "minecraft:leaf_litter"))
        && matches!(input_states.get("growth"), Some(StateValue::Int(g)) if *g >= 4)
    {
        replace_any(&mut c, "growth", int(3));
    }
    if eq(&id, "minecraft:pitcher_crop") && matches!(input_states.get("growth"), Some(StateValue::Int(g)) if *g >= 4) {
        replace_any(&mut c, "growth", int(3));
    }
    if ends(&id, "_mushroom_block") {
        for from in [11, 12, 13] {
            replace(&mut c, "huge_mushroom_bits", int(from), int(0));
        }
    }
    if eq(&id, "minecraft:mushroom_stem") {
        if let Some(StateValue::Int(bits)) = input_states.get("huge_mushroom_bits") {
            if *bits != 10 && *bits != 15 {
                c.insert("huge_mushroom_bits".into(), int(15));
            }
        }
    }
    if eq(&id, "minecraft:tallgrass") {
        replace(&mut c, "tall_grass_type", text("default"), text("tall"));
        replace(&mut c, "tall_grass_type", text("snow"), text("fern"));
    }
    (id.to_lowercase(), c)
}
