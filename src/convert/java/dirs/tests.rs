// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
use super::*;
use crate::convert::java::nbtio;
use crate::convert::java::region::RegionWriter;
use crate::convert::java::testutil::{TestDir, sample_chunk};
use crate::convert::nbt::{Compound, Tag};

fn level_dat_bytes(data_version: i32, name: &str) -> Vec<u8> {
    let mut version = Compound::new();
    version.insert("Id", Tag::Int(data_version));
    version.insert("Name", Tag::String("test".into()));
    let mut data = Compound::new();
    data.insert("LevelName", Tag::String(name.into()));
    data.insert("Version", Tag::Compound(version));
    let mut root = Compound::new();
    root.insert("Data", Tag::Compound(data));
    nbtio::gzip(&nbtio::to_vec("", &Tag::Compound(root)))
}

fn make_region(dir: &Path, x: i32, z: i32) {
    fs::create_dir_all(dir).unwrap();
    let mut w = RegionWriter::create_in(dir, x, z).unwrap();
    w.write_compound(0, 0, &sample_chunk(0, 0)).unwrap();
    w.finish().unwrap();
}

#[test]
fn legacy_layout_paths() {
    let dirs = LevelDirs::with_layout("/w", false);
    assert_eq!(dirs.dimension_dir(&JavaDimension::Overworld), PathBuf::from("/w"));
    assert_eq!(dirs.dimension_dir(&JavaDimension::Nether), PathBuf::from("/w/DIM-1"));
    assert_eq!(dirs.dimension_dir(&JavaDimension::End), PathBuf::from("/w/DIM1"));
    let custom = JavaDimension::from_identifier("mod:deep/cave").unwrap();
    assert_eq!(dirs.dimension_dir(&custom), PathBuf::from("/w/dimensions/mod/deep/cave"));
    assert_eq!(dirs.region_dir(&JavaDimension::Nether), PathBuf::from("/w/DIM-1/region"));
    assert_eq!(dirs.entities_dir(&JavaDimension::Overworld), PathBuf::from("/w/entities"));
    assert_eq!(dirs.poi_dir(&JavaDimension::End), PathBuf::from("/w/DIM1/poi"));
    assert_eq!(dirs.players_dir(), PathBuf::from("/w/playerdata"));
    assert_eq!(dirs.maps_dir(), PathBuf::from("/w/data"));
    assert_eq!(dirs.map_file(12), PathBuf::from("/w/data/map_12.dat"));
    assert_eq!(dirs.map_counter_file(), PathBuf::from("/w/data/idcounts.dat"));
    assert_eq!(dirs.datapacks_dir(), PathBuf::from("/w/datapacks"));
    assert_eq!(dirs.level_dat(), PathBuf::from("/w/level.dat"));
    assert_eq!(dirs.parse_map_file_name("map_7.dat"), Some(7));
    assert_eq!(dirs.parse_map_file_name("7.dat"), None);
    assert_eq!(dirs.parse_map_file_name("idcounts.dat"), None);
}

#[test]
fn namespaced_layout_paths() {
    let dirs = LevelDirs::new("/w", &JavaFormat::for_data_version(4786));
    assert_eq!(dirs.dimension_dir(&JavaDimension::Overworld), PathBuf::from("/w/dimensions/minecraft/overworld"));
    assert_eq!(dirs.dimension_dir(&JavaDimension::Nether), PathBuf::from("/w/dimensions/minecraft/the_nether"));
    assert_eq!(dirs.dimension_dir(&JavaDimension::End), PathBuf::from("/w/dimensions/minecraft/the_end"));
    assert_eq!(dirs.region_dir(&JavaDimension::Overworld), PathBuf::from("/w/dimensions/minecraft/overworld/region"));
    assert_eq!(dirs.dimension_data_dir(&JavaDimension::End), PathBuf::from("/w/dimensions/minecraft/the_end/data/minecraft"));
    assert_eq!(dirs.players_dir(), PathBuf::from("/w/players/data"));
    assert_eq!(dirs.minecraft_data_dir(), PathBuf::from("/w/data/minecraft"));
    assert_eq!(dirs.maps_dir(), PathBuf::from("/w/data/minecraft/maps"));
    assert_eq!(dirs.map_file(3), PathBuf::from("/w/data/minecraft/maps/3.dat"));
    assert_eq!(dirs.map_counter_file(), PathBuf::from("/w/data/minecraft/maps/last_id.dat"));
    assert_eq!(dirs.parse_map_file_name("3.dat"), Some(3));
    assert_eq!(dirs.parse_map_file_name("map_3.dat"), None);
    assert_eq!(dirs.parse_map_file_name("-3.dat"), None);
    // The layout follows the format switch, not the version name.
    assert!(!LevelDirs::new("/w", &JavaFormat::for_data_version(4671)).dimension_dir(&JavaDimension::Nether).ends_with("dimensions/minecraft/the_nether"));
}

