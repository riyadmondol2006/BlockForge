// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Region files written chunk by chunk with a bounded number of open files.
//!
//! Chunker keeps one `RandomAccessFile` per region for the whole run. Here at most `max_open` region files are
//! open at once: the least recently used one is "parked" (its header is written and the handle closed) and read
//! back from disk when another chunk of that region arrives. A parked region costs no memory at all, so a world of
//! thousands of regions is written with a few hundred kilobytes of state, in whatever order the columns come.
//!
//! Layout is the one of [`super::region`]: the header (locations, timestamps) is sector 0 and 1, every chunk is
//! appended at the end of the file padded to whole sectors, chunks over 255 sectors go to `c.X.Z.mcc`.

use crate::convert::java::region::{
    Compression, HEADER_LEN, MAX_SECTORS, REGION_SIDE, SECTOR, chunk_to_region, external_file_name, region_file_name,
};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The largest sector number a location entry can hold.
const MAX_SECTOR_NUMBER: u64 = 0xFF_FFFF;

struct OpenRegion {
    dir: PathBuf,
    region: (i32, i32),
    file: File,
    locations: Vec<u32>,
    timestamps: Vec<u32>,
    next_sector: u64,
    dirty: bool,
}

impl OpenRegion {
    fn path(&self) -> PathBuf {
        self.dir.join(region_file_name(self.region.0, self.region.1))
    }

    /// Open (or create) the region file; an existing file keeps its chunks.
    fn open(dir: &Path, region: (i32, i32)) -> io::Result<OpenRegion> {
        fs::create_dir_all(dir)?;
        let path = dir.join(region_file_name(region.0, region.1));
        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        let len = file.metadata()?.len();
        let mut locations = vec![0u32; 1024];
        let mut timestamps = vec![0u32; 1024];
        let next_sector = if len >= HEADER_LEN as u64 {
            let mut header = vec![0u8; HEADER_LEN];
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut header)?;
            for i in 0..1024 {
                locations[i] =
                    u32::from_be_bytes([header[i * 4], header[i * 4 + 1], header[i * 4 + 2], header[i * 4 + 3]]);
                let at = SECTOR + i * 4;
                timestamps[i] = u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
            }
            len.div_ceil(SECTOR as u64)
        } else {
            file.set_len(0)?;
            file.write_all(&[0u8; HEADER_LEN])?;
            2
        };
        Ok(OpenRegion { dir: dir.to_path_buf(), region, file, locations, timestamps, next_sector, dirty: false })
    }

    fn append(
        &mut self,
        local_x: u8,
        local_z: u8,
        compression: Compression,
        data: &[u8],
        timestamp: u32,
    ) -> io::Result<()> {
        let index = (local_x as usize & 31) + ((local_z as usize & 31) << 5);
        let external = (data.len() + 5).div_ceil(SECTOR) > MAX_SECTORS;
        let (stored_len, type_byte, count): (u32, u8, usize) = if external {
            let chunk_x = self.region.0 * REGION_SIDE + local_x as i32;
            let chunk_z = self.region.1 * REGION_SIDE + local_z as i32;
            fs::write(self.dir.join(external_file_name(chunk_x, chunk_z)), data)?;
            (1, 0x80 | compression as u8, 1)
        } else {
            (data.len() as u32 + 1, compression as u8, (data.len() + 5).div_ceil(SECTOR))
        };
        if self.next_sector + count as u64 > MAX_SECTOR_NUMBER {
            return Err(io::Error::other(format!("the region file {} is full", self.path().display())));
        }
        let mut block = Vec::with_capacity(count * SECTOR);
        block.extend_from_slice(&stored_len.to_be_bytes());
        block.push(type_byte);
        if !external {
            block.extend_from_slice(data);
        }
        block.resize(count * SECTOR, 0);
        self.file.seek(SeekFrom::Start(self.next_sector * SECTOR as u64))?;
        self.file.write_all(&block)?;
        self.locations[index] = ((self.next_sector as u32) << 8) | count as u32;
        self.timestamps[index] = timestamp;
        self.next_sector += count as u64;
        self.dirty = true;
        Ok(())
    }

    /// Write the location and timestamp tables.
    fn write_header(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let mut header = Vec::with_capacity(HEADER_LEN);
        for l in &self.locations {
            header.extend_from_slice(&l.to_be_bytes());
        }
        for t in &self.timestamps {
            header.extend_from_slice(&t.to_be_bytes());
        }
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header)?;
        self.file.flush()?;
        self.dirty = false;
        Ok(())
    }
}

/// A set of region files being written, with at most `max_open` of them open.
pub struct RegionSet {
    max_open: usize,
    timestamp: u32,
    /// Least recently used first.
    open: Vec<OpenRegion>,
    /// How many times a parked region had to be opened again (for tests and tuning).
    reopened: usize,
}

impl RegionSet {
    /// A set that keeps at most `max_open` (at least 1) region files open.
    pub fn new(max_open: usize) -> RegionSet {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().min(u32::MAX as u64) as u32)
            .unwrap_or(0);
        RegionSet { max_open: max_open.max(1), timestamp, open: Vec::new(), reopened: 0 }
    }

    /// Number of region files open right now.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    /// How many regions were opened again after being parked.
    pub fn reopened(&self) -> usize {
        self.reopened
    }

    /// Append one compressed chunk (absolute chunk coordinates) to the region file in `dir`.
    pub fn write_chunk(
        &mut self,
        dir: &Path,
        chunk_x: i32,
        chunk_z: i32,
        compression: Compression,
        data: &[u8],
    ) -> io::Result<()> {
        let (rx, rz, lx, lz) = chunk_to_region(chunk_x, chunk_z);
        let at = match self.open.iter().position(|r| r.region == (rx, rz) && r.dir == dir) {
            Some(i) => i,
            None => {
                while self.open.len() >= self.max_open {
                    let mut oldest = self.open.remove(0);
                    oldest.write_header()?;
                }
                let opened = OpenRegion::open(dir, (rx, rz))?;
                if opened.next_sector > 2 {
                    self.reopened += 1;
                }
                self.open.push(opened);
                self.open.len() - 1
            }
        };
        // Most recently used goes last.
        let last = self.open.len() - 1;
        self.open.swap(at, last);
        let region = &mut self.open[last];
        region.append(lx, lz, compression, data, self.timestamp)?;
        Ok(())
    }

    /// Write the headers of, and close, every open region.
    pub fn flush(&mut self) -> io::Result<()> {
        let mut first_error = None;
        for mut region in self.open.drain(..) {
            if let Err(e) = region.write_header() {
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Write the headers of, and close, the regions of one dimension folder (`region/`, `entities/`, `poi/`).
    pub fn flush_dimension(&mut self, dimension_dir: &Path) -> io::Result<()> {
        let mut first_error = None;
        let mut keep = Vec::with_capacity(self.open.len());
        for mut region in self.open.drain(..) {
            if region.dir.parent() == Some(dimension_dir) {
                if let Err(e) = region.write_header() {
                    first_error.get_or_insert(e);
                }
            } else {
                keep.push(region);
            }
        }
        self.open = keep;
        first_error.map_or(Ok(()), Err)
    }

    /// Close every region without writing headers (used when aborting).
    pub fn discard(&mut self) {
        self.open.clear();
    }
}
