// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! What every handler needs: the item writer and its hooks, the format and the game version.

use serde_json::Value as Json;

use super::JavaItems;
use crate::convert::java::format::JavaFormat;
use crate::convert::java::resolvers::itemstack::ItemHooks;
use crate::convert::model::{ItemId, ItemStack};
use crate::convert::nbt::{Compound, Tag};
use crate::convert::util::{Version, json_text};

pub(super) struct Ctx<'a> {
    pub items: &'a JavaItems,
    /// Passed to every item written (map ids, the block entity data of items inside containers).
    pub hooks: &'a dyn ItemHooks,
    pub version: Version,
}

impl<'a> Ctx<'a> {
    pub fn with_hooks(items: &'a JavaItems, format: &'a JavaFormat, hooks: &'a dyn ItemHooks) -> Ctx<'a> {
        Ctx { items, hooks, version: format.version }
    }

    /// `version >= major.minor.patch`.
    pub fn since(&self, major: i32, minor: i32, patch: i32) -> bool {
        self.version.is_greater_than_or_equal(major, minor, patch)
    }

    /// A text component as the version stores it (a JSON string before 1.21.5, NBT after).
    pub fn text(&self, json: &Json) -> Option<Tag> {
        json_text::to_nbt_for(json, self.version).ok()
    }

    /// A list of text components padded to `minimum` entries.
    pub fn text_list(&self, lines: &[Json], minimum: usize) -> Option<Tag> {
        json_text::to_nbt_list(lines, minimum, self.version).ok()
    }

    /// An item compound, `None` when the stack is air or has no Java form.
    pub fn item(&self, stack: &ItemStack) -> Option<Compound> {
        if stack.is_air() {
            return None;
        }
        self.items.write_with(stack, self.hooks)
    }

    /// The item id string (`minecraft:brick`) of an identity.
    pub fn item_id(&self, id: &ItemId) -> Option<String> {
        self.items.write(&ItemStack::new(id.clone()))?.get_str("id").map(str::to_owned)
    }
}
