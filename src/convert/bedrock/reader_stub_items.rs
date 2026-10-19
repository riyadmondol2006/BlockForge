// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! STUB of `bedrock::resolvers::{itemstack, block_entity}` until the items track is merged: the same signatures,
//! a few typed block entities and `Unknown` for the rest. Delete this file (and its `mod` line in `reader.rs`)
//! when merging `conv-items-bedrock`.

use crate::convert::bedrock::data_version::BedrockVersion;
use crate::convert::model::blockentity::{CommandBlock, RandomContainer, Sign};
use crate::convert::model::{BlockEntity, BlockEntityData, ItemStack};
use crate::convert::nbt::{Compound, Tag};

/// `bedrock::resolvers::itemstack::BedrockItems`.
pub struct BedrockItems {
    _version: &'static BedrockVersion,
}

impl BedrockItems {
    pub fn new(version: &'static BedrockVersion) -> BedrockItems {
        BedrockItems { _version: version }
    }

    pub fn read(&self, _tag: &Compound) -> Option<ItemStack> {
        None
    }
}

fn int(tag: &Compound, key: &str) -> i32 {
    match tag.get(key) {
        Some(Tag::Int(v)) => *v,
        _ => 0,
    }
}

/// `bedrock::resolvers::block_entity::read_block_entity`.
pub fn read_block_entity(_items: &BedrockItems, tag: &Compound, keep_unknown: bool) -> Option<BlockEntity> {
    let id = tag.get_str("id")?;
    let data = match id {
        "CommandBlock" => BlockEntityData::CommandBlock(CommandBlock::default()),
        "Chest" => BlockEntityData::Chest(RandomContainer::default()),
        "Sign" => BlockEntityData::Sign(Box::default()),
        "HangingSign" => BlockEntityData::HangingSign(Box::<Sign>::default()),
        other => BlockEntityData::Unknown(other.into()),
    };
    let mut be = BlockEntity::new(int(tag, "x"), int(tag, "y"), int(tag, "z"), data);
    if keep_unknown {
        be.original_nbt = Some(tag.clone());
    }
    Some(be)
}
