// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Tests of the world reader: a small world built in memory, and the real test world when it is on this machine.

use super::*;
use crate::convert::bedrock::db::MemDb;
use crate::convert::bedrock::keys::{ChunkTag, chunk_key, sub_chunk_key};
use crate::convert::bedrock::palette::{
    BlockStateRaw, Data3D, PaletteData, RawPalette, RawSubChunk, encode_data3d, encode_nbt_concat, encode_sub_chunk,
};
use crate::convert::model::{BlockEntityData, BlockType, Palette, VanillaBiome};

fn raw(name: &str) -> BlockStateRaw {
    BlockStateRaw::new(name, Compound::new(), 18168865)
}

fn level_dat() -> LevelDat {
    let mut root = Compound::new();
    root.insert("LevelName", Tag::String("Test World".into()));
    root.insert("StorageVersion", Tag::Int(10));
    LevelDat { storage_version: 10, root_name: String::new(), root }
}

fn chest(x: i32, y: i32, z: i32) -> Tag {
    let mut c = Compound::new();
    c.insert("id", Tag::String("Chest".into()));
    c.insert("x", Tag::Int(x));
    c.insert("y", Tag::Int(y));
    c.insert("z", Tag::Int(z));
    Tag::Compound(c)
}

fn synthetic_db() -> MemDb {
    let mut db = MemDb::new();
    let ow = DimensionId::OVERWORLD;
    for (y, name) in [(-4i8, "minecraft:deepslate"), (0, "minecraft:stone"), (4, "minecraft:not_a_block")] {
        let sub = RawSubChunk { version: 9, y, layers: vec![RawPalette::uniform(raw(name))] };
        db.put(&sub_chunk_key(ow, 1, -2, y), &encode_sub_chunk(&sub).unwrap()).unwrap();
    }
    let data3d = Data3D {
        heights: (0..256).map(|i| (i % 16) as i16).collect(),
        sections: vec![PaletteData::uniform(1), PaletteData::inherit()],
    };
    db.put(&chunk_key(ow, 1, -2, ChunkTag::Data3D), &encode_data3d(&data3d).unwrap()).unwrap();
    db.put(&chunk_key(ow, 1, -2, ChunkTag::BlockEntity), &encode_nbt_concat(&[chest(16, 0, -32)]).unwrap()).unwrap();
    db.put(&chunk_key(ow, 1, -2, ChunkTag::Version), &[42]).unwrap();
    db.put(&chunk_key(ow, 1, -2, ChunkTag::FinalizedState), &1i32.to_le_bytes()).unwrap();
    // A nether column, and a column in a dimension nobody registered.
    let sub = RawSubChunk { version: 9, y: 0, layers: vec![RawPalette::uniform(raw("minecraft:netherrack"))] };
    db.put(&sub_chunk_key(DimensionId::NETHER, -1, 5, 0), &encode_sub_chunk(&sub).unwrap()).unwrap();
    db.put(&sub_chunk_key(DimensionId(77), 0, 0, 0), &encode_sub_chunk(&sub).unwrap()).unwrap();
    db
}

fn reader(settings: &ConversionSettings) -> BedrockWorldReader {
    let db = synthetic_db();
    let opener: DbOpener = Box::new(move || Ok(Box::new(db) as Box<dyn BedrockDb>));
    let version = BedrockVersion::exact([1, 26, 50]).unwrap();
    BedrockWorldReader::from_parts(opener, level_dat(), version, [1, 26, 50], settings).unwrap()
}

#[test]
fn reader_is_sync() {
    fn check<T: Send + Sync>() {}
    check::<BedrockWorldReader>();
}

#[test]
fn dimensions_and_columns() {
    let r = reader(&ConversionSettings::default());
    let dims = r.dimensions();
    assert_eq!(dims, [Dimension::overworld(), Dimension::nether()]);
    assert_eq!(r.columns(&Dimension::overworld()), [(1, -2)]);
    assert_eq!(r.columns(&Dimension::nether()), [(-1, 5)]);
    assert!(r.columns(&Dimension::the_end()).is_empty());
}

#[test]
fn settings_filter_dimensions_and_columns() {
    let mut s = ConversionSettings::default();
    s.dimension_mapping = Some([("minecraft:overworld".to_string(), "minecraft:overworld".to_string())].into());
    let r = reader(&s);
    assert_eq!(r.dimensions(), [Dimension::overworld()]);

    use crate::convert::level::{PruningConfig, PruningRegion};
    let mut s = ConversionSettings::default();
    s.pruning_configs = Some(
        [(
            "minecraft:overworld".to_string(),
            PruningConfig { include: false, regions: vec![PruningRegion::new(0, -5, 3, 0)] },
        )]
        .into(),
    );
    let r = reader(&s);
    assert!(r.columns(&Dimension::overworld()).is_empty());
    assert_eq!(r.columns(&Dimension::nether()), [(-1, 5)]);
}

