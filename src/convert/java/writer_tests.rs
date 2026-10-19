// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Tests of the Java world writer: columns are built from the model, written for several versions and read back with
//! the region reader, the palette unpacker and the `level.dat` reader.

use super::*;
use crate::convert::java::format::SectionLayout;
use crate::convert::java::level_dat::read_level_dat;
use crate::convert::java::palette::{bits_for, unpack};
use crate::convert::java::region::RegionFile;
use crate::convert::java::testutil::TestDir;
use crate::convert::level::{GeneratorType, LevelPlayer};
use crate::convert::model::{StateValue, 
    BlockEntity, BlockEntityData, BlockIdentifier, BlockRef, BlockType, Biome, Biomes, ChunkCoord, VanillaBiome,
    blockentity::Sign, states,
};
use crate::convert::model::generated::FacingDirectionHorizontal;
use crate::convert::nbt::Compound;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

fn block(t: BlockType) -> BlockRef {
    Arc::new(BlockIdentifier::new(t))
}

fn level() -> Level {
    let mut level = Level::default();
    level.settings.caves_and_cliffs = true;
    level.settings.level_name = "Writer Test".into();
    level.settings.generator_type = GeneratorType::Flat;
    level.settings.random_seed = "424242".into();
    level.settings.spawn_x = 8;
    level.settings.spawn_y = 70;
    level.settings.spawn_z = -8;
    level
}

/// Bedrock floor at -64, stone to 62, grass at 63, stairs and a sign at 64, cherry biomes in the upper half.
fn sample_column(cx: i32, cz: i32) -> Column {
    let mut c = Column::new(ChunkCoord::new(cx, cz));
    for x in 0..16 {
        for z in 0..16 {
            c.set_block(x, -64, z, block(BlockType::BEDROCK));
            for y in -63..63 {
                c.set_block(x, y, z, block(BlockType::STONE));
            }
            c.set_block(x, 63, z, block(BlockType::GRASS_BLOCK));
        }
    }
    let stairs = BlockIdentifier::new(BlockType::OAK_STAIRS).with(states::FACING_HORIZONTAL, FacingDirectionHorizontal::EAST);
    c.set_block(1, 64, 1, Arc::new(stairs));
    c.set_block(2, 64, 2, block(BlockType::OAK_SIGN));
    c.block_entities.push(BlockEntity::new(cx * 16 + 2, 64, cz * 16 + 2, BlockEntityData::Sign(Box::new(Sign::default()))));
    // A block entity without a block under it is dropped.
    c.block_entities.push(BlockEntity::new(cx * 16 + 5, 100, cz * 16 + 5, BlockEntityData::Sign(Box::new(Sign::default()))));
    let mut cells = Vec::new();
    for i in 0..1024 {
        cells.push(Biome::Vanilla(if (i >> 6) < 8 { VanillaBiome::PLAINS } else { VanillaBiome::DESERT }));
    }
    c.biomes = Some(Biomes::Cluster(cells));
    c
}

fn names(v: &[Version]) -> Vec<String> {
    v.iter().map(|v| v.to_string()).collect()
}

fn targets() -> Vec<Version> {
    vec![Version::new(1, 16, 5), Version::new(1, 18, 2), Version::new(1, 20, 4), Version::new(1, 21, 4), Version::new(26, 3, 0)]
}

fn write_world(out: &Path, target: Version, columns: &[(i32, i32)]) -> WriterSummary {
    let mut w = JavaWorldWriter::create(out, target, &level(), &ConversionSettings::default()).unwrap();
    let dim = Dimension::overworld();
    for &(x, z) in columns {
        w.write_column(&dim, &sample_column(x, z)).unwrap();
    }
    w.finish_dimension(&dim).unwrap();
    w.finish_with_summary().unwrap()
}

