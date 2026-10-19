// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Local stand-in for `resolvers::itemstack::JavaItems` until the items-java branch is merged.

use crate::convert::java::format::JavaFormat;
use crate::convert::model::ItemStack;
use crate::convert::nbt::Compound;
use crate::convert::util::Version;

pub struct JavaItems;

impl JavaItems {
    pub fn new(_format: &JavaFormat, _version: Version) -> JavaItems {
        JavaItems
    }

    pub fn write(&self, _stack: &ItemStack) -> Option<Compound> {
        None
    }
}