#[test]
fn reads_a_column() {
    let r = reader(&ConversionSettings::default());
    let c = r.read_column(&Dimension::overworld(), 1, -2).unwrap().expect("column");
    assert_eq!((c.position.x, c.position.z), (1, -2));
    assert_eq!(c.chunks.keys().copied().collect::<Vec<_>>(), [-4, 0, 4]);
    assert_eq!(c.block(0, -64, 0).block_type(), Some(BlockType::DEEPSLATE));
    assert_eq!(c.block(5, 3, 9).block_type(), Some(BlockType::STONE));
    // The unknown block became air and was counted.
    assert!(c.block(0, 64, 0).is_air());
    assert_eq!(r.unmapped_blocks().get("minecraft:not_a_block"), Some(&1));
    // Height map, x-major view of a z * 16 + x layout.
    let Some(crate::convert::model::HeightMap::Bedrock(h)) = &c.height_map else { panic!("no height map") };
    assert_eq!((h[3][7], h[15][0]), (3, 15));
    // Biomes: the trailing inherit section is dropped.
    let Some(crate::convert::model::Biomes::Palette(p)) = &c.biomes else { panic!("no biomes") };
    assert_eq!(p.len(), 1);
    assert!(matches!(&p[0], Palette::Single(crate::convert::model::Biome::Vanilla(VanillaBiome::PLAINS))));
    // Block entity.
    assert_eq!(c.block_entities.len(), 1);
    assert!(matches!(c.block_entities[0].data, BlockEntityData::Chest(_)));
    assert_eq!((c.block_entities[0].x, c.block_entities[0].z), (16, -32));
    // FinalizedState 1 is counted, nothing else to complain about.
    assert_eq!(r.stats().unfinalized, 1);
    assert_eq!(r.stats().sub_chunks, 3);
    assert!(r.warnings().is_empty(), "{:?}", r.warnings());
}

#[test]
fn missing_and_other_columns() {
    let r = reader(&ConversionSettings::default());
    assert!(r.read_column(&Dimension::overworld(), 9, 9).unwrap().is_none());
    let nether = r.read_column(&Dimension::nether(), -1, 5).unwrap().unwrap();
    assert_eq!(nether.block(0, 0, 0).block_type(), Some(BlockType::NETHERRACK));
    assert!(nether.biomes.is_none() && nether.height_map.is_none());
}

#[test]
fn process_flags_are_honoured() {
    let mut s = ConversionSettings::default();
    s.process_biomes = false;
    s.process_height_map = false;
    s.process_block_entities = false;
    let r = reader(&s);
    let c = r.read_column(&Dimension::overworld(), 1, -2).unwrap().unwrap();
    assert!(c.biomes.is_none() && c.height_map.is_none() && c.block_entities.is_empty());
    assert_eq!(c.chunks.len(), 3);
}

#[test]
fn damaged_sub_chunks_are_skipped_with_a_warning() {
    let mut db = synthetic_db();
    db.put(&sub_chunk_key(DimensionId::OVERWORLD, 1, -2, 7), &[9, 1]).unwrap();
    db.put(&sub_chunk_key(DimensionId::OVERWORLD, 1, -2, 8), &[3, 1, 0]).unwrap();
    let opener: DbOpener = Box::new(move || Ok(Box::new(db) as Box<dyn BedrockDb>));
    let version = BedrockVersion::exact([1, 26, 50]).unwrap();
    let r = BedrockWorldReader::from_parts(opener, level_dat(), version, [1, 26, 50], &ConversionSettings::default())
        .unwrap();
    let c = r.read_column(&Dimension::overworld(), 1, -2).unwrap().unwrap();
    assert_eq!(c.chunks.len(), 3);
    assert_eq!(r.warnings().len(), 2, "{:?}", r.warnings());
}

#[test]
fn threads_read_columns_at_once() {
    let r = reader(&ConversionSettings::default());
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for _ in 0..25 {
                    let c = r.read_column(&Dimension::overworld(), 1, -2).unwrap().unwrap();
                    assert_eq!(c.chunks.len(), 3);
                }
            });
        }
    });
    assert_eq!(r.stats().columns, 100);
}

#[test]
fn level_of_the_synthetic_world() {
    let r = reader(&ConversionSettings::default());
    let level = r.level().unwrap();
    assert_eq!(level.settings.level_name, "Test World");
    assert!(level.player.is_none() && level.maps.is_empty() && level.portals.is_empty());
    assert!(level.original_level_data.is_some());
}