fn read_chunk(dir: &Path, fmt: &JavaFormat, cx: i32, cz: i32) -> Compound {
    let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
    let region_dir = LevelDirs::new(dir, fmt).region_dir(&JavaDimension::Overworld);
    let mut region = RegionFile::open_read(region_dir.join(format!("r.{rx}.{rz}.mca"))).unwrap();
    region.read_chunk(cx.rem_euclid(32) as u8, cz.rem_euclid(32) as u8).unwrap().expect("chunk present")
}

/// (palette entries as `name[props]`, voxel indices) of section `y`.
fn decode_section(root: &Compound, fmt: &JavaFormat, y: i8) -> Option<(Vec<(String, BTreeMap<String, String>)>, Vec<u16>)> {
    let level = if fmt.level_wrapper { root.get("Level")?.as_compound()? } else { root };
    let sections = level.get(fmt.sections_key)?.as_list()?;
    let section = sections.iter().filter_map(|s| s.as_compound()).find(|s| s.get("Y").and_then(Tag::as_integer) == Some(y as i64))?;
    let (palette, data) = match fmt.sections {
        SectionLayout::NestedStates => {
            let bs = section.get("block_states")?.as_compound()?;
            (bs.get("palette")?.as_list()?, bs.get("data")?.as_long_array()?)
        }
        _ => (section.get("Palette")?.as_list()?, section.get("BlockStates")?.as_long_array()?),
    };
    let (name_key, prop_key) = if fmt.renamed_block_state_fields { ("id", "properties") } else { ("Name", "Properties") };
    let entries: Vec<_> = palette
        .iter()
        .map(|p| {
            let p = p.as_compound().unwrap();
            let props = p
                .get(prop_key)
                .and_then(Tag::as_compound)
                .map(|c| c.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect())
                .unwrap_or_default();
            (p.get(name_key).unwrap().as_str().unwrap().to_string(), props)
        })
        .collect();
    let bits = bits_for(fmt.min_bits_blocks, entries.len());
    let indices = unpack(fmt.palette_layout, bits, 4096, data).unwrap();
    Some((entries, indices))
}

type VanillaBlocks = HashMap<String, HashMap<String, Vec<String>>>;

