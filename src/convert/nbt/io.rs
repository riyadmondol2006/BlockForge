// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Binary NBT reading and writing in the two on-disk flavors.
//!
//! Reading is defensive: lengths are checked against Chunker's limits before anything is allocated, nesting is capped
//! and truncated or malformed input is an [`NbtError`], never a panic.

use std::borrow::Cow;
use std::io::{Read, Write};

use super::compound::Compound;
use super::error::NbtError;
use super::mutf8;
use super::tag::{Tag, TagType, check_homogeneous};

/// Longest tag name read (bytes for Bedrock, UTF-16 units for Java), as in Chunker.
pub const MAX_NAME_LENGTH: usize = 256;
/// Longest string value read, as in Chunker (`Short.MAX_VALUE`).
pub const MAX_STRING_LENGTH: usize = 32767;
/// Most elements of a byte, int or long array read, as in Chunker.
pub const MAX_ARRAY_LENGTH: usize = 65536;
/// Most elements of a list read, as in Chunker (one per block of a chunk section column).
pub const MAX_LIST_LENGTH: usize = 16 * 16 * 4096;
/// Most entries of a compound read, as in Chunker.
pub const MAX_COMPOUND_LENGTH: usize = 16 * 16 * 4096;
/// Deepest nesting of lists and compounds (Minecraft's own limit).
pub const MAX_DEPTH: usize = 512;

/// The two byte layouts of NBT.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flavor {
    /// Java edition: big-endian numbers, strings in Java's modified UTF-8.
    Java,
    /// Bedrock edition: little-endian numbers, strings in plain UTF-8.
    Bedrock,
}

// ---------------------------------------------------------------------------------------------------------------
// Input sources

trait Input {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], NbtError>;
    fn take(&mut self, n: usize) -> Result<Cow<'_, [u8]>, NbtError>;
    /// Bytes left when known (used to bound pre-allocation).
    fn remaining(&self) -> Option<usize>;
}

struct SliceIn<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Input for SliceIn<'_> {
    #[inline]
    fn array<const N: usize>(&mut self) -> Result<[u8; N], NbtError> {
        let end = self.pos.checked_add(N).filter(|&e| e <= self.data.len()).ok_or(NbtError::UnexpectedEof)?;
        let mut out = [0u8; N];
        out.copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        Ok(out)
    }

    #[inline]
    fn take(&mut self, n: usize) -> Result<Cow<'_, [u8]>, NbtError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(NbtError::UnexpectedEof)?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(Cow::Borrowed(out))
    }

    fn remaining(&self) -> Option<usize> {
        Some(self.data.len() - self.pos)
    }
}

struct StreamIn<R>(R);

