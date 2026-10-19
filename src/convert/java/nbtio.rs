// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
#![allow(dead_code)]
//! A private adapter over the NBT surface this module needs (Java flavor only).
//!
//! The names match `crate::convert::nbt::io` and `crate::convert::util::lz4_block` so that, when those land, this
//! file becomes a handful of re-exports. Until then it carries a small stand-in so the region code is testable.

use crate::convert::nbt::{Compound, Tag};
use flate2::Compression;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};
use std::fmt;
use std::io::{Read, Write};

/// An NBT or compression failure.
#[derive(Debug)]
pub struct NbtError(pub String);

impl fmt::Display for NbtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NbtError {}

fn err<T>(msg: impl Into<String>) -> Result<T, NbtError> {
    Err(NbtError(msg.into()))
}

/// Largest output any decompressor here will produce.
pub const MAX_DECOMPRESSED: u64 = 1 << 30;

fn read_limited(mut reader: impl Read, limit: u64) -> Result<Vec<u8>, NbtError> {
    let mut out = Vec::new();
    (&mut reader).take(limit + 1).read_to_end(&mut out).map_err(|e| NbtError(e.to_string()))?;
    if out.len() as u64 > limit {
        return err("decompressed data is larger than the limit");
    }
    Ok(out)
}

pub fn gunzip(data: &[u8], limit: u64) -> Result<Vec<u8>, NbtError> {
    read_limited(GzDecoder::new(data), limit)
}

pub fn unzlib(data: &[u8], limit: u64) -> Result<Vec<u8>, NbtError> {
    read_limited(ZlibDecoder::new(data), limit)
}

pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

/// Gzip, zlib or raw, by looking at the first bytes.
pub fn decompress_auto(data: &[u8]) -> Result<Vec<u8>, NbtError> {
    if data.starts_with(&[0x1f, 0x8b]) {
        gunzip(data, MAX_DECOMPRESSED)
    } else if data.len() > 2 && data[0] == 0x78 && matches!(data[1], 0x01 | 0x5e | 0x9c | 0xda) {
        unzlib(data, MAX_DECOMPRESSED)
    } else {
        Ok(data.to_vec())
    }
}

// ---- LZ4Block (Mojang's stream of LZ4 blocks, as used by region compression type 4) ----

const LZ4_MAGIC: &[u8; 8] = b"LZ4Block";

/// Decode a stream of `LZ4Block` blocks (checksums are not verified by this stand-in).
pub fn lz4_block_decode(data: &[u8], limit: u64) -> Result<Vec<u8>, NbtError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let head = data.get(pos..pos + 21).ok_or_else(|| NbtError("truncated LZ4 block header".into()))?;
        if &head[..8] != LZ4_MAGIC {
            return err("bad LZ4 block magic");
        }
        let method = head[8] & 0xF0;
        let compressed = u32::from_le_bytes([head[9], head[10], head[11], head[12]]) as usize;
        let raw = u32::from_le_bytes([head[13], head[14], head[15], head[16]]) as usize;
        pos += 21;
        let body = data.get(pos..pos.checked_add(compressed).ok_or_else(|| NbtError("LZ4 length".into()))?);
        let body = body.ok_or_else(|| NbtError("truncated LZ4 block body".into()))?;
        pos += compressed;
        if raw == 0 {
            break;
        }
        if out.len() as u64 + raw as u64 > limit {
            return err("decompressed data is larger than the limit");
        }
        match method {
            0x10 => {
                if body.len() != raw {
                    return err("bad stored LZ4 block");
                }
                out.extend_from_slice(body);
            }
            0x20 => lz4_sequences(body, raw, &mut out)?,
            _ => return err("unknown LZ4 block method"),
        }
    }
    Ok(out)
}

