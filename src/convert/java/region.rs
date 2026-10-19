// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Anvil `.mca` region files: a reader over the present chunks and a streaming writer.
//!
//! Layout: 4 KiB sectors. Sector 0 holds 1024 location entries (3-byte big-endian sector offset, 1-byte sector
//! count, entry `i` is chunk `x = i & 31`, `z = i >> 5`), sector 1 the 1024 timestamps. A chunk is a 4-byte length
//! (counting the type byte), a compression type byte and the payload. Types: 1 gzip, 2 zlib, 3 none, 4 LZ4Block
//! (1.20.5+); bit 0x80 means the payload lives in `c.<chunkX>.<chunkZ>.mcc` beside the region file.
//!
//! Nothing here panics or sizes an allocation from an unchecked length: every length is compared with the file size
//! first and decompressed output is capped at [`MAX_CHUNK_BYTES`].

use super::nbtio;
use crate::convert::nbt::{Compound, Tag};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Bytes in a sector.
pub const SECTOR: usize = 4096;
/// Bytes of header (locations + timestamps).
pub const HEADER_LEN: usize = 2 * SECTOR;
/// Most sectors a chunk can span inside the region file (the count is one byte); bigger ones go to `.mcc`.
pub const MAX_SECTORS: usize = 255;
/// Cap on the decompressed size of one chunk (Minecraft's own chunks stay far below this).
pub const MAX_CHUNK_BYTES: u64 = 256 << 20;
/// Cap on the size of an external `.mcc` file.
pub const MAX_EXTERNAL_BYTES: u64 = 512 << 20;
/// Chunks per region side.
pub const REGION_SIDE: i32 = 32;

/// How a chunk payload is compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Gzip = 1,
    Zlib = 2,
    None = 3,
    Lz4 = 4,
}

impl Compression {
    fn from_id(id: u8) -> Option<Compression> {
        match id {
            1 => Some(Compression::Gzip),
            2 => Some(Compression::Zlib),
            3 => Some(Compression::None),
            4 => Some(Compression::Lz4),
            _ => None,
        }
    }

    /// Compress bytes with this method.
    pub fn compress(self, data: &[u8]) -> Vec<u8> {
        match self {
            Compression::Gzip => nbtio::gzip(data),
            Compression::Zlib => nbtio::zlib(data),
            Compression::None => data.to_vec(),
            Compression::Lz4 => nbtio::lz4_block_encode(data),
        }
    }

    /// Decompress bytes with this method, producing at most `limit` bytes.
    pub fn decompress(self, data: &[u8], limit: u64) -> Result<Vec<u8>, RegionError> {
        let out = match self {
            Compression::Gzip => nbtio::gunzip(data, limit),
            Compression::Zlib => nbtio::unzlib(data, limit),
            Compression::None => Ok(data.to_vec()),
            Compression::Lz4 => nbtio::lz4_block_decode(data, limit),
        };
        let out = out.map_err(|e| RegionError::Decompress(e.to_string()))?;
        if out.len() as u64 > limit {
            return Err(RegionError::Decompress("chunk is larger than the limit".into()));
        }
        Ok(out)
    }
}

