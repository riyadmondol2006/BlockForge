// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Cross-checks against a real Java world on this machine (skipped cleanly when there is none).
//!
//! Looked up in order: `$CONVERT_JAVA_WORLD`, `~/.local/share/blockforge/instances/*/saves/*`, `~/.minecraft/saves/*`,
//! and `~/Minecraft263-server/*/world` (a 26.x server world). Run with `--nocapture` to see the report.

use super::dirs::{LevelDirs, detect_java};
use super::format::SectionLayout;
use super::nbtio;
use super::palette::{self, Layout};
use super::region::{RegionFile, RegionKind, list_region_files};
use crate::convert::nbt::{Compound, Tag};
use std::path::PathBuf;
use std::time::Instant;

fn candidate_worlds() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("CONVERT_JAVA_WORLD") {
        out.push(PathBuf::from(p));
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return out };
    let mut push_children = |dir: PathBuf| {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("level.dat").is_file()).collect();
            v.sort();
            out.extend(v);
        }
    };
    if let Ok(rd) = std::fs::read_dir(home.join(".local/share/blockforge/instances")) {
        for inst in rd.flatten() {
            push_children(inst.path().join("saves"));
        }
    }
    push_children(home.join(".minecraft/saves"));
    if let Ok(rd) = std::fs::read_dir(home.join("Minecraft263-server")) {
        for server in rd.flatten() {
            if server.path().join("world/level.dat").is_file() {
                out.push(server.path().join("world"));
            }
        }
    }
    out
}

fn list<'a>(c: &'a Compound, key: &str) -> Option<&'a Vec<Tag>> {
    match c.get(key) {
        Some(Tag::List(l)) => Some(l),
        _ => None,
    }
}

fn longs<'a>(c: &'a Compound, key: &str) -> Option<&'a Vec<i64>> {
    match c.get(key) {
        Some(Tag::LongArray(l)) => Some(l),
        _ => None,
    }
}

#[derive(Default, Debug)]
struct Counts {
    chunks: usize,
    sections: usize,
    block_palettes: usize,
    block_with_data: usize,
    block_exact_repack: usize,
    biome_with_data: usize,
    biome_exact_repack: usize,
    padded: usize,
    spanning: usize,
    max_keys: usize,
}

/// decode -> encode -> decode must give identical indices; returns whether the packed longs were identical too.
fn check_palette(min_bits: u32, count: usize, keys: usize, data: &[i64]) -> (bool, Layout) {
    let first = palette::unpack_auto(min_bits, count, keys, data).expect("decode a real palette");
    assert!(first.iter().all(|&i| (i as usize) < keys.max(1)), "an index beyond the palette: {keys} keys");
    let bits = palette::bits_for(min_bits, keys);
    let layout = if data.len() == palette::packed_len(Layout::Padded, count, bits) { Layout::Padded } else { Layout::Spanning };
    let again = palette::pack_palette(layout, min_bits, count, keys, Some(&first)).expect("encode");
    let second = palette::unpack_auto(min_bits, count, keys, &again).expect("decode what we encoded");
    assert_eq!(first, second, "decode(encode(decode(x))) differs");
    (again == data, layout)
}

fn check_chunk(root: &Compound, counts: &mut Counts) {
    let level = root.get_compound("Level").unwrap_or(root);
    let Some(sections) = list(level, "sections").or_else(|| list(level, "Sections")) else { return };
    counts.chunks += 1;
    for section in sections {
        let Tag::Compound(s) = section else { continue };
        counts.sections += 1;
        // 1.18+: block_states / biomes compounds; 1.13-1.17: Palette + BlockStates in the section.
        let (palette_tag, data) = match s.get_compound("block_states") {
            Some(bs) => (list(bs, "palette"), longs(bs, "data")),
            None => (list(s, "Palette"), longs(s, "BlockStates")),
        };
        if let Some(keys) = palette_tag {
            counts.block_palettes += 1;
            counts.max_keys = counts.max_keys.max(keys.len());
            if let (Some(data), true) = (data, keys.len() > 1) {
                counts.block_with_data += 1;
                let (exact, layout) = check_palette(palette::MIN_BITS_BLOCKS, 4096, keys.len(), data);
                counts.block_exact_repack += exact as usize;
                match layout {
                    Layout::Padded => counts.padded += 1,
                    Layout::Spanning => counts.spanning += 1,
                }
            }
        }
        if let Some(b) = s.get_compound("biomes") {
            if let (Some(keys), Some(data)) = (list(b, "palette"), longs(b, "data")) {
                if keys.len() > 1 {
                    counts.biome_with_data += 1;
                    let (exact, _) = check_palette(palette::MIN_BITS_BIOMES, 64, keys.len(), data);
                    counts.biome_exact_repack += exact as usize;
                }
            }
        }
    }
}

#[test]
fn real_world_region_round_trip() {
    let mut seen = Vec::new();
    for world in candidate_worlds() {
        let Some(info) = detect_java(&world) else { continue };
        let canonical = std::fs::canonicalize(&info.root).unwrap_or(info.root.clone());
        if seen.contains(&canonical) {
            continue;
        }
        seen.push(canonical);
        check_world(info);
    }
    if seen.is_empty() {
        eprintln!("real_world: no Java world found on this machine, skipping");
    }
}

