//! The fields of each type of Bedrock actor (the tables of the entity mapping spec).

use super::EntityContext;
use super::common::{attribute, block_of, block_pos_compound, group, put_identity, put_owner, put_position};
use super::mob::{ALWAYS_DROPS, Equipment, Slot, bedrock_equipment, is_horse, write_attributes, write_mob};
use crate::convert::bedrock::resolvers::entity::{Kind, is_boat, kind};
use crate::convert::model::{Entity, EntityType, VanillaEntityType as V};
use crate::convert::nbt::{Compound, Tag};

/// Cat variants by Bedrock `Variant`.
const CATS: [&str; 11] = [
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

/// The `CatType` number of a cat variant (before 1.19).
fn cat_type(name: &str) -> i32 {
    const ORDER: [&str; 11] = [
        "tabby",
        "black",
        "red",
        "siamese",
        "british_shorthair",
        "calico",
        "persian",
        "ragdoll",
        "white",
        "jellie",
        "all_black",
    ];
    ORDER.iter().position(|n| *n == name).unwrap_or(0) as i32
}

/// Villager professions by Bedrock `Variant`.
const PROFESSIONS: [&str; 15] = [
    "none",
    "farmer",
    "fisherman",
    "shepherd",
    "fletcher",
    "librarian",
    "cartographer",
    "cleric",
    "armorer",
    "weaponsmith",
    "toolsmith",
    "butcher",
    "leatherworker",
    "mason",
    "nitwit",
];

/// Villager types by Bedrock `MarkVariant`.
const VILLAGER_TYPES: [&str; 7] = ["plains", "desert", "jungle", "savanna", "snow", "swamp", "taiga"];

/// Panda genes, as Bedrock names the phenotype groups.
const PANDA_GENES: [&str; 6] = ["lazy", "worried", "playful", "aggressive", "weak", "brown"];

/// Boat wood names by the boat type (`Type` before 1.21.2).
fn boat_wood(ty: V) -> &'static str {
    let name = ty.name();
    let wood = name.trim_end_matches("_CHEST_BOAT").trim_end_matches("_BOAT").trim_end_matches("_CHEST_RAFT");
    match wood.trim_end_matches("_RAFT") {
        "SPRUCE" => "spruce",
        "BIRCH" => "birch",
        "JUNGLE" => "jungle",
        "ACACIA" => "acacia",
        "DARK_OAK" => "dark_oak",
        "MANGROVE" => "mangrove",
        "BAMBOO" => "bamboo",
        "CHERRY" => "cherry",
        "PALE_OAK" => "pale_oak",
        "POPLAR" => "poplar",
        _ => "oak",
    }
}

fn is_chest_boat(ty: V) -> bool {
    ty.name().contains("CHEST")
}

/// The id of a type for this version; boats before 1.21.2 are `boat` / `chest_boat` with a `Type`.
fn type_id(ctx: &EntityContext<'_>, ty: V) -> Option<String> {
    if is_boat(ty) && !ctx.since(1, 21, 2) {
        let chest = is_chest_boat(ty) && ctx.since(1, 19, 0);
        let base = if chest { V::OAK_CHEST_BOAT } else { V::OAK_BOAT };
        return ctx.entity_id(&EntityType::Vanilla(base));
    }
    ctx.entity_id(&EntityType::Vanilla(ty))
}

/// The items of a Bedrock container list (`ChestItems`, `Items`) as Java `Items` entries with their `Slot`, for the
/// slots `range`, moved by `offset`.
fn slotted_items(
    ctx: &EntityContext<'_>,
    source: &Compound,
    key: &str,
    range: std::ops::RangeFrom<i32>,
    offset: i32,
) -> Vec<Tag> {
    let Some(list) = source.get_list(key) else { return Vec::new() };
    let mut out = Vec::new();
    for (i, nbt) in list.iter().filter_map(Tag::as_compound).enumerate() {
        let slot = nbt.get_i8("Slot").map_or(i as i32, i32::from);
        if !range.contains(&slot) {
            continue;
        }
        if let Some(mut item) = ctx.source_item(nbt) {
            item.put("Slot", (slot + offset) as i8);
            out.push(Tag::Compound(item));
        }
    }
    out
}

