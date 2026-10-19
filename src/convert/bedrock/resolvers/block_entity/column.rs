// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The column step of the Bedrock block entity handlers (`updateBeforeProcess` and `shouldRemoveBeforeProcess`):
//! block entity data that lives in block states in the model is applied to the blocks of the column.

use std::sync::Arc;

use crate::convert::model::states;
use crate::convert::model::{
    BlockEntity, BlockEntityData, BlockIdentifier, BlockKind, BlockType, Bool, ChestType, Column, Entity, EntityData,
    Note, Rotation, StateId,
};
use crate::convert::model::entity::{ItemFrame, Tile};
use crate::convert::nbt::Compound;

use super::{BedrockOnly, BedrockOnlyKind, Extra, Read, block_entity_id, read_block_entity_full};
use crate::convert::bedrock::resolvers::itemstack::{BedrockItems, Missing};

/// Plants that have a potted block (`BedrockFlowerPotBlockEntityHandler.POTTED_TO_PLANT`).
const POTTED_PLANTS: &[&str] = &[
    "TORCHFLOWER",
    "OAK_SAPLING",
    "SPRUCE_SAPLING",
    "BIRCH_SAPLING",
    "JUNGLE_SAPLING",
    "ACACIA_SAPLING",
    "CHERRY_SAPLING",
    "DARK_OAK_SAPLING",
    "PALE_OAK_SAPLING",
    "POPLAR_SAPLING",
    "MANGROVE_PROPAGULE",
    "FERN",
    "DANDELION",
    "POPPY",
    "BLUE_ORCHID",
    "ALLIUM",
    "AZURE_BLUET",
    "RED_TULIP",
    "ORANGE_TULIP",
    "WHITE_TULIP",
    "PINK_TULIP",
    "OXEYE_DAISY",
    "CORNFLOWER",
    "LILY_OF_THE_VALLEY",
    "WITHER_ROSE",
    "RED_MUSHROOM",
    "BROWN_MUSHROOM",
    "DEAD_BUSH",
    "CACTUS",
    "BAMBOO",
    "CRIMSON_FUNGUS",
    "CRIMSON_ROOTS",
    "WARPED_FUNGUS",
    "WARPED_ROOTS",
    "AZALEA",
    "FLOWERING_AZALEA",
    "OPEN_EYEBLOSSOM",
    "CLOSED_EYEBLOSSOM",
    "GOLDEN_DANDELION",
];

/// The potted block of a plant block type.
fn potted_of(plant: BlockType) -> Option<BlockType> {
    let name = plant.name();
    if !POTTED_PLANTS.contains(&name) {
        return None;
    }
    let potted = match name {
        "AZALEA" => "POTTED_AZALEA_BUSH".to_string(),
        "FLOWERING_AZALEA" => "POTTED_FLOWERING_AZALEA_BUSH".to_string(),
        other => format!("POTTED_{other}"),
    };
    BlockType::from_name(&potted)
}

/// Skull types as stored by Bedrock: (type id, standing block, wall block).
const SKULLS: [(i8, &str, &str); 7] = [
    (0, "SKELETON_SKULL", "SKELETON_WALL_SKULL"),
    (1, "WITHER_SKELETON_SKULL", "WITHER_SKELETON_WALL_SKULL"),
    (2, "ZOMBIE_HEAD", "ZOMBIE_WALL_HEAD"),
    (3, "PLAYER_HEAD", "PLAYER_WALL_HEAD"),
    (4, "CREEPER_HEAD", "CREEPER_WALL_HEAD"),
    (5, "DRAGON_HEAD", "DRAGON_WALL_HEAD"),
    (6, "PIGLIN_HEAD", "PIGLIN_WALL_HEAD"),
];

