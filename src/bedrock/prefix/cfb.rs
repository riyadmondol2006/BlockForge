//! A bounds-checked reader for the OLE compound file ("CFB") container an MSI is stored in.
//!
//! Ported from BedrockOnLinux `bol/gameinput.py` (`_msi_embedded_cab`, MIT), with credit, and hardened: BOL
//! trusted every sector number and chain it found. Here a sector that lies outside the file, a chain that loops
//! or runs through a free/special sector, a size bigger than its chain, and a table with more entries than the
//! header allows are all errors, never an out-of-range read or an endless walk.

use std::fmt;

const SIGNATURE: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
const FREE: u32 = 0xFFFF_FFFF;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
/// More than any installer needs; keeps a hostile header from asking for gigabytes.
const MAX_STREAM: u64 = 512 * 1024 * 1024;
const MAX_DIR_ENTRIES: usize = 1 << 20;

#[derive(Debug)]
pub struct CfbError(pub String);

impl fmt::Display for CfbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, CfbError> {
    Err(CfbError(msg.into()))
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    /// 1 storage, 2 stream, 5 root.
    pub kind: u8,
    pub start: u32,
    pub size: u64,
}

pub struct Cfb<'a> {
    data: &'a [u8],
    sector_size: usize,
    mini_size: usize,
    mini_cutoff: u64,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    entries: Vec<DirEntry>,
    root: DirEntry,
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, CfbError> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| CfbError("the file ends inside its header".into()))
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, CfbError> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| CfbError("the file ends inside its header".into()))
}