/// The item in one slot of a Bedrock container list.
fn slot_item(source: &Compound, key: &str, slot: i32) -> Option<Compound> {
    source
        .get_list(key)?
        .iter()
        .filter_map(Tag::as_compound)
        .enumerate()
        .find(|(i, c)| c.get_i8("Slot").map_or(*i as i32, i32::from) == slot)
        .map(|(_, c)| c.clone())
}

/// A loot table path as Bedrock stores it (`loot_tables/chests/abandoned_mineshaft.json`) as a Java id.
fn loot_table(path: &str) -> String {
    let id = path.trim_start_matches("loot_tables/").trim_end_matches(".json");
    if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") }
}

/// One Bedrock actor as a Java entity compound.
pub fn write_actor(ctx: &EntityContext<'_>, entity: &Entity, ty: V, source: &Compound) -> Option<Compound> {
    let mut out = Compound::new();
    out.put("id", type_id(ctx, ty)?);
    put_position(&mut out, entity.position, [0.0; 3], entity.yaw, entity.pitch);
    put_identity(ctx, &mut out, source);
    let mut equipment = Equipment::default();
    let mob = matches!(kind(ty), Kind::Mob | Kind::Hostile);
    // The horse family keeps its saddle and armour in its own inventory, read below.
    if (mob || ty == V::ARMOR_STAND) && !is_horse(ty) {
        bedrock_equipment(ctx, source, &mut equipment);
    }
    if mob {
        write_mob(ctx, entity, ty, source, &mut out);
    }
    let variant = source.get_i32_or("Variant", 0);
    let mark = source.get_i32_or("MarkVariant", 0);
    let color = source.get_i8_or("Color", 0);
    let tamed = source.get_i8_or("IsTamed", 0) != 0;
    match ty {
        V::WOLF | V::CAT | V::PARROT => {
            let owned = put_owner(ctx, &mut out, source);
            out.put("Sitting", source.get_i8_or("Sitting", 0) != 0);
            if owned && ty != V::PARROT {
                // Red, the collar a pet gets when it is tamed.
                out.put("CollarColor", source.get_i8_or("Color", 14));
            }
            match ty {
                V::CAT => {
                    let name = CATS.get(variant as usize).copied().unwrap_or("tabby");
                    if ctx.since(1, 19, 0) {
                        out.put("variant", format!("minecraft:{name}"));
                    } else {
                        out.put("CatType", cat_type(name));
                    }
                }
                V::PARROT => {
                    out.put("Variant", variant.clamp(0, 4));
                }
                _ => {}
            }
        }
        _ if is_horse(ty) => write_horse(ctx, ty, source, tamed, &mut out, &mut equipment),
        V::PIG | V::STRIDER => {
            if source.get_i8_or("Saddled", 0) != 0 {
                let saddle = Compound::new().with("id", "minecraft:saddle");
                let saddle = if ctx.format.items == crate::convert::java::format::ItemFormat::Components {
                    saddle.with("count", 1)
                } else {
                    saddle.with("Count", 1i8)
                };
                equipment.set(Slot::Saddle, saddle, ALWAYS_DROPS);
            }
        }
        V::RABBIT => {
            out.put("RabbitType", variant);
        }
        V::FOX => {
            out.put("Type", if variant == 1 { "snow" } else { "red" });
        }
        V::PANDA => {
            let gene = PANDA_GENES.iter().find(|g| group(source, &format!("minecraft:panda_{g}"))).unwrap_or(&"normal");
            out.put("MainGene", *gene);
            out.put("HiddenGene", *gene);
        }
        V::SHEEP => {
            out.put("Color", color);
            out.put("Sheared", source.get_i8_or("Sheared", 0) != 0);
        }
        V::OCELOT => {
            out.put("Trusting", source.get_i8_or("IsTrusting", 0) != 0);
        }
        V::BEE => {
            if let Some([x, y, z]) = block_of(source, "HomePos") {
                if ctx.since(1, 20, 5) {
                    out.put("hive_pos", vec![x, y, z]);
                } else {
                    out.put("HivePos", block_pos_compound(x, y, z));
                }
            }
            out.put("HasNectar", group(source, "has_nectar"));
        }
        V::TURTLE => {
            if let Some([x, y, z]) = block_of(source, "HomePos") {
                if ctx.since(1, 21, 5) {
                    out.put("home_pos", vec![x, y, z]);
                } else {
                    out.put("HomePosX", x);
                    out.put("HomePosY", y);
                    out.put("HomePosZ", z);
                }
            }
        }
        V::SLIME | V::MAGMA_CUBE => {
            out.put("Size", (variant - 1).max(0));
        }
        V::SHULKER => {
            out.put("Color", variant.clamp(0, 16) as i8);
        }
        V::ENDERMAN => {
            if let Some(state) = source.get_compound("carriedBlock").and_then(|b| ctx.source_block(b)) {
                out.put("carriedBlockState", state);
            }
        }
        V::IRON_GOLEM => {
            out.put("PlayerCreated", !group(source, "minecraft:village_created"));
        }
        V::SNOW_GOLEM => {
            out.put("Pumpkin", source.get_i8_or("Sheared", 0) == 0);
        }
        V::CREEPER => {
            out.put(
                "powered",
                group(source, "minecraft:charged_creeper") || group(source, "minecraft:charged_exploding"),
            );
        }
        V::PILLAGER | V::VINDICATOR | V::EVOKER | V::ILLUSIONER | V::RAVAGER | V::WITCH => {
            if source.get_i8_or("IsIllagerCaptain", 0) != 0 {
                out.put("PatrolLeader", true);
            }
        }
        V::TROPICAL_FISH => {
            let base = i32::from(color) & 0xFF;
            let pattern_color = i32::from(source.get_i8_or("Color2", 0)) & 0xFF;
            out.put("Variant", (variant & 0xFF) | ((mark & 0xFF) << 8) | (base << 16) | (pattern_color << 24));
        }
        V::AXOLOTL => {
            // Bedrock: lucy, cyan, gold, wild, blue; Java: lucy, wild, gold, cyan, blue.
            out.put("Variant", [0, 3, 2, 1, 4].get(variant as usize).copied().unwrap_or(0));
        }
        V::FROG => {
            let name = ["temperate", "cold", "warm"].get(variant as usize).copied().unwrap_or("temperate");
            out.put("variant", format!("minecraft:{name}"));
        }
        V::MOOSHROOM => {
            out.put("Type", if variant == 1 { "brown" } else { "red" });
        }
        V::VILLAGER | V::ZOMBIE_VILLAGER | V::WANDERING_TRADER => write_villager(ctx, ty, source, &mut out),
        V::ARMOR_STAND => {
            out.put("ShowArms", true);
            if let Some(index) = source.get_compound("Pose").and_then(|p| p.get_i32("PoseIndex")) {
                if index == 1 {
                    let zero = || Tag::List(vec![Tag::Float(0.0); 3]);
                    let pose = Compound::new()
                        .with("Head", zero())
                        .with("Body", zero())
                        .with("LeftArm", zero())
                        .with("RightArm", zero())
                        .with("LeftLeg", zero())
                        .with("RightLeg", zero());
                    out.put("Pose", pose);
                }
            }
        }
        V::CHEST_MINECART | V::HOPPER_MINECART => {
            if let Some(table) = source.get_str("LootTable").filter(|t| !t.is_empty()) {
                out.put("LootTable", loot_table(table));
                if let Some(seed) = source.get_i64("LootTableSeed") {
                    out.put("LootTableSeed", seed);
                }
            } else {
                out.put("Items", Tag::List(slotted_items(ctx, source, "ChestItems", 0.., 0)));
            }
        }
        V::ITEM => {
            let item = source.get_compound("Item").and_then(|i| ctx.source_item(i))?;
            out.put("Item", item);
            out.put("Age", source.get_i16_or("Age", 0));
            out.put("PickupDelay", 0i16);
        }
        V::EXPERIENCE_ORB => {
            let value = source.get_i32("experience value").unwrap_or(1).clamp(1, i16::MAX as i32);
            out.put("Value", value as i16);
        }
        V::FALLING_BLOCK => {
            let state = source.get_compound("FallingBlock").and_then(|b| ctx.source_block(b))?;
            out.put("BlockState", state);
            out.put("Time", 1);
        }
        V::END_CRYSTAL => {
            if let Some(show) = source.get_i8("ShowBottom") {
                out.put("ShowBottom", show != 0);
            }
        }
        _ if is_boat(ty) => {
            if !ctx.since(1, 21, 2) {
                out.put("Type", boat_wood(ty));
            }
            if is_chest_boat(ty) && ctx.since(1, 19, 0) {
                out.put("Items", Tag::List(slotted_items(ctx, source, "ChestItems", 0.., 0)));
            }
        }
        _ => {}
    }
    equipment.write(ctx, ty, &mut out);
    Some(out)
}

