// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The write direction (model to Bedrock item compound), best effort: identity, amount, durability, display,
//! enchantments, repair cost, spawn eggs, books, trims, fireworks, bundles and the block lists. Block entity data,
//! maps and lodestones are not written yet.

use std::sync::Arc;

use crate::convert::mapping::{ItemKind, PivotItem, StateMap, StateValue};
use crate::convert::model::item::{FireworkExplosion, StewEffect};
use crate::convert::model::{BlockIdentifier, Color, EntityType, ItemId, ItemProps, ItemStack};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::json_text;

use super::pivot::{block_to_pivot, map_value};
use super::{BedrockItems, Missing};

/// Names that differ between the `Block` compound and the `Name` tag of a block item.
const FIXED_LEGACY_NAMES: [(&str, &str); 5] = [
    ("minecraft:concretePowder", "minecraft:concrete_powder"),
    ("minecraft:stone_slab", "minecraft:double_stone_slab"),
    ("minecraft:stone_slab2", "minecraft:double_stone_slab2"),
    ("minecraft:stone_slab3", "minecraft:double_stone_slab3"),
    ("minecraft:stone_slab4", "minecraft:double_stone_slab4"),
];

impl BedrockItems {
    /// The Bedrock item compound of a stack (without `Slot`), or `None` when it cannot be represented.
    pub fn write(&self, stack: &ItemStack) -> Option<Compound> {
        let mut out = self.create_output(stack)?;
        self.write_props(&stack.props, &mut out);
        Some(out)
    }

    /// `Name`/`Damage` (items) or `Block`/`Name`/`Damage` (blocks).
    fn create_output(&self, stack: &ItemStack) -> Option<Compound> {
        let (items, blocks) = self.resolvers();
        let mut out = Compound::new();
        let pivot = match &stack.id {
            ItemId::Item(item) => {
                Some(PivotItem { kind: ItemKind::Item(Arc::from(item.name())), props: item_row_props(&stack.props) })
            }
            ItemId::Block(block) => block.block_type().map(|ty| {
                let mut props = item_row_props(&stack.props);
                if let Some(pivot) = block_to_pivot(block) {
                    for (state, value) in pivot.states {
                        props.insert(Arc::from(format!("S:{state}")), value);
                    }
                }
                PivotItem { kind: ItemKind::Block(Arc::from(ty.name())), props }
            }),
        };
        if let Some((name, states)) = pivot.as_ref().and_then(|p| items.from_pivot(p)) {
            out.put("Name", name.to_string());
            out.put("Damage", Tag::Short(states.get("data").and_then(data_value).unwrap_or(0) as i16));
            return Some(out);
        }
        let ItemId::Block(block) = &stack.id else {
            self.missing(Missing::Item, &format!("{:?}", stack.id));
            return None;
        };
        let (name, states) = self.write_block(block, blocks)?;
        let mut state_tag = Compound::new();
        for (key, value) in &states {
            if &**key == "waterlogged" {
                continue;
            }
            state_tag.put(key.to_string(), state_tag_value(value));
        }
        let mut block_tag = Compound::new();
        block_tag.put("name", name.to_string());
        block_tag.put("version", self.version().state_version());
        block_tag.put("states", Tag::Compound(state_tag));
        let name =
            FIXED_LEGACY_NAMES.iter().find(|(from, _)| *from == &*name).map_or(&*name, |(_, to)| *to).to_string();
        out.put("Block", Tag::Compound(block_tag));
        out.put("Name", name);
        out.put("Damage", Tag::Short(0));
        Some(out)
    }

    fn write_block(
        &self,
        block: &BlockIdentifier,
        blocks: &crate::convert::mapping::BlockResolver,
    ) -> Option<(Arc<str>, StateMap)> {
        if let Some(pivot) = block_to_pivot(block)
            && let Some(hit) = blocks.from_pivot(&pivot)
        {
            return Some(hit);
        }
        if let crate::convert::model::BlockKind::Custom(id) = block.kind() {
            if !self.custom_allowed() {
                return None;
            }
            let states = block.states().custom().iter().map(|(k, v)| (k.clone(), map_value(v))).collect();
            return Some((id.clone(), states));
        }
        self.missing(Missing::Block, &block.state_string());
        None
    }