/// Read every block entity compound of a column, as `BedrockColumnReader.readBlockEntity` and its post processing
/// do: the model entities are added to `column.block_entities` after the block states they imply were applied, and
/// item frames become entities of the column. The ids that could not be read are returned.
pub fn read_into_column(items: &BedrockItems, tags: &[Compound], column: &mut Column, keep_unknown: bool) -> Vec<String> {
    let mut missing = Vec::new();
    let mut entities = Vec::new();
    let mut bedrock_only = Vec::new();
    for tag in tags {
        match read_block_entity_full(items, tag, keep_unknown) {
            Some(Read::Entity { entity, extra }) => entities.push((entity, extra)),
            Some(Read::BedrockOnly(only)) => bedrock_only.push(only),
            None => {
                let id = block_entity_id(tag).map_or_else(|| format!("{tag:?}"), str::to_string);
                items.missing(Missing::Block, &id);
                missing.push(id);
            }
        }
    }
    for (entity, extra) in entities {
        let entity = update_before_process(column, entity, extra);
        column.block_entities.push(entity);
    }
    for only in bedrock_only {
        remove_before_process(column, only);
    }
    missing
}

fn replace_type(block: &BlockIdentifier, ty: BlockType) -> BlockIdentifier {
    BlockIdentifier::from_parts(BlockKind::Vanilla(ty), block.states().clone(), block.preserved().cloned())
}

/// `updateBeforeProcess` of the handler that read `entity`.
fn update_before_process(column: &mut Column, mut entity: BlockEntity, extra: Extra) -> BlockEntity {
    let (x, y, z) = (entity.x, entity.y, entity.z);
    let block = column.block(x, y, z).clone();
    match (&mut entity.data, extra) {
        (BlockEntityData::Chest(chest), Extra::Chest { pair, .. }) => {
            let direction = block.get(states::FACING_HORIZONTAL);
            let chest_type = match (pair, direction) {
                (Some((pair_x, pair_z)), Some(direction)) => {
                    let left = direction.rotate_clockwise();
                    let right = direction.rotate_anti_clockwise();
                    if x + left.x() == pair_x && z + left.z() == pair_z {
                        ChestType::LEFT
                    } else if x + right.x() == pair_x && z + right.z() == pair_z {
                        ChestType::RIGHT
                    } else {
                        ChestType::SINGLE
                    }
                }
                _ => ChestType::SINGLE,
            };
            if !block.is_air() && block.kind().supports(StateId::CHEST_TYPE) {
                column.set_block(x, y, z, Arc::new(block.with(states::CHEST_TYPE, chest_type)));
            }
            if block.block_type() == Some(BlockType::TRAPPED_CHEST) {
                entity.data = BlockEntityData::TrappedChest(std::mem::take(chest));
            }
        }
        (BlockEntityData::Bed, Extra::Bed { color }) => {
            if !block.is_air()
                && let Some(ty) = BlockType::from_name(&format!("{}_BED", color.name()))
            {
                column.set_block(x, y, z, Arc::new(replace_type(&block, ty)));
            }
        }
        (BlockEntityData::Banner(banner), _) => {
            if let Some(base) = banner.base {
                let wall = block.kind().supports(StateId::FACING_HORIZONTAL);
                let name = format!("{}{}_BANNER", base.name(), if wall { "_WALL" } else { "" });
                let ty = BlockType::from_name(&name).unwrap_or(BlockType::WHITE_BANNER);
                if !block.is_air() {
                    column.set_block(x, y, z, Arc::new(replace_type(&block, ty)));
                }
                banner.base = None;
            }
        }
        (BlockEntityData::Skull(_), Extra::Skull { skull_type, rotation, moving_mouth }) => {
            if !block.is_air() {
                let wall = block.kind().supports(StateId::FACING_HORIZONTAL);
                let new_type = SKULLS
                    .iter()
                    .find(|(id, _, _)| *id == skull_type)
                    .and_then(|(_, standing, wall_name)| BlockType::from_name(if wall { wall_name } else { standing }));
                let mut new_block = match new_type {
                    Some(ty) => replace_type(&block, ty),
                    None => (*block).clone(),
                };
                if !wall {
                    let ordinal = (rotation as i32).rem_euclid(Rotation::COUNT as i32) as usize;
                    new_block.set(states::ROTATION, Rotation::ALL[ordinal]);
                }
                new_block.set(states::POWERED, if moving_mouth { Bool::TRUE } else { Bool::FALSE });
                column.set_block(x, y, z, Arc::new(new_block));
            }
        }
        (BlockEntityData::ShulkerBox(_), Extra::Shulker { facing }) => {
            if !block.is_air() {
                column.set_block(x, y, z, Arc::new(block.with(states::FACING_ALL, facing)));
            }
        }
        (BlockEntityData::CopperGolemStatue, Extra::CopperGolem { pose }) => {
            if !block.is_air() {
                column.set_block(x, y, z, Arc::new(block.with(states::COPPER_GOLEM_POSE, pose)));
            }
        }
        (BlockEntityData::Jukebox { record }, _) => {
            if block.block_type() == Some(BlockType::JUKEBOX) {
                let has = record.as_ref().is_some_and(|r| !r.is_air());
                column.set_block(x, y, z, Arc::new(block.with(states::HAS_RECORD, if has { Bool::TRUE } else { Bool::FALSE })));
            }
        }
        (BlockEntityData::Lectern { book, .. }, _) => {
            if block.block_type() == Some(BlockType::LECTERN) {
                let has = book.as_ref().is_some_and(|b| !b.is_air());
                column.set_block(x, y, z, Arc::new(block.with(states::HAS_BOOK, if has { Bool::TRUE } else { Bool::FALSE })));
            }
        }
        _ => {}
    }
    entity
}