/// The horse family: `Tame`, the owner, the coat, the speed and jump attributes and the inventory. Bedrock keeps
/// the saddle (llamas: the carpet) in chest slot 0, horse armour in slot 1, and the chest of donkeys, mules and llamas
/// from slot 1; Java's chest starts at slot 2 before 1.20.5 and at 0 since.
fn write_horse(
    ctx: &EntityContext<'_>,
    ty: V,
    source: &Compound,
    tamed: bool,
    out: &mut Compound,
    equipment: &mut Equipment,
) {
    out.put("Tame", tamed);
    put_owner(ctx, out, source);
    let llama = matches!(ty, V::LLAMA | V::TRADER_LLAMA);
    if let Some(first) = slot_item(source, "ChestItems", 0).and_then(|s| ctx.source_item(&s)) {
        equipment.set(if llama { Slot::Body } else { Slot::Saddle }, first, ALWAYS_DROPS);
    }
    let chested = matches!(ty, V::DONKEY | V::MULE | V::LLAMA | V::TRADER_LLAMA);
    if !chested && let Some(armor) = slot_item(source, "ChestItems", 1).and_then(|s| ctx.source_item(&s)) {
        equipment.set(Slot::Body, armor, ALWAYS_DROPS);
    }
    if chested {
        let has_chest = source.get_i8_or("Chested", 0) != 0;
        out.put("ChestedHorse", has_chest);
        if has_chest {
            let offset = if ctx.since(1, 20, 5) { -1 } else { 1 };
            out.put("Items", Tag::List(slotted_items(ctx, source, "ChestItems", 1.., offset)));
        }
    }
    match ty {
        V::HORSE => {
            out.put("Variant", (source.get_i32_or("Variant", 0) & 0xFF) | (source.get_i32_or("MarkVariant", 0) << 8));
        }
        V::LLAMA | V::TRADER_LLAMA => {
            out.put("Variant", source.get_i32_or("Variant", 0).clamp(0, 3));
            out.put("Strength", source.get_i32_or("Strength", 3).clamp(1, 5));
        }
        V::SKELETON_HORSE => {
            if group(source, "minecraft:skeleton_trap") {
                out.put("SkeletonTrap", true);
            }
        }
        _ => {}
    }
    let mut attributes = Vec::new();
    if let Some(max) = attribute(source, "minecraft:health", "Max") {
        attributes.push(("max_health", max));
    }
    if let Some(speed) = attribute(source, "minecraft:movement", "Current") {
        attributes.push(("movement_speed", speed));
    }
    if let Some(jump) = attribute(source, "minecraft:horse.jump_strength", "Current") {
        attributes.push(("jump_strength", jump));
    }
    write_attributes(ctx, &attributes, out);
}