/// What can go wrong with a region file.
#[derive(Debug)]
pub enum RegionError {
    Io(io::Error),
    /// The file is too short or cut off: what part.
    Truncated(String),
    /// A chunk entry is inconsistent: chunk (x, z), reason.
    BadChunk(u8, u8, String),
    /// A compression type this reader does not know.
    UnsupportedCompression(u8),
    /// The chunk points at an external `.mcc` file that is not there (or has no known region coordinates).
    MissingExternal(PathBuf),
    Decompress(String),
    Nbt(String),
    /// Chunk coordinates outside 0..32, or a payload that does not fit a region.
    BadArgument(String),
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegionError::Io(e) => write!(f, "region file I/O: {e}"),
            RegionError::Truncated(what) => write!(f, "region file truncated: {what}"),
            RegionError::BadChunk(x, z, why) => write!(f, "bad chunk ({x}, {z}) in region file: {why}"),
            RegionError::UnsupportedCompression(t) => write!(f, "unsupported chunk compression type {t}"),
            RegionError::MissingExternal(p) => write!(f, "external chunk file {} is missing", p.display()),
            RegionError::Decompress(e) => write!(f, "chunk decompression failed: {e}"),
            RegionError::Nbt(e) => write!(f, "chunk NBT is invalid: {e}"),
            RegionError::BadArgument(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RegionError {}

impl From<io::Error> for RegionError {
    fn from(e: io::Error) -> RegionError {
        RegionError::Io(e)
    }
}

/// `r.X.Z.mca`.
pub fn region_file_name(region_x: i32, region_z: i32) -> String {
    format!("r.{region_x}.{region_z}.mca")
}

/// `c.X.Z.mcc` for an oversized chunk (absolute chunk coordinates).
pub fn external_file_name(chunk_x: i32, chunk_z: i32) -> String {
    format!("c.{chunk_x}.{chunk_z}.mcc")
}

/// Parse `r.X.Z.mca` (exactly four dot-separated parts, like Chunker).
pub fn parse_region_file_name(name: &str) -> Option<(i32, i32)> {
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
        return None;
    }
    Some((parts[1].parse().ok()?, parts[2].parse().ok()?))
}

/// Split absolute chunk coordinates into (region x, region z, local x, local z).
pub fn chunk_to_region(chunk_x: i32, chunk_z: i32) -> (i32, i32, u8, u8) {
    (
        chunk_x.div_euclid(REGION_SIDE),
        chunk_z.div_euclid(REGION_SIDE),
        chunk_x.rem_euclid(REGION_SIDE) as u8,
        chunk_z.rem_euclid(REGION_SIDE) as u8,
    )
}

/// The folders that hold region-format files for a dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// `region/`: chunks.
    Region,
    /// `entities/`: entity chunks (1.17+).
    Entities,
    /// `poi/`: points of interest.
    Poi,
}

impl RegionKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            RegionKind::Region => "region",
            RegionKind::Entities => "entities",
            RegionKind::Poi => "poi",
        }
    }

    /// The folder of this kind under a dimension directory.
    pub fn dir(self, dimension_dir: &Path) -> PathBuf {
        dimension_dir.join(self.dir_name())
    }
}

/// A region file found in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionRef {
    pub x: i32,
    pub z: i32,
    pub path: PathBuf,
    pub len: u64,
}

/// List the `r.X.Z.mca` files of a folder (sorted by x then z). Files under one sector are skipped like Chunker does;
/// a missing folder gives an empty list.
pub fn list_region_files(dir: &Path) -> io::Result<Vec<RegionRef>> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some((x, z)) = name.to_str().and_then(parse_region_file_name) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || meta.len() < SECTOR as u64 {
            continue;
        }
        out.push(RegionRef { x, z, path: entry.path(), len: meta.len() });
    }
    out.sort_by_key(|r| (r.x, r.z));
    Ok(out)
}

fn check_local(x: u8, z: u8) -> Result<usize, RegionError> {
    if x >= 32 || z >= 32 {
        return Err(RegionError::BadArgument(format!("chunk ({x}, {z}) is outside a 32 x 32 region")));
    }
    Ok(x as usize + ((z as usize) << 5))
}

/// An open region file for reading.
#[derive(Debug)]
pub struct RegionFile {
    file: File,
    path: PathBuf,
    len: u64,
    coords: Option<(i32, i32)>,
    locations: Vec<u32>,
    timestamps: Vec<u32>,
}

impl RegionFile {
    /// Open a region file. The region coordinates come from the file name (`r.X.Z.mca`); they are only needed for
    /// external `.mcc` chunks. An empty file is a region with no chunks; a file between 1 and 4095 bytes is an error.
    pub fn open_read(path: impl AsRef<Path>) -> Result<RegionFile, RegionError> {
        let path = path.as_ref();
        let coords = path.file_name().and_then(|n| n.to_str()).and_then(parse_region_file_name);
        RegionFile::open_inner(path, coords)
    }

    /// Open with explicit region coordinates (for files not named `r.X.Z.mca`).
    pub fn open_read_at(path: impl AsRef<Path>, region_x: i32, region_z: i32) -> Result<RegionFile, RegionError> {
        RegionFile::open_inner(path.as_ref(), Some((region_x, region_z)))
    }

