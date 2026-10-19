// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Bedrock item stacks: one item compound (`Name`, `Count`, `Damage`, `tag`, `Block`, ...) to and from the model
//! [`ItemStack`] (`BedrockItemStackResolver`, `BedrockLegacyItemStackResolver` and the identifier resolvers around
//! them).

mod pivot;
mod props;
mod tables;
mod write;

use std::sync::Arc;

use crate::convert::bedrock::data_version::BedrockVersion;
use crate::convert::mapping::{BlockResolver, Edition, ItemResolver, StateMap, StateValue, Version};
use crate::convert::model::item::Lodestone;
use crate::convert::model::{BlockIdentifier, ItemId, ItemStack, StateId};
use crate::convert::nbt::Compound;

use pivot::{block_from_pivot, item_from_pivot, model_value, states_of_compound};
pub(crate) use tables::Tables;

/// What was being looked up when a mapping was missing (`Converter.MissingMappingType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Missing {
    Block,
    Item,
    Enchantment,
    EntityType,
    TrimPattern,
    TrimMaterial,
    BannerPattern,
    Potion,
    Effect,
}

type LodestoneLookup = Arc<dyn Fn(i32) -> Option<Lodestone> + Send + Sync>;
type MapLookup = Arc<dyn Fn(i64) -> Option<i32> + Send + Sync>;
type MissingLogger = Arc<dyn Fn(Missing, &str) + Send + Sync>;

#[derive(Clone, Default)]
struct Hooks {
    lodestone: Option<LodestoneLookup>,
    map_index: Option<MapLookup>,
    missing: Option<MissingLogger>,
}

/// Reads (and best effort writes) Bedrock item compounds for one Bedrock version.
pub struct BedrockItems {
    version: &'static BedrockVersion,
    pub(crate) tables: Tables,
    items: ItemResolver,
    blocks: BlockResolver,
    allow_custom: bool,
    keep_nbt: bool,
    loot_tables: bool,
    hooks: Hooks,
}

impl BedrockItems {
    /// Items for `version`; unknown namespaced identifiers pass through as custom ones and original block entity
    /// NBT is not kept (see [`BedrockItems::with_custom_identifiers`], [`BedrockItems::with_original_nbt`]).
    pub fn new(version: &'static BedrockVersion) -> BedrockItems {
        let game = Version::new(version.game[0] as u32, version.game[1] as u32, version.game[2] as u32);
        BedrockItems {
            version,
            tables: Tables::load(game),
            items: ItemResolver::new(Edition::Bedrock, game),
            blocks: BlockResolver::new(Edition::Bedrock, game),
            allow_custom: true,
            keep_nbt: false,
            loot_tables: true,
            hooks: Hooks::default(),
        }
    }

    /// Whether unknown namespaced identifiers become custom blocks / entity types (`shouldAllowCustomIdentifiers`).
    pub fn with_custom_identifiers(mut self, allowed: bool) -> BedrockItems {
        self.allow_custom = allowed;
        self
    }

    /// Whether block entities read from item NBT keep their original tag (`shouldAllowNBTCopying`).
    pub fn with_original_nbt(mut self, keep: bool) -> BedrockItems {
        self.keep_nbt = keep;
        self
    }

    /// Whether loot tables are read from containers (`shouldProcessLootTables`, on by default).
    pub fn with_loot_tables(mut self, enabled: bool) -> BedrockItems {
        self.loot_tables = enabled;
        self
    }

    /// The lookup of the tracked lodestone positions by `trackingHandle` (the level's `PosTrackDB`).
    pub fn with_lodestones(mut self, f: impl Fn(i32) -> Option<Lodestone> + Send + Sync + 'static) -> BedrockItems {
        self.hooks.lodestone = Some(Arc::new(f));
        self
    }

    /// The lookup of the map index by the original `map_uuid` (`findMapIndexByOriginalID`).
    pub fn with_map_indexes(mut self, f: impl Fn(i64) -> Option<i32> + Send + Sync + 'static) -> BedrockItems {
        self.hooks.map_index = Some(Arc::new(f));
        self
    }

    /// A callback told about every missing mapping (`logMissingMapping`).
    pub fn with_missing_logger(mut self, f: impl Fn(Missing, &str) + Send + Sync + 'static) -> BedrockItems {
        self.hooks.missing = Some(Arc::new(f));
        self
    }

