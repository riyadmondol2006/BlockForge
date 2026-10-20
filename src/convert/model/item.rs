// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Item stacks (`ChunkerItemStack` and its properties) and the small item enums.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value as Json;

use super::block::{BlockIdentifier, BlockKind, BlockRef, PreservedIdentifier};
use super::blockentity::{BlockEntity, BlockEntityKind};
use super::color::Color;
use super::dimension::Dimension;
use super::entity::{Entity, EntityType};
use super::generated::{
    BlockType, DyeColor, EffectType, EnchantmentType, ExplorerMap, FireworkShape, HornInstrument, ITEM_ENTITY,
    ItemType, PotionBottleType, PotionType, TrimMaterial, TrimPattern,
};

impl ItemType {
    /// The block entity kind carried by the item (banners, signs), if any.
    pub fn block_entity(self) -> Option<BlockEntityKind> {
        ITEM_ENTITY[self as usize]
    }
}

impl DyeColor {
    /// Lower case name (`light_blue`).
    pub const fn lower_name(self) -> &'static str {
        Self::LOWER_NAMES[self as usize]
    }

    pub const fn bedrock_rgb(self) -> u32 {
        Self::BEDROCK_RGB[self as usize]
    }

    pub const fn java_rgb(self) -> u32 {
        Self::JAVA_RGB[self as usize]
    }

    /// The numeric id (the ordinal).
    pub const fn id(self) -> u8 {
        self as u8
    }

    /// The id as used by Bedrock and legacy Java for some data (`15 - ordinal`).
    pub const fn reversed_id(self) -> u8 {
        15 - self as u8
    }

    pub fn by_id(id: i32) -> Option<DyeColor> {
        usize::try_from(id).ok().and_then(DyeColor::from_ordinal)
    }

    pub fn by_reversed_id(id: i32) -> Option<DyeColor> {
        DyeColor::by_id(15 - id)
    }

    /// The colour called `name` (`light_blue`).
    pub fn by_lower_name(name: &str) -> Option<DyeColor> {
        Self::LOWER_NAMES.iter().position(|n| *n == name).map(|i| Self::ALL[i])
    }

    /// The dye whose Java or Bedrock colour is nearest (squared RGB distance); the first minimum wins, the Java
    /// colour of an entry is tried before its Bedrock colour.
    pub fn find_closest(color: Color) -> DyeColor {
        let mut min = f64::MAX;
        let mut min_index = 0;
        for (i, dye) in Self::ALL.iter().enumerate() {
            for rgb in [dye.java_rgb(), dye.bedrock_rgb()] {
                let (r, g, b) = ((rgb >> 16 & 0xff) as i32, (rgb >> 8 & 0xff) as i32, (rgb & 0xff) as i32);
                let dr = r - color.r as i32;
                let dg = g - color.g as i32;
                let db = b - color.b as i32;
                let score = (dr * dr + dg * dg + db * db) as f64;
                if score < min {
                    min = score;
                    min_index = i;
                }
            }
        }
        Self::ALL[min_index]
    }
}

impl PotionType {
    /// Identifier without namespace (`long_swiftness`).
    pub const fn identifier(self) -> &'static str {
        Self::IDENTIFIERS[self as usize]
    }

    pub fn by_identifier(identifier: &str) -> Option<PotionType> {
        Self::IDENTIFIERS.iter().position(|n| *n == identifier).map(|i| Self::ALL[i])
    }
}

impl PotionBottleType {
    /// The bottle with the given id (`-1` is unknown, 0 regular, 1 splash, 2 lingering); anything else is unknown.
    pub fn from_id(id: i32) -> PotionBottleType {
        let index = id.wrapping_add(1);
        usize::try_from(index).ok().and_then(PotionBottleType::from_ordinal).unwrap_or(PotionBottleType::UNKNOWN)
    }

    pub const fn id(self) -> i32 {
        self as i32 - 1
    }
}

