// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! LEB128 variable-length integers as Bedrock uses them (unsigned, and zigzag for signed values).

use std::fmt;
use std::io::{self, Read};

/// A varint that is cut off or longer than the type allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarIntError {
    /// The input ended inside the number.
    Truncated,
    /// More continuation bytes than the type can hold.
    TooLong,
}

impl fmt::Display for VarIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarIntError::Truncated => f.write_str("varint is truncated"),
            VarIntError::TooLong => f.write_str("varint is too long"),
        }
    }
}

impl std::error::Error for VarIntError {}

/// Append `v` as an unsigned varint (1..=5 bytes).
pub fn write_u32(out: &mut Vec<u8>, v: u32) {
    write_u64(out, v as u64);
}

/// Append `v` as an unsigned varint (1..=10 bytes).
pub fn write_u64(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Zigzag mapping of a signed value.
pub fn zigzag32(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

/// Inverse of [`zigzag32`].
pub fn unzigzag32(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

/// Zigzag mapping of a signed 64-bit value.
pub fn zigzag64(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Inverse of [`zigzag64`].
pub fn unzigzag64(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// Append a signed value zigzag encoded.
pub fn write_i32(out: &mut Vec<u8>, v: i32) {
    write_u32(out, zigzag32(v));
}

/// Append a signed 64-bit value zigzag encoded.
pub fn write_i64(out: &mut Vec<u8>, v: i64) {
    write_u64(out, zigzag64(v));
}

fn read_with(mut next: impl FnMut() -> Result<u8, VarIntError>, max_bytes: usize) -> Result<u64, VarIntError> {
    let mut value = 0u64;
    for i in 0..max_bytes {
        let b = next()?;
        value |= ((b & 0x7F) as u64) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(VarIntError::TooLong)
}

/// Read an unsigned varint from the front of `data`: the value and the bytes used.
pub fn read_u32(data: &[u8]) -> Result<(u32, usize), VarIntError> {
    let mut used = 0;
    let v = read_with(
        || {
            let b = *data.get(used).ok_or(VarIntError::Truncated)?;
            used += 1;
            Ok(b)
        },
        5,
    )?;
    u32::try_from(v).map(|v| (v, used)).map_err(|_| VarIntError::TooLong)
}

/// Read an unsigned 64-bit varint from the front of `data`: the value and the bytes used.
pub fn read_u64(data: &[u8]) -> Result<(u64, usize), VarIntError> {
    let mut used = 0;
    let v = read_with(
        || {
            let b = *data.get(used).ok_or(VarIntError::Truncated)?;
            used += 1;
            Ok(b)
        },
        10,
    )?;
    Ok((v, used))
}

/// Read a zigzag signed varint from the front of `data`.
pub fn read_i32(data: &[u8]) -> Result<(i32, usize), VarIntError> {
    read_u32(data).map(|(v, n)| (unzigzag32(v), n))
}

/// Read a zigzag signed 64-bit varint from the front of `data`.
pub fn read_i64(data: &[u8]) -> Result<(i64, usize), VarIntError> {
    read_u64(data).map(|(v, n)| (unzigzag64(v), n))
}

fn read_stream(r: &mut impl Read, max_bytes: usize) -> io::Result<u64> {
    read_with(
        || {
            let mut b = [0u8; 1];
            r.read_exact(&mut b).map_err(|_| VarIntError::Truncated)?;
            Ok(b[0])
        },
        max_bytes,
    )
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Read an unsigned varint from a stream.
pub fn read_u32_from(r: &mut impl Read) -> io::Result<u32> {
    u32::try_from(read_stream(r, 5)?).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, VarIntError::TooLong))
}

/// Read an unsigned 64-bit varint from a stream.
pub fn read_u64_from(r: &mut impl Read) -> io::Result<u64> {
    read_stream(r, 10)
}

/// Read a zigzag signed varint from a stream.
pub fn read_i32_from(r: &mut impl Read) -> io::Result<i32> {
    read_u32_from(r).map(unzigzag32)
}

/// Read a zigzag signed 64-bit varint from a stream.
pub fn read_i64_from(r: &mut impl Read) -> io::Result<i64> {
    read_u64_from(r).map(unzigzag64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_encodings() {
        let mut v = Vec::new();
        write_u32(&mut v, 300);
        assert_eq!(v, [0xAC, 0x02]);
        v.clear();
        write_i32(&mut v, -1);
        assert_eq!(v, [1]);
        v.clear();
        write_u32(&mut v, u32::MAX);
        assert_eq!(v, [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    }

    #[test]
    fn round_trips() {
        for v in [0, 1, 127, 128, 16383, 16384, i32::MAX, i32::MIN, -1, -64, 64] {
            let mut b = Vec::new();
            write_i32(&mut b, v);
            assert_eq!(read_i32(&b).unwrap(), (v, b.len()));
            assert_eq!(read_i32_from(&mut b.as_slice()).unwrap(), v);
        }
        for v in [0u64, 1, u32::MAX as u64 + 1, u64::MAX] {
            let mut b = Vec::new();
            write_u64(&mut b, v);
            assert_eq!(read_u64(&b).unwrap(), (v, b.len()));
        }
        for v in [i64::MIN, i64::MAX, -1, 0] {
            let mut b = Vec::new();
            write_i64(&mut b, v);
            assert_eq!(read_i64(&b).unwrap().0, v);
        }
    }

    #[test]
    fn bad_input_is_an_error() {
        assert_eq!(read_u32(&[]), Err(VarIntError::Truncated));
        assert_eq!(read_u32(&[0x80]), Err(VarIntError::Truncated));
        assert_eq!(read_u32(&[0x80; 6]), Err(VarIntError::TooLong));
        // Five bytes that overflow 32 bits.
        assert_eq!(read_u32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F]), Err(VarIntError::TooLong));
        assert!(read_u32_from(&mut [0x80u8].as_slice()).is_err());
    }
}
