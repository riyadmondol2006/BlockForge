// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
use super::*;
use crate::convert::java::testutil::{Rng, TestDir, sample_chunk};

fn write_full_region(path: &Path, compression: Compression) {
    let mut w = RegionWriter::create(path, 0, 0).unwrap();
    w.set_compression(compression);
    w.set_timestamp(1_700_000_000);
    for z in 0..32u8 {
        for x in 0..32u8 {
            w.write_compound(x, z, &sample_chunk(x as i32, z as i32)).unwrap();
        }
    }
    assert_eq!(w.chunk_count(), 1024);
    w.finish().unwrap();
}

#[test]
fn round_trip_each_compression() {
    let dir = TestDir::new("rt");
    for compression in [Compression::Gzip, Compression::Zlib, Compression::None, Compression::Lz4] {
        let path = dir.path().join(format!("r.0.0.{}.mca", compression as u8));
        let path = {
            let p = dir.path().join(format!("{}", compression as u8));
            std::fs::create_dir_all(&p).unwrap();
            let _ = path;
            p.join("r.0.0.mca")
        };
        write_full_region(&path, compression);
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(len % SECTOR as u64, 0, "file is a whole number of sectors");
        let mut r = RegionFile::open_read(&path).unwrap();
        assert_eq!(r.region_coords(), Some((0, 0)));
        assert_eq!(r.present_chunks().len(), 1024);
        assert_eq!(r.timestamp(3, 4), 1_700_000_000);
        let mut seen = 0;
        for item in r.chunks() {
            let (x, z, bytes) = item.unwrap();
            let (_, tag) = nbtio::from_slice(&bytes).unwrap();
            assert_eq!(tag, Tag::Compound(sample_chunk(x as i32, z as i32)), "{compression:?} chunk {x},{z}");
            seen += 1;
        }
        assert_eq!(seen, 1024);
        assert_eq!(r.read_chunk(31, 31).unwrap().unwrap(), sample_chunk(31, 31));
    }
}

#[test]
fn sparse_region_and_absent_chunks() {
    let dir = TestDir::new("sparse");
    let path = dir.path().join("r.-3.2.mca");
    let mut w = RegionWriter::create(&path, -3, 2).unwrap();
    w.write_compound(5, 6, &sample_chunk(-96 + 5, 64 + 6)).unwrap();
    w.write_compound(0, 0, &sample_chunk(-96, 64)).unwrap();
    w.finish().unwrap();
    let mut r = RegionFile::open_read(&path).unwrap();
    assert_eq!(r.region_coords(), Some((-3, 2)));
    assert_eq!(r.present_chunks(), vec![(0, 0), (5, 6)]);
    assert!(r.has_chunk(5, 6) && !r.has_chunk(6, 5) && !r.has_chunk(40, 0));
    assert_eq!(r.read_chunk_bytes(1, 1).unwrap(), None, "chunk is not here");
    assert!(r.read_chunk(5, 6).unwrap().is_some());
    assert!(matches!(r.read_chunk_bytes(32, 0), Err(RegionError::BadArgument(_))));
}

#[test]
fn empty_and_header_only_files() {
    let dir = TestDir::new("empty");
    let p0 = dir.path().join("r.0.0.mca");
    std::fs::write(&p0, b"").unwrap();
    let mut r = RegionFile::open_read(&p0).unwrap();
    assert!(r.is_empty() && r.present_chunks().is_empty());
    assert_eq!(r.chunks().count(), 0);
    let p1 = dir.path().join("r.1.0.mca");
    std::fs::write(&p1, vec![0u8; HEADER_LEN]).unwrap();
    let mut r = RegionFile::open_read(&p1).unwrap();
    assert_eq!(r.chunks().count(), 0);
    let p2 = dir.path().join("r.2.0.mca");
    std::fs::write(&p2, vec![0u8; 100]).unwrap();
    assert!(matches!(RegionFile::open_read(&p2), Err(RegionError::Truncated(_))));
    assert!(RegionFile::open_read(dir.path().join("nope.mca")).is_err());
    // A finished writer with no chunks is a valid empty region.
    let p3 = dir.path().join("r.3.0.mca");
    RegionWriter::create(&p3, 3, 0).unwrap().finish().unwrap();
    assert_eq!(std::fs::metadata(&p3).unwrap().len(), HEADER_LEN as u64);
    assert_eq!(RegionFile::open_read(&p3).unwrap().present_chunks().len(), 0);
}

/// A chunk of about `bytes` incompressible bytes, as 4096-long arrays (the NBT reader caps one array at 65536).
fn big_chunk(bytes: usize, seed: u64) -> Compound {
    let mut rng = Rng(seed);
    let arrays: Vec<Tag> =
        (0..bytes.div_ceil(32768)).map(|_| Tag::LongArray((0..4096).map(|_| rng.next() as i64).collect())).collect();
    let mut c = Compound::new();
    c.insert("xPos", Tag::Int(1));
    c.insert("blob", Tag::List(arrays));
    c
}