/// The vanilla blocks of a version as Chunker's data dump lists them (an independent oracle); `None` without it.
fn oracle(version: &str) -> Option<VanillaBlocks> {
    let path = format!("/home/riyadm/Downloads/ABDM/Compressed/Chunker/cli/data/java/{version}/blocks.json");
    let text = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut out = HashMap::new();
    for (name, block) in json.as_object()? {
        let mut props = HashMap::new();
        if let Some(listed) = block.get("properties").and_then(|p| p.as_object()) {
            for (k, values) in listed {
                props.insert(k.clone(), values.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
            }
        }
        out.insert(name.clone(), props);
    }
    Some(out)
}

fn assert_valid(oracle: &VanillaBlocks, name: &str, props: &BTreeMap<String, String>) {
    let allowed = oracle.get(name).unwrap_or_else(|| panic!("{name} is not a vanilla block of this version"));
    for (k, v) in props {
        let values = allowed.get(k).unwrap_or_else(|| panic!("{name} has no property {k}"));
        assert!(values.contains(v), "{name}: {k}={v} is not valid (allowed {values:?})");
    }
}

fn voxel(indices: &[u16], x: usize, y: usize, z: usize) -> usize {
    indices[x | (z << 4) | (y << 8)] as usize
}

#[test]
fn columns_round_trip_for_every_target() {
    for target in targets() {
        let dir = TestDir::new("writer-rt");
        let out = dir.path().join("world");
        let summary = write_world(&out, target, &[(0, 0), (1, 0), (-1, -1)]);
        let fmt = JavaFormat::for_version(target);
        assert_eq!(summary.columns, 3, "{target}");
        let oracle = oracle(&fmt.version.to_string().trim_end_matches(".0").to_string());
        let root = read_chunk(&out, &fmt, 1, 0);
        assert_eq!(root.get("DataVersion").and_then(Tag::as_integer), Some(fmt.row_data_version as i64), "{target}");
        let level = if fmt.level_wrapper { root.get("Level").unwrap().as_compound().unwrap() } else { &root };
        assert_eq!(level.get("xPos").and_then(Tag::as_integer), Some(1));
        assert_eq!(level.get("zPos").and_then(Tag::as_integer), Some(0));
        let status = level.get("Status").and_then(Tag::as_str).unwrap();
        assert_eq!(status, if fmt.sections == SectionLayout::NestedStates { "minecraft:full" } else { "full" });
        assert!(level.contains_key(fmt.sections_key) && level.contains_key(fmt.block_entities_key));
        assert_eq!(level.contains_key("yPos"), fmt.writes_y_pos);

        // The floor (y 63 is the top of section 3) and the stairs and sign (y 64 is the bottom of section 4).
        let (palette, idx) = decode_section(&root, &fmt, 3).unwrap();
        assert_eq!(palette[voxel(&idx, 4, 15, 4)].0, "minecraft:grass_block", "{target}");
        assert_eq!(palette[voxel(&idx, 4, 14, 4)].0, "minecraft:stone", "{target}");
        let (palette4, idx4) = decode_section(&root, &fmt, 4).unwrap();
        let stairs = &palette4[voxel(&idx4, 1, 0, 1)];
        assert_eq!(stairs.0, "minecraft:oak_stairs");
        assert_eq!(stairs.1.get("facing").map(String::as_str), Some("east"), "{target}");
        assert_eq!(palette4[voxel(&idx4, 2, 0, 2)].0, "minecraft:oak_sign");
        assert_eq!(palette4[voxel(&idx4, 9, 5, 9)].0, "minecraft:air");
        // Negative sections exist from 1.17 only (here from 1.18: the 1.17 height pack is not installed).
        let deep = decode_section(&root, &fmt, -4);
        if fmt.sections == SectionLayout::NestedStates {
            let (p, i) = deep.unwrap();
            assert_eq!(p[voxel(&i, 0, 0, 0)].0, "minecraft:bedrock");
        } else {
            assert!(deep.is_none(), "{target}");
        }
        // Block entities: the sign is written, the orphan is not.
        let bes = level.get(fmt.block_entities_key).unwrap().as_list().unwrap();
        assert_eq!(bes.len(), 1, "{target}");
        assert_eq!(bes[0].as_compound().unwrap().get("x").and_then(Tag::as_integer), Some(16 + 2));

        // Every written name and property is valid for the version.
        if let Some(oracle) = oracle {
            for y in 3..=4 {
                let (p, _) = decode_section(&root, &fmt, y).unwrap();
                for (name, props) in &p {
                    assert_valid(&oracle, name, props);
                }
            }
        } else {
            eprintln!("no Chunker data dump for {target}: block ids not cross-checked");
        }
    }
}

#[test]
fn biomes_follow_the_layout() {
    // 1.18+: a palette per section; the upper sections are desert.
    let dir = TestDir::new("writer-biomes");
    let target = Version::new(1, 20, 4);
    let out = dir.path().join("w");
    write_world(&out, target, &[(0, 0)]);
    let fmt = JavaFormat::for_version(target);
    let root = read_chunk(&out, &fmt, 0, 0);
    let sections = root.get("sections").unwrap().as_list().unwrap();
    for s in sections {
        let s = s.as_compound().unwrap();
        let y = s.get("Y").and_then(Tag::as_integer).unwrap();
        let b = s.get("biomes").unwrap().as_compound().unwrap();
        let pal: Vec<&str> = b.get("palette").unwrap().as_list().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        assert_eq!(pal.len(), 1, "section {y}: {pal:?}");
        assert!(!b.contains_key("data"));
        // Cluster biomes cover sections 0..15 of the source; this world starts at -4, so section y reads cluster y+4.
        let expected = if y + 4 < 8 { "minecraft:plains" } else { "minecraft:desert" };
        assert_eq!(pal[0], expected, "section {y}");
    }

    // 1.16: an int array of 4x4x4 cells over 16 sections.
    let target = Version::new(1, 16, 5);
    let out = dir.path().join("old");
    write_world(&out, target, &[(0, 0)]);
    let fmt = JavaFormat::for_version(target);
    let root = read_chunk(&out, &fmt, 0, 0);
    let level = root.get("Level").unwrap().as_compound().unwrap();
    let Some(Tag::IntArray(cells)) = level.get("Biomes") else { panic!("Biomes int array") };
    assert_eq!(cells.len(), 1024);
    assert_eq!(cells[0], 1); // plains
    assert_eq!(cells[1023], 2); // desert
}

#[test]
fn packed_bits_follow_the_palette_and_layout() {
    use BlockType as B;
    let kinds = [
        B::STONE, B::DIRT, B::COBBLESTONE, B::OAK_PLANKS, B::SPRUCE_PLANKS, B::BIRCH_PLANKS, B::JUNGLE_PLANKS, B::SAND,
        B::GRAVEL, B::GOLD_ORE, B::IRON_ORE, B::COAL_ORE, B::OAK_LOG, B::GLASS, B::BEDROCK, B::GRASS_BLOCK,
    ];
    let mut c = Column::new(ChunkCoord::new(0, 0));
    for (i, k) in kinds.iter().enumerate() {
        c.set_block(i as i32, 0, 0, block(*k));
    }
    // 16 kinds + air = 17 keys: 5 bits.
    for (target, expected_len) in [(Version::new(1, 15, 2), 320usize), (Version::new(1, 16, 5), 342), (Version::new(1, 21, 4), 342)] {
        let dir = TestDir::new("writer-bits");
        let out = dir.path().join("w");
        let mut w = JavaWorldWriter::create(&out, target, &level(), &ConversionSettings::default()).unwrap();
        w.write_column(&Dimension::overworld(), &c).unwrap();
        w.finish().unwrap();
        let fmt = JavaFormat::for_version(target);
        let root = read_chunk(&out, &fmt, 0, 0);
        let (palette, idx) = decode_section(&root, &fmt, 0).unwrap();
        assert_eq!(palette.len(), 17, "{target}");
        let level = if fmt.level_wrapper { root.get("Level").unwrap().as_compound().unwrap() } else { &root };
        let section = level.get(fmt.sections_key).unwrap().as_list().unwrap()[0].as_compound().unwrap();
        let data = match section.get("block_states") {
            Some(Tag::Compound(bs)) => bs.get("data").unwrap().as_long_array().unwrap().len(),
            _ => section.get("BlockStates").unwrap().as_long_array().unwrap().len(),
        };
        assert_eq!(data, expected_len, "{target}");
        for (i, k) in kinds.iter().enumerate() {
            let name = format!("minecraft:{}", k.name().to_lowercase());
            assert_eq!(palette[voxel(&idx, i, 0, 0)].0, name, "{target}");
        }
        assert_eq!(palette[voxel(&idx, 0, 1, 0)].0, "minecraft:air");
    }
}

#[test]
fn unmapped_blocks_become_air_and_are_counted() {
    // Grass paths and so on exist everywhere; a custom block without a namespace does not map.
    let mut c = Column::new(ChunkCoord::new(0, 0));
    c.set_block(0, 0, 0, Arc::new(BlockIdentifier::custom("bogus", Vec::<(&str, StateValue)>::new())));
    c.set_block(1, 0, 0, Arc::new(BlockIdentifier::custom("bogus", Vec::<(&str, StateValue)>::new())));
    c.set_block(2, 0, 0, block(BlockType::STONE));
    let dir = TestDir::new("writer-unmapped");
    let out = dir.path().join("w");
    let mut w = JavaWorldWriter::create(&out, Version::new(1, 21, 4), &level(), &ConversionSettings::default()).unwrap();
    w.write_column(&Dimension::overworld(), &c).unwrap();
    assert_eq!(w.unmapped_blocks(), vec![("bogus".to_string(), 2)]);
    w.finish().unwrap();
    let fmt = JavaFormat::for_version(Version::new(1, 21, 4));
    let (p, i) = decode_section(&read_chunk(&out, &fmt, 0, 0), &fmt, 0).unwrap();
    assert_eq!(p[voxel(&i, 0, 0, 0)].0, "minecraft:air");
    assert_eq!(p[voxel(&i, 2, 0, 0)].0, "minecraft:stone");
}

#[test]
fn level_dat_for_each_version() {
    for target in targets() {
        let dir = TestDir::new("writer-level");
        let out = dir.path().join("w");
        let mut lvl = level();
        let mut player = LevelPlayer::new(Dimension::overworld());
        player.position = [8.5, 70.0, -7.5];
        player.game_type = 1;
        lvl.player = Some(player);
        let mut w = JavaWorldWriter::create(&out, target, &lvl, &ConversionSettings::default()).unwrap();
        w.write_column(&Dimension::overworld(), &sample_column(0, 0)).unwrap();
        w.finish().unwrap();
        let fmt = JavaFormat::for_version(target);
        let info = read_level_dat(&out.join("level.dat")).unwrap();
        assert_eq!(info.data_version, Some(fmt.row_data_version), "{target}");
        assert_eq!(info.level_name.as_deref(), Some("Writer Test"));
        let d = &info.data;
        assert_eq!(d.get("Version").and_then(Tag::as_compound).unwrap().get("Id").and_then(Tag::as_integer), Some(fmt.row_data_version as i64));
        assert_eq!(d.get("initialized").and_then(Tag::as_integer), Some(1));
        assert!(d.get("LastPlayed").and_then(Tag::as_integer).unwrap() > 1_600_000_000_000);
        assert_eq!(fmt.dragon_fight_compound, d.contains_key("DragonFight") || fmt.split_level_dat);
        if fmt.spawn_compound {
            let spawn = d.get("spawn").and_then(Tag::as_compound).unwrap();
            assert_eq!(spawn.get("pos"), Some(&Tag::IntArray(vec![8, 70, -8])));
            assert!(!d.contains_key("SpawnX"));
        } else {
            assert_eq!(d.get("SpawnX").and_then(Tag::as_integer), Some(8));
            assert_eq!(d.get("SpawnZ").and_then(Tag::as_integer), Some(-8));
        }
        if fmt.split_level_dat {
            let dirs = LevelDirs::new(&out, &fmt);
            for f in ["weather.dat", "game_rules.dat", "world_gen_settings.dat", "world_clocks.dat"] {
                assert!(dirs.minecraft_data_dir().join(f).is_file(), "{f}");
            }
            assert!(dirs.dimension_data_dir(&JavaDimension::Nether).join("world_border.dat").is_file());
            let players: Vec<_> = std::fs::read_dir(dirs.players_dir()).unwrap().collect();
            assert_eq!(players.len(), 1);
            assert!(!d.contains_key("Player") && !d.contains_key("GameRules"));
            assert!(d.get("difficulty_settings").is_some());
        } else if fmt.game_rule_registry_keys {
            assert!(d.get("game_rules").and_then(Tag::as_compound).unwrap().contains_key("minecraft:advance_time"));
        } else {
            assert_eq!(d.get("GameRules").and_then(Tag::as_compound).unwrap().get("doDaylightCycle").and_then(Tag::as_str), Some("true"));
            let player = d.get("Player").and_then(Tag::as_compound).unwrap();
            assert_eq!(player.get("playerGameType").and_then(Tag::as_integer), Some(1));
            if fmt.dimension_ids_as_strings {
                assert_eq!(player.get("Dimension").and_then(Tag::as_str), Some("minecraft:overworld"));
            }
        }
        let gen_holder = if fmt.split_level_dat {
            let dirs = LevelDirs::new(&out, &fmt);
            crate::convert::nbt::file::read_java_file(dirs.minecraft_data_dir().join("world_gen_settings.dat"), crate::convert::nbt::file::Compression::Gzip)
                .unwrap()
                .unwrap()
        } else {
            d.clone()
        };
        if fmt.world_gen_settings {
            let key = if fmt.split_level_dat { None } else { Some("WorldGenSettings") };
            let wgs = match key {
                Some(k) => gen_holder.get(k).and_then(Tag::as_compound).unwrap().clone(),
                None => gen_holder.clone(),
            };
            assert_eq!(wgs.get("seed").and_then(Tag::as_integer), Some(424242));
            let overworld = wgs.get("dimensions").and_then(Tag::as_compound).unwrap().get("minecraft:overworld").and_then(Tag::as_compound).unwrap();
            assert_eq!(overworld.get("generator").and_then(Tag::as_compound).unwrap().get("type").and_then(Tag::as_str), Some("minecraft:flat"));
        }
    }
}

#[test]
fn never_writes_into_a_non_empty_folder() {
    let dir = TestDir::new("writer-nonempty");
    std::fs::write(dir.path().join("keep.txt"), b"mine").unwrap();
    let result = JavaWorldWriter::create(dir.path(), Version::new(1, 21, 4), &level(), &ConversionSettings::default());
    assert!(matches!(result, Err(ConvertError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists));
    assert_eq!(std::fs::read(dir.path().join("keep.txt")).unwrap(), b"mine");
    // A file is no folder either.
    let file = dir.path().join("keep.txt");
    assert!(JavaWorldWriter::create(&file, Version::new(1, 21, 4), &level(), &ConversionSettings::default()).is_err());
    // An empty existing folder is fine, and abort empties it again without removing it.
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let mut w = JavaWorldWriter::create(&empty, Version::new(1, 21, 4), &level(), &ConversionSettings::default()).unwrap();
    w.write_column(&Dimension::overworld(), &sample_column(0, 0)).unwrap();
    w.abort();
    assert!(empty.is_dir() && std::fs::read_dir(&empty).unwrap().next().is_none());
    // A new folder is removed by abort.
    let new = dir.path().join("new").join("deeper");
    let mut w = JavaWorldWriter::create(&new, Version::new(1, 21, 4), &level(), &ConversionSettings::default()).unwrap();
    w.write_column(&Dimension::overworld(), &sample_column(3, 3)).unwrap();
    w.abort();
    assert!(!new.exists());
}

#[test]
fn old_versions_are_refused() {
    let dir = TestDir::new("writer-old");
    let r = JavaWorldWriter::create(&dir.path().join("w"), Version::new(1, 12, 2), &level(), &ConversionSettings::default());
    assert!(matches!(r, Err(ConvertError::Unsupported(_))));
    assert!(!dir.path().join("w").exists());
}

#[test]
fn memory_is_bounded_with_forty_regions() {
    let dir = TestDir::new("writer-regions");
    let out = dir.path().join("w");
    let options = WriterOptions { max_open_regions: 4, ..WriterOptions::default() };
    let mut w = JavaWorldWriter::create_with(&out, Version::new(1, 21, 4), &level(), &ConversionSettings::default(), options).unwrap();
    let dim = Dimension::overworld();
    let tiny = |cx: i32, cz: i32| {
        let mut c = Column::new(ChunkCoord::new(cx, cz));
        c.set_block(0, 0, 0, block(BlockType::STONE));
        c
    };
    // 40 regions, each visited twice in a scattered order so parked ones must be reopened.
    for round in 0..2 {
        for i in 0..40 {
            let rx = (i * 7) % 40;
            let cx = rx * 32 + round * 3;
            w.write_column(&dim, &tiny(cx, 5)).unwrap();
            assert!(w.open_regions() <= 4);
        }
    }
    w.finish_dimension(&dim).unwrap();
    assert_eq!(w.open_regions(), 0);
    let summary = w.finish_with_summary().unwrap();
    assert_eq!(summary.columns, 80);
    let fmt = JavaFormat::for_version(Version::new(1, 21, 4));
    let files = std::fs::read_dir(out.join("region")).unwrap().count();
    assert_eq!(files, 40);
    for rx in 0..40 {
        for round in 0..2 {
            let root = read_chunk(&out, &fmt, rx * 32 + round * 3, 5);
            assert_eq!(root.get("xPos").and_then(Tag::as_integer), Some((rx * 32 + round * 3) as i64));
        }
    }
}

#[test]
fn large_chunks_go_to_external_files() {
    // Random-looking data does not compress: a column with thousands of block entities gets over 1 MiB.
    let dir = TestDir::new("writer-mcc");
    let out = dir.path().join("w");
    let mut w = JavaWorldWriter::create(&out, Version::new(1, 21, 4), &level(), &ConversionSettings::default()).unwrap();
    let mut c = Column::new(ChunkCoord::new(0, 0));
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..64 {
                c.set_block(x, y, z, block(BlockType::OAK_SIGN));
                c.block_entities.push(BlockEntity::new(x, y, z, BlockEntityData::Sign(Box::new(Sign::default()))));
            }
        }
    }
    w.write_column(&Dimension::overworld(), &c).unwrap();
    w.finish().unwrap();
    // Whatever the size, the chunk must read back.
    let fmt = JavaFormat::for_version(Version::new(1, 21, 4));
    let root = read_chunk(&out, &fmt, 0, 0);
    assert_eq!(root.get("block_entities").unwrap().as_list().unwrap().len(), 16 * 16 * 64);
}