    fn open_inner(path: &Path, coords: Option<(i32, i32)>) -> Result<RegionFile, RegionError> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut locations = vec![0u32; 1024];
        let mut timestamps = vec![0u32; 1024];
        if len > 0 {
            if len < SECTOR as u64 {
                return Err(RegionError::Truncated(format!("{len} bytes, the location table needs {SECTOR}")));
            }
            let mut buf = vec![0u8; SECTOR];
            file.read_exact(&mut buf)?;
            for (i, c) in buf.chunks_exact(4).enumerate() {
                locations[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
            }
            // The timestamp sector may be missing in damaged files; it is not needed to read chunks.
            if len >= HEADER_LEN as u64 && file.read_exact(&mut buf).is_ok() {
                for (i, c) in buf.chunks_exact(4).enumerate() {
                    timestamps[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                }
            }
        }
        Ok(RegionFile { file, path: path.to_path_buf(), len, coords, locations, timestamps })
    }

    /// The path this was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The region coordinates, when known.
    pub fn region_coords(&self) -> Option<(i32, i32)> {
        self.coords
    }

    /// Absolute chunk coordinates of a local chunk, when the region coordinates are known.
    pub fn chunk_coords(&self, x: u8, z: u8) -> Option<(i32, i32)> {
        let (rx, rz) = self.coords?;
        Some((rx * REGION_SIDE + x as i32, rz * REGION_SIDE + z as i32))
    }

    /// The file size in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when the location table has an entry for the chunk (its data may still be damaged).
    pub fn has_chunk(&self, x: u8, z: u8) -> bool {
        check_local(x, z).map(|i| self.locations[i] >> 8 != 0).unwrap_or(false)
    }

    /// The last-modified timestamp (seconds) of a chunk, 0 when absent.
    pub fn timestamp(&self, x: u8, z: u8) -> u32 {
        check_local(x, z).map(|i| self.timestamps[i]).unwrap_or(0)
    }

    /// Local coordinates of every chunk with a location entry, in table order.
    pub fn present_chunks(&self) -> Vec<(u8, u8)> {
        (0..1024).filter(|&i| self.locations[i] >> 8 != 0).map(|i| ((i & 31) as u8, (i >> 5) as u8)).collect()
    }

    /// The decompressed NBT bytes of a chunk, `None` if the chunk is not in the file ("chunk is not here").
    pub fn read_chunk_bytes(&mut self, x: u8, z: u8) -> Result<Option<Vec<u8>>, RegionError> {
        let index = check_local(x, z)?;
        let entry = self.locations[index];
        let sector = (entry >> 8) as u64;
        if sector == 0 {
            return Ok(None);
        }
        let bad = |why: &str| RegionError::BadChunk(x, z, why.to_string());
        if sector < 2 {
            return Err(bad("the chunk starts inside the header"));
        }
        let pos = sector * SECTOR as u64;
        if pos + 5 > self.len {
            return Err(bad("the chunk starts past the end of the file"));
        }
        self.file.seek(SeekFrom::Start(pos))?;
        let mut head = [0u8; 5];
        self.file.read_exact(&mut head)?;
        let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as u64;
        if length == 0 {
            return Err(bad("the chunk length is zero"));
        }
        let payload_len = length - 1;
        let raw_type = head[4];
        let external = raw_type & 0x80 != 0;
        let compression = match raw_type & 0x7F {
            0 => return Ok(None),
            t => Compression::from_id(t).ok_or(RegionError::UnsupportedCompression(t))?,
        };
        let compressed = if external {
            let (rx, rz) = self.coords.ok_or_else(|| {
                RegionError::MissingExternal(self.path.with_file_name("c.?.?.mcc (region coordinates unknown)"))
            })?;
            let path =
                self.path.with_file_name(external_file_name(rx * REGION_SIDE + x as i32, rz * REGION_SIDE + z as i32));
            let meta = fs::metadata(&path).map_err(|_| RegionError::MissingExternal(path.clone()))?;
            if meta.len() > MAX_EXTERNAL_BYTES {
                return Err(bad("the external chunk file is too large"));
            }
            fs::read(&path).map_err(|_| RegionError::MissingExternal(path))?
        } else {
            if payload_len > self.len - pos - 5 {
                return Err(bad("the chunk data runs past the end of the file"));
            }
            let mut buf = vec![0u8; payload_len as usize];
            self.file.read_exact(&mut buf)?;
            buf
        };
        compression.decompress(&compressed, MAX_CHUNK_BYTES).map(Some)
    }

    /// A chunk parsed to its root compound (`None` if absent).
    pub fn read_chunk(&mut self, x: u8, z: u8) -> Result<Option<Compound>, RegionError> {
        match self.read_chunk_bytes(x, z)? {
            None => Ok(None),
            Some(bytes) => match nbtio::from_slice(&bytes) {
                Ok((_, Tag::Compound(c))) => Ok(Some(c)),
                Ok(_) => Err(RegionError::Nbt("the root tag is not a compound".into())),
                Err(e) => Err(RegionError::Nbt(e.to_string())),
            },
        }
    }

    /// Iterate over the present chunks as `(local x, local z, decompressed NBT bytes)`. A damaged chunk yields an
    /// `Err` and iteration continues with the next one.
    pub fn chunks(&mut self) -> Chunks<'_> {
        let positions = self.present_chunks();
        Chunks { region: self, positions: positions.into_iter() }
    }
}

