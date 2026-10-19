// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! NBT: the tag tree Minecraft stores almost everything in.
//!
//! Derived from Chunker's `nbt` package (https://github.com/HiveGamesOSS/Chunker, Copyright (c) 2024 Hive Games, MIT
//! License). [`Tag`] and [`Compound`] are the data model, [`io`] reads and writes the two binary flavors, [`file`]
//! wraps whole files (compression, `level.dat` headers) and `snbt` prints text for logs.

mod compound;
mod error;
pub mod file;
pub mod io;
pub mod mutf8;
mod snbt;
mod tag;

#[cfg(test)]
mod tests;

pub use compound::Compound;
#[allow(unused_imports)]
pub(crate) use compound::compound;
pub use error::NbtError;
pub use file::Compression;
pub use io::Flavor;
pub use tag::{FromTag, Tag, TagType};