impl FireworkShape {
    /// Lower case name (`small_ball`).
    pub const fn lower_name(self) -> &'static str {
        Self::LOWER_NAMES[self as usize]
    }

    pub fn by_lower_name(name: &str) -> Option<FireworkShape> {
        Self::LOWER_NAMES.iter().position(|n| *n == name).map(|i| Self::ALL[i])
    }

    pub fn by_id(id: i32) -> Option<FireworkShape> {
        usize::try_from(id).ok().and_then(FireworkShape::from_ordinal)
    }

    pub const fn id(self) -> u8 {
        self as u8
    }
}

/// The identity of an item stack (`ChunkerItemStackIdentifier`): a block (keeping its states) or a plain item.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemId {
    Block(BlockRef),
    Item(ItemType),
}

/// The type part of an [`ItemId`] (`ChunkerItemStackIdentifierType`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemStackType {
    Block(BlockKind),
    Item(ItemType),
}

impl ItemStackType {
    /// The block entity kind the type carries.
    pub fn block_entity(&self) -> Option<BlockEntityKind> {
        match self {
            ItemStackType::Block(k) => k.block_entity(),
            ItemStackType::Item(i) => i.block_entity(),
        }
    }
}

impl ItemId {
    pub fn item_stack_type(&self) -> ItemStackType {
        match self {
            ItemId::Block(b) => ItemStackType::Block(b.kind().clone()),
            ItemId::Item(i) => ItemStackType::Item(*i),
        }
    }

    /// Air (a block of type `AIR`) is the only "empty" item.
    pub fn is_air(&self) -> bool {
        matches!(self, ItemId::Block(b) if b.block_type() == Some(BlockType::AIR))
    }
}

impl From<ItemType> for ItemId {
    fn from(value: ItemType) -> Self {
        ItemId::Item(value)
    }
}

impl From<BlockType> for ItemId {
    fn from(value: BlockType) -> Self {
        ItemId::Block(Arc::new(BlockIdentifier::new(value)))
    }
}

impl From<BlockIdentifier> for ItemId {
    fn from(value: BlockIdentifier) -> Self {
        ItemId::Block(Arc::new(value))
    }
}

/// `ChunkerItemDisplay`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemDisplay {
    pub display_name: Option<Json>,
    pub lore: Option<Vec<Json>>,
    pub color: Option<Color>,
}

/// `ChunkerLodestoneData`.
#[derive(Clone, Debug, PartialEq)]
pub struct Lodestone {
    pub dimension: Dimension,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub tracked: bool,
}

/// `ChunkerFireworkExplosion`.
#[derive(Clone, Debug, PartialEq)]
pub struct FireworkExplosion {
    pub shape: FireworkShape,
    pub colors: Vec<Color>,
    pub fade_colors: Vec<Color>,
    pub trail: bool,
    pub twinkle: bool,
}

/// `ChunkerFireworks`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Fireworks {
    pub flight_duration: i8,
    pub explosions: Vec<FireworkExplosion>,
}

/// `ChunkerMapDecoration`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapDecoration {
    pub structure: ExplorerMap,
    pub x: f64,
    pub z: f64,
    pub rotation: f32,
}

/// `ChunkerStewEffect`: effect to duration.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StewEffect {
    pub effects: BTreeMap<EffectType, i32>,
}

/// `ChunkerTrim`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trim {
    pub material: TrimMaterial,
    pub pattern: TrimPattern,
}

