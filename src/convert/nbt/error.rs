// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The error type of the NBT layer.

use std::fmt;

use super::tag::TagType;

/// Everything that can go wrong reading, writing or querying NBT.
#[derive(Debug)]
pub enum NbtError {
    /// An I/O error from the underlying stream (not an end of input).
    Io(std::io::Error),
    /// The input ended in the middle of a tag.
    UnexpectedEof,
    /// A tag id that is not 0..=12.
    UnknownTagId(u8),
    /// The root was the END tag (nothing to read).
    EmptyRoot,
    /// A length or count above one of Chunker's limits (or negative).
    LimitExceeded { what: &'static str, len: i64, max: usize },
    /// Compounds and lists nested deeper than [`super::io::MAX_DEPTH`].
    TooDeep,
    /// A string that is not valid (modified) UTF-8, or too long to write.
    BadString(String),
    /// A list whose elements are not all of the same type (cannot be written).
    MixedList { expected: TagType, found: TagType },
    /// A key that is not in the compound.
    Missing(String),
    /// A key that is present but of another type.
    WrongType { key: String, expected: &'static str, found: TagType },
    /// A compression stream that could not be decoded, or output beyond the allowed size.
    Compression(String),
    /// Anything else, with a message.
    Other(String),
}

impl fmt::Display for NbtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NbtError::Io(e) => write!(f, "NBT I/O error: {e}"),
            NbtError::UnexpectedEof => f.write_str("NBT data ended unexpectedly"),
            NbtError::UnknownTagId(id) => write!(f, "unknown NBT tag id {id}"),
            NbtError::EmptyRoot => f.write_str("NBT root is the END tag"),
            NbtError::LimitExceeded { what, len, max } => {
                write!(f, "NBT {what} of length {len} is outside the allowed range 0..={max}")
            }
            NbtError::TooDeep => f.write_str("NBT is nested too deeply"),
            NbtError::BadString(m) => write!(f, "invalid NBT string: {m}"),
            NbtError::MixedList { expected, found } => {
                write!(f, "NBT list of {} contains a {}", expected.name(), found.name())
            }
            NbtError::Missing(key) => write!(f, "Could not access {key}"),
            NbtError::WrongType { key, expected, found } => {
                write!(f, "Unexpected tag type {} for {key}, expected {expected}", found.name())
            }
            NbtError::Compression(m) => write!(f, "NBT compression error: {m}"),
            NbtError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for NbtError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            NbtError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for NbtError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof { NbtError::UnexpectedEof } else { NbtError::Io(e) }
    }
}

impl From<NbtError> for std::io::Error {
    fn from(e: NbtError) -> Self {
        match e {
            NbtError::Io(e) => e,
            NbtError::UnexpectedEof => std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "NBT data ended"),
            other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
        }
    }
}
