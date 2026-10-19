// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Integration tests on the real test world (skipped when it is not on this machine).

use super::data_version::BedrockVersion;
use super::db::{BedrockDb, DbOptions, LevelDb, ScratchDir};
use super::detect::detect_bedrock;
use super::keys::{self, ChunkKey, ChunkTag, DimensionId};
use super::level_dat::LevelDat;
use super::palette;
use std::collections::BTreeSet;
use std::time::Instant;

/// Open the test world's db through a scratch copy; `None` (skip) when the world is absent.
fn open_world() -> Option<(super::detect::BedrockWorldInfo, LevelDb)> {
    let Some(path) = super::test_world_path() else {
        eprintln!("test world not present, skipping");
        return None;
    };
    let info = detect_bedrock(&path)?;
    let db = LevelDb::open_read(&info.db_path()).expect("open db");
    Some((info, db))
}

#[test]
fn test_world_open_and_scan() {
    let Some(path) = super::test_world_path() else {
        eprintln!("test world not present, skipping");
        return;
    };
    let t0 = Instant::now();
    let info = detect_bedrock(&path).expect("detect");
    let t_detect = t0.elapsed();
    let t1 = Instant::now();
    let mut db = LevelDb::open_read(&info.db_path()).expect("open");
    let t_open = t1.elapsed();
    let t2 = Instant::now();
    let keys = db.iter_keys().unwrap();
    let t_scan = t2.elapsed();
    assert_eq!(keys.len(), 511, "live keys");

    // columns
    let cols = keys::enumerate_columns(&mut db).unwrap();
    {
        let mut census = std::collections::BTreeMap::new();
        for k in &keys {
            *census.entry((k.len(), k.get(k.len().wrapping_sub(2)).copied())).or_insert(0) += 1;
        }
        eprintln!("census {census:?}");
        let mut by_tag: std::collections::BTreeMap<u8, std::collections::BTreeSet<(i32, i32)>> = Default::default();
        for k in &keys {
            if let Some(p) = ChunkKey::parse(k) {
                by_tag.entry(p.tag.byte()).or_default().insert((p.x, p.z));
            }
        }
        for (t, set) in &by_tag {
            eprintln!("tag {t}: {} columns", set.len());
        }
    }
    // Chunker's rule (terrain, biome, block entity or entity records) finds 46; every column with any record is 93.
    assert_eq!(cols.len(), 46);
    let all = keys::all_columns_from_keys(keys.iter().map(|k| k.as_slice()));
    assert_eq!(all.len(), 93);
    assert!(cols.iter().all(|c| all.contains(c)));
    let cols = all;
    assert!(cols.iter().all(|c| c.dim == DimensionId::OVERWORLD));
    assert_eq!((cols.iter().map(|c| c.x).min(), cols.iter().map(|c| c.x).max()), (Some(-5), Some(22)));
    assert_eq!((cols.iter().map(|c| c.z).min(), cols.iter().map(|c| c.z).max()), (Some(-23), Some(2)));

    // key census
    let sub_keys: Vec<_> = keys.iter().filter_map(|k| ChunkKey::parse(k)).filter(|k| k.tag == ChunkTag::SubChunkPrefix).collect();
    assert_eq!(sub_keys.len(), 69);

    // decode every sub-chunk, re-encode it
    let t3 = Instant::now();
    let mut names = BTreeSet::new();
    let mut identical = 0;
    for k in &sub_keys {
        let key = keys::sub_chunk_key(k.dim, k.x, k.z, k.sub_y.unwrap());
        let value = db.get(&key).unwrap().expect("sub-chunk value");
        let sub = palette::decode_sub_chunk(&value, k.sub_y.unwrap()).expect("decode");
        assert_eq!((sub.version, sub.y), (9, k.sub_y.unwrap()));
        assert!(!sub.layers.is_empty());
        for layer in &sub.layers {
            for e in &layer.entries {
                names.insert(e.name.clone());
            }
        }
        if palette::encode_sub_chunk(&sub).unwrap() == value {
            identical += 1;
        }
    }
    let t_decode = t3.elapsed();
    eprintln!("distinct block names: {}", names.len());
    eprintln!("{:?}", names);
    assert_eq!(names.len(), 78);
    assert_eq!(identical, sub_keys.len(), "sub-chunk re-encoding is byte-identical");

    // Data3D
    let mut data3d = 0;
    for c in &cols {
        let key = keys::chunk_key(c.dim, c.x, c.z, ChunkTag::Data3D);
        if let Some(v) = db.get(&key).unwrap() {
            let d = palette::decode_data3d(&v).expect("data3d");
            assert_eq!(d.heights.len(), 256);
            assert!(!d.sections.is_empty());
            assert_eq!(palette::encode_data3d(&d).unwrap(), v, "Data3D re-encoding is byte-identical");
            data3d += 1;
        }
    }
    assert_eq!(data3d, 46);

    // the digp records and the actors they list
    let digp = db.scan_prefix(keys::DIGP_PREFIX).unwrap();
    assert_eq!(digp.len(), 93);
    let listed: usize = digp.iter().map(|(_, v)| keys::parse_digp_value(v).len()).sum();
    let actors = db.scan_prefix(keys::ACTOR_PREFIX).unwrap();
    assert_eq!(actors.len(), 2);
    eprintln!("digp entries listing {listed} actors, {} actorprefix records", actors.len());

    // block entities decode as concatenated named compounds
    let mut be = 0;
    for c in &cols {
        if let Some(v) = db.get(&keys::chunk_key(c.dim, c.x, c.z, ChunkTag::BlockEntity)).unwrap() {
            be += palette::decode_nbt_concat(&v).expect("block entities").len();
        }
    }
    eprintln!("block entities: {be}");

    eprintln!(
        "timing: detect+extract {t_detect:?}, open (scratch copy) {t_open:?}, scan {} keys {t_scan:?}, decode {} sub-chunks {t_decode:?}",
        keys.len(),
        sub_keys.len()
    );
}

