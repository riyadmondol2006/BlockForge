// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! The column (`ChunkerColumn`): one 16x16 column of chunk sections plus biomes, height map, entities and block
//! entities. It is the unit that flows from a reader through the handlers to a writer.

use std::collections::BTreeMap;
use std::fmt;

use super::biome::Biomes;
use super::block::{BlockRef, air};
use super::blockentity::BlockEntity;
use super::coords::{ChunkCoord, Edge, EdgeSet};
use super::entity::Entity;
use super::palette::Palette;

/// Palette of the blocks of a section.
pub type BlockPalette = Palette<BlockRef, 16>;

/// Light levels of a section, one nibble per byte in palette order (`Palette::index`: Y, Z, X).
pub type LightArray = Box<[u8; 4096]>;

/// A 16x16x16 section of blocks (`ChunkerChunk`).
#[derive(Clone, Debug)]
pub struct Chunk {
    /// Section index (block y >> 4).
    pub y: i8,
    pub blocks: BlockPalette,
    /// Block light; only the legacy Java reader fills it.
    pub block_light: Option<LightArray>,
    /// Sky light; only the legacy Java reader fills it.
    pub sky_light: Option<LightArray>,
}

impl Chunk {
    /// A section filled with air (a `Single` palette).
    pub fn new(y: i8) -> Chunk {
        Chunk { y, blocks: Palette::Single(air().clone()), block_light: None, sky_light: None }
    }

    /// A section with the given blocks.
    pub fn with_blocks(y: i8, blocks: BlockPalette) -> Chunk {
        Chunk { y, blocks, block_light: None, sky_light: None }
    }

    /// Whether the section holds no non-air block *key*. This is Chunker's `isEmpty`: it looks at the palette
    /// keys, not at the voxels, so a key that no voxel uses (until `compact`) still makes a section non-empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() || !self.blocks.contains_key(|b| !b.is_air())
    }
}

/// A height map in one of the two formats Chunker keeps opaque (it can only be written by the same format).
/// Indexed `[x][z]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeightMap {
    JavaLegacy(Box<[[i16; 16]; 16]>),
    Bedrock(Box<[[i16; 16]; 16]>),
}

/// The neighbouring columns a pre-transform may look at, indexed by [`Edge`]; a missing neighbour is `None`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Neighbours<'a> {
    columns: [Option<&'a Column>; 4],
}

impl<'a> Neighbours<'a> {
    pub fn new() -> Neighbours<'a> {
        Neighbours::default()
    }

    pub fn with(mut self, edge: Edge, column: &'a Column) -> Neighbours<'a> {
        self.columns[edge as usize] = Some(column);
        self
    }

    pub fn set(&mut self, edge: Edge, column: Option<&'a Column>) {
        self.columns[edge as usize] = column;
    }

    pub fn get(&self, edge: Edge) -> Option<&'a Column> {
        self.columns[edge as usize]
    }
}

/// A pre-transform: run once on the column with its neighbours (`Consumer<Map<Edge, ChunkerColumn>>`).
pub type PreTransformHandler = Box<dyn FnOnce(&mut Column, &Neighbours<'_>) + Send + Sync>;

/// One 16x16 column of the world.
pub struct Column {
    pub position: ChunkCoord,
    /// Sections by index (`y >> 4`), sorted ascending.
    pub chunks: BTreeMap<i8, Chunk>,
    pub biomes: Option<Biomes>,
    pub height_map: Option<HeightMap>,
    pub entities: Vec<Entity>,
    pub block_entities: Vec<BlockEntity>,
    /// Java `isLightOn` / Bedrock `LightPopulated`.
    pub light_populated: bool,
    required_edges: EdgeSet,
    pre_transform: Option<PreTransformHandler>,
}

impl fmt::Debug for Column {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Column")
            .field("position", &self.position)
            .field("chunks", &self.chunks.keys().collect::<Vec<_>>())
            .field("biomes", &self.biomes.is_some())
            .field("height_map", &self.height_map.is_some())
            .field("entities", &self.entities.len())
            .field("block_entities", &self.block_entities.len())
            .field("light_populated", &self.light_populated)
            .field("required_edges", &self.required_edges)
            .field("pre_transform", &self.pre_transform.is_some())
            .finish()
    }
}

impl Column {
    pub fn new(position: ChunkCoord) -> Column {
        Column {
            position,
            chunks: BTreeMap::new(),
            biomes: None,
            height_map: None,
            entities: Vec::new(),
            block_entities: Vec::new(),
            light_populated: false,
            required_edges: EdgeSet::EMPTY,
            pre_transform: None,
        }
    }

