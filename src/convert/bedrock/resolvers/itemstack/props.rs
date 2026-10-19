// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The property handlers of `BedrockItemStackResolver`, run in Chunker's registration order.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::convert::bedrock::resolvers::block_entity;
use crate::convert::mapping::StateMap;
use crate::convert::model::item::{FireworkExplosion, Fireworks, ItemDisplay, Trim};
use crate::convert::model::{BlockIdentifier, BlockType, Color, DyeColor, FireworkShape, ItemId, ItemStack, ItemType};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::json_text;

use super::{BedrockItems, Missing, without_waterlogged};

impl BedrockItems {
    /// Read every property into `stack`; `false` means the item is not valid (an unknown spawn egg).
    pub(super) fn read_props(&self, tag: &Compound, stack: &mut ItemStack) -> bool {
        let item_tag = tag.get_compound("tag");

        // Amount
        stack.props.amount = Some(tag.get_integer("Count").unwrap_or(1) as i32);

        if let Some(t) = item_tag {
            // Durability
            if let Some(damage) = t.get_integer("Damage") {
                stack.props.durability = Some(damage as i32);
            }
            // Display
            stack.props.display = read_display(t).map(Box::new);
        }

        // Can place on / can destroy
        stack.props.can_place_on = self.read_block_list(tag, "CanPlaceOn");
        stack.props.can_destroy = self.read_block_list(tag, "CanDestroy");

        if let Some(t) = item_tag {
            // Repair cost
            if let Some(cost) = t.get_integer("RepairCost") {
                stack.props.repair_cost = Some(cost as i32);
            }
            // Enchantments
            stack.props.enchantments = self.read_enchantments(t);
            // Maps
            if let (Some(uuid), Some(lookup)) = (t.get_i64("map_uuid"), &self.hooks.map_index)
                && let Some(index) = lookup(uuid)
            {
                stack.props.map_index = Some(index);
            }
        }

        // Spawn eggs
        if let Some(identifier) = tag.get_str("ItemIdentifier") {
            match self.read_entity_type(identifier) {
                Some(entity) => stack.props.spawn_egg_mob = Some(entity),
                None => {
                    self.missing(Missing::EntityType, identifier);
                    if stack.id == ItemId::Item(ItemType::SPAWN_EGG) {
                        return false;
                    }
                }
            }
        }

        if let Some(t) = item_tag {
            // Books
            if let Some(title) = t.get_str("title") {
                stack.props.book_title = Some(title.to_string());
            }
            if let Some(author) = t.get_str("author") {
                stack.props.book_author = Some(author.to_string());
            }
            if let Some(pages) = t.get_list("pages")
                && pages.iter().all(|p| matches!(p, Tag::Compound(_)))
            {
                let pages = pages
                    .iter()
                    .filter_map(Tag::as_compound)
                    .filter_map(|page| page.get_str("text"))
                    .map(json_text::from_text)
                    .collect();
                stack.props.book_pages = Some(pages);
            }

            // Trim, fireworks, bundles and lodestones need a newer item format than 1.12.
            if self.version.at_least([1, 13, 0]) {
                stack.props.trim = self.read_trim(t);
            }
            stack.props.fireworks = self.read_fireworks(t).map(Box::new);
            stack.props.firework_explosion = t.get_compound("FireworksItem").map(|c| Box::new(read_explosion(c)));
            if self.version.at_least([1, 13, 0]) {
                if let Some(content) = t.get_list("storage_item_component_content")
                    && content.iter().all(|c| matches!(c, Tag::Compound(_)))
                {
                    let items = content
                        .iter()
                        .filter_map(Tag::as_compound)
                        .filter_map(|c| self.read(c))
                        .filter(|item| !item.is_air())
                        .collect();
                    stack.props.bundle_contents = Some(items);
                }
                if let Some(handle) = t.get_i32("trackingHandle") {
                    if stack.id == ItemId::Item(ItemType::COMPASS) {
                        stack.id = ItemId::Item(ItemType::LODESTONE_COMPASS);
                    }
                    if let Some(lookup) = &self.hooks.lodestone
                        && let Some(data) = lookup(handle)
                    {
                        stack.props.lodestone_data = Some(Box::new(data));
                    }
                }
            }
        }

        // Block entities (banners, signs, shulker boxes, ...) carried by the item NBT.
        if let (Some(kind), Some(t)) = (stack.id.item_stack_type().block_entity(), item_tag) {
            let trapped = matches!(&stack.id, ItemId::Block(b) if b.block_type() == Some(BlockType::TRAPPED_CHEST));
            stack.props.block_entity = block_entity::read_item_block_entity(self, kind, trapped, t).map(Box::new);
        }
        true
    }

