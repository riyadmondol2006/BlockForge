// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Stand-in for `java::resolvers::itemstack::JavaItems` (owner: items-java) until that branch is merged. It has the
//! same constructor and `write` signature; the item compound is only `{id, Count}` / `{id, count}`.

use crate::convert::java::format::{ItemFormat, JavaFormat};
use crate::convert::model::{ItemId, ItemStack};
use crate::convert::nbt::Compound;
use crate::convert::util::Version;

pub struct JavaItems {
    components: bool,
}

impl JavaItems {
    pub fn new(format: &JavaFormat, _version: Version) -> JavaItems {
        JavaItems { components: format.items == ItemFormat::Components }
    }

    pub fn write(&self, stack: &ItemStack) -> Option<Compound> {
        let name = match &stack.id {
            ItemId::Item(item) => item.name().to_ascii_lowercase(),
            ItemId::Block(block) => block.block_type()?.name().to_ascii_lowercase(),
        };
        let amount = stack.props.amount.unwrap_or(1);
        let mut out = Compound::new();
        out.put("id", format!("minecraft:{name}"));
        if self.components {
            out.put("count", amount);
        } else {
            out.put("Count", amount as i8);
        }
        Some(out)
    }
}
