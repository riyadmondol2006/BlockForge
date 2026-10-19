// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Helpers shared by the tests of this module.

use crate::convert::nbt::{Compound, Tag};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory removed when dropped.
pub struct TestDir(pub PathBuf);

impl TestDir {
    pub fn new(name: &str) -> TestDir {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bf-java-{}-{}-{}", std::process::id(), n, name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        TestDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// xorshift64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A small chunk-like compound that depends on (x, z).
pub fn sample_chunk(x: i32, z: i32) -> Compound {
    let mut c = Compound::new();
    c.insert("DataVersion", Tag::Int(3465));
    c.insert("xPos", Tag::Int(x));
    c.insert("zPos", Tag::Int(z));
    c.insert("Status", Tag::String("minecraft:full".into()));
    c.insert("LongArray", Tag::LongArray((0..64).map(|i| (x as i64) * 1000 + z as i64 + i).collect()));
    c
}