    /// `CanPlaceOn` / `CanDestroy`: block names without states, air skipped.
    fn read_block_list(&self, tag: &Compound, key: &str) -> Option<Vec<Arc<BlockIdentifier>>> {
        let names = tag.get_list(key)?;
        if !names.iter().all(|n| matches!(n, Tag::String(_))) {
            return None;
        }
        let mut blocks = Vec::with_capacity(names.len());
        for name in names.iter().filter_map(Tag::as_str) {
            if let Some(block) = self.read_item_block(name, &StateMap::new()) {
                let block = without_waterlogged(block);
                if !block.is_air() {
                    blocks.push(Arc::new(block));
                }
            }
        }
        Some(blocks)
    }

    /// `tag.ench`: legacy numeric ids.
    fn read_enchantments(&self, t: &Compound) -> Option<BTreeMap<crate::convert::model::EnchantmentType, i32>> {
        let list = t.get_list("ench")?;
        if !list.iter().all(|e| matches!(e, Tag::Compound(_))) {
            return None;
        }
        let mut enchantments = BTreeMap::new();
        for enchantment in list.iter().filter_map(Tag::as_compound) {
            let id = enchantment.get_i16_or("id", -1);
            if id == -1 {
                continue;
            }
            let Some(kind) = self.tables.enchantment(id as i32) else {
                self.missing(Missing::Enchantment, &id.to_string());
                continue;
            };
            enchantments.insert(kind, enchantment.get_i16_or("lvl", 1) as i32);
        }
        Some(enchantments)
    }

    fn read_trim(&self, t: &Compound) -> Option<Trim> {
        let trim = t.get_compound("Trim")?;
        let pattern_id = trim.get_str("Pattern")?;
        let Some(pattern) = self.tables.trim_pattern(pattern_id) else {
            self.missing(Missing::TrimPattern, pattern_id);
            return None;
        };
        let material_id = trim.get_str("Material")?;
        let Some(material) = self.tables.trim_material(material_id) else {
            self.missing(Missing::TrimMaterial, material_id);
            return None;
        };
        Some(Trim { material, pattern })
    }

    fn read_fireworks(&self, t: &Compound) -> Option<Fireworks> {
        let fireworks = t.get_compound("Fireworks")?;
        let flight_duration = fireworks.get_i8_or("Flight", 1);
        let explosions = match fireworks.get_list("Explosions") {
            Some(list) if list.iter().all(|e| matches!(e, Tag::Compound(_))) => {
                list.iter().filter_map(Tag::as_compound).map(read_explosion).collect()
            }
            _ => Vec::new(),
        };
        Some(Fireworks { flight_duration, explosions })
    }

    /// An entity type by identifier, custom ones when allowed (`BedrockEntityTypeResolver.to`).
    pub(crate) fn read_entity_type(&self, identifier: &str) -> Option<crate::convert::model::EntityType> {
        if !identifier.contains(':') {
            return self.tables.entity_type(&format!("minecraft:{identifier}"));
        }
        if !identifier.starts_with("minecraft:") && self.allow_custom {
            return Some(crate::convert::model::EntityType::Custom(Arc::from(identifier)));
        }
        self.tables.entity_type(identifier)
    }
}

/// `tag.display` and `tag.customColor`.
fn read_display(t: &Compound) -> Option<ItemDisplay> {
    let mut name = None;
    let mut lore = None;
    if let Some(display) = t.get_compound("display") {
        name = display.get_str("Name").map(json_text::from_text);
        if let Some(lines) = display.get_list("Lore")
            && lines.iter().all(|l| matches!(l, Tag::String(_)))
        {
            lore = Some(lines.iter().filter_map(Tag::as_str).map(json_text::from_text).collect());
        }
    }
    let color = t.contains_key("customColor").then(|| Color::from_rgb(t.get_i32_or("customColor", 0) as u32));
    (name.is_some() || lore.is_some() || color.is_some()).then_some(ItemDisplay { display_name: name, lore, color })
}

/// One firework explosion compound.
fn read_explosion(c: &Compound) -> FireworkExplosion {
    let shape =
        c.get_i8("FireworkType").and_then(|b| FireworkShape::by_id(b as i32)).unwrap_or(FireworkShape::SMALL_BALL);
    FireworkExplosion {
        shape,
        colors: firework_colors(c.get_byte_array("FireworkColor")),
        fade_colors: firework_colors(c.get_byte_array("FireworkFade")),
        trail: c.get_i8_or("FireworkTrail", 0) == 1,
        twinkle: c.get_i8_or("FireworkFlicker", 0) == 1,
    }
}

/// Reversed dye ids to Java colours (`mapFireworkColors`).
fn firework_colors(ids: Option<&[i8]>) -> Vec<Color> {
    ids.unwrap_or_default()
        .iter()
        .filter_map(|id| DyeColor::by_reversed_id(*id as i32))
        .map(|dye| Color::from_rgb(dye.java_rgb()))
        .collect()
}
