// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Local stand-in for `resolvers::block_entity::write_block_entity` until the be-java branch is merged.

use crate::convert::java::format::JavaFormat;
use super::stub_items::JavaItems;
use crate::convert::model::BlockEntity;
use crate::convert::nbt::{Compound, Tag};

pub fn write_block_entity(_items: &JavaItems, _format: &JavaFormat, be: &BlockEntity) -> Option<Compound> {
    let mut c = Compound::new();
    c.insert("id", Tag::String(format!("minecraft:{}", format!("{:?}", be.kind()).to_lowercase())));
    c.insert("x", Tag::Int(be.x));
    c.insert("y", Tag::Int(be.y));
    c.insert("z", Tag::Int(be.z));
    Some(c)
}
