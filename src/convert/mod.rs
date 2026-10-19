//! World conversion between Minecraft editions and versions (Java <-> Bedrock).
//!
//! This is a Rust port of Chunker by Hive Games (https://github.com/HiveGamesOSS/Chunker, MIT License, Copyright (c)
//! 2024 Hive Games). The layers mirror Chunker's: `nbt` -> `model` (the version-neutral world) -> `java` and
//! `bedrock` (readers and writers) -> `mapping` (identifier tables). `PORTING.md` maps every Chunker package to its
//! destination here and tracks the status.

pub mod backend;
pub mod bedrock;
pub mod java;
pub mod level;
pub mod mapping;
pub mod model;
pub mod nbt;
pub mod util;
