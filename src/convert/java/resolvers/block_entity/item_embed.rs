// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Block entity data carried by an item (a banner, a shulker box, a player head): the `writeToItemNBT` hooks of the
//! handlers plus the `BlockEntityTag` (before 1.20.5) or `minecraft:block_entity_data` (1.20.5+) fallback of the
//! item resolvers' `BLOCK_ENTITY` property.

use super::containers::{component_items, fix_loot_table};
use super::ctx::Ctx;
use super::misc::container_component;
use super::skull;
use super::tables;
use super::{JavaItems, write_block_entity};
use crate::convert::java::format::JavaFormat;
use crate::convert::model::{BlockEntity, BlockEntityData};
use crate::convert::nbt::{Compound, Tag};

/// Loot tables Chunker leaves out.
const UNSUPPORTED_LOOT_TABLES: [&str; 2] = ["minecraft:chests/monster_room", "minecraft:chests/village_two_room_house"];

/// Add the block entity to the item compound being written (`item` already has `id`; `components` or `tag` are
/// created as needed). Returns whether anything was written.
pub fn write_block_entity_to_item(
    items: &JavaItems,
    format: &JavaFormat,
    be: &BlockEntity,
    item: &mut Compound,
) -> bool {
    let ctx = Ctx::new(items, format);
    let before = item.clone();
    // The hooks of the handlers decide whether the plain block entity data is written too.
    let write_data = if ctx.since(1, 20, 5) { write_components(&ctx, be, item) } else { true };
    if write_data && let Some(mut tag) = write_block_entity(items, format, be) {
        for key in ["x", "y", "z", "Rotation", "Rot", "facing", "Facing"] {
            tag.remove(key);
        }
        if ctx.since(1, 20, 5) {
            item.get_or_create_compound("components").put("minecraft:block_entity_data", tag);
        } else {
            tag.remove("id");
            item.get_or_create_compound("tag").put("BlockEntityTag", tag);
        }
    }
    *item != before
}

/// The 1.20.5+ item components of the kinds that have a hook; `true` when the plain data should follow.
fn write_components(ctx: &Ctx, be: &BlockEntity, item: &mut Compound) -> bool {
    let mut write_data = true;
    let kind = be.kind();
    if kind.is_container()
        && let Some(container) = be.data.container()
    {
        let list = component_items(ctx, container.items.iter().map(|(slot, stack)| (*slot as i32, stack)));
        let components = item.get_or_create_compound("components");
        components.put("minecraft:container", list);
        if let Some(name) = be.custom_name.as_ref().and_then(|n| ctx.text(n)) {
            components.put("minecraft:custom_name", name);
        }
        write_data = false;
    }
    if kind.is_randomizable() {
        if let Some(table) = be
            .data
            .loot_table()
            .and_then(|t| t.as_deref())
            .filter(|t| !t.is_empty() && !UNSUPPORTED_LOOT_TABLES.contains(t))
        {
            let loot = Compound::new().with("loot_table", fix_loot_table(ctx, table));
            item.get_or_create_compound("components").put("minecraft:container_loot", loot);
        }
        write_data = false;
    }
    match &be.data {
        BlockEntityData::Banner(banner) => {
            let components = item.get_or_create_compound("components");
            if let Some(base) = banner.base {
                components.put("minecraft:base_color", base.lower_name());
            }
            if !banner.patterns.is_empty() {
                let patterns = banner.patterns.iter().filter_map(|(color, pattern)| {
                    let id = tables::banner_pattern_id(*pattern, ctx.version)?;
                    Some(Tag::Compound(Compound::new().with("color", color.lower_name()).with("pattern", id)))
                });
                components.put("minecraft:banner_patterns", Tag::List(patterns.collect()));
            }
            write_data = false;
        }
        BlockEntityData::Campfire(campfire) => {
            let list = container_component(ctx, &campfire.items);
            item.get_or_create_compound("components").put("minecraft:container", list);
            write_data = false;
        }
        BlockEntityData::ChiseledBookshelf(books) => {
            let list = container_component(ctx, &books[..]);
            item.get_or_create_compound("components").put("minecraft:container", list);
            write_data = false;
        }
        BlockEntityData::Shelf(shelf) => {
            let list = container_component(ctx, &shelf[..]);
            item.get_or_create_compound("components").put("minecraft:container", list);
            write_data = false;
        }
        BlockEntityData::DecoratedPot(pot) => {
            let components = item.get_or_create_compound("components");
            let ids = super::misc::pot_sherds(ctx, pot);
            if ctx.since(26, 3, 0) {
                components.put("minecraft:pot_decorations", super::misc::sherd_map(&ids));
            } else {
                components.put("minecraft:pot_decorations", Tag::list(ids.iter().map(String::as_str)));
            }
            if let Some(stack) = pot.item.as_ref() {
                let list = component_items(ctx, [(0, stack)].into_iter());
                if list.as_list().is_some_and(|l| !l.is_empty()) {
                    components.put("minecraft:container", list);
                }
            }
            write_data = false;
        }
        BlockEntityData::Skull(s) => {
            if s.owner_id.is_some() || s.texture.is_some() || s.texture_signature.is_some() || s.owner_name.is_some() {
                let profile = skull::profile(ctx, s);
                item.get_or_create_compound("components").put("minecraft:profile", profile);
            }
            write_data = false;
        }
        _ => {}
    }
    write_data
}
