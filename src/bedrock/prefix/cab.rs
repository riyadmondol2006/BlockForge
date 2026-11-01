//! Cabinet (CAB) extraction for the MSZip and stored folders an MSI's embedded cabinet uses, with a small inflate
//! of its own because MSZip keeps the previous block's output as the dictionary of the next one, which the
//! zlib bindings BlockForge already carries do not let a raw stream do.
//!
//! Ported from BedrockOnLinux `bol/gameinput.py` (`_cab_payload`, MIT), with credit, and bounds-checked: every
//! offset and length is checked against the cabinet before it is used, outputs are capped, and a block whose
//! decoded size is not the size its header declared is an error.

use std::fmt;

/// Larger than any redistributable's folder; stops a bomb before it fills memory.
const MAX_FOLDER_BYTES: usize = 256 * 1024 * 1024;
const MAX_FILES: usize = 16_384;
const MAX_FOLDERS: usize = 256;
const BLOCK_MAX: usize = 32 * 1024;

#[derive(Debug)]
pub struct CabError(pub String);

impl fmt::Display for CabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, CabError> {
    Err(CabError(msg.into()))
}

// ---------------------------------------------------------------- inflate

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_cnt: u32,
}

impl<'a> Bits<'a> {
    fn bits(&mut self, need: u32) -> Result<u32, CabError> {
        let mut value = self.bit_buf;
        while self.bit_cnt < need {
            let b = *self.data.get(self.pos).ok_or_else(|| CabError("a compressed block ends too soon".into()))?;
            self.pos += 1;
            value |= (b as u32) << self.bit_cnt;
            self.bit_cnt += 8;
        }
        self.bit_buf = if need >= 32 { 0 } else { value >> need };
        self.bit_cnt -= need;
        Ok(value & (if need >= 32 { u32::MAX } else { (1u32 << need) - 1 }))
    }
}

struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huffman {
    /// Build a canonical code from the lengths of each symbol's code. An over-subscribed set is an error; an
    /// incomplete one is allowed (a block may use a single distance code).
    fn new(lengths: &[u8]) -> Result<Self, CabError> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left: i32 = 1;
        for len in 1..16 {
            left <<= 1;
            left -= count[len] as i32;
            if left < 0 {
                return err("a compressed block has an impossible code");
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Self { count, symbol })
    }

    fn decode(&self, s: &mut Bits) -> Result<u16, CabError> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= s.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return self
                    .symbol
                    .get((index + (code - first)) as usize)
                    .copied()
                    .ok_or_else(|| CabError("a compressed block has a bad code".into()));
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        err("a compressed block has a bad code")
    }
}