/// Iterator returned by [`RegionFile::chunks`].
pub struct Chunks<'a> {
    region: &'a mut RegionFile,
    positions: std::vec::IntoIter<(u8, u8)>,
}

impl Iterator for Chunks<'_> {
    type Item = Result<(u8, u8, Vec<u8>), RegionError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (x, z) = self.positions.next()?;
            match self.region.read_chunk_bytes(x, z) {
                Ok(Some(bytes)) => return Some(Ok((x, z, bytes))),
                Ok(None) => continue,
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// Streams chunks into a new region file. The header is reserved up front, every chunk is appended in whole
/// sectors, and the location and timestamp tables are written by [`RegionWriter::finish`]. Memory use is one chunk.
/// Writing the same chunk twice leaves the first copy's sectors unused (the file stays valid).
pub struct RegionWriter {
    out: BufWriter<File>,
    path: PathBuf,
    region: (i32, i32),
    locations: Vec<u32>,
    timestamps: Vec<u32>,
    next_sector: u64,
    compression: Compression,
    timestamp: u32,
}

impl RegionWriter {
    /// Create (truncate) `path` as region (`region_x`, `region_z`); the coordinates name external chunk files.
    pub fn create(path: impl AsRef<Path>, region_x: i32, region_z: i32) -> Result<RegionWriter, RegionError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut out = BufWriter::with_capacity(1 << 16, File::create(path)?);
        out.write_all(&[0u8; HEADER_LEN])?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().min(u32::MAX as u64) as u32)
            .unwrap_or(0);
        Ok(RegionWriter {
            out,
            path: path.to_path_buf(),
            region: (region_x, region_z),
            locations: vec![0; 1024],
            timestamps: vec![0; 1024],
            next_sector: 2,
            compression: Compression::Zlib,
            timestamp,
        })
    }

    /// Create `<dir>/r.X.Z.mca`.
    pub fn create_in(dir: &Path, region_x: i32, region_z: i32) -> Result<RegionWriter, RegionError> {
        RegionWriter::create(dir.join(region_file_name(region_x, region_z)), region_x, region_z)
    }

    /// Compress with this method from now on (zlib by default, like vanilla and Chunker).
    pub fn set_compression(&mut self, compression: Compression) {
        self.compression = compression;
    }

    /// The timestamp (seconds) recorded for chunks written from now on (default: the time of `create`).
    pub fn set_timestamp(&mut self, seconds: u32) {
        self.timestamp = seconds;
    }

    /// The path being written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of chunks written so far.
    pub fn chunk_count(&self) -> usize {
        self.locations.iter().filter(|&&l| l != 0).count()
    }

    /// Write one chunk from its uncompressed NBT bytes.
    pub fn write_chunk(&mut self, x: u8, z: u8, nbt: &[u8]) -> Result<(), RegionError> {
        let compressed = self.compression.compress(nbt);
        self.write_compressed(x, z, self.compression, &compressed)
    }

    /// Write one chunk from a compound (serialized as unnamed-root Java NBT).
    pub fn write_compound(&mut self, x: u8, z: u8, root: &Compound) -> Result<(), RegionError> {
        let bytes = nbtio::to_vec("", &Tag::Compound(root.clone()));
        self.write_chunk(x, z, &bytes)
    }

    /// Write one chunk that is already compressed with `compression`. Payloads that would need more than 255
    /// sectors go to `c.X.Z.mcc` and the region keeps a one-sector stub.
    pub fn write_compressed(&mut self, x: u8, z: u8, compression: Compression, data: &[u8]) -> Result<(), RegionError> {
        let index = check_local(x, z)?;
        let sectors = (data.len() + 5).div_ceil(SECTOR);
        let external = sectors > MAX_SECTORS;
        let (stored_len, type_byte, count): (u32, u8, usize) = if external {
            let (rx, rz) = self.region;
            let path =
                self.path.with_file_name(external_file_name(rx * REGION_SIDE + x as i32, rz * REGION_SIDE + z as i32));
            fs::write(&path, data)?;
            (1, 0x80 | compression as u8, 1)
        } else {
            (data.len() as u32 + 1, compression as u8, sectors)
        };
        if self.next_sector + count as u64 > 0xFF_FFFF {
            return Err(RegionError::BadArgument("the region file is too large".into()));
        }
        self.out.write_all(&stored_len.to_be_bytes())?;
        self.out.write_all(&[type_byte])?;
        let mut written = 5usize;
        if !external {
            self.out.write_all(data)?;
            written += data.len();
        }
        let pad = count * SECTOR - written;
        io::copy(&mut io::repeat(0).take(pad as u64), &mut self.out)?;
        self.locations[index] = ((self.next_sector as u32) << 8) | count as u32;
        self.timestamps[index] = self.timestamp;
        self.next_sector += count as u64;
        Ok(())
    }

    /// Write the location and timestamp tables and flush. The file is not a valid region until this runs.
    pub fn finish(self) -> Result<(), RegionError> {
        let RegionWriter { out, locations, timestamps, .. } = self;
        let mut file = out.into_inner().map_err(|e| RegionError::Io(e.into_error()))?;
        let mut header = Vec::with_capacity(HEADER_LEN);
        for l in &locations {
            header.extend_from_slice(&l.to_be_bytes());
        }
        for t in &timestamps {
            header.extend_from_slice(&t.to_be_bytes());
        }
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header)?;
        file.flush()?;
        Ok(())
    }
}