impl<R: Read> Input for StreamIn<R> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], NbtError> {
        let mut out = [0u8; N];
        self.0.read_exact(&mut out)?;
        Ok(out)
    }

    fn take(&mut self, n: usize) -> Result<Cow<'_, [u8]>, NbtError> {
        // Grow with the data that actually arrives so a huge claimed length cannot allocate up front.
        let mut out = Vec::with_capacity(n.min(8192));
        let got = (&mut self.0).take(n as u64).read_to_end(&mut out)?;
        if got != n {
            return Err(NbtError::UnexpectedEof);
        }
        Ok(Cow::Owned(out))
    }

    fn remaining(&self) -> Option<usize> {
        None
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Decoding

struct Decoder<I> {
    inp: I,
    le: bool,
    java: bool,
}

impl<I: Input> Decoder<I> {
    fn new(inp: I, flavor: Flavor) -> Self {
        Decoder { inp, le: flavor == Flavor::Bedrock, java: flavor == Flavor::Java }
    }

    #[inline]
    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.inp.array::<1>()?[0])
    }

    #[inline]
    fn i16(&mut self) -> Result<i16, NbtError> {
        let b = self.inp.array::<2>()?;
        Ok(if self.le { i16::from_le_bytes(b) } else { i16::from_be_bytes(b) })
    }

    #[inline]
    fn i32(&mut self) -> Result<i32, NbtError> {
        let b = self.inp.array::<4>()?;
        Ok(if self.le { i32::from_le_bytes(b) } else { i32::from_be_bytes(b) })
    }

    #[inline]
    fn i64(&mut self) -> Result<i64, NbtError> {
        let b = self.inp.array::<8>()?;
        Ok(if self.le { i64::from_le_bytes(b) } else { i64::from_be_bytes(b) })
    }

    /// A string with a short length prefix; `max` is the longest accepted (see the constants).
    fn string(&mut self, max: usize, what: &'static str) -> Result<String, NbtError> {
        if self.java {
            let len = u16::from_be_bytes(self.inp.array::<2>()?) as usize;
            let bytes = self.inp.take(len)?;
            let s = mutf8::decode(&bytes)?;
            if len > max && mutf8::utf16_len(&s) > max {
                return Err(NbtError::LimitExceeded { what, len: mutf8::utf16_len(&s) as i64, max });
            }
            Ok(s)
        } else {
            let len = self.i16()?;
            if len < 0 || len as usize > max {
                return Err(NbtError::LimitExceeded { what, len: len as i64, max });
            }
            let bytes = self.inp.take(len as usize)?;
            match bytes {
                Cow::Borrowed(b) => std::str::from_utf8(b)
                    .map(str::to_owned)
                    .map_err(|e| NbtError::BadString(format!("not valid UTF-8: {e}"))),
                Cow::Owned(v) => {
                    String::from_utf8(v).map_err(|e| NbtError::BadString(format!("not valid UTF-8: {}", e.utf8_error())))
                }
            }
        }
    }

    fn array_len(&mut self, what: &'static str, max: usize) -> Result<usize, NbtError> {
        let len = self.i32()?;
        if len < 0 || len as usize > max {
            return Err(NbtError::LimitExceeded { what, len: len as i64, max });
        }
        Ok(len as usize)
    }

    fn tag_type(&mut self) -> Result<TagType, NbtError> {
        let id = self.u8()?;
        TagType::from_id(id).ok_or(NbtError::UnknownTagId(id))
    }

    /// A root or compound entry: `None` at an END tag.
    fn named(&mut self, depth: usize) -> Result<Option<(String, Tag)>, NbtError> {
        let ty = self.tag_type()?;
        if ty == TagType::End {
            return Ok(None);
        }
        let name = self.string(MAX_NAME_LENGTH, "name")?;
        let tag = self.value(ty, depth)?;
        Ok(Some((name, tag)))
    }

    fn value(&mut self, ty: TagType, depth: usize) -> Result<Tag, NbtError> {
        Ok(match ty {
            TagType::End => return Err(NbtError::UnknownTagId(0)),
            TagType::Byte => Tag::Byte(self.u8()? as i8),
            TagType::Short => Tag::Short(self.i16()?),
            TagType::Int => Tag::Int(self.i32()?),
            TagType::Long => Tag::Long(self.i64()?),
            TagType::Float => Tag::Float(f32::from_bits(self.i32()? as u32)),
            TagType::Double => Tag::Double(f64::from_bits(self.i64()? as u64)),
            TagType::String => Tag::String(self.string(MAX_STRING_LENGTH, "string")?),
            TagType::ByteArray => {
                let len = self.array_len("byte array", MAX_ARRAY_LENGTH)?;
                let bytes = self.inp.take(len)?;
                Tag::ByteArray(bytes.iter().map(|&b| b as i8).collect())
            }
            TagType::IntArray => {
                let len = self.array_len("int array", MAX_ARRAY_LENGTH)?;
                let bytes = self.inp.take(len * 4)?;
                let le = self.le;
                Tag::IntArray(
                    bytes
                        .chunks_exact(4)
                        .map(|c| {
                            let b = [c[0], c[1], c[2], c[3]];
                            if le { i32::from_le_bytes(b) } else { i32::from_be_bytes(b) }
                        })
                        .collect(),
                )
            }
            TagType::LongArray => {
                let len = self.array_len("long array", MAX_ARRAY_LENGTH)?;
                let bytes = self.inp.take(len * 8)?;
                let le = self.le;
                Tag::LongArray(
                    bytes
                        .chunks_exact(8)
                        .map(|c| {
                            let b = [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]];
                            if le { i64::from_le_bytes(b) } else { i64::from_be_bytes(b) }
                        })
                        .collect(),
                )
            }
            TagType::List => {
                if depth >= MAX_DEPTH {
                    return Err(NbtError::TooDeep);
                }
                let elem = self.tag_type()?;
                let len = self.array_len("list", MAX_LIST_LENGTH)?;
                // As Chunker: an END-typed list is empty whatever its length claims (older versions use BYTE).
                if elem == TagType::End || len == 0 {
                    return Ok(Tag::List(Vec::new()));
                }
                let cap = len.min(self.inp.remaining().unwrap_or(4096));
                let mut items = Vec::with_capacity(cap);
                for _ in 0..len {
                    items.push(self.value(elem, depth + 1)?);
                }
                Tag::List(items)
            }
            TagType::Compound => {
                if depth >= MAX_DEPTH {
                    return Err(NbtError::TooDeep);
                }
                let mut map = Compound::with_capacity(8);
                while let Some((name, tag)) = self.named(depth + 1)? {
                    map.0.insert(name, tag);
                    if map.0.len() > MAX_COMPOUND_LENGTH {
                        return Err(NbtError::LimitExceeded {
                            what: "compound",
                            len: map.0.len() as i64,
                            max: MAX_COMPOUND_LENGTH,
                        });
                    }
                }
                Tag::Compound(map)
            }
        })
    }
}