#[test]
fn test_world_level_dat() {
    let Some((info, _db)) = open_world() else { return };
    let bytes = std::fs::read(info.level_dat_path()).unwrap();
    let level = LevelDat::from_bytes(&bytes).unwrap();
    assert_eq!(level.storage_version, 10);
    let i = level.info();
    assert_eq!(i.last_opened_with, Some(vec![1, 26, 51, 1, 0]));
    assert_eq!(i.minimum_compatible, Some(vec![1, 26, 50, 0, 0]));
    assert_eq!(i.network_version, Some(1001));
    assert_eq!(i.storage_version_tag, Some(10));
    assert_eq!(i.inventory_version.as_deref(), Some("1.26.33"));
    assert_eq!(level.bedrock_version().game, [1, 26, 50]);
    assert_eq!(BedrockVersion::nearest_at_or_below([1, 26, 51]).protocol, 2192);
    let re = level.to_bytes().unwrap();
    if re != bytes {
        eprintln!("level.dat re-encoding differs: {} vs {} bytes", re.len(), bytes.len());
    }
    // Whatever the byte layout, it must parse back to the same data.
    assert_eq!(LevelDat::from_bytes(&re).unwrap(), level);
    assert_eq!(re, bytes, "level.dat re-encoding is byte-identical");
}

#[test]
fn copy_of_the_test_world_through_a_new_db() {
    let Some((_info, mut db)) = open_world() else { return };
    let scratch = ScratchDir::new("copy").unwrap();
    let out = scratch.path().join("db");
    let mut entries = Vec::new();
    db.scan(&[], &mut |k, v| {
        entries.push((k.to_vec(), v.to_vec()));
        true
    })
    .unwrap();
    assert_eq!(entries.len(), 511);
    {
        let mut new = LevelDb::create_new(&out, &DbOptions::default()).unwrap();
        let mut batch = super::db::Batch::new();
        for (k, v) in &entries {
            batch.put(k, v);
        }
        new.write_batch(&batch).unwrap();
        new.compact().unwrap();
    }
    let mut re = LevelDb::open_read(&out).unwrap();
    let mut back = Vec::new();
    re.scan(&[], &mut |k, v| {
        back.push((k.to_vec(), v.to_vec()));
        true
    })
    .unwrap();
    assert_eq!(back, entries);
    // Leave the copy where an independent reader can be pointed at it.
    if let Some(dir) = std::env::var_os("BEDROCK_KEEP_COPY") {
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for e in std::fs::read_dir(&out).unwrap() {
            let e = e.unwrap();
            std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
        }
    }
}