#[test]
fn dimension_ids() {
    for (id, dim) in [
        ("minecraft:overworld", JavaDimension::Overworld),
        ("minecraft:the_nether", JavaDimension::Nether),
        ("minecraft:the_end", JavaDimension::End),
    ] {
        assert_eq!(JavaDimension::from_identifier(id), Some(dim.clone()));
        assert_eq!(dim.identifier(), id);
    }
    assert_eq!(JavaDimension::Nether.legacy_id(), Some(-1));
    assert_eq!(JavaDimension::from_identifier("a:b").unwrap().legacy_id(), None);
    for bad in ["", "overworld", ":x", "x:", "a:..", "a:../b", "../x:y", "a:b//c", "a:/b", "a:b/", "A:b", "a b:c", "a:b\\c", "a:."] {
        assert_eq!(JavaDimension::from_identifier(bad), None, "{bad:?}");
    }
}

#[test]
fn uuid_from_ints() {
    assert_eq!(
        LevelDirs::uuid_from_ints(&[136456559, -1164358010, -2128179942, 990118895]).as_deref(),
        Some("0822296f-ba99-4e86-8126-8d1a3b0403ef")
    );
    assert_eq!(LevelDirs::uuid_from_ints(&[0, 0, 0, 0]).as_deref(), Some("00000000-0000-0000-0000-000000000000"));
    assert_eq!(LevelDirs::uuid_from_ints(&[1, 2, 3]), None);
    let dirs = LevelDirs::with_layout("/w", true);
    assert!(dirs.player_file("0822296f-ba99-4e86-8126-8d1a3b0403ef").is_some());
    assert!(dirs.player_file("../evil").is_none());
    assert!(dirs.player_file("").is_none());
}

#[test]
fn detects_a_legacy_folder_world() {
    let w = TestDir::new("legacy");
    fs::write(w.path().join("level.dat"), level_dat_bytes(3465, "Legacy Layout")).unwrap();
    make_region(&w.path().join("region"), 0, 0);
    make_region(&w.path().join("region"), -1, 0);
    make_region(&w.path().join("DIM-1").join("region"), 0, 0);
    fs::create_dir_all(w.path().join("entities")).unwrap();
    fs::create_dir_all(w.path().join("poi")).unwrap();
    fs::create_dir_all(w.path().join("DIM1")).unwrap(); // no region folder: not reported
    make_region(&w.path().join("dimensions").join("mod").join("deep").join("region"), 0, 0);
    // A session.lock is neither needed nor touched.
    let info = detect_java(w.path()).unwrap();
    assert_eq!(info.name, "Legacy Layout");
    assert_eq!(info.data_version, 3465);
    assert_eq!(info.version, Version::new(1, 20, 1));
    assert!(!info.from_zip && info.scratch.is_none());
    let ids: Vec<_> = info.dimensions.iter().map(|d| d.dimension.identifier()).collect();
    assert_eq!(ids, vec!["minecraft:overworld", "minecraft:the_nether", "mod:deep"]);
    assert_eq!(info.dimensions[0].region_files, 2);
    assert!(info.dimensions[0].has_entities && info.dimensions[0].has_poi && !info.dimensions[1].has_entities);
    assert!(!w.path().join("session.lock").exists());
    assert_eq!(info.dirs().players_dir(), w.path().join("playerdata"));
}

