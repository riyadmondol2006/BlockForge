// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Private adapter between this module and the NBT layer: little-endian (Bedrock flavour) named tags.
//!
//! Until the NBT phase lands the real IO, this holds a small little-endian reader and writer over
//! `crate::convert::nbt::{Tag, Compound}`. The functions mirror `nbt::io::{read_named, write_named}` with
//! `Flavor::Bedrock`, so switching is a change inside this file only.

use super::{BedrockError, Result};
use crate::convert::nbt::{Compound, Tag};
use std::io::Cursor;

/// Nesting limit (Mojang's own readers stop far earlier).
const MAX_DEPTH: usize = 64;

fn err(m: impl Into<String>) -> BedrockError {
    BedrockError::Nbt(m.into())
}

fn take<'a>(c: &mut Cursor<&'a [u8]>, n: usize) -> Result<&'a [u8]> {
    let data: &'a [u8] = c.get_ref();
    let pos = c.position() as usize;
    let end = pos.checked_add(n).filter(|e| *e <= data.len()).ok_or_else(|| err("unexpected end of NBT"))?;
    c.set_position(end as u64);
    Ok(&data[pos..end])
}

fn read_u8(c: &mut Cursor<&[u8]>) -> Result<u8> {
    Ok(take(c, 1)?[0])
}

fn read_i16(c: &mut Cursor<&[u8]>) -> Result<i16> {
    Ok(i16::from_le_bytes(take(c, 2)?.try_into().map_err(|_| err("short"))?))
}

fn read_i32(c: &mut Cursor<&[u8]>) -> Result<i32> {
    Ok(i32::from_le_bytes(take(c, 4)?.try_into().map_err(|_| err("short"))?))
}

fn read_i64(c: &mut Cursor<&[u8]>) -> Result<i64> {
    Ok(i64::from_le_bytes(take(c, 8)?.try_into().map_err(|_| err("short"))?))
}

fn read_string(c: &mut Cursor<&[u8]>) -> Result<String> {
    let len = u16::from_le_bytes(take(c, 2)?.try_into().map_err(|_| err("short"))?) as usize;
    Ok(String::from_utf8_lossy(take(c, len)?).into_owned())
}

fn read_len(c: &mut Cursor<&[u8]>, elem: usize) -> Result<usize> {
    let n = read_i32(c)?;
    let n = usize::try_from(n).map_err(|_| err("negative array length"))?;
    if n.checked_mul(elem).is_none_or(|b| b > c.get_ref().len().saturating_sub(c.position() as usize)) {
        return Err(err("array longer than the data"));
    }
    Ok(n)
}

fn read_payload(c: &mut Cursor<&[u8]>, id: u8, depth: usize) -> Result<Tag> {
    if depth > MAX_DEPTH {
        return Err(err("NBT nested too deeply"));
    }
    Ok(match id {
        1 => Tag::Byte(read_u8(c)? as i8),
        2 => Tag::Short(read_i16(c)?),
        3 => Tag::Int(read_i32(c)?),
        4 => Tag::Long(read_i64(c)?),
        5 => Tag::Float(f32::from_bits(read_i32(c)? as u32)),
        6 => Tag::Double(f64::from_bits(read_i64(c)? as u64)),
        7 => {
            let n = read_len(c, 1)?;
            Tag::ByteArray(take(c, n)?.iter().map(|b| *b as i8).collect())
        }
        8 => Tag::String(read_string(c)?),
        9 => {
            let elem = read_u8(c)?;
            let n = read_i32(c)?;
            let n = usize::try_from(n).map_err(|_| err("negative list length"))?;
            if elem == 0 && n > 0 {
                return Err(err("list of end tags"));
            }
            if n > c.get_ref().len() {
                return Err(err("list longer than the data"));
            }
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(read_payload(c, elem, depth + 1)?);
            }
            Tag::List(v)
        }
        10 => {
            let mut comp = Compound::new();
            loop {
                let t = read_u8(c)?;
                if t == 0 {
                    break;
                }
                let name = read_string(c)?;
                comp.insert(name, read_payload(c, t, depth + 1)?);
            }
            Tag::Compound(comp)
        }
        11 => {
            let n = read_len(c, 4)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(read_i32(c)?);
            }
            Tag::IntArray(v)
        }
        12 => {
            let n = read_len(c, 8)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(read_i64(c)?);
            }
            Tag::LongArray(v)
        }
        other => return Err(err(format!("unknown tag id {other}"))),
    })
}

/// Read one named tag from `c`, leaving the cursor after it.
pub(crate) fn read_named(c: &mut Cursor<&[u8]>) -> Result<(String, Tag)> {
    let id = read_u8(c)?;
    if id == 0 {
        return Err(err("unexpected end tag"));
    }
    let name = read_string(c)?;
    Ok((name, read_payload(c, id, 0)?))
}

/// Read every named tag in `bytes` back to back (block entities, legacy entities).
pub(crate) fn read_all_named(bytes: &[u8]) -> Result<Vec<(String, Tag)>> {
    let mut c = Cursor::new(bytes);
    let mut out = Vec::new();
    while (c.position() as usize) < bytes.len() {
        out.push(read_named(&mut c)?);
    }
    Ok(out)
}

/// Read a single named tag that fills `bytes`.
pub(crate) fn from_slice(bytes: &[u8]) -> Result<(String, Tag)> {
    read_named(&mut Cursor::new(bytes))
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

fn write_string(out: &mut Vec<u8>, s: &str) -> Result<()> {
    let len = u16::try_from(s.len()).map_err(|_| err("string longer than 65535 bytes"))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

fn write_payload(out: &mut Vec<u8>, t: &Tag) -> Result<()> {
    match t {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_le_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_le_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_le_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
        Tag::ByteArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_le_bytes());
            out.extend(v.iter().map(|b| *b as u8));
        }
        Tag::String(s) => write_string(out, s)?,
        Tag::List(v) => {
            let id = v.first().map_or(0, tag_id);
            if v.iter().any(|e| tag_id(e) != id) {
                return Err(err("list with mixed element types"));
            }
            out.push(id);
            out.extend_from_slice(&(v.len() as i32).to_le_bytes());
            for e in v {
                write_payload(out, e)?;
            }
        }
        Tag::Compound(c) => {
            for (k, v) in &c.0 {
                out.push(tag_id(v));
                write_string(out, k)?;
                write_payload(out, v)?;
            }
            out.push(0);
        }
        Tag::IntArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_le_bytes());
            for e in v {
                out.extend_from_slice(&e.to_le_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend_from_slice(&(v.len() as i32).to_le_bytes());
            for e in v {
                out.extend_from_slice(&e.to_le_bytes());
            }
        }
    }
    Ok(())
}

/// Append one named tag to `out`.
pub(crate) fn write_named(out: &mut Vec<u8>, name: &str, tag: &Tag) -> Result<()> {
    out.push(tag_id(tag));
    write_string(out, name)?;
    write_payload(out, tag)
}

/// Encode one named tag.
pub(crate) fn to_vec(name: &str, tag: &Tag) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_named(&mut out, name, tag)?;
    Ok(out)
}
