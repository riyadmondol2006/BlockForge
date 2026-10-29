//! The fields every mob has: health, equipment and the chance to drop it, age, persistence, leash and attributes.

use super::EntityContext;
use super::common::block_pos_compound;
use crate::convert::bedrock::resolvers::entity::{Kind, actor_link, kind};
use crate::convert::model::{Entity, VanillaEntityType as V};
use crate::convert::nbt::{Compound, Tag};

/// The chance an item a mob was given drops (Java's default).
pub const DEFAULT_DROP_CHANCE: f32 = 0.085;
/// The chance of an item a mob picked up (or a saddle, horse armour) dropping: always.
pub const ALWAYS_DROPS: f32 = 2.0;

/// The equipment slots, in the order of the 1.21.5 `equipment` compound keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    /// Horse armour, llama carpets, wolf armour.
    Body,
    Saddle,
}

impl Slot {
    fn key(self) -> &'static str {
        match self {
            Slot::MainHand => "mainhand",
            Slot::OffHand => "offhand",
            Slot::Feet => "feet",
            Slot::Legs => "legs",
            Slot::Chest => "chest",
            Slot::Head => "head",
            Slot::Body => "body",
            Slot::Saddle => "saddle",
        }
    }
}

/// What a mob wears and holds, with drop chances, written in the version's layout.
#[derive(Default)]
pub struct Equipment {
    slots: Vec<(Slot, Compound, f32)>,
}

impl Equipment {
    pub fn set(&mut self, slot: Slot, item: Compound, drop_chance: f32) {
        self.slots.retain(|(s, _, _)| *s != slot);
        self.slots.push((slot, item, drop_chance));
    }

    pub fn has(&self, slot: Slot) -> bool {
        self.slots.iter().any(|(s, _, _)| *s == slot)
    }

    fn get(&self, slot: Slot) -> Option<&(Slot, Compound, f32)> {
        self.slots.iter().find(|(s, _, _)| *s == slot)
    }

    /// From 1.21.5 one `equipment` compound and the `drop_chances` that are not the default; before, `HandItems`
    /// `[main, off]` and `ArmorItems` `[feet, legs, chest, head]` with their chance lists, the body slot as
    /// `body_armor_item` (1.20.5) or the type's own field (`ArmorItem`, `DecorItem`) and the saddle as `SaddleItem`
    /// (horses) or `Saddle` (pigs, striders).
    pub fn write(&self, ctx: &EntityContext<'_>, ty: V, out: &mut Compound) {
        if self.slots.is_empty() {
            return;
        }
        if ctx.format.split_equipment {
            let mut equipment = Compound::new();
            let mut chances = Compound::new();
            for slot in [
                Slot::MainHand,
                Slot::OffHand,
                Slot::Feet,
                Slot::Legs,
                Slot::Chest,
                Slot::Head,
                Slot::Body,
                Slot::Saddle,
            ] {
                if let Some((_, item, chance)) = self.get(slot) {
                    equipment.put(slot.key(), item.clone());
                    if (*chance - DEFAULT_DROP_CHANCE).abs() > f32::EPSILON {
                        chances.put(slot.key(), *chance);
                    }
                }
            }
            out.put("equipment", equipment);
            if !chances.is_empty() {
                out.put("drop_chances", chances);
            }
            return;
        }
        let hands = [Slot::MainHand, Slot::OffHand];
        let armor = [Slot::Feet, Slot::Legs, Slot::Chest, Slot::Head];
        let lists = |slots: &[Slot]| -> (Tag, Tag) {
            let items = slots.iter().map(|s| self.get(*s).map(|e| e.1.clone())).collect::<Vec<_>>();
            let chances = slots.iter().map(|s| Tag::Float(self.get(*s).map_or(DEFAULT_DROP_CHANCE, |e| e.2)));
            (super::slots(&items), Tag::List(chances.collect()))
        };
        if hands.iter().any(|s| self.has(*s)) {
            let (items, chances) = lists(&hands);
            out.put("HandItems", items);
            out.put("HandDropChances", chances);
        }
        if armor.iter().any(|s| self.has(*s)) {
            let (items, chances) = lists(&armor);
            out.put("ArmorItems", items);
            out.put("ArmorDropChances", chances);
        }
        if let Some((_, item, chance)) = self.get(Slot::Body) {
            if ctx.since(1, 20, 5) {
                out.put("body_armor_item", item.clone());
                out.put("body_armor_drop_chance", *chance);
            } else if matches!(ty, V::LLAMA | V::TRADER_LLAMA) {
                out.put("DecorItem", item.clone());
            } else if is_horse(ty) {
                out.put("ArmorItem", item.clone());
            }
        }
        if let Some((_, item, _)) = self.get(Slot::Saddle) {
            if is_horse(ty) {
                out.put("SaddleItem", item.clone());
            } else {
                out.put("Saddle", true);
            }
        }
    }
}

/// The horse family (`AbstractHorse`): horses, donkeys, mules, skeleton and zombie horses, llamas, camels.
pub fn is_horse(ty: V) -> bool {
    matches!(
        ty,
        V::HORSE
            | V::DONKEY
            | V::MULE
            | V::SKELETON_HORSE
            | V::ZOMBIE_HORSE
            | V::LLAMA
            | V::TRADER_LLAMA
            | V::CAMEL
            | V::CAMEL_HUSK
    )
}