impl<'a> Cfb<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self, CfbError> {
        if data.len() < 512 || data[..8] != SIGNATURE {
            return err("not an OLE compound file");
        }
        if u16_at(data, 0x1c)? != 0xFFFE {
            return err("the compound file is not little-endian");
        }
        let sector_shift = u16_at(data, 0x1e)?;
        let mini_shift = u16_at(data, 0x20)?;
        if !(sector_shift == 9 || sector_shift == 12) || mini_shift != 6 {
            return err("the compound file uses sector sizes this reader does not know");
        }
        let sector_size = 1usize << sector_shift;
        let mini_size = 1usize << mini_shift;
        let mut this = Cfb {
            data,
            sector_size,
            mini_size,
            mini_cutoff: u32_at(data, 0x38)? as u64,
            fat: Vec::new(),
            mini_fat: Vec::new(),
            entries: Vec::new(),
            root: DirEntry { name: String::new(), kind: 5, start: END_OF_CHAIN, size: 0 },
        };
        let total_sectors = (data.len() / sector_size).saturating_sub(1);
        let n_fat = u32_at(data, 0x2c)? as usize;
        let dir_start = u32_at(data, 0x30)?;
        let minifat_start = u32_at(data, 0x3c)?;
        let difat_start = u32_at(data, 0x44)?;
        let n_difat = u32_at(data, 0x48)? as usize;
        if n_fat > total_sectors || n_difat > total_sectors {
            return err("the compound file claims more table sectors than it has room for");
        }

        // The sector numbers of the FAT sectors: 109 in the header, the rest in a chain of DIFAT sectors.
        let mut fat_sectors: Vec<u32> = Vec::new();
        for i in 0..109 {
            fat_sectors.push(u32_at(data, 0x4c + i * 4)?);
        }
        let mut next = difat_start;
        for _ in 0..n_difat {
            if next == FREE || next == END_OF_CHAIN {
                break;
            }
            let sector = this.sector_full(next)?;
            let per = sector_size / 4 - 1;
            for i in 0..per {
                fat_sectors.push(u32::from_le_bytes(sector[i * 4..i * 4 + 4].try_into().unwrap()));
            }
            next = u32::from_le_bytes(sector[per * 4..per * 4 + 4].try_into().unwrap());
        }
        let mut fat = Vec::new();
        for (used, s) in fat_sectors.into_iter().filter(|&s| s != FREE).enumerate() {
            if n_fat != 0 && used >= n_fat {
                break;
            }
            let sector = this.sector_full(s)?;
            for i in 0..sector_size / 4 {
                fat.push(u32::from_le_bytes(sector[i * 4..i * 4 + 4].try_into().unwrap()));
            }
        }
        this.fat = fat;

        // The directory.
        let dir_chain = this.chain(dir_start)?;
        let mut dir = Vec::with_capacity(dir_chain.len() * sector_size);
        for s in &dir_chain {
            dir.extend_from_slice(this.sector_full(*s)?);
        }
        let mut entries = Vec::new();
        let major = u16_at(data, 0x1a)?;
        for e in dir.chunks_exact(128).take(MAX_DIR_ENTRIES) {
            let name_len = u16::from_le_bytes([e[64], e[65]]) as usize;
            if name_len == 0 {
                continue;
            }
            let units: Vec<u16> = e[..name_len.saturating_sub(2).min(64)]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let mut size = u64::from_le_bytes(e[120..128].try_into().unwrap());
            if major < 4 {
                size &= 0xFFFF_FFFF; // a version 3 file's high half is not part of the size
            }
            entries.push(DirEntry {
                name: String::from_utf16_lossy(&units),
                kind: e[66],
                start: u32::from_le_bytes(e[116..120].try_into().unwrap()),
                size,
            });
        }
        this.root = entries
            .iter()
            .find(|e| e.kind == 5)
            .cloned()
            .ok_or_else(|| CfbError("the compound file has no root entry".into()))?;
        this.entries = entries;

        // The mini FAT (the table of the small-stream sectors).
        if minifat_start != END_OF_CHAIN && minifat_start != FREE {
            for s in this.chain(minifat_start)? {
                let sector = this.sector_full(s)?;
                for i in 0..sector_size / 4 {
                    this.mini_fat.push(u32::from_le_bytes(sector[i * 4..i * 4 + 4].try_into().unwrap()));
                }
            }
        }
        Ok(this)
    }

    /// Sector `n`, only if all of it is in the file.
    fn sector_full(&self, n: u32) -> Result<&'a [u8], CfbError> {
        let start = (n as usize).checked_add(1).and_then(|s| s.checked_mul(self.sector_size));
        let end = start.and_then(|s| s.checked_add(self.sector_size));
        match (start, end) {
            (Some(s), Some(e)) if e <= self.data.len() => Ok(&self.data[s..e]),
            _ => err(format!("sector {n} is outside the compound file")),
        }
    }

    /// Sector `n` as far as the file goes: the last sector of a stream may be cut short by a writer.
    fn sector_partial(&self, n: u32) -> Result<&'a [u8], CfbError> {
        let start = (n as usize).checked_add(1).and_then(|s| s.checked_mul(self.sector_size));
        match start {
            Some(s) if s < self.data.len() => Ok(&self.data[s..(s + self.sector_size).min(self.data.len())]),
            _ => err(format!("sector {n} is outside the compound file")),
        }
    }

    fn walk(table: &[u32], start: u32, what: &str) -> Result<Vec<u32>, CfbError> {
        let mut out = Vec::new();
        let mut seen = vec![false; table.len()];
        let mut n = start;
        while n != END_OF_CHAIN {
            if n as usize >= table.len() {
                return err(format!("a {what} chain leaves the table (sector {n})"));
            }
            if seen[n as usize] {
                return err(format!("a {what} chain loops back on itself"));
            }
            seen[n as usize] = true;
            out.push(n);
            n = table[n as usize];
            if n == FREE || n >= 0xFFFF_FFFC && n != END_OF_CHAIN {
                return err(format!("a {what} chain runs into a free or reserved sector"));
            }
        }
        Ok(out)
    }

    fn chain(&self, start: u32) -> Result<Vec<u32>, CfbError> {
        Self::walk(&self.fat, start, "sector")
    }

    pub fn entries(&self) -> &[DirEntry] {
        &self.entries
    }

    fn is_mini(&self, e: &DirEntry) -> bool {
        e.size < self.mini_cutoff
    }

    /// The first `want` bytes of a stream (fewer if the stream is shorter), without reading the rest.
    pub fn read_prefix(&self, e: &DirEntry, want: usize) -> Result<Vec<u8>, CfbError> {
        let want = want.min(e.size as usize);
        let mut out = self.read_limited(e, Some(want))?;
        out.truncate(want);
        Ok(out)
    }

    pub fn read_stream(&self, e: &DirEntry) -> Result<Vec<u8>, CfbError> {
        if e.size > MAX_STREAM {
            return err("a stream in the compound file is implausibly large");
        }
        self.read_limited(e, None)
    }

    fn read_limited(&self, e: &DirEntry, limit: Option<usize>) -> Result<Vec<u8>, CfbError> {
        if e.kind != 2 && e.kind != 5 {
            return err("not a stream");
        }
        let size = e.size as usize;
        let mut out = Vec::new();
        if self.is_mini(e) && e.kind == 2 {
            let mini_stream = self.mini_stream()?;
            for n in Self::walk(&self.mini_fat, e.start, "mini sector")? {
                let at = (n as usize) * self.mini_size;
                let chunk = mini_stream
                    .get(at..at + self.mini_size)
                    .ok_or_else(|| CfbError("a small stream runs past the mini stream".into()))?;
                out.extend_from_slice(chunk);
                if limit.is_some_and(|l| out.len() >= l) || out.len() >= size {
                    break;
                }
            }
        } else {
            let chain = self.chain(e.start)?;
            let need = limit.unwrap_or(size).min(size);
            if (chain.len() as u64) * (self.sector_size as u64) < e.size {
                return err("a stream is longer than the sectors that hold it");
            }
            for n in chain {
                out.extend_from_slice(self.sector_partial(n)?);
                if out.len() >= need {
                    break;
                }
            }
        }
        if out.len() < limit.unwrap_or(size).min(size) {
            return err("a stream is longer than the file holds");
        }
        out.truncate(size);
        Ok(out)
    }

    fn mini_stream(&self) -> Result<Vec<u8>, CfbError> {
        if self.root.size > MAX_STREAM {
            return err("the mini stream is implausibly large");
        }
        let chain = self.chain(self.root.start)?;
        let mut out = Vec::with_capacity(chain.len() * self.sector_size);
        for n in chain {
            out.extend_from_slice(self.sector_partial(n)?);
        }
        out.truncate(self.root.size as usize);
        Ok(out)
    }
}