/// Read one named root tag from a stream (wrap files in a `BufReader`). An END tag in place of the root is
/// [`NbtError::EmptyRoot`]. Bytes after the root are left unread.
pub fn read_named<R: Read>(r: &mut R, flavor: Flavor) -> Result<(String, Tag), NbtError> {
    let mut dec = Decoder::new(StreamIn(r), flavor);
    dec.named(0)?.ok_or(NbtError::EmptyRoot)
}

/// Read one named root tag from the start of `bytes`; trailing bytes are ignored (see [`read_compound_stream`]).
pub fn from_slice(bytes: &[u8], flavor: Flavor) -> Result<(String, Tag), NbtError> {
    let mut dec = Decoder::new(SliceIn { data: bytes, pos: 0 }, flavor);
    dec.named(0)?.ok_or(NbtError::EmptyRoot)
}

/// Read back-to-back root tags until the slice ends (or an END tag is met), as Bedrock stores several compounds in
/// one database value.
pub fn read_compound_stream(bytes: &[u8], flavor: Flavor) -> Result<Vec<(String, Tag)>, NbtError> {
    let mut dec = Decoder::new(SliceIn { data: bytes, pos: 0 }, flavor);
    let mut out = Vec::new();
    while dec.inp.pos < bytes.len() {
        match dec.named(0)? {
            Some(pair) => out.push(pair),
            None => break,
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------
// Encoding

struct Encoder {
    out: Vec<u8>,
    le: bool,
    java: bool,
}

impl Encoder {
    fn new(flavor: Flavor, capacity: usize) -> Self {
        Encoder { out: Vec::with_capacity(capacity), le: flavor == Flavor::Bedrock, java: flavor == Flavor::Java }
    }

    #[inline]
    fn i16(&mut self, v: i16) {
        self.out.extend_from_slice(&if self.le { v.to_le_bytes() } else { v.to_be_bytes() });
    }

    #[inline]
    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&if self.le { v.to_le_bytes() } else { v.to_be_bytes() });
    }

    #[inline]
    fn i64(&mut self, v: i64) {
        self.out.extend_from_slice(&if self.le { v.to_le_bytes() } else { v.to_be_bytes() });
    }

    fn string(&mut self, s: &str) -> Result<(), NbtError> {
        let len_at = self.out.len();
        self.out.extend_from_slice(&[0, 0]);
        let (len, max) = if self.java {
            mutf8::encode(s, &mut self.out);
            (self.out.len() - len_at - 2, u16::MAX as usize)
        } else {
            self.out.extend_from_slice(s.as_bytes());
            (s.len(), MAX_STRING_LENGTH)
        };
        if len > max {
            self.out.truncate(len_at);
            return Err(NbtError::BadString(format!("string of {len} bytes is longer than {max}")));
        }
        let prefix = if self.le { (len as u16).to_le_bytes() } else { (len as u16).to_be_bytes() };
        self.out[len_at..len_at + 2].copy_from_slice(&prefix);
        Ok(())
    }

    fn len(&mut self, len: usize, what: &'static str) -> Result<(), NbtError> {
        let v = i32::try_from(len).map_err(|_| NbtError::LimitExceeded { what, len: len as i64, max: i32::MAX as usize })?;
        self.i32(v);
        Ok(())
    }

    fn named(&mut self, name: &str, tag: &Tag, depth: usize) -> Result<(), NbtError> {
        self.out.push(tag.type_id());
        self.string(name)?;
        self.value(tag, depth)
    }

    fn value(&mut self, tag: &Tag, depth: usize) -> Result<(), NbtError> {
        match tag {
            Tag::Byte(v) => self.out.push(*v as u8),
            Tag::Short(v) => self.i16(*v),
            Tag::Int(v) => self.i32(*v),
            Tag::Long(v) => self.i64(*v),
            Tag::Float(v) => self.i32(v.to_bits() as i32),
            Tag::Double(v) => self.i64(v.to_bits() as i64),
            Tag::String(s) => self.string(s)?,
            Tag::ByteArray(a) => {
                self.len(a.len(), "byte array")?;
                self.out.extend(a.iter().map(|&b| b as u8));
            }
            Tag::IntArray(a) => {
                self.len(a.len(), "int array")?;
                self.out.reserve(a.len() * 4);
                for &v in a {
                    self.i32(v);
                }
            }
            Tag::LongArray(a) => {
                self.len(a.len(), "long array")?;
                self.out.reserve(a.len() * 8);
                for &v in a {
                    self.i64(v);
                }
            }
            Tag::List(items) => {
                if depth >= MAX_DEPTH {
                    return Err(NbtError::TooDeep);
                }
                check_homogeneous(items)?;
                self.out.push(items.first().map_or(0, Tag::type_id));
                self.len(items.len(), "list")?;
                for item in items {
                    self.value(item, depth + 1)?;
                }
            }
            Tag::Compound(c) => {
                if depth >= MAX_DEPTH {
                    return Err(NbtError::TooDeep);
                }
                for (name, tag) in &c.0 {
                    self.named(name, tag, depth + 1)?;
                }
                self.out.push(0);
            }
        }
        Ok(())
    }
}

/// Write one named root tag.
pub fn write_named<W: Write>(w: &mut W, name: &str, tag: &Tag, flavor: Flavor) -> Result<(), NbtError> {
    let bytes = to_vec(name, tag, flavor)?;
    w.write_all(&bytes)?;
    Ok(())
}

/// Encode one named root tag to bytes.
pub fn to_vec(name: &str, tag: &Tag, flavor: Flavor) -> Result<Vec<u8>, NbtError> {
    let mut enc = Encoder::new(flavor, 1024);
    enc.named(name, tag, 0)?;
    Ok(enc.out)
}

/// Encode `tag` as a value only: no tag id and no name (the payload of a list element).
pub fn value_to_vec(tag: &Tag, flavor: Flavor) -> Result<Vec<u8>, NbtError> {
    let mut enc = Encoder::new(flavor, 256);
    enc.value(tag, 0)?;
    Ok(enc.out)
}

/// Read a value of the given type without id or name.
pub fn value_from_slice(bytes: &[u8], ty: TagType, flavor: Flavor) -> Result<Tag, NbtError> {
    Decoder::new(SliceIn { data: bytes, pos: 0 }, flavor).value(ty, 0)
}

// ---------------------------------------------------------------------------------------------------------------
// Compression

/// The most a decompressed NBT blob may grow to (a bound against decompression bombs).
pub const MAX_DECOMPRESSED: usize = 1 << 30;

fn read_limited<R: Read>(r: R, what: &str) -> Result<Vec<u8>, NbtError> {
    let mut out = Vec::new();
    let mut limited = r.take(MAX_DECOMPRESSED as u64 + 1);
    limited.read_to_end(&mut out).map_err(|e| NbtError::Compression(format!("{what}: {e}")))?;
    if out.len() > MAX_DECOMPRESSED {
        return Err(NbtError::Compression(format!("{what}: output is larger than {MAX_DECOMPRESSED} bytes")));
    }
    Ok(out)
}

/// Decompress gzip.
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, NbtError> {
    read_limited(flate2::read::GzDecoder::new(bytes), "gzip")
}

/// Decompress zlib.
pub fn unzlib(bytes: &[u8]) -> Result<Vec<u8>, NbtError> {
    read_limited(flate2::read::ZlibDecoder::new(bytes), "zlib")
}

/// Decompress a raw deflate stream (no header), as Bedrock's LevelDB blocks use.
pub fn inflate_raw(bytes: &[u8]) -> Result<Vec<u8>, NbtError> {
    read_limited(flate2::read::DeflateDecoder::new(bytes), "deflate")
}

/// Compress as gzip with the default level, as Java's `GZIPOutputStream` does.
pub fn gzip(bytes: &[u8]) -> Result<Vec<u8>, NbtError> {
    let mut enc = flate2::write::GzEncoder::new(Vec::with_capacity(bytes.len() / 2 + 32), flate2::Compression::default());
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

/// Compress as zlib with the fastest level (Chunker uses `Deflater.BEST_SPEED`).
pub fn zlib(bytes: &[u8]) -> Result<Vec<u8>, NbtError> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::with_capacity(bytes.len() / 2 + 32), flate2::Compression::fast());
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

/// Whether `bytes` starts like a zlib stream (CMF 0x_8, header checksum a multiple of 31).
fn looks_zlib(bytes: &[u8]) -> bool {
    matches!(bytes, [cmf, flg, ..] if cmf & 0x0F == 8 && cmf >> 4 <= 7 && (u16::from(*cmf) << 8 | u16::from(*flg)) % 31 == 0)
}

/// Decompress by looking at the first bytes: gzip (`1F 8B`) and zlib are unwrapped, bytes that start like an NBT
/// root (a tag id 1..=12) are returned unchanged, anything else is tried as raw deflate and, failing that, returned
/// unchanged.
pub fn decompress_auto(bytes: &[u8]) -> Result<Vec<u8>, NbtError> {
    match bytes {
        [0x1F, 0x8B, ..] => gunzip(bytes),
        // A Java root is `0A 00 xx`, which never passes the zlib header check; Bedrock `0A 00 00` neither.
        _ if looks_zlib(bytes) => unzlib(bytes),
        [id, ..] if (1..=12).contains(id) => Ok(bytes.to_vec()),
        [] => Ok(Vec::new()),
        _ => inflate_raw(bytes).or_else(|_| Ok(bytes.to_vec())),
    }
}