/// The profession: the component group (`+farmer`), else `Variant`.
fn profession(source: &Compound) -> &'static str {
    PROFESSIONS
        .iter()
        .skip(1)
        .find(|p| group(source, p) || group(source, &format!("minecraft:{p}")))
        .copied()
        .unwrap_or_else(|| PROFESSIONS.get(source.get_i32_or("Variant", 0) as usize).copied().unwrap_or("none"))
}

/// Villagers, zombie villagers and wandering traders: `VillagerData`, experience, trades and inventory.
fn write_villager(ctx: &EntityContext<'_>, ty: V, source: &Compound, out: &mut Compound) {
    let tier = source.get_i32_or("TradeTier", 0);
    if ty != V::WANDERING_TRADER {
        let kind = VILLAGER_TYPES.get(source.get_i32_or("MarkVariant", 0) as usize).copied().unwrap_or("plains");
        let data = Compound::new()
            .with("profession", format!("minecraft:{}", profession(source)))
            .with("type", format!("minecraft:{kind}"))
            .with("level", (tier + 1).clamp(1, 5));
        out.put("VillagerData", data);
        if ctx.since(26, 1, 0) {
            out.put("VillagerDataFinalized", true);
        }
    }
    if ty == V::VILLAGER {
        out.put("Xp", source.get_i32_or("TradeExperience", 0));
        let inventory = source
            .get_list("ChestItems")
            .map(|l| l.iter().filter_map(Tag::as_compound).filter_map(|i| ctx.source_item(i)).map(Tag::Compound))
            .map(|items| items.collect())
            .unwrap_or_default();
        out.put("Inventory", Tag::List(inventory));
    }
    if ty == V::ZOMBIE_VILLAGER {
        out.put("ConversionTime", -1);
    }
    if let Some(recipes) = source.get_compound("Offers").and_then(|o| o.get_list("Recipes")) {
        let list: Vec<Tag> = recipes
            .iter()
            .filter_map(Tag::as_compound)
            .filter(|r| ty == V::WANDERING_TRADER || r.get_i32("tier").is_none_or(|t| t <= tier))
            .filter_map(|r| trade(ctx, r))
            .map(Tag::Compound)
            .collect();
        out.put("Offers", Compound::new().with("Recipes", Tag::List(list)));
    }
}

