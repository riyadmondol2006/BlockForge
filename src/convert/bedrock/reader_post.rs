// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Block entities of a column and the fix-ups that tie them to the blocks (`BedrockColumnReader.postProcess`).
//!
//! Bedrock keeps some block data in the block entity that Java keeps in the block: the colour of a bed or banner,
//! the type and rotation of a skull, the plant of a flower pot, the note of a note block, whether a piston is
//! extended or a lectern holds a book, which side a shulker box faces, whether a chest is the left or the right half
//! of a double chest. Chunker's block entity handlers move that data into the column's blocks after reading
//! (`updateBeforeProcess` / `shouldRemoveBeforeProcess`); this module does the same, working on the raw tag where
//! the model block entity does not carry the data (it has no bed colour, for instance).
//!
//! Handled here: `Bed`, `Skull`, `Banner`, `ShulkerBox`, `Chest` (chest type and trapped chests), `Lectern`,
//! `Jukebox`, and the Bedrock-only `Music` (note blocks), `PistonArm` and `FlowerPot`, which only exist to set a
//! block and are not kept. `ItemFrame` block entities become entities in Chunker; entities are out of scope for
//! now, so they are dropped. Not handled: copper golem statue pose.

use std::sync::Arc;

use super::reader_blocks::BlockReader;
use super::items::{BedrockItems, read_block_entity};
use crate::convert::model::{
    BlockEntityData, BlockIdentifier, BlockKind, BlockType, Bool, ChestType, Column, DyeColor, FacingDirection,
    Note, Rotation, StateId, StateSet, states,
};
use crate::convert::nbt::{Compound, Tag};

/// What the block entities of a column needed from outside.
pub(super) struct PostContext<'a> {
    pub blocks: &'a BlockReader,
    pub items: &'a BedrockItems,
    pub keep_unknown: bool,
}

/// The outcome of reading the block entities of a column.
#[derive(Debug, Default)]
pub(super) struct PostReport {
    /// Ids that no handler knows (`logMissingMapping(BLOCK_ENTITY, ...)`).
    pub unmapped: Vec<String>,
}

fn retyped(block: &BlockIdentifier, ty: BlockType) -> BlockIdentifier {
    BlockIdentifier::from_parts(BlockKind::Vanilla(ty), block.states().clone(), block.preserved().cloned())
}

/// Replace the block at a world position by `f(block)` when that is a change and the block is not air.
fn update(column: &mut Column, x: i32, y: i32, z: i32, f: impl FnOnce(&BlockIdentifier) -> Option<BlockIdentifier>) {
    let current = column.block(x, y, z).clone();
    if current.is_air() {
        return;
    }
    if let Some(new) = f(&current) {
        if new != *current {
            column.set_block(x, y, z, Arc::new(new));
        }
    }
}

/// `f(block)` on the block even when it is air (jukebox, lectern, note blocks check the type themselves).
fn update_any(column: &mut Column, x: i32, y: i32, z: i32, f: impl FnOnce(&BlockIdentifier) -> Option<BlockIdentifier>) {
    let current = column.block(x, y, z).clone();
    if let Some(new) = f(&current) {
        if new != *current {
            column.set_block(x, y, z, Arc::new(new));
        }
    }
}

fn has_item(tag: &Compound, key: &str) -> bool {
    tag.get_compound(key).is_some_and(|c| c.get_str("Name").is_some_and(|n| n != "minecraft:air" && !n.is_empty()))
}

fn dye(index: i32) -> DyeColor {
    usize::try_from(index).ok().and_then(DyeColor::from_ordinal).unwrap_or(DyeColor::WHITE)
}

/// The skull block types by `(SkullType, wall)`.
fn skull_type(kind: i8, wall: bool) -> Option<BlockType> {
    let name = match kind {
        0 => "SKELETON",
        1 => "WITHER_SKELETON",
        2 => "ZOMBIE",
        3 => "PLAYER",
        4 => "CREEPER",
        5 => "DRAGON",
        6 => "PIGLIN",
        _ => return None,
    };
    let suffix = if matches!(kind, 0 | 1) { "SKULL" } else { "HEAD" };
    let full = if wall { format!("{name}_WALL_{suffix}") } else { format!("{name}_{suffix}") };
    // Chunker names zombie/player/... walls `ZOMBIE_WALL_HEAD` and skeleton walls `SKELETON_WALL_SKULL`.
    BlockType::from_name(&full)
}

/// The skull rotation (0..16): `Rotation` is a float in degrees, older worlds have the byte `Rot`.
fn skull_rotation(tag: &Compound) -> i32 {
    match tag.get("Rotation") {
        Some(Tag::Float(f)) => (((*f + 360.0) % 360.0) / 22.5).round() as i32,
        Some(Tag::Double(f)) => (((*f as f32 + 360.0) % 360.0) / 22.5).round() as i32,
        Some(Tag::Byte(b)) => *b as i32,
        Some(Tag::Int(i)) => *i,
        _ => i32::from(tag.get_i8("Rot").unwrap_or(0)),
    }
}