fn lz4_sequences(src: &[u8], raw: usize, out: &mut Vec<u8>) -> Result<(), NbtError> {
    let start = out.len();
    let mut i = 0usize;
    let len_ext = |i: &mut usize, mut n: usize| -> Result<usize, NbtError> {
        if n == 15 {
            loop {
                let b = *src.get(*i).ok_or_else(|| NbtError("truncated LZ4 length".into()))?;
                *i += 1;
                n += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        Ok(n)
    };
    while i < src.len() {
        let token = src[i];
        i += 1;
        let lit = len_ext(&mut i, (token >> 4) as usize)?;
        let lits = src.get(i..i.saturating_add(lit)).ok_or_else(|| NbtError("truncated LZ4 literals".into()))?;
        if out.len() - start + lit > raw {
            return err("LZ4 literals overrun the block");
        }
        out.extend_from_slice(lits);
        i += lit;
        if i >= src.len() {
            break;
        }
        let off = src.get(i..i + 2).ok_or_else(|| NbtError("truncated LZ4 offset".into()))?;
        let off = u16::from_le_bytes([off[0], off[1]]) as usize;
        i += 2;
        let mlen = len_ext(&mut i, (token & 15) as usize)? + 4;
        if off == 0 || off > out.len() - start {
            return err("bad LZ4 match offset");
        }
        if out.len() - start + mlen > raw {
            return err("LZ4 match overruns the block");
        }
        for _ in 0..mlen {
            let b = out[out.len() - off];
            out.push(b);
        }
    }
    if out.len() - start != raw {
        return err("LZ4 block has the wrong length");
    }
    Ok(())
}

/// Encode as `LZ4Block` using stored (uncompressed) blocks: valid for any reader, used by tests.
pub fn lz4_block_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(1 << 16) {
        out.extend_from_slice(LZ4_MAGIC);
        out.push(0x10 | 0x07);
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(LZ4_MAGIC);
    out.push(0x10 | 0x07);
    out.extend_from_slice(&[0; 12]);
    out
}

// ---- Java NBT (big endian) ----

const MAX_DEPTH: usize = 512;

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], NbtError> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len());
        match end {
            Some(e) => {
                let s = &self.b[self.p..e];
                self.p = e;
                Ok(s)
            }
            None => err("unexpected end of NBT"),
        }
    }
    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, NbtError> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap_or([0; 2])))
    }
    fn i32(&mut self) -> Result<i32, NbtError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap_or([0; 4])))
    }
    fn i64(&mut self) -> Result<i64, NbtError> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap_or([0; 8])))
    }
    fn string(&mut self) -> Result<String, NbtError> {
        let n = self.i16()? as u16 as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn len(&mut self, elem: usize) -> Result<usize, NbtError> {
        let n = self.i32()?;
        if n < 0 || (n as usize).saturating_mul(elem) > self.b.len() - self.p {
            return err("NBT array longer than the data");
        }
        Ok(n as usize)
    }
}

fn read_payload(c: &mut Cur, id: u8, depth: usize) -> Result<Tag, NbtError> {
    if depth > MAX_DEPTH {
        return err("NBT nested too deeply");
    }
    Ok(match id {
        1 => Tag::Byte(c.u8()? as i8),
        2 => Tag::Short(c.i16()?),
        3 => Tag::Int(c.i32()?),
        4 => Tag::Long(c.i64()?),
        5 => Tag::Float(f32::from_bits(c.i32()? as u32)),
        6 => Tag::Double(f64::from_bits(c.i64()? as u64)),
        7 => {
            let n = c.len(1)?;
            Tag::ByteArray(c.take(n)?.iter().map(|&b| b as i8).collect())
        }
        8 => Tag::String(c.string()?),
        9 => {
            let elem = c.u8()?;
            let n = c.len(1)?;
            if elem == 0 && n > 0 {
                return err("NBT list of end tags");
            }
            let mut v = Vec::with_capacity(n.min(1024));
            for _ in 0..n {
                v.push(read_payload(c, elem, depth + 1)?);
            }
            Tag::List(v)
        }
        10 => {
            let mut m = Compound::new();
            loop {
                let id = c.u8()?;
                if id == 0 {
                    break;
                }
                let name = c.string()?;
                m.insert(name, read_payload(c, id, depth + 1)?);
            }
            Tag::Compound(m)
        }
        11 => {
            let n = c.len(4)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(c.i32()?);
            }
            Tag::IntArray(v)
        }
        12 => {
            let n = c.len(8)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(c.i64()?);
            }
            Tag::LongArray(v)
        }
        _ => return err(format!("unknown NBT tag id {id}")),
    })
}

/// Parse a named root tag from uncompressed Java NBT bytes.
pub fn from_slice(bytes: &[u8]) -> Result<(String, Tag), NbtError> {
    let mut c = Cur { b: bytes, p: 0 };
    let id = c.u8()?;
    if id == 0 {
        return err("NBT root is an end tag");
    }
    let name = c.string()?;
    let tag = read_payload(&mut c, id, 0)?;
    Ok((name, tag))
}