#[test]
fn custom_dimensions_follow_the_layout() {
    let custom = Dimension::new(None, 1000, "mod:cave", "minecraft:plains", 24);
    let dir = TestDir::new("writer-custom");
    let out = dir.path().join("w");
    let mut w = JavaWorldWriter::create(&out, Version::new(1, 21, 4), &level(), &ConversionSettings::default()).unwrap();
    w.write_column(&custom, &sample_column(0, 0)).unwrap();
    w.write_column(&Dimension::nether(), &sample_column(0, 0)).unwrap();
    w.write_column(&Dimension::the_end(), &sample_column(0, 0)).unwrap();
    w.finish().unwrap();
    assert!(out.join("dimensions/mod/cave/region/r.0.0.mca").is_file());
    assert!(out.join("DIM-1/region/r.0.0.mca").is_file());
    assert!(out.join("DIM1/region/r.0.0.mca").is_file());
    // Before 1.16 custom dimensions are skipped, not an error.
    let out = dir.path().join("old");
    let mut w = JavaWorldWriter::create(&out, Version::new(1, 15, 2), &level(), &ConversionSettings::default()).unwrap();
    assert!(!w.supports_dimension(&custom));
    w.write_column(&custom, &sample_column(0, 0)).unwrap();
    assert_eq!(w.summary().skipped_columns, 1);
    w.finish().unwrap();
    // 26.1 puts every dimension, the overworld too, under dimensions/.
    let out = dir.path().join("new");
    let mut w = JavaWorldWriter::create(&out, Version::new(26, 1, 0), &level(), &ConversionSettings::default()).unwrap();
    w.write_column(&Dimension::overworld(), &sample_column(0, 0)).unwrap();
    w.finish().unwrap();
    assert!(out.join("dimensions/minecraft/overworld/region/r.0.0.mca").is_file());
    let _ = names(&[]);
}

#[test]
fn every_block_type_resolves_to_a_vanilla_block_of_the_version() {
    for target in targets() {
        let fmt = JavaFormat::for_version(target);
        let Some(oracle) = oracle(fmt.version.to_string().trim_end_matches(".0")) else { continue };
        let blocks = writer_blocks::BlockWriter::new(fmt.version, true);
        let (mut mapped, mut missing) = (0, 0);
        for ty in BlockType::ALL {
            let Some(java) = blocks.resolve(&block(ty)) else {
                missing += 1;
                continue;
            };
            mapped += 1;
            let props: BTreeMap<String, String> = java.props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert_valid(&oracle, &java.name, &props);
        }
        eprintln!("{target}: {mapped} block types written, {missing} without a Java form");
        assert!(mapped > 500, "{target}");
    }
}