/// Test-only writer for compound files: just enough to build fixtures (version 3, 512-byte sectors).
#[cfg(test)]
pub mod build {
    use super::*;

    pub struct Stream {
        pub name: &'static str,
        pub data: Vec<u8>,
    }

    /// Lay out `streams` as a compound file. Streams under 4096 bytes go in the mini stream.
    pub fn compound_file(streams: &[Stream]) -> Vec<u8> {
        const SS: usize = 512;
        const MS: usize = 64;
        let mut big: Vec<(usize, Vec<u8>)> = Vec::new(); // (stream index, data)
        let mut mini: Vec<u8> = Vec::new();
        let mut mini_fat: Vec<u32> = Vec::new();
        let mut entry_start = vec![END_OF_CHAIN; streams.len()];
        for (i, s) in streams.iter().enumerate() {
            if (s.data.len() as u64) < 4096 {
                if s.data.is_empty() {
                    continue;
                }
                let first = (mini.len() / MS) as u32;
                let n = s.data.len().div_ceil(MS);
                for k in 0..n {
                    mini_fat.push(if k + 1 == n { END_OF_CHAIN } else { first + k as u32 + 1 });
                }
                mini.extend_from_slice(&s.data);
                mini.resize(mini.len().div_ceil(MS) * MS, 0);
                entry_start[i] = first;
            } else {
                big.push((i, s.data.clone()));
            }
        }
        // Sector plan: [FAT] [directory...] [minifat] [ministream...] [big streams...]
        let dir_entries = 1 + streams.len();
        let dir_sectors = (dir_entries * 128).div_ceil(SS);
        let minifat_sectors = if mini_fat.is_empty() { 0 } else { (mini_fat.len() * 4).div_ceil(SS) };
        let mini_sectors = mini.len().div_ceil(SS);
        let big_sectors: usize = big.iter().map(|(_, d)| d.len().div_ceil(SS)).sum();
        let non_fat = dir_sectors + minifat_sectors + mini_sectors + big_sectors;
        let mut fat_sectors = 1;
        while fat_sectors * 128 < non_fat + fat_sectors {
            fat_sectors += 1;
        }
        assert!(fat_sectors <= 109);
        let total = fat_sectors + non_fat;
        let mut fat = vec![FREE; fat_sectors * 128];
        let mut cursor = 0usize;
        for _ in 0..fat_sectors {
            fat[cursor] = 0xFFFF_FFFD;
            cursor += 1;
        }
        let alloc = |count: usize, fat: &mut Vec<u32>, cursor: &mut usize| -> u32 {
            let first = *cursor as u32;
            for k in 0..count {
                fat[*cursor + k] = if k + 1 == count { END_OF_CHAIN } else { (*cursor + k + 1) as u32 };
            }
            *cursor += count;
            first
        };
        let dir_first = alloc(dir_sectors, &mut fat, &mut cursor);
        let minifat_first =
            if minifat_sectors > 0 { alloc(minifat_sectors, &mut fat, &mut cursor) } else { END_OF_CHAIN };
        let mini_first = if mini_sectors > 0 { alloc(mini_sectors, &mut fat, &mut cursor) } else { END_OF_CHAIN };
        for (i, d) in &big {
            entry_start[*i] = alloc(d.len().div_ceil(SS), &mut fat, &mut cursor);
        }
        assert_eq!(cursor, total);

        let mut file = vec![0u8; SS * (1 + total)];
        file[..8].copy_from_slice(&SIGNATURE);
        file[0x18..0x1a].copy_from_slice(&0x3eu16.to_le_bytes());
        file[0x1a..0x1c].copy_from_slice(&3u16.to_le_bytes());
        file[0x1c..0x1e].copy_from_slice(&0xFFFEu16.to_le_bytes());
        file[0x1e..0x20].copy_from_slice(&9u16.to_le_bytes());
        file[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
        file[0x2c..0x30].copy_from_slice(&(fat_sectors as u32).to_le_bytes());
        file[0x30..0x34].copy_from_slice(&dir_first.to_le_bytes());
        file[0x38..0x3c].copy_from_slice(&4096u32.to_le_bytes());
        file[0x3c..0x40].copy_from_slice(&minifat_first.to_le_bytes());
        file[0x40..0x44].copy_from_slice(&(minifat_sectors as u32).to_le_bytes());
        file[0x44..0x48].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        for i in 0..109 {
            let v = if i < fat_sectors { i as u32 } else { FREE };
            file[0x4c + i * 4..0x50 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        let put = |file: &mut Vec<u8>, sector: usize, bytes: &[u8]| {
            let at = (sector + 1) * SS;
            file[at..at + bytes.len()].copy_from_slice(bytes);
        };
        for (i, v) in fat.iter().enumerate() {
            let at = SS + i * 4;
            file[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        // Directory.
        let mut dir = vec![0u8; dir_sectors * SS];
        let mut write_entry = |idx: usize, name: &str, kind: u8, start: u32, size: u64| {
            let e = &mut dir[idx * 128..idx * 128 + 128];
            let units: Vec<u16> = name.encode_utf16().collect();
            for (k, u) in units.iter().enumerate() {
                e[k * 2..k * 2 + 2].copy_from_slice(&u.to_le_bytes());
            }
            e[64..66].copy_from_slice(&(((units.len() + 1) * 2) as u16).to_le_bytes());
            e[66] = kind;
            e[116..120].copy_from_slice(&start.to_le_bytes());
            e[120..128].copy_from_slice(&size.to_le_bytes());
        };
        write_entry(0, "Root Entry", 5, mini_first, mini.len() as u64);
        for (i, s) in streams.iter().enumerate() {
            write_entry(1 + i, s.name, 2, entry_start[i], s.data.len() as u64);
        }
        put(&mut file, dir_first as usize, &dir);
        if minifat_sectors > 0 {
            let mut mf = vec![0xFFu8; minifat_sectors * SS];
            for (i, v) in mini_fat.iter().enumerate() {
                mf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            put(&mut file, minifat_first as usize, &mf);
        }
        if mini_sectors > 0 {
            put(&mut file, mini_first as usize, &mini);
        }
        for (i, d) in &big {
            put(&mut file, entry_start[*i] as usize, d);
        }
        file
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    fn sample() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let small: Vec<u8> = (0..300u32).map(|i| (i * 7) as u8).collect();
        let big: Vec<u8> = (0..20_000u32).map(|i| (i * 13 + 5) as u8).collect();
        let file =
            compound_file(&[Stream { name: "small", data: small.clone() }, Stream { name: "big", data: big.clone() }]);
        (file, small, big)
    }

    #[test]
    fn reads_small_and_large_streams() {
        let (file, small, big) = sample();
        let cfb = Cfb::open(&file).unwrap();
        let names: Vec<_> = cfb.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["Root Entry", "small", "big"]);
        let s = cfb.entries().iter().find(|e| e.name == "small").unwrap();
        assert_eq!(cfb.read_stream(s).unwrap(), small);
        let b = cfb.entries().iter().find(|e| e.name == "big").unwrap();
        assert_eq!(cfb.read_stream(b).unwrap(), big);
        assert_eq!(cfb.read_prefix(b, 4).unwrap(), &big[..4]);
        assert_eq!(cfb.read_prefix(s, 4).unwrap(), &small[..4]);
    }

    #[test]
    fn a_truncated_file_is_an_error_not_a_panic() {
        let (file, _, _) = sample();
        for cut in [0, 7, 100, 511, 512, 1000, file.len() / 2, file.len() - 600, file.len() - 1] {
            let slice = &file[..cut];
            if let Ok(cfb) = Cfb::open(slice) {
                for e in cfb.entries().to_vec() {
                    let _ = cfb.read_stream(&e);
                }
            }
        }
    }

    #[test]
    fn a_looping_fat_chain_is_refused() {
        let (mut file, _, _) = sample();
        // Directory is sector 1 (sector 0 is the FAT). Make its FAT entry point at itself.
        let fat_entry = 512 + 4;
        file[fat_entry..fat_entry + 4].copy_from_slice(&1u32.to_le_bytes());
        assert!(Cfb::open(&file).is_err());
    }

    #[test]
    fn a_big_stream_chain_that_loops_or_leaves_the_table_is_refused() {
        let (file, _, _) = sample();
        let cfb = Cfb::open(&file).unwrap();
        let big = cfb.entries().iter().find(|e| e.name == "big").unwrap().clone();
        for tamper in [big.start as usize + 1, big.start as usize] {
            let mut f = file.clone();
            let at = 512 + (tamper) * 4;
            // Point the chain's second link back at its first (a loop), then off the end of the table.
            f[at..at + 4].copy_from_slice(&big.start.to_le_bytes());
            let cfb = Cfb::open(&f).unwrap();
            let b = cfb.entries().iter().find(|e| e.name == "big").unwrap().clone();
            assert!(cfb.read_stream(&b).is_err(), "loop at {tamper}");
            let mut f = file.clone();
            f[at..at + 4].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
            let cfb = Cfb::open(&f).unwrap();
            let b = cfb.entries().iter().find(|e| e.name == "big").unwrap().clone();
            assert!(cfb.read_stream(&b).is_err(), "out of range at {tamper}");
        }
    }

    #[test]
    fn a_looping_mini_fat_is_refused() {
        let (file, _, _) = sample();
        let cfb = Cfb::open(&file).unwrap();
        let small = cfb.entries().iter().find(|e| e.name == "small").unwrap().clone();
        // The mini FAT is the sector after the directory (sector 2), its first entries are the small stream.
        let mut f = file.clone();
        let at = 512 * 3;
        f[at..at + 4].copy_from_slice(&small.start.to_le_bytes());
        let cfb = Cfb::open(&f).unwrap();
        let s = cfb.entries().iter().find(|e| e.name == "small").unwrap().clone();
        assert!(cfb.read_stream(&s).is_err());
        let mut f = file.clone();
        f[at..at + 4].copy_from_slice(&0x0100_0000u32.to_le_bytes());
        let cfb = Cfb::open(&f).unwrap();
        let s = cfb.entries().iter().find(|e| e.name == "small").unwrap().clone();
        assert!(cfb.read_stream(&s).is_err());
    }

    #[test]
    fn a_size_larger_than_the_chain_is_refused() {
        let (file, _, _) = sample();
        let cfb = Cfb::open(&file).unwrap();
        let mut e = cfb.entries().iter().find(|e| e.name == "big").unwrap().clone();
        e.size = 5_000_000;
        assert!(cfb.read_stream(&e).is_err());
        e.size = u64::MAX;
        assert!(cfb.read_stream(&e).is_err());
    }

    #[test]
    fn hostile_headers_are_refused() {
        let (file, _, _) = sample();
        let mut f = file.clone();
        f[0x2c..0x30].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Cfb::open(&f).is_err());
        let mut f = file.clone();
        f[0x1e..0x20].copy_from_slice(&30u16.to_le_bytes());
        assert!(Cfb::open(&f).is_err());
        let mut f = file.clone();
        f[0x30..0x34].copy_from_slice(&0x7fff_fff0u32.to_le_bytes());
        assert!(Cfb::open(&f).is_err());
        let mut f = file.clone();
        f[0x4c..0x50].copy_from_slice(&0x7fff_fff0u32.to_le_bytes());
        assert!(Cfb::open(&f).is_err());
        assert!(Cfb::open(b"").is_err());
        assert!(Cfb::open(&[0u8; 600]).is_err());
    }
}