const LEN_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Inflate a raw deflate stream whose back-references may reach into `history` (the previous block's output).
/// Returns only the new bytes, never more than `max_out`.
pub fn inflate_raw(input: &[u8], history: &[u8], max_out: usize) -> Result<Vec<u8>, CabError> {
    let mut out: Vec<u8> = Vec::with_capacity(history.len() + max_out.min(BLOCK_MAX));
    out.extend_from_slice(history);
    let base = history.len();
    let mut s = Bits { data: input, pos: 0, bit_buf: 0, bit_cnt: 0 };
    loop {
        let last = s.bits(1)?;
        match s.bits(2)? {
            0 => {
                s.bit_buf = 0;
                s.bit_cnt = 0;
                let head =
                    input.get(s.pos..s.pos + 4).ok_or_else(|| CabError("a stored block ends too soon".into()))?;
                let len = u16::from_le_bytes([head[0], head[1]]);
                let nlen = u16::from_le_bytes([head[2], head[3]]);
                if len != !nlen {
                    return err("a stored block's length does not check");
                }
                s.pos += 4;
                let body = input
                    .get(s.pos..s.pos + len as usize)
                    .ok_or_else(|| CabError("a stored block ends too soon".into()))?;
                if out.len() - base + body.len() > max_out {
                    return err("a compressed block expands past its declared size");
                }
                out.extend_from_slice(body);
                s.pos += len as usize;
            }
            1 => {
                let mut lengths = [0u8; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                let lit = Huffman::new(&lengths)?;
                let dist = Huffman::new(&[5u8; 30])?;
                codes(&mut s, &mut out, base, max_out, &lit, &dist)?;
            }
            2 => {
                let nlen = s.bits(5)? as usize + 257;
                let ndist = s.bits(5)? as usize + 1;
                let ncode = s.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return err("a compressed block declares too many codes");
                }
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut lengths = [0u8; 320];
                for &o in ORDER.iter().take(ncode) {
                    lengths[o] = s.bits(3)? as u8;
                }
                let code_code = Huffman::new(&lengths[..19])?;
                let mut lens = [0u8; 320];
                let mut index = 0;
                while index < nlen + ndist {
                    let sym = code_code.decode(&mut s)?;
                    if sym < 16 {
                        lens[index] = sym as u8;
                        index += 1;
                    } else {
                        let (value, repeat) = match sym {
                            16 => {
                                if index == 0 {
                                    return err("a compressed block repeats a length that is not there");
                                }
                                (lens[index - 1], 3 + s.bits(2)? as usize)
                            }
                            17 => (0, 3 + s.bits(3)? as usize),
                            _ => (0, 11 + s.bits(7)? as usize),
                        };
                        if index + repeat > nlen + ndist {
                            return err("a compressed block's code lengths overrun");
                        }
                        lens[index..index + repeat].fill(value);
                        index += repeat;
                    }
                }
                if lens[256] == 0 {
                    return err("a compressed block has no end code");
                }
                let lit = Huffman::new(&lens[..nlen])?;
                let dist = Huffman::new(&lens[nlen..nlen + ndist])?;
                codes(&mut s, &mut out, base, max_out, &lit, &dist)?;
            }
            _ => return err("a compressed block has an unknown type"),
        }
        if last == 1 {
            break;
        }
    }
    out.drain(..base);
    Ok(out)
}

fn codes(
    s: &mut Bits,
    out: &mut Vec<u8>,
    base: usize,
    max_out: usize,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), CabError> {
    loop {
        let sym = lit.decode(s)? as usize;
        if sym < 256 {
            if out.len() - base >= max_out {
                return err("a compressed block expands past its declared size");
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let sym = sym - 257;
            if sym >= 29 {
                return err("a compressed block has a bad length code");
            }
            let len = LEN_BASE[sym] as usize + s.bits(LEN_EXTRA[sym] as u32)? as usize;
            let dsym = dist.decode(s)? as usize;
            if dsym >= 30 {
                return err("a compressed block has a bad distance code");
            }
            let d = DIST_BASE[dsym] as usize + s.bits(DIST_EXTRA[dsym] as u32)? as usize;
            if d > out.len() {
                return err("a compressed block reaches back before its dictionary");
            }
            if out.len() - base + len > max_out {
                return err("a compressed block expands past its declared size");
            }
            for _ in 0..len {
                let b = out[out.len() - d];
                out.push(b);
            }
        }
    }
}

// ---------------------------------------------------------------- the cabinet

pub struct CabFile {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, CabError> {
    d.get(at..at.saturating_add(2))
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| CabError("the cabinet is cut off".into()))
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, CabError> {
    d.get(at..at.saturating_add(4))
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| CabError("the cabinet is cut off".into()))
}

fn cstr(d: &[u8], at: usize) -> Result<(&[u8], usize), CabError> {
    let rest = d.get(at..).ok_or_else(|| CabError("the cabinet is cut off".into()))?;
    let end =
        rest.iter().position(|&b| b == 0).ok_or_else(|| CabError("a name in the cabinet is not terminated".into()))?;
    Ok((&rest[..end], at + end + 1))
}

/// Every file of a single-cabinet, MSZip or stored CAB. Multi-cabinet sets and other compression are errors.
pub fn extract_all(cab: &[u8]) -> Result<Vec<CabFile>, CabError> {
    if cab.get(..4) != Some(b"MSCF") {
        return err("not a cabinet");
    }
    let cb_cabinet = u32_at(cab, 8)? as usize;
    if cb_cabinet > cab.len() {
        return err("the cabinet is cut off (it is shorter than it says)");
    }
    // Everything below is read from the cabinet's own length, not from whatever follows it in the stream.
    let cab = &cab[..cb_cabinet];
    let coff_files = u32_at(cab, 16)? as usize;
    let n_folders = u16_at(cab, 26)? as usize;
    let n_files = u16_at(cab, 28)? as usize;
    let flags = u16_at(cab, 30)?;
    if n_folders > MAX_FOLDERS || n_files > MAX_FILES {
        return err("the cabinet declares too many folders or files");
    }
    let mut o = 36usize;
    let (mut cb_folder, mut cb_data) = (0usize, 0usize);
    if flags & 4 != 0 {
        let cb_header = u16_at(cab, o)? as usize;
        cb_folder = *cab.get(o + 2).ok_or_else(|| CabError("the cabinet is cut off".into()))? as usize;
        cb_data = *cab.get(o + 3).ok_or_else(|| CabError("the cabinet is cut off".into()))? as usize;
        o += 4 + cb_header;
    }
    if flags & 1 != 0 {
        o = cstr(cab, o)?.1;
        o = cstr(cab, o)?.1;
    }
    if flags & 2 != 0 {
        o = cstr(cab, o)?.1;
        o = cstr(cab, o)?.1;
    }
    if flags & 3 != 0 {
        return err("the cabinet is one of a set; this reader only takes a single cabinet");
    }
    let mut folders = Vec::new();
    for _ in 0..n_folders {
        let coff = u32_at(cab, o)? as usize;
        let n_data = u16_at(cab, o + 4)? as usize;
        let kind = u16_at(cab, o + 6)?;
        o = o.checked_add(8 + cb_folder).ok_or_else(|| CabError("the cabinet is cut off".into()))?;
        folders.push((coff, n_data, kind & 0x0f));
    }
    let mut files = Vec::new();
    let mut p = coff_files;
    for _ in 0..n_files {
        let size = u32_at(cab, p)? as usize;
        let offset = u32_at(cab, p + 4)? as usize;
        let folder = u16_at(cab, p + 8)? as usize;
        let (name, next) = cstr(cab, p + 16)?;
        p = next;
        if folder >= folders.len() {
            return err("a file in the cabinet continues in another cabinet or names a missing folder");
        }
        files.push((name.to_vec(), offset, size, folder));
    }

    let mut folder_data: Vec<Option<Vec<u8>>> = vec![None; folders.len()];
    let decode_folder = |i: usize| -> Result<Vec<u8>, CabError> {
        let (coff, n_data, kind) = folders[i];
        if kind > 1 {
            return err("the cabinet uses a compression this reader does not support (only stored and MSZip)");
        }
        let mut q = coff;
        let mut out: Vec<u8> = Vec::new();
        for _ in 0..n_data {
            let cb = u16_at(cab, q + 4)? as usize;
            let cb_uncomp = u16_at(cab, q + 6)? as usize;
            q = q.checked_add(8 + cb_data).ok_or_else(|| CabError("the cabinet is cut off".into()))?;
            let block = cab
                .get(q..q.saturating_add(cb))
                .ok_or_else(|| CabError("the cabinet is cut off inside a data block".into()))?;
            q += cb;
            if cb_uncomp > BLOCK_MAX {
                return err("a data block in the cabinet is larger than a block can be");
            }
            if out.len() + cb_uncomp > MAX_FOLDER_BYTES {
                return err("a folder in the cabinet is implausibly large");
            }
            let decoded = if kind == 0 {
                if cb != cb_uncomp {
                    return err("a stored data block's sizes disagree");
                }
                block.to_vec()
            } else {
                if block.get(..2) != Some(b"CK") {
                    return err("a data block in the cabinet is not MSZip");
                }
                // The previous block's output (up to 32 KiB) is the dictionary of this one.
                let history = &out[out.len().saturating_sub(BLOCK_MAX)..];
                inflate_raw(&block[2..], history, BLOCK_MAX)?
            };
            if decoded.len() != cb_uncomp {
                return err("a data block in the cabinet decoded to a different size than it declares");
            }
            out.extend_from_slice(&decoded);
        }
        Ok(out)
    };
    let mut out = Vec::with_capacity(files.len());
    for (name, offset, size, folder) in files {
        if folder_data[folder].is_none() {
            folder_data[folder] = Some(decode_folder(folder)?);
        }
        let data = folder_data[folder].as_ref().expect("just decoded");
        let end =
            offset.checked_add(size).ok_or_else(|| CabError("a file in the cabinet overruns its folder".into()))?;
        let slice =
            data.get(offset..end).ok_or_else(|| CabError("a file in the cabinet overruns its folder".into()))?;
        out.push(CabFile { name, data: slice.to_vec() });
    }
    Ok(out)
}

/// Test-only cabinet writer.
#[cfg(test)]
pub mod build {
    use flate2::{Compression, write::DeflateEncoder};
    use std::io::Write;

    /// A deflate stream (raw) of `data`.
    pub fn deflate(data: &[u8]) -> Vec<u8> {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// A fixed-Huffman final block of `literal` followed by a match of `len` bytes at distance `dist` (which may
    /// reach back into a previous block's output when this is decoded with that history).
    pub fn deflate_with_match(literal: &[u8], len: usize, dist: usize) -> Vec<u8> {
        struct W {
            out: Vec<u8>,
            acc: u32,
            n: u32,
        }
        impl W {
            fn put(&mut self, v: u32, bits: u32) {
                for i in 0..bits {
                    self.acc |= ((v >> i) & 1) << self.n;
                    self.n += 1;
                    if self.n == 8 {
                        self.out.push(self.acc as u8);
                        self.acc = 0;
                        self.n = 0;
                    }
                }
            }
            // Huffman codes are packed most significant bit first.
            fn code(&mut self, code: u32, bits: u32) {
                for i in (0..bits).rev() {
                    self.put((code >> i) & 1, 1);
                }
            }
            fn lit(&mut self, v: u32) {
                match v {
                    0..=143 => self.code(0x30 + v, 8),
                    144..=255 => self.code(0x190 + v - 144, 9),
                    256..=279 => self.code(v - 256, 7),
                    _ => self.code(0xc0 + v - 280, 8),
                }
            }
        }
        let mut w = W { out: Vec::new(), acc: 0, n: 0 };
        w.put(1, 1); // final
        w.put(1, 2); // fixed Huffman
        for &b in literal {
            w.lit(b as u32);
        }
        // Length symbols: this helper supports 3..=10 (symbols 257..=264, no extra bits) and the distance table.
        assert!((3..=10).contains(&len));
        w.lit(257 + (len as u32 - 3));
        const DIST_BASE: [usize; 30] = [
            1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073,
            4097, 6145, 8193, 12289, 16385, 24577,
        ];
        const DIST_EXTRA: [u32; 30] =
            [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
        let sym = (0..30).rev().find(|&i| DIST_BASE[i] <= dist).unwrap();
        w.code(sym as u32, 5);
        w.put((dist - DIST_BASE[sym]) as u32, DIST_EXTRA[sym]);
        w.lit(256);
        if w.n > 0 {
            w.out.push(w.acc as u8);
        }
        w.out
    }

    /// One block of a folder: `uncompressed` is the size it will inflate to.
    pub struct Block {
        pub body: Vec<u8>,
        pub uncompressed: usize,
    }

    pub fn mszip(raw: &[u8]) -> Block {
        let mut body = b"CK".to_vec();
        body.extend(deflate(raw));
        Block { body, uncompressed: raw.len() }
    }

    pub fn stored(raw: &[u8]) -> Block {
        Block { body: raw.to_vec(), uncompressed: raw.len() }
    }

    /// A cabinet with one folder of `blocks` (type `kind`: 1 MSZip, 0 stored) and `files` as (name, offset, size).
    pub fn cabinet(kind: u16, blocks: &[Block], files: &[(&str, usize, usize)]) -> Vec<u8> {
        let mut data = Vec::new();
        for b in blocks {
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&(b.body.len() as u16).to_le_bytes());
            data.extend_from_slice(&(b.uncompressed as u16).to_le_bytes());
            data.extend_from_slice(&b.body);
        }
        let mut filebytes = Vec::new();
        for (name, off, size) in files {
            filebytes.extend_from_slice(&(*size as u32).to_le_bytes());
            filebytes.extend_from_slice(&(*off as u32).to_le_bytes());
            filebytes.extend_from_slice(&0u16.to_le_bytes());
            filebytes.extend_from_slice(&[0u8; 6]);
            filebytes.extend_from_slice(name.as_bytes());
            filebytes.push(0);
        }
        let coff_files = 36 + 8;
        let coff_data = coff_files + filebytes.len();
        let total = coff_data + data.len();
        let mut cab = Vec::new();
        cab.extend_from_slice(b"MSCF");
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&(total as u32).to_le_bytes());
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&(coff_files as u32).to_le_bytes());
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&[3, 1]);
        cab.extend_from_slice(&1u16.to_le_bytes());
        cab.extend_from_slice(&(files.len() as u16).to_le_bytes());
        cab.extend_from_slice(&0u16.to_le_bytes());
        cab.extend_from_slice(&0u16.to_le_bytes());
        cab.extend_from_slice(&0u16.to_le_bytes());
        // The folder entry.
        cab.extend_from_slice(&(coff_data as u32).to_le_bytes());
        cab.extend_from_slice(&(blocks.len() as u16).to_le_bytes());
        cab.extend_from_slice(&kind.to_le_bytes());
        assert_eq!(cab.len(), coff_files);
        cab.extend_from_slice(&filebytes);
        cab.extend_from_slice(&data);
        cab
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    fn pattern(n: usize, seed: u32) -> Vec<u8> {
        (0..n as u32).map(|i| (i.wrapping_mul(seed) >> 3) as u8).collect()
    }

    #[test]
    fn inflate_matches_a_real_encoder() {
        for data in [vec![], b"a".to_vec(), pattern(20_000, 31), vec![7u8; 30_000], pattern(32_768, 977)] {
            let packed = deflate(&data);
            assert_eq!(inflate_raw(&packed, &[], BLOCK_MAX).unwrap(), data);
        }
    }

    #[test]
    fn stored_blocks_inflate() {
        let raw = b"hello stored world";
        let mut s = vec![1u8];
        s.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        s.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        s.extend_from_slice(raw);
        assert_eq!(inflate_raw(&s, &[], 100).unwrap(), raw);
        s[3] ^= 1;
        assert!(inflate_raw(&s, &[], 100).is_err());
    }

    #[test]
    fn a_match_can_reach_into_the_previous_blocks_output() {
        let history = b"ABCDEFGH";
        // "xy" then copy 5 bytes from 8 back: "ABCDE"... distance 10 from the end of "xy" lands on 'A'.
        let packed = deflate_with_match(b"xy", 5, 10);
        assert_eq!(inflate_raw(&packed, history, 100).unwrap(), b"xyABCDE");
        // With no history the same stream is a reach before the start of the data.
        assert!(inflate_raw(&packed, &[], 100).is_err());
    }

    #[test]
    fn inflate_refuses_garbage_and_bombs_without_panicking() {
        for n in 0..300u32 {
            let junk = pattern(64, n * 2 + 1);
            let _ = inflate_raw(&junk, &[], 1000);
        }
        let bomb = deflate(&vec![0u8; 100_000]);
        assert!(inflate_raw(&bomb, &[], BLOCK_MAX).is_err());
        let ok = deflate(&pattern(5000, 3));
        for cut in 0..ok.len() {
            assert!(inflate_raw(&ok[..cut], &[], BLOCK_MAX).is_err() || cut == ok.len());
        }
    }

    #[test]
    fn extracts_files_that_span_several_blocks() {
        let a = pattern(32_768, 11);
        let b = pattern(32_768, 29);
        let c = pattern(5000, 7);
        let mut all = a.clone();
        all.extend(&b);
        all.extend(&c);
        let cab = cabinet(
            1,
            &[mszip(&a), mszip(&b), mszip(&c)],
            &[("first.bin", 0, 40_000), ("second.bin", 40_000, all.len() - 40_000)],
        );
        let files = extract_all(&cab).unwrap();
        assert_eq!(files[0].name, b"first.bin");
        assert_eq!(files[0].data, all[..40_000]);
        assert_eq!(files[1].data, all[40_000..]);
    }

    #[test]
    fn dictionary_continuity_between_blocks_is_honoured() {
        let first = b"ABCDEFGHIJ".to_vec();
        // The second block is "xy" + a 5-byte copy from before it: only valid when the first block is the dictionary.
        let second_body = {
            let mut b = b"CK".to_vec();
            b.extend(deflate_with_match(b"xy", 5, 12));
            Block { body: b, uncompressed: 7 }
        };
        let cab = cabinet(1, &[mszip(&first), second_body], &[("f", 0, 17)]);
        let files = extract_all(&cab).unwrap();
        assert_eq!(files[0].data, b"ABCDEFGHIJxyABCDE");
    }

    #[test]
    fn stored_folders_extract_too() {
        let cab = cabinet(0, &[stored(b"hello "), stored(b"world")], &[("hi", 3, 6)]);
        assert_eq!(extract_all(&cab).unwrap()[0].data, b"lo wor");
    }

    #[test]
    fn damaged_cabinets_are_refused() {
        let raw = pattern(10_000, 5);
        let cab = cabinet(1, &[mszip(&raw)], &[("f", 0, 10_000)]);
        // Truncation anywhere.
        for cut in [0, 3, 20, 36, 44, 60, cab.len() - 1] {
            assert!(extract_all(&cab[..cut]).is_err(), "cut {cut}");
        }
        // A file that runs past its folder.
        let over = cabinet(1, &[mszip(&raw)], &[("f", 9_000, 5_000)]);
        assert!(extract_all(&over).is_err());
        let huge = cabinet(1, &[mszip(&raw)], &[("f", usize::MAX >> 40, 10)]);
        assert!(extract_all(&huge).is_err());
        // A block that does not say what it holds.
        let lying = cabinet(1, &[Block { body: mszip(&raw).body, uncompressed: 9_999 }], &[("f", 0, 100)]);
        assert!(extract_all(&lying).is_err());
        // Not MSZip.
        let no_ck = cabinet(1, &[Block { body: deflate(&raw), uncompressed: raw.len() }], &[("f", 0, 100)]);
        assert!(extract_all(&no_ck).is_err());
        // A compression this reader does not know (LZX is type 3).
        let lzx = cabinet(3, &[mszip(&raw)], &[("f", 0, 10)]);
        assert!(extract_all(&lzx).is_err());
        // Corrupt deflate data.
        let mut bad = cab.clone();
        let at = bad.len() - 20;
        for b in &mut bad[at..] {
            *b ^= 0xA5;
        }
        assert!(extract_all(&bad).is_err());
        // A cabinet that says it is longer than the bytes given.
        let mut longer = cab.clone();
        longer[8..12].copy_from_slice(&(cab.len() as u32 + 100).to_le_bytes());
        assert!(extract_all(&longer).is_err());
        // Declaring a continuation in another cabinet.
        let mut set = cab.clone();
        set[30] |= 2;
        assert!(extract_all(&set).is_err());
    }
}