#[test]
fn detects_a_namespaced_world_and_finds_misplaced_vanilla_dims() {
    let w = TestDir::new("ns");
    fs::write(w.path().join("level.dat"), level_dat_bytes(5017, "")).unwrap();
    make_region(&w.path().join("dimensions/minecraft/overworld/region"), 0, 0);
    make_region(&w.path().join("dimensions/minecraft/the_end/region"), 1, 1);
    make_region(&w.path().join("DIM-1/region"), 0, 0); // old-style nether in a 26.x world: still found
    let info = detect_java(w.path()).unwrap();
    assert_eq!(info.name, w.path().file_name().unwrap().to_str().unwrap(), "falls back to the folder name");
    let ids: Vec<_> = info.dimensions.iter().map(|d| d.dimension.identifier()).collect();
    assert_eq!(ids, vec!["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]);
    assert!(info.dimensions[1].dir.ends_with("DIM-1"));
    assert!(info.format.split_level_dat);
}

fn zip_with(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let mut zw = zip::ZipWriter::new(File::create(path).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(data).unwrap();
    }
    zw.finish().unwrap();
}

fn region_bytes() -> Vec<u8> {
    let t = TestDir::new("regionbytes");
    let p = t.path().join("r.0.0.mca");
    let mut w = RegionWriter::create(&p, 0, 0).unwrap();
    w.write_compound(0, 0, &sample_chunk(0, 0)).unwrap();
    w.finish().unwrap();
    fs::read(&p).unwrap()
}

#[test]
fn detects_a_zip_with_a_top_folder() {
    let t = TestDir::new("zip");
    let zip_path = t.path().join("world.zip");
    zip_with(
        &zip_path,
        &[
            ("My World/level.dat", level_dat_bytes(2586, "Zipped")),
            ("My World/session.lock", b"locked".to_vec()),
            ("My World/region/r.0.0.mca", region_bytes()),
            ("My World/DIM-1/region/r.0.0.mca", region_bytes()),
        ],
    );
    let scratch = t.path().join("scratch");
    let info = detect_java_checked(&zip_path, &scratch, ZipLimits::default()).unwrap();
    assert!(info.from_zip);
    assert_eq!(info.name, "Zipped");
    assert_eq!(info.version, Version::new(1, 16, 5));
    assert_eq!(info.dimensions.len(), 2);
    assert!(info.root.starts_with(&scratch));
    assert!(!info.root.join("session.lock").exists(), "session.lock is not extracted");
    let root = info.root.clone();
    drop(info);
    assert!(!root.exists(), "the scratch copy goes away with the info");
}

#[test]
fn zip_with_level_dat_at_the_top() {
    let t = TestDir::new("zip-top");
    let zip_path = t.path().join("w.ZIP");
    zip_with(&zip_path, &[("level.dat", level_dat_bytes(1343, "Top")), ("region/r.0.0.mca", region_bytes())]);
    let info = detect_java_checked(&zip_path, &t.path().join("s"), ZipLimits::default()).unwrap();
    assert_eq!(info.version, Version::new(1, 12, 2));
    assert!(info.format.legacy_ids);
}

#[test]
fn zip_traversal_and_limits_are_rejected() {
    let t = TestDir::new("zip-bad");
    let scratch = t.path().join("s");
    for name in ["../evil.txt", "a/../../evil.txt", "/abs/evil.txt", "a\\..\\evil.txt"] {
        let p = t.path().join("bad.zip");
        zip_with(&p, &[("level.dat", level_dat_bytes(3465, "x")), (name, b"x".to_vec())]);
        let err = detect_java_checked(&p, &scratch, ZipLimits::default()).unwrap_err();
        assert!(matches!(err, DetectError::BadZip(_)), "{name}: {err}");
    }
    assert!(!t.path().join("evil.txt").exists());
    let p = t.path().join("big.zip");
    zip_with(&p, &[("level.dat", level_dat_bytes(3465, "x")), ("a.bin", vec![0u8; 100_000]), ("b.bin", vec![0u8; 100_000])]);
    let tiny_total = ZipLimits { max_total_bytes: 50_000, ..ZipLimits::default() };
    assert!(matches!(detect_java_checked(&p, &scratch, tiny_total), Err(DetectError::BadZip(_))));
    let tiny_entry = ZipLimits { max_entry_bytes: 10_000, ..ZipLimits::default() };
    assert!(matches!(detect_java_checked(&p, &scratch, tiny_entry), Err(DetectError::BadZip(_))));
    let few = ZipLimits { max_entries: 2, ..ZipLimits::default() };
    assert!(matches!(detect_java_checked(&p, &scratch, few), Err(DetectError::BadZip(_))));
    // Failed extractions leave no scratch folders behind.
    let left: Vec<_> = fs::read_dir(&scratch).map(|r| r.flatten().collect()).unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn not_a_world() {
    let t = TestDir::new("none");
    assert!(detect_java(t.path()).is_none(), "empty folder");
    assert!(detect_java(&t.path().join("missing")).is_none());
    fs::write(t.path().join("file.txt"), b"hi").unwrap();
    assert!(detect_java(&t.path().join("file.txt")).is_none());
    fs::write(t.path().join("bad.zip"), b"not a zip").unwrap();
    assert!(detect_java(&t.path().join("bad.zip")).is_none());
    // A level.dat without a Java data version (e.g. garbage) is not a world.
    fs::write(t.path().join("level.dat"), b"garbage").unwrap();
    assert!(detect_java(t.path()).is_none());
    // Two candidate worlds one folder down are ambiguous.
    let m = TestDir::new("multi");
    for n in ["a", "b"] {
        fs::create_dir_all(m.path().join(n)).unwrap();
        fs::write(m.path().join(n).join("level.dat"), level_dat_bytes(3465, n)).unwrap();
    }
    assert!(matches!(detect_java_checked(m.path(), m.path(), ZipLimits::default()), Err(DetectError::NoLevelDat)));
}

#[test]
fn a_world_one_folder_down_is_found() {
    let m = TestDir::new("nested");
    fs::create_dir_all(m.path().join("saves-copy")).unwrap();
    fs::write(m.path().join("saves-copy/level.dat"), level_dat_bytes(3465, "Nested")).unwrap();
    let info = detect_java(m.path()).unwrap();
    assert_eq!(info.name, "Nested");
    assert!(info.root.ends_with("saves-copy"));
}