/// The typed properties of an item stack (`ChunkerItemProperty`, 25 of them). `None` means the property is absent.
/// Anything an item carries that is not here is dropped, as in Chunker.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemProps {
    pub amount: Option<i32>,
    pub block_entity: Option<Box<BlockEntity>>,
    pub book_author: Option<String>,
    pub book_pages: Option<Vec<Json>>,
    pub book_title: Option<String>,
    pub can_destroy: Option<Vec<BlockRef>>,
    pub can_place_on: Option<Vec<BlockRef>>,
    pub display: Option<Box<ItemDisplay>>,
    pub durability: Option<i32>,
    pub enchantments: Option<BTreeMap<EnchantmentType, i32>>,
    pub entity: Option<Box<Entity>>,
    pub explorer_map: Option<ExplorerMap>,
    pub fireworks: Option<Box<Fireworks>>,
    pub firework_explosion: Option<Box<FireworkExplosion>>,
    pub horn_instrument: Option<HornInstrument>,
    pub map_decoration: Option<MapDecoration>,
    pub map_index: Option<i32>,
    pub ominous_bottle_amplifier: Option<i32>,
    pub potion: Option<PotionType>,
    pub repair_cost: Option<i32>,
    pub spawn_egg_mob: Option<EntityType>,
    pub stew_effects: Option<StewEffect>,
    pub trim: Option<Trim>,
    pub bundle_contents: Option<Vec<ItemStack>>,
    pub lodestone_data: Option<Box<Lodestone>>,
}

impl ItemProps {
    /// The property names, in `ChunkerItemProperty` declaration order.
    pub const NAMES: [&'static str; 25] = [
        "amount",
        "block_entity",
        "book_author",
        "book_pages",
        "book_title",
        "can_destroy",
        "can_place_on",
        "display",
        "durability",
        "enchantments",
        "entity",
        "explorer_map",
        "fireworks",
        "firework_explosion",
        "horn_instrument",
        "map_decoration",
        "map_index",
        "ominous_bottle_amplifier",
        "potion",
        "repair_cost",
        "spawn_egg_mob",
        "stew_effects",
        "trim",
        "bundle_contents",
        "lodestone_data",
    ];

    /// Whether the property with the given name is set.
    pub fn has(&self, name: &str) -> Option<bool> {
        Some(match name {
            "amount" => self.amount.is_some(),
            "block_entity" => self.block_entity.is_some(),
            "book_author" => self.book_author.is_some(),
            "book_pages" => self.book_pages.is_some(),
            "book_title" => self.book_title.is_some(),
            "can_destroy" => self.can_destroy.is_some(),
            "can_place_on" => self.can_place_on.is_some(),
            "display" => self.display.is_some(),
            "durability" => self.durability.is_some(),
            "enchantments" => self.enchantments.is_some(),
            "entity" => self.entity.is_some(),
            "explorer_map" => self.explorer_map.is_some(),
            "fireworks" => self.fireworks.is_some(),
            "firework_explosion" => self.firework_explosion.is_some(),
            "horn_instrument" => self.horn_instrument.is_some(),
            "map_decoration" => self.map_decoration.is_some(),
            "map_index" => self.map_index.is_some(),
            "ominous_bottle_amplifier" => self.ominous_bottle_amplifier.is_some(),
            "potion" => self.potion.is_some(),
            "repair_cost" => self.repair_cost.is_some(),
            "spawn_egg_mob" => self.spawn_egg_mob.is_some(),
            "stew_effects" => self.stew_effects.is_some(),
            "trim" => self.trim.is_some(),
            "bundle_contents" => self.bundle_contents.is_some(),
            "lodestone_data" => self.lodestone_data.is_some(),
            _ => return None,
        })
    }

    /// True when no property is set.
    pub fn is_empty(&self) -> bool {
        *self == ItemProps::default()
    }
}

/// An item stack (`ChunkerItemStack`): identity, an optional preserved identifier for custom items, and the typed
/// properties.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub id: ItemId,
    pub preserved: Option<Arc<PreservedIdentifier>>,
    pub props: ItemProps,
}

impl ItemStack {
    pub fn new(id: impl Into<ItemId>) -> ItemStack {
        ItemStack { id: id.into(), preserved: None, props: ItemProps::default() }
    }

    /// A stack with an amount.
    pub fn with_amount(id: impl Into<ItemId>, amount: i32) -> ItemStack {
        let mut stack = ItemStack::new(id);
        stack.props.amount = Some(amount);
        stack
    }

    pub fn is_air(&self) -> bool {
        self.id.is_air()
    }
}