fn pos(tag: &Compound) -> (i32, i32, i32) {
    (tag.get_i32("x").unwrap_or(0), tag.get_i32("y").unwrap_or(0), tag.get_i32("z").unwrap_or(0))
}

/// The potted block for a plant block type (`POTTED_TO_PLANT` inverted): `POTTED_<plant>`, with the two azaleas
/// named `..._BUSH`.
fn potted(plant: BlockType) -> Option<BlockType> {
    let name = match plant {
        BlockType::AZALEA => "POTTED_AZALEA_BUSH".to_string(),
        BlockType::FLOWERING_AZALEA => "POTTED_FLOWERING_AZALEA_BUSH".to_string(),
        other => format!("POTTED_{}", other.name()),
    };
    BlockType::from_name(&name)
}

/// Read the block entities of a column and apply the fix-ups to its blocks. Tags that fail to convert are reported
/// in [`PostReport::unmapped`].
pub(super) fn read_block_entities(ctx: &PostContext<'_>, column: &mut Column, tags: &[Compound]) -> PostReport {
    let mut report = PostReport::default();
    for tag in tags {
        let id = tag.get_str("id").unwrap_or("");
        let (x, y, z) = pos(tag);
        match id {
            // Bedrock-only block entities that only exist to set a block state.
            "PistonArm" => {
                let extended = tag.get_i8("State").unwrap_or(0) == 2;
                update_any(column, x, y, z, |b| {
                    matches!(b.block_type(), Some(BlockType::PISTON | BlockType::STICKY_PISTON))
                        .then(|| b.with(states::EXTENDED, if extended { Bool::TRUE } else { Bool::FALSE }))
                });
                continue;
            }
            "Music" => {
                let note = Note::from_ordinal(tag.get_i8("note").unwrap_or(0).max(0) as usize);
                update_any(column, x, y, z, |b| match (b.block_type(), note) {
                    (Some(BlockType::NOTE_BLOCK), Some(note)) => Some(b.with(states::NOTE, note)),
                    _ => None,
                });
                continue;
            }
            "FlowerPot" => {
                if let Some(plant) = tag.get_compound("PlantBlock") {
                    let name = plant.get_str("name").unwrap_or("minecraft:air");
                    let plant = ctx.blocks.resolve_named(name, plant.get_compound("states").unwrap_or(&Compound::new()), false);
                    if let Some(pot) = plant.block_type().filter(|t| !t.is_air()).and_then(potted) {
                        update_any(column, x, y, z, |old| {
                            let mut set = StateSet::new();
                            set.set(states::WATERLOGGED, old.get(states::WATERLOGGED).unwrap_or(Bool::FALSE));
                            Some(BlockIdentifier::with_states(pot, set))
                        });
                    }
                }
                continue;
            }
            // Item frames become entities in Chunker.
            "ItemFrame" | "GlowItemFrame" => continue,
            _ => {}
        }

        let Some(mut entity) = read_block_entity(ctx.items, tag, ctx.keep_unknown) else {
            report.unmapped.push(if id.is_empty() { format!("{tag:?}") } else { id.to_string() });
            continue;
        };

        match id {
            "Bed" => {
                let color = dye(i32::from(tag.get_i8("color").unwrap_or(0)));
                let ty = BlockType::from_name(&format!("{}_BED", color.name())).unwrap_or(BlockType::WHITE_BED);
                update(column, x, y, z, |b| Some(retyped(b, ty)));
            }
            "Skull" => {
                let kind = tag.get_i8("SkullType").unwrap_or(0);
                let rotation = skull_rotation(tag);
                let powered = tag.get_i8("DoingAnimation").unwrap_or(0) == 1;
                update(column, x, y, z, |b| {
                    let wall = b.kind().supports(StateId::FACING_HORIZONTAL);
                    let ty = skull_type(kind, wall).or(b.block_type())?;
                    let mut new = retyped(b, ty);
                    if !wall {
                        let r = Rotation::from_ordinal(rotation.rem_euclid(16) as usize)?;
                        new.set(states::ROTATION, r);
                    }
                    new.set(states::POWERED, if powered { Bool::TRUE } else { Bool::FALSE });
                    Some(new)
                });
            }
            "Banner" => {
                if let Some(base) = tag.get_i32("Base") {
                    let color = dye(15 - base);
                    update(column, x, y, z, |b| {
                        let wall = b.kind().supports(StateId::FACING_HORIZONTAL);
                        let name = format!("{}_{}BANNER", color.name(), if wall { "WALL_" } else { "" });
                        Some(retyped(b, BlockType::from_name(&name).unwrap_or(BlockType::WHITE_BANNER)))
                    });
                    // The base now lives in the block.
                    if let BlockEntityData::Banner(banner) = &mut entity.data {
                        banner.base = None;
                    }
                }
            }
            "ShulkerBox" => {
                let facing = FacingDirection::from_3d_byte(tag.get_i8("facing").unwrap_or(0));
                update(column, x, y, z, |b| Some(b.with(states::FACING_ALL, facing)));
            }
            "Lectern" => {
                let has_book = has_item(tag, "book");
                update_any(column, x, y, z, |b| {
                    (b.block_type() == Some(BlockType::LECTERN))
                        .then(|| b.with(states::HAS_BOOK, if has_book { Bool::TRUE } else { Bool::FALSE }))
                });
            }
            "Jukebox" => {
                let has_record = has_item(tag, "RecordItem");
                update_any(column, x, y, z, |b| {
                    (b.block_type() == Some(BlockType::JUKEBOX))
                        .then(|| b.with(states::HAS_RECORD, if has_record { Bool::TRUE } else { Bool::FALSE }))
                });
            }
            "Chest" => {
                let pair = match (tag.get_i32("pairx"), tag.get_i32("pairz")) {
                    (Some(px), Some(pz)) => Some((px, pz)),
                    _ => None,
                };
                update(column, x, y, z, |b| {
                    let kind = match (pair, b.get(states::FACING_HORIZONTAL)) {
                        (Some((px, pz)), Some(direction)) => {
                            let left = direction.rotate_clockwise();
                            let right = direction.rotate_anti_clockwise();
                            if (x + left.x(), z + left.z()) == (px, pz) {
                                ChestType::LEFT
                            } else if (x + right.x(), z + right.z()) == (px, pz) {
                                ChestType::RIGHT
                            } else {
                                ChestType::SINGLE
                            }
                        }
                        _ => ChestType::SINGLE,
                    };
                    Some(b.with(states::CHEST_TYPE, kind))
                });
                if column.block(x, y, z).block_type() == Some(BlockType::TRAPPED_CHEST) {
                    if let BlockEntityData::Chest(container) = &entity.data {
                        entity.data = BlockEntityData::TrappedChest(container.clone());
                    }
                }
            }
            _ => {}
        }
        column.block_entities.push(entity);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::mapping::Version;
    use crate::convert::model::{BlockRef, ChunkCoord, FacingDirectionHorizontal};

    fn ctx_parts() -> (BlockReader, BedrockItems) {
        (BlockReader::new(Version::new(1, 26, 50), true), BedrockItems::new(crate::convert::bedrock::data_version::BedrockVersion::latest()))
    }

    fn block(t: BlockType) -> BlockRef {
        Arc::new(BlockIdentifier::new(t))
    }

    fn be(id: &str, x: i32, y: i32, z: i32) -> Compound {
        let mut c = Compound::new();
        c.insert("id", Tag::String(id.into()));
        c.insert("x", Tag::Int(x));
        c.insert("y", Tag::Int(y));
        c.insert("z", Tag::Int(z));
        c
    }

    fn run(column: &mut Column, tags: Vec<Compound>) -> PostReport {
        let (blocks, items) = ctx_parts();
        read_block_entities(&PostContext { blocks: &blocks, items: &items, keep_unknown: false }, column, &tags)
    }

    #[test]
    fn bed_colour_sets_the_block_type() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        c.set_block(3, 64, 4, block(BlockType::WHITE_BED));
        let mut tag = be("Bed", 3, 64, 4);
        tag.insert("color", Tag::Byte(14));
        run(&mut c, vec![tag]);
        assert_eq!(c.block(3, 64, 4).block_type(), Some(BlockType::RED_BED));
        assert_eq!(c.block_entities.len(), 1);
        // Air stays air.
        let mut tag = be("Bed", 5, 64, 4);
        tag.insert("color", Tag::Byte(14));
        run(&mut c, vec![tag]);
        assert!(c.block(5, 64, 4).is_air());
    }

    #[test]
    fn skull_type_rotation_and_power() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        c.set_block(1, 70, 1, block(BlockType::SKELETON_SKULL));
        let mut tag = be("Skull", 1, 70, 1);
        tag.insert("SkullType", Tag::Byte(3));
        tag.insert("Rotation", Tag::Float(90.0));
        tag.insert("DoingAnimation", Tag::Byte(1));
        run(&mut c, vec![tag]);
        let b = c.block(1, 70, 1);
        assert_eq!(b.block_type(), Some(BlockType::PLAYER_HEAD));
        assert_eq!(b.get(states::ROTATION), Some(Rotation::_4));
        assert_eq!(b.get(states::POWERED), Some(Bool::TRUE));
    }

    #[test]
    fn note_piston_and_flower_pot_set_blocks_and_vanish() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        c.set_block(0, 60, 0, block(BlockType::NOTE_BLOCK));
        c.set_block(1, 60, 0, block(BlockType::STICKY_PISTON));
        c.set_block(2, 60, 0, block(BlockType::FLOWER_POT));
        let mut music = be("Music", 0, 60, 0);
        music.insert("note", Tag::Byte(7));
        let mut piston = be("PistonArm", 1, 60, 0);
        piston.insert("State", Tag::Byte(2));
        let mut pot = be("FlowerPot", 2, 60, 0);
        let mut plant = Compound::new();
        plant.insert("name", Tag::String("minecraft:red_mushroom".into()));
        plant.insert("states", Tag::Compound(Compound::new()));
        pot.insert("PlantBlock", Tag::Compound(plant));
        let report = run(&mut c, vec![music, piston, pot]);
        assert!(report.unmapped.is_empty());
        assert!(c.block_entities.is_empty());
        assert_eq!(c.block(0, 60, 0).get(states::NOTE), Some(Note::_7));
        assert_eq!(c.block(1, 60, 0).get(states::EXTENDED), Some(Bool::TRUE));
        assert_eq!(c.block(2, 60, 0).block_type(), Some(BlockType::POTTED_RED_MUSHROOM));
    }

    #[test]
    fn double_chest_halves() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        let facing_north = Arc::new(BlockIdentifier::new(BlockType::CHEST).with(states::FACING_HORIZONTAL, FacingDirectionHorizontal::NORTH));
        c.set_block(4, 65, 4, facing_north.clone());
        c.set_block(5, 65, 4, facing_north.clone());
        c.set_block(8, 65, 4, facing_north);
        let mut a = be("Chest", 4, 65, 4);
        a.insert("pairx", Tag::Int(5));
        a.insert("pairz", Tag::Int(4));
        let mut b = be("Chest", 5, 65, 4);
        b.insert("pairx", Tag::Int(4));
        b.insert("pairz", Tag::Int(4));
        let single = be("Chest", 8, 65, 4);
        run(&mut c, vec![a, b, single]);
        // Facing north: clockwise (left) is east.
        assert_eq!(c.block(4, 65, 4).get(states::CHEST_TYPE), Some(ChestType::LEFT));
        assert_eq!(c.block(5, 65, 4).get(states::CHEST_TYPE), Some(ChestType::RIGHT));
        assert_eq!(c.block(8, 65, 4).get(states::CHEST_TYPE), Some(ChestType::SINGLE));
        assert_eq!(c.block_entities.len(), 3);
    }

    #[test]
    fn trapped_chest_becomes_trapped_container() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        c.set_block(0, 65, 0, block(BlockType::TRAPPED_CHEST));
        run(&mut c, vec![be("Chest", 0, 65, 0)]);
        assert!(matches!(c.block_entities[0].data, BlockEntityData::TrappedChest(_)));
    }

    #[test]
    fn lectern_jukebox_banner_shulker() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        c.set_block(0, 70, 0, block(BlockType::LECTERN));
        c.set_block(1, 70, 0, block(BlockType::JUKEBOX));
        c.set_block(2, 70, 0, block(BlockType::WHITE_BANNER));
        c.set_block(3, 70, 0, block(BlockType::SHULKER_BOX));
        let mut lectern = be("Lectern", 0, 70, 0);
        let mut book = Compound::new();
        book.insert("Name", Tag::String("minecraft:writable_book".into()));
        lectern.insert("book", Tag::Compound(book));
        let jukebox = be("Jukebox", 1, 70, 0);
        let mut banner = be("Banner", 2, 70, 0);
        banner.insert("Base", Tag::Int(1)); // reversed id 1 = 14 = red
        let mut shulker = be("ShulkerBox", 3, 70, 0);
        shulker.insert("facing", Tag::Byte(5));
        run(&mut c, vec![lectern, jukebox, banner, shulker]);
        assert_eq!(c.block(0, 70, 0).get(states::HAS_BOOK), Some(Bool::TRUE));
        assert_eq!(c.block(1, 70, 0).get(states::HAS_RECORD), Some(Bool::FALSE));
        assert_eq!(c.block(2, 70, 0).block_type(), Some(BlockType::RED_BANNER));
        assert_eq!(c.block(3, 70, 0).get(states::FACING_ALL), Some(FacingDirection::EAST));
    }

    #[test]
    fn unknown_ids_are_reported_by_the_reader_when_it_gives_up() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        // The stub and the real reader both accept an id; a tag without one cannot be read at all.
        let mut tag = Compound::new();
        tag.insert("x", Tag::Int(0));
        let report = run(&mut c, vec![tag]);
        assert_eq!(report.unmapped.len(), 1);
        assert!(c.block_entities.is_empty());
    }
}