fn check_world(info: super::dirs::JavaWorldInfo) {
    eprintln!(
        "real_world: {} ({}), data version {}, {:?}, dimensions {:?}",
        info.name,
        info.root.display(),
        info.data_version,
        info.version,
        info.dimensions.iter().map(|d| (d.dimension.identifier(), d.region_files)).collect::<Vec<_>>()
    );
    let dirs = LevelDirs::new(&info.root, &info.format);
    let Some(dim) = info.dimensions.iter().find(|d| d.region_files > 0) else {
        eprintln!("real_world: the world has no region files, skipping");
        return;
    };
    let dim = dim.clone();
    let regions = list_region_files(&RegionKind::Region.dir(&dim.dir)).unwrap();
    // The region with the most chunks.
    let mut best: Option<(usize, PathBuf)> = None;
    for r in &regions {
        let n = RegionFile::open_read(&r.path).map(|f| f.present_chunks().len()).unwrap_or(0);
        if best.as_ref().is_none_or(|(b, _)| n > *b) {
            best = Some((n, r.path.clone()));
        }
    }
    let (present, path) = best.unwrap();
    eprintln!("real_world: region {} with {present} chunks, {} bytes", path.display(), std::fs::metadata(&path).unwrap().len());

    // Timing: read + decompress every chunk, then parse every chunk's NBT.
    let started = Instant::now();
    let mut region = RegionFile::open_read(&path).unwrap();
    let mut blobs = Vec::new();
    let mut raw_bytes = 0usize;
    for item in region.chunks() {
        let (x, z, bytes) = item.expect("a real chunk reads");
        raw_bytes += bytes.len();
        blobs.push((x, z, bytes));
    }
    let read_time = started.elapsed();
    let started = Instant::now();
    let mut counts = Counts::default();
    let mut roots = Vec::new();
    for (_, _, bytes) in &blobs {
        let (_, tag) = nbtio::from_slice(bytes).expect("a real chunk parses");
        let Tag::Compound(root) = tag else { panic!("chunk root is not a compound") };
        roots.push(root);
    }
    let parse_time = started.elapsed();
    let started = Instant::now();
    for root in &roots {
        check_chunk(root, &mut counts);
    }
    let repack_time = started.elapsed();
    eprintln!(
        "real_world: read+decompress {} chunks ({} MiB of NBT) in {:?}; parse NBT in {:?}; palette re-pack check in {:?}",
        blobs.len(),
        raw_bytes >> 20,
        read_time,
        parse_time,
        repack_time
    );
    eprintln!("real_world: {counts:?}");
    assert_eq!(blobs.len(), present);
    rewrite_and_compare(&path, &blobs);
    if counts.sections > 0 {
        assert!(counts.block_palettes > 0);
    }
    // Entities and POI folders of the same dimension read too, when present.
    for kind in [RegionKind::Entities, RegionKind::Poi] {
        let files = list_region_files(&kind.dir(&dim.dir)).unwrap();
        if let Some(f) = files.first() {
            let mut r = RegionFile::open_read(&f.path).unwrap();
            let n = r.chunks().filter(|c| c.is_ok()).count();
            eprintln!("real_world: {:?} {}: {n} chunks", kind, f.path.file_name().unwrap().to_string_lossy());
        }
    }
    // The section layout matches what the format switch says for chunks that have sections.
    if counts.sections > 0 && info.format.sections == SectionLayout::NestedStates {
        assert_eq!(counts.spanning, 0, "1.18+ worlds use the padded layout");
    }
    let _ = dirs;
}

/// Write the chunks back with every compression type and read them again: the bytes must be identical.
fn rewrite_and_compare(original: &std::path::Path, blobs: &[(u8, u8, Vec<u8>)]) {
    use super::region::{Compression, RegionWriter};
    let (rx, rz) = RegionFile::open_read(original).unwrap().region_coords().unwrap();
    let dir = super::testutil::TestDir::new("rewrite-real");
    for compression in [Compression::Zlib, Compression::Gzip, Compression::None, Compression::Lz4] {
        let sub = dir.path().join(format!("{}", compression as u8));
        let started = Instant::now();
        let mut w = RegionWriter::create_in(&sub, rx, rz).unwrap();
        w.set_compression(compression);
        for (x, z, bytes) in blobs {
            w.write_chunk(*x, *z, bytes).unwrap();
        }
        let path = sub.join(super::region::region_file_name(rx, rz));
        w.finish().unwrap();
        let write_time = started.elapsed();
        let mut r = RegionFile::open_read(&path).unwrap();
        let mut n = 0;
        for item in r.chunks() {
            let (x, z, bytes) = item.unwrap();
            let orig = blobs.iter().find(|(ox, oz, _)| (*ox, *oz) == (x, z)).unwrap();
            assert!(orig.2 == bytes, "chunk {x},{z} differs after a {compression:?} round trip");
            n += 1;
        }
        assert_eq!(n, blobs.len());
        eprintln!("real_world: rewrote {n} chunks as {compression:?} in {write_time:?}, {} bytes", std::fs::metadata(&path).unwrap().len());
        if compression == Compression::Zlib {
            if let Some(keep) = std::env::var_os("CONVERT_JAVA_SCRATCH") {
                let target = std::path::PathBuf::from(keep);
                let _ = std::fs::create_dir_all(&target);
                let _ = std::fs::copy(&path, target.join(super::region::region_file_name(rx, rz)));
            }
        }
    }
}