// ---- entity and POI chunk helpers (1.17+ keeps entities in `entities/` region files) ----

/// The root compound of an `entities/` chunk: `{DataVersion, Position: [chunkX, chunkZ], Entities}`.
pub fn entities_root(data_version: i32, chunk_x: i32, chunk_z: i32, entities: Tag) -> Compound {
    let mut c = Compound::new();
    c.insert("Entities", entities);
    c.insert("DataVersion", Tag::Int(data_version));
    c.insert("Position", Tag::IntArray(vec![chunk_x, chunk_z]));
    c
}

/// Writer side (1.17 to 1.20.x `WorldWriter.writeMCAData`): take the `Entities` list out of a chunk root (inside
/// `Level` below 1.18) and return it when it is a non-empty list; the chunk then goes to `region/` without it.
pub fn take_entities(chunk_root: &mut Compound) -> Option<Tag> {
    let target = match chunk_root.0.get_mut("Level") {
        Some(Tag::Compound(level)) => level,
        _ => chunk_root,
    };
    match target.remove("Entities") {
        Some(Tag::List(list)) if !list.is_empty() => Some(Tag::List(list)),
        _ => None,
    }
}

/// Reader side (1.17+ `WorldReader.combineColumnCompounds`): merge the `entities/` chunk into the `region/` chunk.
/// With only an entities chunk, a minimal root with `xPos`, `zPos` and `Entities` is made from its `Position`.
pub fn merge_entities(main: Option<Compound>, entities: Option<Compound>) -> Option<Compound> {
    match (main, entities) {
        (main, None) => main,
        (None, Some(e)) => {
            let Some(Tag::IntArray(pos)) = e.get("Position") else { return None };
            let (&x, &z) = (pos.first()?, pos.get(1)?);
            let mut main = Compound::new();
            main.insert("xPos", Tag::Int(x));
            main.insert("zPos", Tag::Int(z));
            if let Some(list) = e.get("Entities") {
                main.insert("Entities", list.clone());
            }
            Some(main)
        }
        (Some(mut main), Some(e)) => {
            if let Some(list) = e.get("Entities") {
                let target = match main.0.get_mut("Level") {
                    Some(Tag::Compound(level)) => level,
                    _ => &mut main,
                };
                target.insert("Entities", list.clone());
            }
            Some(main)
        }
    }
}

#[cfg(test)]
mod tests;