#[test]
fn oversized_chunk_uses_mcc() {
    let dir = TestDir::new("mcc");
    let path = dir.path().join("r.1.-1.mca");
    let mut w = RegionWriter::create(&path, 1, -1).unwrap();
    let big = big_chunk(1_300_000, 77);
    w.write_compound(2, 3, &sample_chunk(34, -29)).unwrap();
    w.write_compound(4, 5, &big).unwrap(); // chunk 36, -27
    w.write_compound(6, 7, &sample_chunk(38, -25)).unwrap();
    w.finish().unwrap();
    let mcc = dir.path().join("c.36.-27.mcc");
    assert!(mcc.exists(), "external chunk file {}", mcc.display());
    assert!(std::fs::metadata(&mcc).unwrap().len() > 1_000_000);
    // The region keeps a one-sector stub: length 1, type 0x82.
    let raw = std::fs::read(&path).unwrap();
    let entry = u32::from_be_bytes(raw[(4 + 5 * 32) * 4..(4 + 5 * 32) * 4 + 4].try_into().unwrap());
    assert_eq!(entry & 0xFF, 1);
    let pos = (entry >> 8) as usize * SECTOR;
    assert_eq!(&raw[pos..pos + 5], &[0, 0, 0, 1, 0x82]);
    assert_eq!(raw.len() % SECTOR, 0);
    let mut r = RegionFile::open_read(&path).unwrap();
    assert_eq!(r.read_chunk(4, 5).unwrap().unwrap(), big);
    assert_eq!(r.read_chunk(6, 7).unwrap().unwrap(), sample_chunk(38, -25));
    // Neighbours are unaffected and the iterator yields all three.
    assert_eq!(r.chunks().filter(|c| c.is_ok()).count(), 3);
    // Removing the external file is an error for that chunk only.
    std::fs::remove_file(&mcc).unwrap();
    assert!(matches!(r.read_chunk_bytes(4, 5), Err(RegionError::MissingExternal(_))));
    assert!(r.read_chunk_bytes(6, 7).unwrap().is_some());
}

#[test]
fn sector_limit_is_exact() {
    let dir = TestDir::new("limit");
    let path = dir.path().join("r.0.0.mca");
    let mut w = RegionWriter::create(&path, 0, 0).unwrap();
    let fits = vec![9u8; MAX_SECTORS * SECTOR - 5];
    let spills = vec![8u8; MAX_SECTORS * SECTOR - 4];
    w.write_compressed(0, 0, Compression::None, &fits).unwrap();
    w.write_compressed(1, 0, Compression::None, &spills).unwrap();
    w.finish().unwrap();
    assert!(!dir.path().join("c.0.0.mcc").exists());
    assert!(dir.path().join("c.1.0.mcc").exists());
    let mut r = RegionFile::open_read(&path).unwrap();
    assert_eq!(r.read_chunk_bytes(0, 0).unwrap().unwrap(), fits);
    assert_eq!(r.read_chunk_bytes(1, 0).unwrap().unwrap(), spills);
}

#[test]
fn rewriting_a_chunk_keeps_the_file_valid() {
    let dir = TestDir::new("rewrite");
    let path = dir.path().join("r.0.0.mca");
    let mut w = RegionWriter::create(&path, 0, 0).unwrap();
    w.write_compound(0, 0, &sample_chunk(0, 0)).unwrap();
    w.write_compound(0, 0, &sample_chunk(100, 100)).unwrap();
    assert_eq!(w.chunk_count(), 1);
    w.finish().unwrap();
    let mut r = RegionFile::open_read(&path).unwrap();
    assert_eq!(r.read_chunk(0, 0).unwrap().unwrap(), sample_chunk(100, 100));
}

/// Build a small region with the raw header bytes under test control.
fn raw_region(entries: &[(usize, u32)], body: &[(usize, Vec<u8>)], total_sectors: usize) -> Vec<u8> {
    let mut buf = vec![0u8; total_sectors * SECTOR];
    for &(i, e) in entries {
        buf[i * 4..i * 4 + 4].copy_from_slice(&e.to_be_bytes());
    }
    for (sector, bytes) in body {
        buf[sector * SECTOR..sector * SECTOR + bytes.len()].copy_from_slice(bytes);
    }
    buf
}