    /// The edges whose neighbour columns the registered pre-transforms need.
    pub fn required_edges(&self) -> EdgeSet {
        self.required_edges
    }

    /// Whether a pre-transform is waiting to run.
    pub fn has_pre_transform(&self) -> bool {
        self.pre_transform.is_some()
    }

    /// Register a pre-transform needing the given edges; handlers run in registration order (`andThen`).
    pub fn add_pre_transform(
        &mut self,
        edges: EdgeSet,
        handler: impl FnOnce(&mut Column, &Neighbours<'_>) + Send + Sync + 'static,
    ) {
        self.pre_transform = Some(match self.pre_transform.take() {
            Some(first) => Box::new(move |column: &mut Column, neighbours: &Neighbours<'_>| {
                first(column, neighbours);
                handler(column, neighbours);
            }),
            None => Box::new(handler),
        });
        self.required_edges = self.required_edges.union(edges);
    }

    /// Run the pre-transform (if any) and drop it, so it only ever runs once (`preTransform`).
    pub fn pre_transform(&mut self, neighbours: &Neighbours<'_>) {
        if let Some(handler) = self.pre_transform.take() {
            handler(self, neighbours);
        }
    }

    /// The block at local x and z (masked to 0..16) and world y; air when the section is missing (`getBlock`).
    pub fn block(&self, x: i32, y: i32, z: i32) -> &BlockRef {
        if self.chunks.is_empty() {
            return air();
        }
        match self.chunks.get(&((y >> 4) as i8)) {
            Some(chunk) => chunk.blocks.get_or((x & 15) as usize, (y & 15) as usize, (z & 15) as usize, air()),
            None => air(),
        }
    }