    fn write_props(&self, props: &ItemProps, out: &mut Compound) {
        out.put("Count", Tag::Byte(props.amount.unwrap_or(1) as i8));
        let mut tag = Compound::new();

        if let Some(durability) = props.durability {
            tag.put("Damage", durability);
        }
        if let Some(display) = &props.display {
            let mut d = Compound::new();
            if let Some(name) = &display.display_name
                && let Ok(text) = json_text::to_legacy(name, true)
            {
                d.put("Name", text);
            }
            if let Some(lore) = &display.lore {
                let lines: Vec<Tag> =
                    lore.iter().filter_map(|l| json_text::to_legacy(l, true).ok()).map(Tag::String).collect();
                d.insert("Lore", Tag::List(lines));
            }
            if !d.is_empty() {
                tag.put("display", Tag::Compound(d));
            }
            if let Some(color) = display.color {
                tag.put("customColor", color.argb() as i32);
            }
        }
        for (key, blocks) in [("CanPlaceOn", &props.can_place_on), ("CanDestroy", &props.can_destroy)] {
            if let Some(blocks) = blocks {
                let (_, resolver) = self.resolvers();
                let names: Vec<Tag> = blocks
                    .iter()
                    .filter_map(|b| self.write_block(b, resolver))
                    .map(|(name, _)| name.to_string())
                    .filter(|name| name != "minecraft:air")
                    .map(Tag::String)
                    .collect();
                out.insert(key, Tag::List(names));
            }
        }
        if let Some(cost) = props.repair_cost {
            tag.put("RepairCost", cost);
        }
        if let Some(enchantments) = &props.enchantments {
            let list: Vec<Tag> = enchantments
                .iter()
                .filter_map(|(kind, level)| {
                    let Some(id) = self.tables.enchantment_id(*kind) else {
                        self.missing(Missing::Enchantment, kind.name());
                        return None;
                    };
                    Some(Tag::Compound(
                        Compound::new().with("id", Tag::Short(id as i16)).with("lvl", Tag::Short(*level as i16)),
                    ))
                })
                .collect();
            tag.insert("ench", Tag::List(list));
        }
        match &props.spawn_egg_mob {
            Some(EntityType::Vanilla(ty)) => match self.tables.entity_type_id(*ty) {
                Some(id) => {
                    out.put("ItemIdentifier", id.to_string());
                }
                None => self.missing(Missing::EntityType, ty.name()),
            },
            Some(EntityType::Custom(id)) if self.custom_allowed() => {
                out.put("ItemIdentifier", id.to_string());
            }
            _ => {}
        }
        if let Some(title) = &props.book_title {
            tag.put("title", title.clone());
        }
        if let Some(author) = &props.book_author {
            tag.put("author", author.clone());
        }
        if let Some(pages) = &props.book_pages {
            let list: Vec<Tag> = pages
                .iter()
                .filter_map(|p| json_text::to_legacy(p, true).ok())
                .map(|text| Tag::Compound(Compound::new().with("photoname", "").with("text", text)))
                .collect();
            tag.insert("pages", Tag::List(list));
        }
        if let Some(trim) = &props.trim
            && let (Some(pattern), Some(material)) =
                (self.tables.trim_pattern_id(trim.pattern), self.tables.trim_material_id(trim.material))
        {
            tag.put("Trim", Compound::new().with("Material", material).with("Pattern", pattern));
        }
        if let Some(fireworks) = &props.fireworks
            && (!fireworks.explosions.is_empty() || fireworks.flight_duration != 0)
        {
            let mut f = Compound::new().with("Flight", Tag::Byte(fireworks.flight_duration));
            if !fireworks.explosions.is_empty() {
                let list = fireworks.explosions.iter().map(|e| Tag::Compound(write_explosion(e))).collect();
                f.insert("Explosions", Tag::List(list));
            }
            tag.put("Fireworks", f);
        }
        if let Some(explosion) = &props.firework_explosion {
            tag.put("FireworksItem", write_explosion(explosion));
        }
        if let Some(contents) = &props.bundle_contents {
            let list: Vec<Tag> = contents
                .iter()
                .filter(|i| !i.is_air())
                .filter_map(|i| self.write(i))
                .enumerate()
                .map(|(slot, mut c)| {
                    c.put("Slot", Tag::Byte(slot as i8));
                    Tag::Compound(c)
                })
                .collect();
            tag.insert("storage_item_component_content", Tag::List(list));
        }
        if !tag.is_empty() {
            out.put("tag", Tag::Compound(tag));
        }
    }
}

/// The `P:` keys the item rows of the Bedrock table can pin.
fn item_row_props(props: &ItemProps) -> std::collections::BTreeMap<Arc<str>, Arc<str>> {
    let mut map = std::collections::BTreeMap::new();
    let mut put = |k: &str, v: String| {
        map.insert(Arc::from(format!("P:{k}")), Arc::from(v));
    };
    if let Some(p) = props.potion {
        put("potion", p.name().to_string());
    }
    if let Some(EntityType::Vanilla(t)) = &props.spawn_egg_mob {
        put("spawn_egg_mob", t.name().to_string());
    }
    if let Some(m) = props.explorer_map {
        put("explorer_map", m.name().to_string());
    }
    if let Some(h) = props.horn_instrument {
        put("horn_instrument", h.name().to_string());
    }
    if let Some(a) = props.ominous_bottle_amplifier {
        put("ominous_bottle_amplifier", format!("i:{a}"));
    }
    if let Some(s) = &props.stew_effects {
        put("stew_effects", stew_text(s));
    }
    map
}

fn stew_text(stew: &StewEffect) -> String {
    let inner: Vec<String> = stew.effects.iter().map(|(e, d)| format!("{}={d}", e.name())).collect();
    format!("?:ChunkerStewEffect{{effects={{{}}}}}", inner.join(", "))
}

fn data_value(v: &StateValue) -> Option<i32> {
    match v {
        StateValue::Int(i) => Some(*i),
        _ => None,
    }
}

fn state_tag_value(v: &StateValue) -> Tag {
    match v {
        StateValue::Bool(b) => Tag::Byte(*b as i8),
        StateValue::Int(i) => Tag::Int(*i),
        StateValue::Str(s) => Tag::String(s.to_string()),
    }
}

fn write_explosion(e: &FireworkExplosion) -> Compound {
    Compound::new()
        .with("FireworkType", Tag::Byte(e.shape.id() as i8))
        .with("FireworkColor", Tag::ByteArray(firework_ids(&e.colors, true)))
        .with("FireworkFade", Tag::ByteArray(firework_ids(&e.fade_colors, false)))
        .with("FireworkTrail", Tag::Byte(e.trail as i8))
        .with("FireworkFlicker", Tag::Byte(e.twinkle as i8))
}

/// Colours to reversed dye ids (`mapFireworkColors`); Bedrock dislikes an empty colour list, so black is the default.
fn firework_ids(colors: &[Color], default_black: bool) -> Vec<i8> {
    use crate::convert::model::DyeColor;
    if default_black && colors.is_empty() {
        return vec![DyeColor::BLACK.reversed_id() as i8];
    }
    colors.iter().map(|c| DyeColor::find_closest(*c).reversed_id() as i8).collect()
}