/// A trade item: Bedrock's "any data" wildcard (`Damage` 32767) is the plain item, and the count is `count_key`.
fn trade_item(ctx: &EntityContext<'_>, recipe: &Compound, key: &str, count_key: &str) -> Option<Compound> {
    let mut nbt = recipe.get_compound(key)?.clone();
    if nbt.get_i16("Damage") == Some(32767) {
        nbt.put("Damage", 0i16);
    }
    if let Some(count) = recipe.get_i32(count_key).filter(|c| *c > 0) {
        nbt.put("Count", count.clamp(1, 127) as i8);
    }
    ctx.source_item(&nbt)
}

/// One `Offers.Recipes` entry.
fn trade(ctx: &EntityContext<'_>, recipe: &Compound) -> Option<Compound> {
    let buy = trade_item(ctx, recipe, "buyA", "buyCountA")?;
    let sell = trade_item(ctx, recipe, "sell", "")?;
    let mut out = Compound::new().with("buy", buy);
    if let Some(b) = trade_item(ctx, recipe, "buyB", "buyCountB") {
        out.put("buyB", b);
    }
    out.put("sell", sell);
    out.put("maxUses", recipe.get_i32_or("maxUses", 12));
    out.put("uses", recipe.get_i32_or("uses", 0));
    out.put("rewardExp", recipe.get_i8_or("rewardExp", 1) != 0);
    out.put("xp", recipe.get_i32_or("traderExp", 1));
    out.put("priceMultiplier", recipe.get_f32_or("priceMultiplierA", 0.05));
    out.put("demand", recipe.get_i32_or("demand", 0));
    out.put("specialPrice", 0);
    Some(out)
}