    /// Set a block, creating an air filled section when needed (`setBlock`).
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block: BlockRef) {
        let chunk_y = (y >> 4) as i8;
        let chunk = self.chunks.entry(chunk_y).or_insert_with(|| Chunk::new(chunk_y));
        chunk.blocks.set((x & 15) as usize, (y & 15) as usize, (z & 15) as usize, block);
    }

    /// The highest block (world y and block) matching `predicate` at local x and z, scanning sections from the
    /// top; sections whose palette has no matching key are skipped (`getHighestBlock`).
    pub fn highest_block(
        &self,
        x: i32,
        z: i32,
        predicate: impl Fn(&BlockRef) -> bool,
    ) -> Option<(i32, &BlockRef)> {
        let (x, z) = ((x & 15) as usize, (z & 15) as usize);
        for chunk in self.chunks.values().rev() {
            if !chunk.blocks.contains_key(&predicate) {
                continue;
            }
            for y in (0..16usize).rev() {
                let block = chunk.blocks.get_or(x, y, z, air());
                if predicate(block) {
                    return Some(((chunk.y as i32) << 4 | y as i32, block));
                }
            }
        }
        None
    }

    /// The block entity at local x and z and world y; a linear scan (`getBlockEntity`).
    pub fn block_entity(&self, x: i32, y: i32, z: i32) -> Option<&BlockEntity> {
        let (x, z) = (x & 15, z & 15);
        self.block_entities.iter().find(|be| be.x & 15 == x && be.y == y && be.z & 15 == z)
    }

    /// Mutable form of [`Column::block_entity`].
    pub fn block_entity_mut(&mut self, x: i32, y: i32, z: i32) -> Option<&mut BlockEntity> {
        let (x, z) = (x & 15, z & 15);
        self.block_entities.iter_mut().find(|be| be.x & 15 == x && be.y == y && be.z & 15 == z)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::convert::model::block::BlockIdentifier;
    use crate::convert::model::blockentity::BlockEntityData;
    use crate::convert::model::generated::BlockType;

    fn block(t: BlockType) -> BlockRef {
        Arc::new(BlockIdentifier::new(t))
    }

    #[test]
    fn missing_sections_are_air() {
        let c = Column::new(ChunkCoord::new(0, 0));
        assert!(c.block(1, 70, 2).is_air());
        assert!(c.block(-1, -5, 99).is_air());
    }

    #[test]
    fn set_and_get_block() {
        let mut c = Column::new(ChunkCoord::new(3, -2));
        c.set_block(5, 70, 9, block(BlockType::STONE));
        assert_eq!(c.block(5, 70, 9).block_type(), Some(BlockType::STONE));
        assert!(c.block(6, 70, 9).is_air());
        // World coordinates are masked.
        assert_eq!(c.block(5 + 16, 70, 9 - 16).block_type(), Some(BlockType::STONE));
        assert_eq!(c.chunks.keys().copied().collect::<Vec<_>>(), vec![4]);
        // Negative y lands in negative sections.
        c.set_block(0, -1, 0, block(BlockType::DIRT));
        c.set_block(0, -64, 0, block(BlockType::BEDROCK));
        assert_eq!(c.chunks.keys().copied().collect::<Vec<_>>(), vec![-4, -1, 4]);
        assert_eq!(c.block(0, -1, 0).block_type(), Some(BlockType::DIRT));
        assert_eq!(c.block(0, -64, 0).block_type(), Some(BlockType::BEDROCK));
        assert!(c.block(0, -63, 0).is_air());
    }

    #[test]
    fn chunk_is_empty_looks_at_keys() {
        let mut chunk = Chunk::new(0);
        assert!(chunk.is_empty());
        chunk.blocks = Palette::Empty;
        assert!(chunk.is_empty());
        chunk.blocks = Palette::Single(block(BlockType::STONE));
        assert!(!chunk.is_empty());
        // A stale unused key keeps the section non-empty until compaction.
        let mut p: BlockPalette = Palette::Single(air().clone());
        p.set(0, 0, 0, block(BlockType::STONE));
        p.set(0, 0, 0, air().clone());
        let stale = Chunk::with_blocks(0, p);
        assert!(!stale.is_empty());
        let compacted = Chunk::with_blocks(0, stale.blocks.clone().compact(air()));
        assert!(compacted.is_empty());
    }

    #[test]
    fn highest_block_scans_from_the_top() {
        let mut c = Column::new(ChunkCoord::new(0, 0));
        assert!(c.highest_block(0, 0, |b| !b.is_air()).is_none());
        c.set_block(2, 5, 3, block(BlockType::STONE));
        c.set_block(2, 40, 3, block(BlockType::DIRT));
        c.set_block(2, 20, 3, block(BlockType::GLASS));
        let (y, b) = c.highest_block(2, 3, |b| !b.is_air()).unwrap();
        assert_eq!((y, b.block_type()), (40, Some(BlockType::DIRT)));
        let (y, _) = c.highest_block(2 + 16, 3, |b| b.block_type() == Some(BlockType::STONE)).unwrap();
        assert_eq!(y, 5);
        assert!(c.highest_block(0, 0, |b| !b.is_air()).is_none());
        // Negative sections have negative world y.
        c.set_block(7, -3, 7, block(BlockType::DEEPSLATE));
        let (y, _) = c.highest_block(7, 7, |b| b.block_type() == Some(BlockType::DEEPSLATE)).unwrap();
        assert_eq!(y, -3);
    }

    #[test]
    fn block_entity_lookup() {
        let mut c = Column::new(ChunkCoord::new(1, 1));
        c.block_entities.push(BlockEntity::new(16 + 3, 70, 16 + 4, BlockEntityData::Bed));
        c.block_entities.push(BlockEntity::new(16 + 3, 71, 16 + 4, BlockEntityData::Bell));
        assert!(matches!(c.block_entity(3, 70, 4).unwrap().data, BlockEntityData::Bed));
        assert!(matches!(c.block_entity(3 + 32, 71, 4).unwrap().data, BlockEntityData::Bell));
        assert!(c.block_entity(3, 72, 4).is_none());
        c.block_entity_mut(3, 70, 4).unwrap().movable = true;
        assert!(c.block_entities[0].movable);
    }

    #[test]
    fn pre_transforms_chain_and_run_once() {
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let mut c = Column::new(ChunkCoord::new(0, 0));
        assert!(!c.has_pre_transform());
        c.add_pre_transform(EdgeSet::EMPTY.with(Edge::PositiveX), |column, _| {
            assert_eq!(RUNS.fetch_add(1, Ordering::SeqCst), 0);
            column.light_populated = true;
        });
        c.add_pre_transform(EdgeSet::EMPTY.with(Edge::NegativeZ), |column, n| {
            assert_eq!(RUNS.fetch_add(1, Ordering::SeqCst), 1);
            assert!(column.light_populated);
            assert!(n.get(Edge::PositiveX).is_some());
            assert!(n.get(Edge::NegativeX).is_none());
        });
        assert!(c.required_edges().contains(Edge::PositiveX) && c.required_edges().contains(Edge::NegativeZ));
        let other = Column::new(ChunkCoord::new(1, 0));
        c.pre_transform(&Neighbours::new().with(Edge::PositiveX, &other));
        assert_eq!(RUNS.load(Ordering::SeqCst), 2);
        assert!(!c.has_pre_transform());
        c.pre_transform(&Neighbours::new());
        assert_eq!(RUNS.load(Ordering::SeqCst), 2);
    }
}