/// `shouldRemoveBeforeProcess` of the Bedrock only block entities: they update the block (or become an entity) and
/// are then dropped.
fn remove_before_process(column: &mut Column, only: BedrockOnly) {
    let BedrockOnly { x, y, z, kind } = only;
    let block = column.block(x, y, z).clone();
    match kind {
        BedrockOnlyKind::PistonArm { extended, .. } => {
            if matches!(block.block_type(), Some(BlockType::PISTON | BlockType::STICKY_PISTON)) {
                let value = if extended { Bool::TRUE } else { Bool::FALSE };
                column.set_block(x, y, z, Arc::new(block.with(states::EXTENDED, value)));
            }
        }
        BedrockOnlyKind::NoteBlock { note } => {
            if let Some(note) = note.and_then(|n| Note::from_ordinal(n as usize))
                && block.block_type() == Some(BlockType::NOTE_BLOCK)
            {
                column.set_block(x, y, z, Arc::new(block.with(states::NOTE, note)));
            }
        }
        BedrockOnlyKind::ItemFrame { glow, item, rotation } => {
            if block.block_type() == Some(BlockType::ITEM_FRAME_BEDROCK) {
                let frame = ItemFrame {
                    tile: Tile { x, y, z },
                    glow,
                    item: item.map(Box::new),
                    direction: block.get(states::FACING_ALL),
                    item_rotation: rotation,
                    map: block.get(states::BEDROCK_FRAME_MAP_BIT) == Some(Bool::TRUE),
                    photo: block.get(states::BEDROCK_FRAME_PHOTO_BIT) == Some(Bool::TRUE),
                };
                let mut entity = Entity::new(EntityData::ItemFrame(frame));
                entity.position = [x as f64, y as f64, z as f64];
                column.entities.push(entity);
                column.set_block(x, y, z, crate::convert::model::air().clone());
            }
        }
        BedrockOnlyKind::FlowerPot { plant } => {
            if let Some(plant) = plant.filter(|p| !p.is_air())
                && let Some(potted) = plant.block_type().and_then(potted_of)
            {
                let waterlogged = block.get(states::WATERLOGGED).unwrap_or(Bool::FALSE);
                let mut set = crate::convert::model::StateSet::new();
                set.set(states::WATERLOGGED, waterlogged);
                column.set_block(x, y, z, Arc::new(BlockIdentifier::with_states(potted, set)));
            }
        }
        BedrockOnlyKind::Marker(_) => {}
    }
}
