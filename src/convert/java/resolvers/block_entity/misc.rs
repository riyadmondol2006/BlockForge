// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The small handlers: command block, comparator, end gateway, jigsaw, structure block, jukebox, lectern, campfire,
//! decorated pot, brushable block, chiseled bookshelf, shelf, sculk sensor and the enchanting table.

use super::containers::{array_items, component_items};
use super::ctx::Ctx;
use crate::convert::model::blockentity::{
    Brushable, Campfire, CommandBlock, DecoratedPot, EndGateway, Jigsaw, Joint, Structure, StructureMirror,
    StructureRotation,
};
use crate::convert::model::{BlockEntity, ItemStack, StructureBlockMode};
use crate::convert::nbt::{Compound, Tag};

/// The custom name of block entities that write one on their own (command block, enchanting table).
pub(super) fn write_custom_name(ctx: &Ctx, out: &mut Compound, be: &BlockEntity) -> Option<()> {
    if let Some(name) = &be.custom_name {
        out.put("CustomName", ctx.text(name)?);
    }
    Some(())
}

pub(super) fn write_command_block(ctx: &Ctx, out: &mut Compound, be: &BlockEntity, block: &CommandBlock) -> Option<()> {
    out.put("Command", block.command.clone().unwrap_or_default());
    out.put("auto", block.auto);
    out.put("TrackOutput", block.track_output);
    write_custom_name(ctx, out, be)
}

pub(super) fn write_end_gateway(out: &mut Compound, gateway: &EndGateway) {
    out.put("Age", gateway.age);
    out.put("exit_portal", vec![gateway.exit_x, gateway.exit_y, gateway.exit_z]);
}

pub(super) fn write_jigsaw(ctx: &Ctx, out: &mut Compound, jigsaw: &Jigsaw) {
    out.put("name", jigsaw.name.as_str());
    out.put("target", jigsaw.target.as_str());
    out.put("pool", jigsaw.pool.as_str());
    out.put("final_state", jigsaw.final_state.as_str());
    out.put("joint", if jigsaw.joint == Joint::Aligned { "aligned" } else { "rollable" });
    // The priorities were added in 1.20.5.
    if ctx.since(1, 20, 5) {
        out.put("selection_priority", jigsaw.selection_priority);
        out.put("placement_priority", jigsaw.placement_priority);
    }
}

pub(super) fn write_structure(ctx: &Ctx, out: &mut Compound, s: &Structure) {
    out.put("name", s.name.as_str());
    out.put("author", s.author.as_str());
    out.put("metadata", s.metadata.as_str());
    out.put("posX", s.pos_x);
    out.put("posY", s.pos_y);
    out.put("posZ", s.pos_z);
    out.put("sizeX", s.size_x);
    out.put("sizeY", s.size_y);
    out.put("sizeZ", s.size_z);
    out.put(
        "rotation",
        match s.rotation {
            StructureRotation::None => "NONE",
            StructureRotation::Clockwise90 => "CLOCKWISE_90",
            StructureRotation::Clockwise180 => "CLOCKWISE_180",
            StructureRotation::CounterClockwise90 => "COUNTERCLOCKWISE_90",
        },
    );
    out.put(
        "mirror",
        match s.mirror {
            StructureMirror::None => "NONE",
            StructureMirror::LeftRight => "LEFT_RIGHT",
            StructureMirror::FrontBack => "FRONT_BACK",
            // Not a Java value: Chunker falls back to the lower case default.
            StructureMirror::Both => "none",
        },
    );
    out.put(
        "mode",
        match s.mode {
            StructureBlockMode::LOAD => "LOAD",
            StructureBlockMode::CORNER => "CORNER",
            StructureBlockMode::DATA => "DATA",
            // Export and invalid are not Java modes.
            StructureBlockMode::SAVE | StructureBlockMode::INVALID | StructureBlockMode::EXPORT => "SAVE",
        },
    );
    out.put("ignoreEntities", s.ignore_entities);
    if ctx.since(1, 21, 5) {
        out.put("strict", s.strict);
    }
    out.put("powered", s.powered);
    out.put("showair", s.show_air);
    out.put("showboundingbox", s.show_bounding_box);
    out.put("integrity", s.integrity);
    out.put("seed", s.seed);
}

