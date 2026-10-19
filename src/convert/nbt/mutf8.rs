// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Java's "modified UTF-8" (`DataInput.readUTF`): NUL as `C0 80`, supplementary characters as two 3-byte surrogates.

use super::error::NbtError;

/// Append `s` as modified UTF-8.
pub fn encode(s: &str, out: &mut Vec<u8>) {
    if s.bytes().all(|b| b != 0 && b < 0x80) {
        out.extend_from_slice(s.as_bytes());
        return;
    }
    let mut units = [0u16; 2];
    for ch in s.chars() {
        for &u in ch.encode_utf16(&mut units).iter() {
            match u {
                0x0001..=0x007F => out.push(u as u8),
                0x0000 | 0x0080..=0x07FF => {
                    out.push(0xC0 | (u >> 6) as u8);
                    out.push(0x80 | (u & 0x3F) as u8);
                }
                _ => {
                    out.push(0xE0 | (u >> 12) as u8);
                    out.push(0x80 | ((u >> 6) & 0x3F) as u8);
                    out.push(0x80 | (u & 0x3F) as u8);
                }
            }
        }
    }
}

/// The encoded length of `s` in bytes.
pub fn encoded_len(s: &str) -> usize {
    s.chars()
        .map(|c| match c as u32 {
            0 => 2,
            1..=0x7F => 1,
            0x80..=0x7FF => 2,
            0x800..=0xFFFF => 3,
            _ => 6,
        })
        .sum()
}

/// Decode modified UTF-8. Valid standard UTF-8 is accepted too (some writers emit 4-byte sequences). Lone surrogates
/// and malformed sequences are errors.
pub fn decode(bytes: &[u8]) -> Result<String, NbtError> {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(s.to_owned());
    }
    let bad = |m: &str| NbtError::BadString(m.to_owned());
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b >> 4 {
            0..=7 => {
                units.push(b as u16);
                i += 1;
            }
            12 | 13 => {
                let c2 = *bytes.get(i + 1).ok_or_else(|| bad("truncated 2-byte sequence"))?;
                if c2 & 0xC0 != 0x80 {
                    return Err(bad("malformed 2-byte sequence"));
                }
                units.push(((b as u16 & 0x1F) << 6) | (c2 as u16 & 0x3F));
                i += 2;
            }
            14 => {
                let (Some(&c2), Some(&c3)) = (bytes.get(i + 1), bytes.get(i + 2)) else {
                    return Err(bad("truncated 3-byte sequence"));
                };
                if c2 & 0xC0 != 0x80 || c3 & 0xC0 != 0x80 {
                    return Err(bad("malformed 3-byte sequence"));
                }
                units.push(((b as u16 & 0x0F) << 12) | ((c2 as u16 & 0x3F) << 6) | (c3 as u16 & 0x3F));
                i += 3;
            }
            _ => return Err(bad("invalid UTF-8 lead byte")),
        }
    }
    String::from_utf16(&units).map_err(|_| bad("unpaired surrogate"))
}

/// The number of UTF-16 units of `s`, which is what Java's `String.length()` counts.
pub fn utf16_len(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.encode_utf16().count() }
}