fn tag_id(t: &Tag) -> u8 {
    match t {
        Tag::Byte(_) => 1,
        Tag::Short(_) => 2,
        Tag::Int(_) => 3,
        Tag::Long(_) => 4,
        Tag::Float(_) => 5,
        Tag::Double(_) => 6,
        Tag::ByteArray(_) => 7,
        Tag::String(_) => 8,
        Tag::List(_) => 9,
        Tag::Compound(_) => 10,
        Tag::IntArray(_) => 11,
        Tag::LongArray(_) => 12,
    }
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn write_payload(out: &mut Vec<u8>, t: &Tag) {
    match t {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
        Tag::ByteArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            out.extend(v.iter().map(|&b| b as u8));
        }
        Tag::String(s) => write_str(out, s),
        Tag::List(v) => {
            out.push(v.first().map_or(0, tag_id));
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for e in v {
                write_payload(out, e);
            }
        }
        Tag::Compound(c) => {
            for (k, v) in &c.0 {
                out.push(tag_id(v));
                write_str(out, k);
                write_payload(out, v);
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for e in v {
                out.extend_from_slice(&e.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_be_bytes());
            for e in v {
                out.extend_from_slice(&e.to_be_bytes());
            }
        }
    }
}

/// Serialize a named root tag as uncompressed Java NBT.
pub fn to_vec(name: &str, tag: &Tag) -> Vec<u8> {
    let mut out = vec![tag_id(tag)];
    write_str(&mut out, name);
    write_payload(&mut out, tag);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_round_trip() {
        let mut inner = Compound::new();
        inner.insert("n", Tag::Int(-5));
        inner.insert("s", Tag::String("héllo".into()));
        let mut c = Compound::new();
        c.insert("inner", Tag::Compound(inner));
        c.insert("longs", Tag::LongArray(vec![1, -2, 3]));
        c.insert("list", Tag::List(vec![Tag::Short(1), Tag::Short(2)]));
        c.insert("bytes", Tag::ByteArray(vec![-1, 0, 1]));
        let tag = Tag::Compound(c);
        let bytes = to_vec("root", &tag);
        let (name, back) = from_slice(&bytes).unwrap();
        assert_eq!(name, "root");
        assert_eq!(back, tag);
    }

    #[test]
    fn nbt_rejects_garbage() {
        assert!(from_slice(&[]).is_err());
        assert!(from_slice(&[10, 0, 0, 7, 0, 1, b'a', 0x7f, 0xff, 0xff, 0xff]).is_err(), "huge array length");
        assert!(from_slice(&[10, 0, 0, 1, 0, 1, b'a']).is_err(), "truncated");
        assert!(from_slice(&[99, 0, 0]).is_err());
    }

    #[test]
    fn lz4_stored_and_sequences() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(lz4_block_decode(&lz4_block_encode(&data), 1 << 30).unwrap(), data);
        // One real LZ4 sequence block: literal "abcd" then a match of length 8 at offset 4.
        let mut s = Vec::new();
        s.extend_from_slice(LZ4_MAGIC);
        s.push(0x20 | 7);
        let body = [0x44u8, b'a', b'b', b'c', b'd', 4, 0];
        s.extend_from_slice(&(body.len() as u32).to_le_bytes());
        s.extend_from_slice(&12u32.to_le_bytes());
        s.extend_from_slice(&0u32.to_le_bytes());
        s.extend_from_slice(&body);
        s.extend_from_slice(&lz4_block_encode(&[])[..]);
        assert_eq!(lz4_block_decode(&s, 1 << 20).unwrap(), b"abcdabcdabcd");
        assert!(lz4_block_decode(&s[..30], 1 << 20).is_err());
        assert!(lz4_block_decode(&lz4_block_encode(&data), 100).is_err());
    }

    #[test]
    fn compression_helpers() {
        let data = vec![7u8; 10_000];
        assert_eq!(decompress_auto(&gzip(&data)).unwrap(), data);
        assert_eq!(decompress_auto(&zlib(&data)).unwrap(), data);
        assert_eq!(decompress_auto(&data).unwrap(), data);
        assert!(gunzip(&gzip(&data), 100).is_err());
    }
}