/// The `RecordItem` of a jukebox.
pub(super) fn write_jukebox(ctx: &Ctx, out: &mut Compound, record: Option<&ItemStack>) {
    if let Some(item) = record.and_then(|r| ctx.item(r)) {
        out.put("RecordItem", item);
    }
}

pub(super) fn write_lectern(ctx: &Ctx, out: &mut Compound, book: Option<&ItemStack>, page: i32) {
    out.put("Page", page);
    if let Some(item) = book.and_then(|b| ctx.item(b)) {
        out.put("Book", item);
    }
}

pub(super) fn write_campfire(ctx: &Ctx, out: &mut Compound, campfire: &Campfire) {
    let mut times = vec![0; 4];
    for (i, stack) in campfire.items.iter().enumerate() {
        if stack.as_ref().is_some_and(|s| ctx.item(s).is_some()) {
            times[i] = campfire.cooking_times[i];
        }
    }
    out.put("Items", array_items(ctx, &campfire.items));
    out.put("CookingTimes", times);
}

/// The four sherds as item ids, back, left, right, front.
pub(super) fn pot_sherds(ctx: &Ctx, pot: &DecoratedPot) -> [String; 4] {
    [&pot.back, &pot.left, &pot.right, &pot.front]
        .map(|id| ctx.item_id(id).unwrap_or_else(|| "minecraft:brick".to_owned()))
}

/// The 26.3 form of the sherds: a compound of `{id}` per side.
pub(super) fn sherd_map(sherds: &[String; 4]) -> Compound {
    let mut map = Compound::new();
    for (side, id) in ["back", "left", "right", "front"].iter().zip(sherds) {
        map.put(*side, Compound::new().with("id", id.as_str()));
    }
    map
}

pub(super) fn write_decorated_pot(ctx: &Ctx, out: &mut Compound, pot: &DecoratedPot) {
    let ids = pot_sherds(ctx, pot);
    if ctx.since(26, 3, 0) {
        out.put("sherds", sherd_map(&ids));
    } else {
        let list = Tag::list(ids.iter().map(String::as_str));
        out.put(if ctx.since(1, 20, 0) { "sherds" } else { "shards" }, list);
    }
    // The item inside the pot exists from 1.20.3.
    if ctx.since(1, 20, 3)
        && let Some(item) = pot.item.as_ref().and_then(|i| ctx.item(i))
    {
        out.put("item", item);
    }
}

pub(super) fn write_brushable(ctx: &Ctx, out: &mut Compound, block: &Brushable) {
    // 6 means the direction is not defined.
    if block.brush_direction != 6 {
        out.put("hit_direction", block.brush_direction);
    }
    if let Some(item) = block.item.as_ref().and_then(|i| ctx.item(i)) {
        out.put("item", item);
    }
    if let Some(table) = block.loot_table.as_deref().filter(|t| !t.is_empty()) {
        out.put("LootTable", table);
    }
}

/// Chiseled bookshelves and shelves: `Items` with the index as slot.
pub(super) fn write_slot_array(ctx: &Ctx, out: &mut Compound, stacks: &[Option<ItemStack>]) {
    out.put("Items", array_items(ctx, stacks));
}

pub(super) fn write_sculk_sensor(out: &mut Compound) {
    let selector = Compound::new().with("tick", -1_i64);
    let listener = Compound::new().with("event_delay", 0).with("selector", selector);
    out.put("listener", listener);
    out.put("last_vibration_frequency", 0);
}

/// Used by the item hooks: the item list as the container component.
pub(super) fn container_component(ctx: &Ctx, stacks: &[Option<ItemStack>]) -> Tag {
    component_items(ctx, stacks.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i as i32, s))))
}