#[test]
fn malformed_entries_are_errors() {
    let dir = TestDir::new("bad");
    let path = dir.path().join("r.0.0.mca");
    let good = nbtio::to_vec("", &Tag::Compound(sample_chunk(0, 0)));
    let zl = Compression::Zlib.compress(&good);
    let mut chunk = ((zl.len() + 1) as u32).to_be_bytes().to_vec();
    chunk.push(2);
    chunk.extend_from_slice(&zl);
    let huge = {
        let mut v = 0xFFFF_FFFFu32.to_be_bytes().to_vec();
        v.push(2);
        v
    };
    let zero_len = vec![0, 0, 0, 0, 2];
    let type0 = vec![0, 0, 0, 5, 0, 1, 2, 3, 4];
    let type9 = vec![0, 0, 0, 5, 9, 1, 2, 3, 4];
    let garbage = vec![0, 0, 0, 5, 2, 1, 2, 3, 4];
    let not_compound = {
        let raw = nbtio::to_vec("", &Tag::Int(3));
        let z = Compression::Zlib.compress(&raw);
        let mut v = ((z.len() + 1) as u32).to_be_bytes().to_vec();
        v.push(2);
        v.extend_from_slice(&z);
        v
    };
    let bytes = raw_region(
        &[
            (0, 2 << 8 | 1),   // good
            (1, 3 << 8 | 1),   // length 0xFFFFFFFF
            (2, 4 << 8 | 1),   // zero length
            (3, 5 << 8 | 1),   // type 0: none
            (4, 6 << 8 | 1),   // type 9
            (5, 7 << 8 | 1),   // garbage zlib
            (6, 1 << 8 | 1),   // inside the header
            (7, 500 << 8 | 1), // far past the end
            (8, 8 << 8 | 1),   // not a compound root
            (9, 9 << 8 | 1),   // starts exactly at the end of the file: no room for a header
        ],
        &[(2, chunk), (3, huge), (4, zero_len), (5, type0), (6, type9), (7, garbage), (8, not_compound)],
        9,
    );
    std::fs::write(&path, &bytes).unwrap();
    let mut r = RegionFile::open_read(&path).unwrap();
    assert!(r.read_chunk(0, 0).unwrap().is_some());
    assert!(matches!(r.read_chunk_bytes(1, 0), Err(RegionError::BadChunk(1, 0, _))));
    assert!(matches!(r.read_chunk_bytes(2, 0), Err(RegionError::BadChunk(..))));
    assert_eq!(r.read_chunk_bytes(3, 0).unwrap(), None);
    assert!(matches!(r.read_chunk_bytes(4, 0), Err(RegionError::UnsupportedCompression(9))));
    assert!(matches!(r.read_chunk_bytes(5, 0), Err(RegionError::Decompress(_))));
    assert!(matches!(r.read_chunk_bytes(6, 0), Err(RegionError::BadChunk(..))));
    assert!(matches!(r.read_chunk_bytes(7, 0), Err(RegionError::BadChunk(..))));
    assert!(matches!(r.read_chunk(8, 0), Err(RegionError::Nbt(_))));
    assert!(matches!(r.read_chunk_bytes(9, 0), Err(RegionError::BadChunk(..))));
    // The iterator reports the errors and carries on.
    let results: Vec<_> = r.chunks().collect();
    assert_eq!(results.len(), 9, "type-0 chunk is skipped");
    assert!(results[0].is_ok());
}

#[test]
fn decompression_is_capped() {
    let zeros = vec![0u8; 5_000_000];
    for c in [Compression::Gzip, Compression::Zlib, Compression::Lz4, Compression::None] {
        let packed = c.compress(&zeros);
        assert_eq!(c.decompress(&packed, 10_000_000).unwrap().len(), zeros.len());
        assert!(c.decompress(&packed, 1_000_000).is_err(), "{c:?} must stop at the limit");
    }
}

#[test]
fn truncation_and_bit_flips_never_panic() {
    let dir = TestDir::new("fuzz");
    let path = dir.path().join("r.0.0.mca");
    let mut w = RegionWriter::create(&path, 0, 0).unwrap();
    for i in 0..40u8 {
        w.write_compound(i % 32, i / 32, &sample_chunk(i as i32, 0)).unwrap();
    }
    w.write_compound(7, 7, &big_chunk(200_000, 5)).unwrap();
    w.finish().unwrap();
    let good = std::fs::read(&path).unwrap();
    let probe = |bytes: &[u8]| {
        let p = dir.path().join("r.0.0.mca");
        std::fs::write(&p, bytes).unwrap();
        if let Ok(mut r) = RegionFile::open_read(&p) {
            for item in r.chunks() {
                let _ = item;
            }
            for z in 0..32u8 {
                for x in 0..32u8 {
                    let _ = r.read_chunk(x, z);
                }
            }
        }
    };
    for cut in [0, 1, 4095, 4096, 4100, 8191, 8192, 8193, 8197, 9000, good.len() / 2, good.len() - 1] {
        probe(&good[..cut]);
    }
    let mut rng = Rng(0xDEADBEEF);
    for round in 0..300 {
        let mut bytes = good.clone();
        let flips = 1 + (round % 8);
        for _ in 0..flips {
            // Mostly hit the header and chunk headers, sometimes the bodies.
            let at =
                if rng.next() % 2 == 0 { (rng.next() % 8200) as usize } else { (rng.next() as usize) % bytes.len() };
            bytes[at] ^= 1 << (rng.next() % 8);
        }
        probe(&bytes);
    }
}