/// The first item compound of a Bedrock item list (`Mainhand`, `Offhand` hold one).
fn list_item(source: &Compound, key: &str, index: usize) -> Option<Compound> {
    source.get_list(key)?.get(index)?.as_compound().cloned()
}

/// A Bedrock item's drop chance: always for an item the mob picked up, the default otherwise.
pub fn drop_chance(item: &Compound) -> f32 {
    if item.get_i8_or("WasPickedUp", 0) != 0 { ALWAYS_DROPS } else { DEFAULT_DROP_CHANCE }
}

/// Hands and armour of a Bedrock mob: `Mainhand[0]`, `Offhand[0]` and `Armor` (head, chest, legs, feet, then body).
pub fn bedrock_equipment(ctx: &EntityContext<'_>, source: &Compound, equipment: &mut Equipment) {
    let mut put = |slot: Slot, nbt: Option<Compound>| {
        if let Some(nbt) = nbt
            && let Some(item) = ctx.source_item(&nbt)
        {
            equipment.set(slot, item, drop_chance(&nbt));
        }
    };
    put(Slot::MainHand, list_item(source, "Mainhand", 0));
    put(Slot::OffHand, list_item(source, "Offhand", 0));
    for (i, slot) in [Slot::Head, Slot::Chest, Slot::Legs, Slot::Feet, Slot::Body].into_iter().enumerate() {
        put(slot, list_item(source, "Armor", i));
    }
}

/// Health, age, persistence, loot pick-up and the leash of a mob.
pub fn write_mob(ctx: &EntityContext<'_>, entity: &Entity, ty: V, source: &Compound, out: &mut Compound) {
    if let Some(health) = super::common::attribute(source, "minecraft:health", "Current") {
        out.put("Health", health as f32);
    }
    if source.get_i8_or("IsBaby", 0) != 0 {
        out.put("IsBaby", true);
        out.put("Age", source.get_i32("Age").filter(|a| *a < 0).unwrap_or(-24000));
    } else if let Some(age) = source.get_i32("Age") {
        out.put("Age", age);
    }
    if source.get_i8_or("canPickupItems", 0) != 0 {
        out.put("CanPickUpLoot", true);
    }
    out.put("PersistenceRequired", persistent(ty, source));
    if let Some(knot) = entity.leash {
        if ctx.since(1, 20, 5) {
            out.put("leash", vec![knot.x, knot.y, knot.z]);
        } else {
            out.put("Leash", block_pos_compound(knot.x, knot.y, knot.z));
        }
    }
}

/// Whether a mob must stay when no player is near. Bedrock's `Persistent` is missing on most actors, so: anything
/// named, tamed, owned or leashed, villagers, traders, golems and every animal but fish stay; other hostile mobs only
/// when named or holding loot they picked up, so they despawn like Java's own.
pub fn persistent(ty: V, source: &Compound) -> bool {
    let named = source.get_str("CustomName").is_some_and(|n| !n.is_empty());
    let cared = named
        || source.get_i8_or("IsTamed", 0) != 0
        || actor_link(source, "OwnerNew").is_some()
        || actor_link(source, "LeasherID").is_some();
    if cared {
        return true;
    }
    match kind(ty) {
        Kind::Hostile => ["Mainhand", "Offhand", "Armor"].iter().any(|key| {
            source
                .get_list(key)
                .is_some_and(|l| l.iter().filter_map(Tag::as_compound).any(|i| i.get_i8_or("WasPickedUp", 0) != 0))
        }),
        _ => !matches!(
            ty,
            V::COD | V::SALMON | V::PUFFERFISH | V::TROPICAL_FISH | V::TADPOLE | V::SQUID | V::GLOW_SQUID | V::BAT
        ),
    }
}

/// Attributes by Java name (`max_health`, `movement_speed`, `jump_strength`), written as the version names them:
/// `generic.maxHealth` (before 1.16), `minecraft:generic.max_health` (1.16), `minecraft:max_health` with `id`/`base`
/// (1.21.2). The jump strength is `horse.*` before 1.20.5.
pub fn write_attributes(ctx: &EntityContext<'_>, values: &[(&str, f64)], out: &mut Compound) {
    if values.is_empty() {
        return;
    }
    let name = |attr: &str| -> String {
        if ctx.since(1, 21, 2) {
            return format!("minecraft:{attr}");
        }
        let family = if attr == "jump_strength" && !ctx.since(1, 20, 5) { "horse" } else { "generic" };
        if ctx.since(1, 16, 0) {
            return format!("minecraft:{family}.{attr}");
        }
        // camelCase before 1.16
        let mut camel = String::new();
        let mut upper = false;
        for c in attr.chars() {
            if c == '_' {
                upper = true;
            } else if upper {
                camel.push(c.to_ascii_uppercase());
                upper = false;
            } else {
                camel.push(c);
            }
        }
        format!("{family}.{camel}")
    };
    let list = values.iter().map(|(attr, base)| {
        let entry = if ctx.since(1, 21, 2) {
            Compound::new().with("id", name(attr)).with("base", *base)
        } else {
            Compound::new().with("Name", name(attr)).with("Base", *base)
        };
        Tag::Compound(entry)
    });
    out.put(if ctx.since(1, 21, 2) { "attributes" } else { "Attributes" }, Tag::List(list.collect()));
}
