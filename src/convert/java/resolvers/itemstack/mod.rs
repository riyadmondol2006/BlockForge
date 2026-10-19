// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Java item stacks: model [`ItemStack`] to the item compound of a Java version.
//!
//! This is `JavaItemStackResolver` (1.13 - 1.20.4, a `tag` compound), `JavaComponentItemStackResolver` (1.20.5+,
//! `components`) and the item identifier logic of `JavaResolversBuilder`, folded into one [`JavaItems`] chosen by
//! [`JavaFormat::items`]. The properties are written in Chunker's handler order and only the 25 typed properties
//! of the model exist, everything else a Java item carries is not converted (as in Chunker).
//!
//! Gaps this wave: before 1.13 (`ItemFormat::Legacy`, numeric ids and `Damage`) `write` returns `None`; `read`
//! (Java to model) is a stub that returns `None`.
//!
//! Anything that needs another resolver comes in through [`ItemHooks`]: block entity data (banners, skulls,
//! shulker boxes ...), entity data (paintings) and the renumbered map ids. [`JavaItems::write`] passes no hooks, so
//! stacks carrying those properties lose them.

mod components;
mod lookup;
mod tag;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::convert::java::format::{ItemFormat, JavaFormat};
use crate::convert::mapping::{ItemKind, PivotItem, Version as MapVersion};
use crate::convert::model::{BlockEntity, Color, Entity, ItemId, ItemStack, PreservedIdentifier};
use crate::convert::nbt::Compound;
use crate::convert::util::{self, Version};

pub use lookup::Lookups;

/// What an item writer asks of the other resolvers. Every method has a "nothing" default.
pub trait ItemHooks {
    /// The data of the block entity of an item (`ItemProps::block_entity`): Chunker's `updateBeforeWrite`,
    /// `writeToItemNBT` and the block entity writer in one. `item` is the item compound being built (`tag` or
    /// `components` are created lazily inside it), so handlers that store data as components (banner patterns,
    /// skull profiles, containers) edit it directly. Return the block entity compound to store as `BlockEntityTag`
    /// / `minecraft:block_entity_data`, or `None` to store none. `x`, `y`, `z`, rotation and facing (and `id` in the
    /// `tag` format) are removed from it afterwards.
    fn block_entity(&self, _stack: &ItemStack, _entity: &BlockEntity, _item: &mut Compound) -> Option<Compound> {
        None
    }

    /// The entity compound of an item (`ItemProps::entity`, paintings): the normal entity writer's output; position,
    /// motion, rotation and facing are removed afterwards.
    fn entity(&self, _entity: &Entity) -> Option<Compound> {
        None
    }

    /// The map id on disk of a model map index (`ItemProps::map_index`), `None` drops the property.
    fn map_id(&self, _index: i32) -> Option<i32> {
        None
    }
}

/// No hooks.
pub struct NoHooks;

impl ItemHooks for NoHooks {}

/// Hooks made of a block entity closure `(stack, block entity, item) -> block entity compound`.
pub struct BlockEntityFn<F>(pub F);

impl<F: Fn(&ItemStack, &BlockEntity, &mut Compound) -> Option<Compound>> ItemHooks for BlockEntityFn<F> {
    fn block_entity(&self, stack: &ItemStack, entity: &BlockEntity, item: &mut Compound) -> Option<Compound> {
        (self.0)(stack, entity, item)
    }
}

/// Writes item stacks for one Java version.
#[derive(Clone)]
pub struct JavaItems {
    format: ItemFormat,
    /// The game version table gates are tested against.
    version: Version,
    lookups: Arc<Lookups>,
}

impl JavaItems {
    /// Items for `format` at game `version` (the registered version a data version floors to, as Chunker passes it).
    pub fn new(format: &JavaFormat, version: Version) -> JavaItems {
        let map_version =
            MapVersion::new(version.major.max(0) as u32, version.minor.max(0) as u32, version.patch.max(0) as u32);
        JavaItems { format: format.items, version, lookups: Lookups::get(map_version) }
    }

    /// The storage format of the version.
    pub fn format(&self) -> ItemFormat {
        self.format
    }

    /// The game version.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The model stack as an item compound `{id, count/Count, components or tag}` of this version, without hooks.
    /// `None` when the stack cannot be written: before 1.13, an item or block without a Java id, a spawn egg of a
    /// mob without one.
    pub fn write(&self, stack: &ItemStack) -> Option<Compound> {
        self.write_with(stack, &NoHooks)
    }

    /// [`JavaItems::write`] with hooks for block entity data, entities and map ids.
    pub fn write_with(&self, stack: &ItemStack, hooks: &dyn ItemHooks) -> Option<Compound> {
        match self.format {
            ItemFormat::Legacy => None,
            ItemFormat::Tag => tag::write(self, stack, hooks),
            ItemFormat::Components => components::write(self, stack, hooks),
        }
    }

    /// Java item compound to model stack. Not implemented in this wave: always `None`.
    pub fn read(&self, _tag: &Compound) -> Option<ItemStack> {
        None
    }

    /// The item id of a stack (`createOutput`): the item table, else the block table.
    pub fn item_id(&self, stack: &ItemStack) -> Option<String> {
        // A custom mapping carried from the reader replaces the id.
        if let Some(PreservedIdentifier { from_reader: true, identifier }) = stack.preserved.as_deref() {
            return Some(identifier.name.to_string());
        }
        match &stack.id {
            ItemId::Item(ty) => {
                let mut props: Vec<(&str, &str)> = Vec::new();
                if let Some(crate::convert::model::EntityType::Vanilla(mob)) = &stack.props.spawn_egg_mob {
                    props.push(("P:spawn_egg_mob", mob.name()));
                }
                if let Some(map) = stack.props.explorer_map {
                    props.push(("P:explorer_map", map.name()));
                }
                let pivot = PivotItem::new(ItemKind::Item(ty.name().into()), &props);
                self.lookups.items.from_pivot(&pivot).map(|(id, _)| id.to_string())
            }
            ItemId::Block(block) => self.lookups.block_id(block),
        }
    }

    pub(crate) fn lookups(&self) -> &Lookups {
        &self.lookups
    }

    pub(crate) fn at_least(&self, major: i32, minor: i32, patch: i32) -> bool {
        self.version.is_greater_than_or_equal(major, minor, patch)
    }
}

/// `color & 0xFFFFFF` of each colour, as the firework tags store them.
pub(crate) fn rgb_ints(colors: &[Color]) -> Vec<i32> {
    colors.iter().map(|c| c.rgb() as i32).collect()
}

/// The text of a writable book page (legacy formatting codes), empty when the text cannot be flattened.
pub(crate) fn legacy_text(page: &serde_json::Value) -> String {
    util::json_text::to_legacy(page, false).unwrap_or_default()
}
