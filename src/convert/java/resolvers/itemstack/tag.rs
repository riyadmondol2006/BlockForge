// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The `tag` item format (1.13 - 1.20.4), `JavaItemStackResolver`'s write direction.

use crate::convert::model::item::{FireworkExplosion, Fireworks};
use crate::convert::model::{BlockRef, EntityData, ItemId, ItemStack, ItemType};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::json_text;

use super::{ItemHooks, JavaItems, legacy_text, rgb_ints};

pub(super) fn write(items: &JavaItems, stack: &ItemStack, hooks: &dyn ItemHooks) -> Option<Compound> {
    let lookups = items.lookups();
    let props = &stack.props;
    let mut out = Compound::new();
    out.put("id", items.item_id(stack)?);

    // AMOUNT (a missing amount is one item; Chunker writes nothing and the game then reads zero)
    out.put("Count", props.amount.unwrap_or(1) as i8);

    // DURABILITY
    if let Some(damage) = props.durability {
        out.get_or_create_compound("tag").put("Damage", damage);
    }

    // DISPLAY
    if let Some(display) = &props.display {
        let tag = out.get_or_create_compound("tag");
        let d = tag.get_or_create_compound("display");
        if let Some(name) = &display.display_name {
            d.put("Name", json_text::to_json(name));
        }
        if let Some(lore) = &display.lore {
            d.put("Lore", Tag::List(lore.iter().map(|l| Tag::String(json_text::to_json(l))).collect()));
        }
        if let Some(color) = display.color {
            // `Color.getRGB()`: the alpha byte is part of the int
            d.put("color", color.argb() as i32);
        }
    }

    // CAN_PLACE_ON / CAN_DESTROY
    for (key, blocks) in [("CanPlaceOn", &props.can_place_on), ("CanDestroy", &props.can_destroy)] {
        if let Some(blocks) = blocks {
            let ids = block_ids(items, blocks).into_iter().map(Tag::String).collect();
            out.get_or_create_compound("tag").put(key, Tag::List(ids));
        }
    }

    // REPAIR_COST
    if let Some(cost) = props.repair_cost {
        out.get_or_create_compound("tag").put("RepairCost", cost);
    }

    // ENCHANTMENTS
    if let Some(enchantments) = &props.enchantments {
        let list = enchantments
            .iter()
            .filter_map(|(e, level)| {
                let id = lookups.enchantment(*e)?;
                Some(Tag::Compound(Compound::new().with("id", id).with("lvl", *level as i16)))
            })
            .collect();
        let name =
            if stack.id == ItemId::Item(ItemType::ENCHANTED_BOOK) { "StoredEnchantments" } else { "Enchantments" };
        out.get_or_create_compound("tag").put(name, Tag::List(list));
    }

    // MAP_DECORATION (the explorer map structure marker)
    if let Some(decoration) = &props.map_decoration
        && let Some(kind) = lookups.map_decoration_id(decoration.structure)
    {
        let marker = Compound::new()
            .with("id", "+")
            .with("type", kind as i8)
            .with("x", decoration.x)
            .with("z", decoration.z)
            .with("rot", decoration.rotation as f64);
        out.get_or_create_compound("tag").put("Decorations", Tag::List(vec![Tag::Compound(marker)]));
    }

    // MAP_INDEX
    if let Some(index) = props.map_index
        && let Some(id) = hooks.map_id(index)
    {
        out.get_or_create_compound("tag").put("map", id);
    }

    // SPAWN_EGG_MOB
    if let Some(mob) = &props.spawn_egg_mob {
        match lookups.entity_type(mob) {
            Some(id) => {
                out.get_or_create_compound("tag").get_or_create_compound("EntityTag").put("id", id);
            }
            None if stack.id == ItemId::Item(ItemType::SPAWN_EGG) => return None,
            None => {}
        }
    }

    // BOOK_TITLE / BOOK_AUTHOR / BOOK_PAGES
    if let Some(title) = &props.book_title {
        out.get_or_create_compound("tag").put("title", title.as_str());
    }
    if let Some(author) = &props.book_author {
        out.get_or_create_compound("tag").put("author", author.as_str());
    }
    let writable = stack.id == ItemId::Item(ItemType::WRITABLE_BOOK);
    let pages = match &props.book_pages {
        Some(pages) => Some(pages.as_slice()),
        // A written book always has pages
        None if stack.id == ItemId::Item(ItemType::WRITTEN_BOOK) => Some(&[][..]),
        None => None,
    };
    if let Some(pages) = pages {
        let list =
            pages.iter().map(|p| Tag::String(if writable { legacy_text(p) } else { json_text::to_json(p) })).collect();
        out.get_or_create_compound("tag").put("pages", Tag::List(list));
    }

    // HORN_INSTRUMENT
    if let Some(horn) = props.horn_instrument {
        out.get_or_create_compound("tag").put("instrument", lookups.horn(horn));
    }

    // POTION
    if let Some(potion) = props.potion {
        out.get_or_create_compound("tag").put("Potion", lookups.potion(potion));
    }

    // TRIM
    if let Some(trim) = &props.trim
        && let (Some(pattern), Some(material)) =
            (lookups.trim_pattern(trim.pattern), lookups.trim_material(trim.material))
    {
        let t = out.get_or_create_compound("tag").get_or_create_compound("Trim");
        t.put("material", material);
        t.put("pattern", pattern);
    }

    // STEW_EFFECT
    if let Some(stew) = &props.stew_effects
        && !stew.effects.is_empty()
    {
        let list = stew
            .effects
            .iter()
            .map(|(effect, duration)| {
                Tag::Compound(Compound::new().with("duration", *duration).with("id", lookups.effect(*effect)))
            })
            .collect();
        out.get_or_create_compound("tag").put("effects", Tag::List(list));
    }

    // FIREWORKS
    if let Some(fireworks) = &props.fireworks {
        write_fireworks(&mut out, fireworks);
    }

    // FIREWORK_EXPLOSION
    if let Some(explosion) = &props.firework_explosion {
        out.get_or_create_compound("tag").put("Explosion", explosion_tag(explosion));
    }

    // LODESTONE_DATA
    if let Some(lodestone) = &props.lodestone_data {
        let tag = out.get_or_create_compound("tag");
        tag.put("LodestoneDimension", &*lodestone.dimension.identifier);
        tag.put("LodestonePos", Compound::new().with("X", lodestone.x).with("Y", lodestone.y).with("Z", lodestone.z));
        tag.put("LodestoneTracked", lodestone.tracked);
    }

    // BUNDLE_CONTENTS
    if let Some(contents) = &props.bundle_contents {
        let list =
            contents.iter().filter(|i| !i.is_air()).filter_map(|i| write(items, i, hooks)).map(Tag::Compound).collect();
        out.get_or_create_compound("tag").put("Items", Tag::List(list));
    }

    // ENTITY
    if let Some(entity) = &props.entity
        && let Some(mut data) = hooks.entity(entity)
    {
        for key in ["block_pos", "TileX", "TileY", "TileZ", "Pos", "Motion", "Rotation", "Rot", "facing", "Facing"] {
            data.remove(key);
        }
        if matches!(entity.data, EntityData::Painting(_)) {
            data.remove("id");
        }
        out.get_or_create_compound("tag").put("EntityTag", data);
    }

    // BLOCK_ENTITY
    if let Some(entity) = &props.block_entity
        && let Some(mut data) = hooks.block_entity(stack, entity, &mut out)
    {
        for key in ["x", "y", "z", "id", "Rotation", "Rot", "facing", "Facing"] {
            data.remove(key);
        }
        out.get_or_create_compound("tag").put("BlockEntityTag", data);
    }

    Some(out)
}

fn block_ids(items: &JavaItems, blocks: &[BlockRef]) -> Vec<String> {
    blocks.iter().filter_map(|b| items.lookups().block_id(b)).filter(|id| id != "minecraft:air").collect()
}

fn write_fireworks(out: &mut Compound, fireworks: &Fireworks) {
    // The default is not written
    if fireworks.explosions.is_empty() && fireworks.flight_duration == 0 {
        return;
    }
    let tag = out.get_or_create_compound("tag").get_or_create_compound("Fireworks");
    tag.put("Flight", fireworks.flight_duration);
    if !fireworks.explosions.is_empty() {
        tag.put("Explosions", Tag::List(fireworks.explosions.iter().map(explosion_tag).collect()));
    }
}

fn explosion_tag(e: &FireworkExplosion) -> Tag {
    Tag::Compound(
        Compound::new()
            .with("Type", e.shape.id() as i8)
            .with("Colors", rgb_ints(&e.colors))
            .with("FadeColors", rgb_ints(&e.fade_colors))
            .with("Trail", e.trail)
            .with("Flicker", e.twinkle),
    )
}
