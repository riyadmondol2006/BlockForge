// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The component item format (1.20.5+), `JavaComponentItemStackResolver`'s write direction.

use crate::convert::model::item::FireworkExplosion;
use crate::convert::model::{BlockRef, ItemId, ItemStack, ItemType};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::json_text;

use super::{ItemHooks, JavaItems, legacy_text, rgb_ints};

/// `components`, created on first use.
fn components(out: &mut Compound) -> &mut Compound {
    out.get_or_create_compound("components")
}

pub(super) fn write(items: &JavaItems, stack: &ItemStack, hooks: &dyn ItemHooks) -> Option<Compound> {
    let lookups = items.lookups();
    let props = &stack.props;
    let version = items.version();
    let flat_levels = items.at_least(1, 21, 5);
    let mut out = Compound::new();
    out.put("id", items.item_id(stack)?);

    // AMOUNT
    if let Some(count) = props.amount {
        out.put("count", count);
    }

    // DURABILITY
    if let Some(damage) = props.durability {
        components(&mut out).put("minecraft:damage", damage);
    }

    // DISPLAY
    if let Some(display) = &props.display {
        let c = components(&mut out);
        if let Some(name) = &display.display_name {
            c.put("minecraft:custom_name", text(name, items));
        }
        if let Some(lore) = &display.lore {
            c.put("minecraft:lore", json_text::to_nbt_list(lore, 0, version).unwrap_or(Tag::List(Vec::new())));
        }
        if let Some(color) = display.color {
            let rgb = color.argb() as i32;
            if flat_levels {
                c.put("minecraft:dyed_color", rgb);
            } else {
                c.put("minecraft:dyed_color", Compound::new().with("rgb", rgb));
            }
        }
    }

    // CAN_PLACE_ON / CAN_DESTROY
    for (key, blocks) in [("minecraft:can_place_on", &props.can_place_on), ("minecraft:can_break", &props.can_destroy)]
    {
        if let Some(blocks) = blocks {
            let predicates = block_ids(items, blocks)
                .into_iter()
                .map(|id| Tag::Compound(Compound::new().with("blocks", id)))
                .collect();
            components(&mut out).put(key, Compound::new().with("predicates", Tag::List(predicates)));
        }
    }

    // REPAIR_COST
    if let Some(cost) = props.repair_cost {
        components(&mut out).put("minecraft:repair_cost", cost);
    }

    // ENCHANTMENTS (wrapped in `levels` before 1.21.5)
    if let Some(enchantments) = &props.enchantments {
        let mut levels = Compound::new();
        for (e, level) in enchantments {
            if let Some(id) = lookups.enchantment(*e) {
                levels.put(id, *level);
            }
        }
        let name = if stack.id == ItemId::Item(ItemType::ENCHANTED_BOOK) {
            "minecraft:stored_enchantments"
        } else {
            "minecraft:enchantments"
        };
        if flat_levels {
            components(&mut out).put(name, levels);
        } else {
            components(&mut out).get_or_create_compound(name).put("levels", levels);
        }
    }

    // MAP_DECORATION
    if let Some(decoration) = &props.map_decoration
        && let Some(kind) = lookups.map_decoration(decoration.structure)
    {
        let marker = Compound::new()
            .with("type", kind)
            .with("x", decoration.x)
            .with("z", decoration.z)
            .with("rotation", decoration.rotation);
        components(&mut out).get_or_create_compound("minecraft:map_decorations").put("+", marker);
    }

    // MAP_INDEX
    if let Some(index) = props.map_index
        && let Some(id) = hooks.map_id(index)
    {
        components(&mut out).put("minecraft:map_id", id);
    }

    // SPAWN_EGG_MOB
    if let Some(mob) = &props.spawn_egg_mob {
        match lookups.entity_type(mob) {
            Some(id) => {
                components(&mut out).get_or_create_compound("minecraft:entity_data").put("id", id);
            }
            None if stack.id == ItemId::Item(ItemType::SPAWN_EGG) => return None,
            None => {}
        }
    }

    // BOOK_TITLE / BOOK_AUTHOR / BOOK_PAGES
    let writable = stack.id == ItemId::Item(ItemType::WRITABLE_BOOK);
    let book = if writable { "minecraft:writable_book_content" } else { "minecraft:written_book_content" };
    if let Some(title) = &props.book_title {
        components(&mut out).get_or_create_compound(book).get_or_create_compound("title").put("raw", title.as_str());
    }
    if let Some(author) = &props.book_author {
        components(&mut out).get_or_create_compound(book).put("author", author.as_str());
    }
    if let Some(pages) = &props.book_pages {
        let list = pages
            .iter()
            .map(|p| {
                let raw = if writable { Tag::String(legacy_text(p)) } else { text(p, items) };
                Tag::Compound(Compound::new().with("raw", raw))
            })
            .collect();
        components(&mut out).get_or_create_compound(book).put("pages", Tag::List(list));
    }

    // HORN_INSTRUMENT
    if let Some(horn) = props.horn_instrument {
        components(&mut out).put("minecraft:instrument", lookups.horn(horn));
    }

    // POTION
    if let Some(potion) = props.potion {
        components(&mut out).get_or_create_compound("minecraft:potion_contents").put("potion", lookups.potion(potion));
    }

    // TRIM
    if let Some(trim) = &props.trim
        && let (Some(pattern), Some(material)) =
            (lookups.trim_pattern(trim.pattern), lookups.trim_material(trim.material))
    {
        let t = components(&mut out).get_or_create_compound("minecraft:trim");
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
        components(&mut out).put("minecraft:suspicious_stew_effects", Tag::List(list));
    }

    // FIREWORKS
    if let Some(fireworks) = &props.fireworks
        && !(fireworks.explosions.is_empty() && fireworks.flight_duration == 0)
    {
        let f = components(&mut out).get_or_create_compound("minecraft:fireworks");
        f.put("flight_duration", fireworks.flight_duration);
        if !fireworks.explosions.is_empty() {
            let list = fireworks.explosions.iter().map(|e| Tag::Compound(explosion(e, flat_levels))).collect();
            f.put("explosions", Tag::List(list));
        }
    }

    // FIREWORK_EXPLOSION
    if let Some(e) = &props.firework_explosion {
        components(&mut out).put("minecraft:firework_explosion", explosion(e, flat_levels));
    }

    // OMINOUS_BOTTLE_AMPLIFIER
    if let Some(amplifier) = props.ominous_bottle_amplifier {
        components(&mut out).put("minecraft:ominous_bottle_amplifier", amplifier);
    }

    // BUNDLE_CONTENTS
    if let Some(contents) = &props.bundle_contents {
        let list =
            contents.iter().filter(|i| !i.is_air()).filter_map(|i| write(items, i, hooks)).map(Tag::Compound).collect();
        components(&mut out).put("minecraft:bundle_contents", Tag::List(list));
    }

    // LODESTONE_DATA
    if let Some(lodestone) = &props.lodestone_data {
        let target = Compound::new()
            .with("dimension", &*lodestone.dimension.identifier)
            .with("pos", vec![lodestone.x, lodestone.y, lodestone.z]);
        let tracker = components(&mut out).get_or_create_compound("minecraft:lodestone_tracker");
        tracker.put("target", target);
        tracker.put("tracked", lodestone.tracked);
    }

    // ENTITY
    if let Some(entity) = &props.entity
        && let Some(mut data) = hooks.entity(entity)
    {
        for key in ["block_pos", "TileX", "TileY", "TileZ", "Pos", "Motion", "Rotation", "Rot", "facing", "Facing"] {
            data.remove(key);
        }
        // Paintings keep their id here, the component needs it
        components(&mut out).put("minecraft:entity_data", data);
    }

    // BLOCK_ENTITY
    if let Some(entity) = &props.block_entity
        && let Some(mut data) = hooks.block_entity(stack, entity, &mut out)
    {
        for key in ["x", "y", "z", "Rotation", "Rot", "facing", "Facing"] {
            data.remove(key);
        }
        components(&mut out).put("minecraft:block_entity_data", data);
    }

    Some(out)
}

/// A text component: NBT from 1.21.5, a JSON string before.
fn text(value: &serde_json::Value, items: &JavaItems) -> Tag {
    json_text::to_nbt_for(value, items.version()).unwrap_or_else(|_| Tag::String(json_text::to_json(value)))
}

fn block_ids(items: &JavaItems, blocks: &[BlockRef]) -> Vec<String> {
    blocks.iter().filter_map(|b| items.lookups().block_id(b)).filter(|id| id != "minecraft:air").collect()
}

/// A firework explosion; the colours are int lists from 1.21.5 and int arrays before.
fn explosion(e: &FireworkExplosion, list_colors: bool) -> Compound {
    let colors = |c: &[crate::convert::model::Color]| -> Tag {
        let ints = rgb_ints(c);
        if list_colors { Tag::List(ints.into_iter().map(Tag::Int).collect()) } else { Tag::IntArray(ints) }
    };
    Compound::new()
        .with("shape", e.shape.lower_name())
        .with("colors", colors(&e.colors))
        .with("fade_colors", colors(&e.fade_colors))
        .with("has_trail", e.trail)
        .with("has_twinkle", e.twinkle)
}