/// The real world: every column of the Mace, Spear, Sword PvP Map. Skipped when the file is not on this machine.
#[test]
fn real_world_mace_map() {
    use crate::convert::bedrock::detect::detect_bedrock;
    use crate::convert::model::{Biome, Biomes, BlockEntityKind};
    use std::time::Instant;

    let Some(path) = crate::convert::bedrock::test_world_path() else {
        eprintln!("test world not found, skipping");
        return;
    };
    let info = detect_bedrock(&path).expect("a Bedrock world");
    assert_eq!(info.game_version, Some([1, 26, 51]));
    let settings = ConversionSettings::default();
    let t = Instant::now();
    let r = BedrockWorldReader::open(&info, &settings).unwrap();
    let open_time = t.elapsed();

    let level = r.level().unwrap();
    assert_eq!(level.settings.level_name, "Mace, Spear, Sword PvP Map! V4");
    assert_eq!(level.settings.random_seed, "1045923935239469310");
    assert_eq!((level.settings.spawn_x, level.settings.spawn_y, level.settings.spawn_z), (0, 57, 0));
    assert_eq!(level.settings.game_type, 0);
    assert!(level.settings.keepinventory && !level.settings.mobgriefing && !level.settings.dodaylightcycle);
    assert_eq!(level.settings.generator_type, crate::convert::level::GeneratorType::Void);
    assert_eq!(r.version().game, [1, 26, 50]);
    assert!(level.player.is_none() && level.maps.is_empty() && level.portals.is_empty());

    assert_eq!(r.dimensions(), [Dimension::overworld()]);
    let overworld = Dimension::overworld();
    let positions = r.columns(&overworld);
    assert_eq!(positions.len(), 46);

    let t = Instant::now();
    let mut columns = Vec::new();
    for &(x, z) in &positions {
        columns.push(r.read_column(&overworld, x, z).unwrap().expect("column"));
    }
    let read_time = t.elapsed();

    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total_non_air = 0u64;
    let mut sections = 0usize;
    let mut kinds: HashMap<BlockEntityKind, usize> = HashMap::new();
    for c in &columns {
        for chunk in c.chunks.values() {
            sections += 1;
            assert!((-4..20).contains(&chunk.y), "section {}", chunk.y);
            for y in 0..16 {
                for z in 0..16 {
                    for x in 0..16 {
                        let b = chunk.blocks.get_or(x, y, z, crate::convert::model::air());
                        let name = match b.block_type() {
                            Some(t) => t.name().to_string(),
                            None => format!("{:?}", b.kind()),
                        };
                        if !b.is_air() {
                            total_non_air += 1;
                        }
                        *counts.entry(name).or_insert(0) += 1;
                    }
                }
            }
        }
        for be in &c.block_entities {
            *kinds.entry(be.kind()).or_insert(0) += 1;
        }
        assert!(c.height_map.is_some());
        let Some(Biomes::Palette(p)) = &c.biomes else { panic!("no palette biomes") };
        assert!(!p.is_empty());
        for palette in p {
            for b in palette.keys() {
                assert!(matches!(b, Biome::Vanilla(_)), "{b:?}");
            }
        }
    }
    let mut top: Vec<_> = counts.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    eprintln!("open {open_time:?}, level + {} columns in {read_time:?}", columns.len());
    eprintln!("{} sections, {total_non_air} non-air blocks, {} distinct block types", sections, counts.len());
    for (name, n) in top.iter().take(20) {
        eprintln!("  {n:>8} {name}");
    }
    eprintln!("block entities: {kinds:?}");
    eprintln!("stats: {:?}", r.stats());
    eprintln!("unmapped blocks: {:?}", r.unmapped_blocks());
    eprintln!("dropped states: {:?}", r.dropped_states());
    eprintln!("warnings: {:?}", r.warnings());

    assert!(r.unmapped_blocks().is_empty(), "{:?}", r.unmapped_blocks());
    assert!(r.unmapped_biomes().is_empty());
    assert!(r.warnings().is_empty(), "{:?}", r.warnings());
    assert!(total_non_air > 1000);
    assert_eq!(kinds.get(&BlockEntityKind::CommandBlock), Some(&24));
    assert_eq!(kinds.get(&BlockEntityKind::HangingSign), Some(&12));
    assert_eq!(kinds.get(&BlockEntityKind::Chest), Some(&7));
    assert_eq!(kinds.get(&BlockEntityKind::Sign), Some(&5));

    // Reading again and from several threads gives the same columns and is fast.
    let t = Instant::now();
    let rounds = 20;
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for _ in 0..rounds / 4 {
                    for &(x, z) in &positions {
                        r.read_column(&overworld, x, z).unwrap().unwrap();
                    }
                }
            });
        }
    });
    let again = t.elapsed();
    eprintln!(
        "{} columns in {again:?} on 4 threads = {:.0} columns/s",
        positions.len() * rounds,
        (positions.len() * rounds) as f64 / again.as_secs_f64()
    );
}