#[test]
fn file_names_and_coordinates() {
    assert_eq!(region_file_name(-1, 20), "r.-1.20.mca");
    assert_eq!(external_file_name(-33, 5), "c.-33.5.mcc");
    assert_eq!(parse_region_file_name("r.-1.20.mca"), Some((-1, 20)));
    assert_eq!(parse_region_file_name("r.1.mca"), None);
    assert_eq!(parse_region_file_name("r.1.2.mcr"), None);
    assert_eq!(parse_region_file_name("r.a.2.mca"), None);
    assert_eq!(parse_region_file_name("x.1.2.mca"), None);
    assert_eq!(parse_region_file_name("r.1.2.mca.bak"), None);
    assert_eq!(chunk_to_region(0, 0), (0, 0, 0, 0));
    assert_eq!(chunk_to_region(31, 32), (0, 1, 31, 0));
    assert_eq!(chunk_to_region(-1, -32), (-1, -1, 31, 0));
    assert_eq!(chunk_to_region(-33, -33), (-2, -2, 31, 31));
    assert_eq!(RegionKind::Entities.dir(Path::new("/w")), PathBuf::from("/w/entities"));
}

#[test]
fn listing_region_files() {
    let dir = TestDir::new("list");
    let region = RegionKind::Region.dir(dir.path());
    std::fs::create_dir_all(&region).unwrap();
    assert!(list_region_files(&dir.path().join("missing")).unwrap().is_empty());
    for (x, z) in [(1, 0), (-1, 5), (0, 0)] {
        RegionWriter::create_in(&region, x, z).unwrap().finish().unwrap();
    }
    std::fs::write(region.join("r.9.9.mca"), b"short").unwrap();
    std::fs::write(region.join("notes.txt"), b"x").unwrap();
    std::fs::write(region.join("c.1.1.mcc"), vec![0u8; 5000]).unwrap();
    let listed = list_region_files(&region).unwrap();
    assert_eq!(listed.iter().map(|r| (r.x, r.z)).collect::<Vec<_>>(), vec![(-1, 5), (0, 0), (1, 0)]);
}

#[test]
fn entity_split_and_merge() {
    let entity = |id: &str| {
        let mut c = Compound::new();
        c.insert("id", Tag::String(id.into()));
        Tag::Compound(c)
    };
    // < 1.18: inside Level.
    let mut level = Compound::new();
    level.insert("Entities", Tag::List(vec![entity("minecraft:cow")]));
    let mut root = Compound::new();
    root.insert("Level", Tag::Compound(level));
    let taken = take_entities(&mut root).unwrap();
    assert!(matches!(&taken, Tag::List(l) if l.len() == 1));
    assert!(take_entities(&mut root).is_none());
    // 1.18+: at the root; an empty list is dropped from the chunk but not written.
    let mut flat = Compound::new();
    flat.insert("Entities", Tag::List(vec![]));
    assert!(take_entities(&mut flat).is_none());
    assert!(flat.get("Entities").is_none());

    let ent_root = entities_root(3465, -4, 9, taken);
    assert_eq!(ent_root.get("Position"), Some(&Tag::IntArray(vec![-4, 9])));
    assert_eq!(ent_root.get_i32("DataVersion"), Some(3465));

    // Merge into an existing main chunk (flat and Level-wrapped), or create one from Position.
    let mut main = Compound::new();
    main.insert("xPos", Tag::Int(-4));
    let merged = merge_entities(Some(main), Some(ent_root.clone())).unwrap();
    assert!(matches!(merged.get("Entities"), Some(Tag::List(l)) if l.len() == 1));
    let mut wrapped = Compound::new();
    wrapped.insert("Level", Tag::Compound(Compound::new()));
    let merged = merge_entities(Some(wrapped), Some(ent_root.clone())).unwrap();
    assert!(merged.get_compound("Level").unwrap().get("Entities").is_some());
    let only = merge_entities(None, Some(ent_root)).unwrap();
    assert_eq!((only.get_i32("xPos"), only.get_i32("zPos")), (Some(-4), Some(9)));
    assert!(merge_entities(None, Some(Compound::new())).is_none());
    assert!(merge_entities(None, None).is_none());
}