    pub fn version(&self) -> &'static BedrockVersion {
        self.version
    }

    pub(crate) fn keep_original_nbt(&self) -> bool {
        self.keep_nbt
    }

    pub(crate) fn loot_tables_enabled(&self) -> bool {
        self.loot_tables
    }

    pub(crate) fn custom_allowed(&self) -> bool {
        self.allow_custom
    }

    /// Report a missing mapping.
    pub(crate) fn missing(&self, kind: Missing, what: &str) {
        if let Some(log) = &self.hooks.missing {
            log(kind, what);
        }
    }

    /// One item compound (`Name`, `Count`, `Damage`, `tag`, `Block`; `Slot` is for the caller) to the model.
    ///
    /// `None` when the compound is not an item or the item has no equivalent (and is not a custom identifier); air
    /// is returned as an air stack. Properties follow Chunker's handler order.
    pub fn read(&self, tag: &Compound) -> Option<ItemStack> {
        let mut stack = self.create_holder(tag)?;
        if !self.read_props(tag, &mut stack) {
            self.missing(Missing::Item, &format!("{tag:?}"));
            return None;
        }
        Some(stack)
    }

    /// The identity of the stack (`createPropertyHolder`).
    fn create_holder(&self, tag: &Compound) -> Option<ItemStack> {
        let (name, states): (String, StateMap) = if let Some(block) = tag.get_compound("Block") {
            // A block item: the block compound is the identifier, with waterlogged off.
            let name = block.get_str("name")?.to_string();
            let mut states = states_of_compound(block.get_compound("states"));
            states.insert(Arc::from("waterlogged"), StateValue::Bool(false));
            (name, states)
        } else if tag.contains_key("Name") {
            let mut name = tag.get_str("Name").unwrap_or_default().to_string();
            if name.is_empty() {
                // Air in a player inventory can be an empty name.
                name = "minecraft:air".to_string();
            }
            let mut states = StateMap::new();
            states.insert(Arc::from("data"), StateValue::Int(tag.get_i16_or("Damage", 0) as i32));
            (name, states)
        } else if tag.is_empty() && self.version.at_least([1, 13, 0]) {
            // Empty armour and off-hand slots are an empty compound.
            ("minecraft:air".to_string(), StateMap::new())
        } else {
            return None;
        };
        let stack = self.read_identifier(&name, &states);
        if stack.is_none() {
            self.missing(Missing::Item, &name);
        }
        stack
    }

    /// An item identifier with its states to a stack: the item table first, then the block table
    /// (`readItemIdentifier`).
    pub(crate) fn read_identifier(&self, name: &str, states: &StateMap) -> Option<ItemStack> {
        if let Some(pivot) = self.items.to_pivot(name, states) {
            let (id, props) = item_from_pivot(&pivot)?;
            let mut stack = ItemStack::new(id);
            stack.props = props;
            return Some(stack);
        }
        let block = self.read_item_block(name, states)?;
        Some(ItemStack::new(ItemId::Block(Arc::new(block))))
    }

    /// A block identifier as used by items (`readItemBlockIdentifier`): the block table, else a custom block.
    pub(crate) fn read_item_block(&self, name: &str, states: &StateMap) -> Option<BlockIdentifier> {
        let mut input = states.clone();
        input.entry(Arc::from("waterlogged")).or_insert(StateValue::Bool(false));
        if let Some(block) = self.blocks.to_pivot(name, &input).as_ref().and_then(block_from_pivot) {
            return Some(block);
        }
        if self.allow_custom && name.contains(':') && !name.starts_with("minecraft:") {
            let custom = states.iter().map(|(k, v)| (k.clone(), model_value(v)));
            return Some(BlockIdentifier::custom(name, custom));
        }
        self.missing(Missing::Block, name);
        None
    }

    /// A block palette compound (`{name, states}`) as a block, for block entities that hold a block.
    pub(crate) fn read_block_compound(&self, tag: &Compound, waterlogged: bool) -> Option<BlockIdentifier> {
        let name = tag.get_str("name")?;
        let mut states = states_of_compound(tag.get_compound("states"));
        states.insert(Arc::from("waterlogged"), StateValue::Bool(waterlogged));
        self.read_item_block(name, &states)
    }

    /// The edition item or block of a model item, for writing.
    pub(crate) fn resolvers(&self) -> (&ItemResolver, &BlockResolver) {
        (&self.items, &self.blocks)
    }
}

/// Remove the waterlogged state (`copyWithout(WATERLOGGED)`).
pub(crate) fn without_waterlogged(block: BlockIdentifier) -> BlockIdentifier {
    let mut block = block;
    let mut states = block.states().clone();
    states.remove(StateId::WATERLOGGED);
    block = BlockIdentifier::from_parts(block.kind().clone(), states, block.preserved().cloned());
    block
}

#[cfg(test)]
mod tests;
