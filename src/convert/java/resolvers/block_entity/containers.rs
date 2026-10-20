// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Containers: `ContainerBlockEntity` (items and custom name), `RandomizableContainerBlockEntity` (loot table) and the
//! furnace, brewing stand and crafter handlers.

use super::ctx::Ctx;
use crate::convert::model::blockentity::{BrewingStand, Container, Crafter, Furnace};
use crate::convert::model::{BlockEntity, ItemStack};
use crate::convert::nbt::{Compound, Tag};

/// Loot tables Chunker refuses to write.
const UNSUPPORTED_LOOT_TABLES: [&str; 2] = ["minecraft:chests/monster_room", "minecraft:chests/village_two_room_house"];
/// Loot table renamed in 1.14: (1.14+ name, older name).
const PRE_1_14_LOOT_TABLES: [(&str, &str); 1] =
    [("minecraft:chests/village/village_weaponsmith", "minecraft:chests/village_blacksmith")];

/// One item with its `Slot` byte, `None` for air or an item without a Java form.
fn slotted(ctx: &Ctx, slot: i8, stack: &ItemStack) -> Option<Tag> {
    let mut item = ctx.item(stack)?;
    item.put("Slot", slot);
    Some(Tag::Compound(item))
}

/// The `Items` list of slot to stack pairs.
pub(super) fn slot_list<'a>(ctx: &Ctx, stacks: impl Iterator<Item = (i8, &'a ItemStack)>) -> Tag {
    Tag::List(stacks.filter_map(|(slot, stack)| slotted(ctx, slot, stack)).collect())
}

/// `Items` from a fixed array of optional stacks (campfire, bookshelf, shelf); the index is the slot.
pub(super) fn array_items(ctx: &Ctx, stacks: &[Option<ItemStack>]) -> Tag {
    slot_list(ctx, stacks.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i as i8, s))))
}

/// The item list of a container as the 1.20.5 `minecraft:container` component writes it (`{slot, item}`).
pub(super) fn component_items<'a>(ctx: &Ctx, stacks: impl Iterator<Item = (i32, &'a ItemStack)>) -> Tag {
    let mut list = Vec::new();
    for (slot, stack) in stacks {
        if let Some(item) = ctx.item(stack) {
            let mut entry = Compound::new();
            entry.put("slot", slot);
            entry.put("item", item);
            list.push(Tag::Compound(entry));
        }
    }
    Tag::List(list)
}

/// `JavaContainerBlockEntityHandler.write`.
pub(super) fn write_container(ctx: &Ctx, out: &mut Compound, be: &BlockEntity, container: &Container) -> Option<()> {
    out.put("Items", slot_list(ctx, container.items.iter().map(|(slot, stack)| (*slot, stack))));
    if let Some(name) = &be.custom_name {
        out.put("CustomName", ctx.text(name)?);
    }
    Some(())
}

/// `JavaRandomizableContainerBlockEntityHandler.write`: loot tables are written as long as the model carries one.
pub(super) fn write_loot_table(ctx: &Ctx, out: &mut Compound, loot_table: &Option<String>) {
    if let Some(table) = loot_table.as_deref().filter(|t| !t.is_empty() && !UNSUPPORTED_LOOT_TABLES.contains(t)) {
        out.put("LootTable", fix_loot_table(ctx, table));
    }
}

/// Apply the 1.14 loot table rename in the direction of the target version.
pub(super) fn fix_loot_table(ctx: &Ctx, table: &str) -> String {
    let before_1_14 = !ctx.since(1, 14, 0);
    for (new, old) in PRE_1_14_LOOT_TABLES {
        if before_1_14 && table == new {
            return old.to_owned();
        }
        if !before_1_14 && table == old {
            return new.to_owned();
        }
    }
    table.to_owned()
}

/// `JavaFurnaceBlockEntityHandler.write`.
pub(super) fn write_furnace(ctx: &Ctx, out: &mut Compound, furnace: &Furnace) {
    if ctx.since(1, 21, 4) {
        // The fields were renamed in 1.21.4 and became ints in 26.3.
        if ctx.since(26, 3, 0) {
            out.put("lit_time_remaining", furnace.burn_time as i32);
            out.put("cooking_time_spent", furnace.cook_time as i32);
            out.put("lit_total_time", furnace.cook_time_total as i32);
        } else {
            out.put("lit_time_remaining", furnace.burn_time);
            out.put("cooking_time_spent", furnace.cook_time);
            out.put("lit_total_time", furnace.cook_time_total);
        }
    } else {
        out.put("BurnTime", furnace.burn_time);
        out.put("CookTime", furnace.cook_time);
        out.put("CookTimeTotal", furnace.cook_time_total);
    }
}

/// `JavaBrewingStandBlockEntityHandler.write`.
pub(super) fn write_brewing_stand(ctx: &Ctx, out: &mut Compound, stand: &BrewingStand) {
    if ctx.since(26, 3, 0) {
        out.put("BrewTime", stand.brew_time as i32);
        out.put("Fuel", stand.fuel as i32);
        out.put("total_fuel", stand.fuel_total as i32);
    } else {
        out.put("BrewTime", stand.brew_time);
        out.put("Fuel", stand.fuel as i8);
    }
}

/// `JavaCrafterBlockEntityHandler.write`: the disabled slots as the list of set bits.
pub(super) fn write_crafter(out: &mut Compound, crafter: &Crafter) {
    let slots: Vec<i32> = (0..16).filter(|i| crafter.disabled_slots & (1 << i) != 0).collect();
    out.put("disabled_slots", slots);
    out.put("crafting_ticks_remaining", crafter.crafting_ticks_remaining);
}
