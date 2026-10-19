// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The Bedrock key-value store: a [`BedrockDb`] trait with a `rusty-leveldb` implementation and an in-memory one.
//!
//! Bedrock worlds are LevelDB databases whose blocks are compressed with raw deflate (id 4), zlib (id 2), snappy (1)
//! or not at all (0). `rusty-leveldb` has the last two; this module adds the first two with `flate2`.
//!
//! `rusty-leveldb` opens every database read-write (it replays the log and may write a new manifest), so a world is
//! never opened in place: [`LevelDb::open_read`] copies `db/` to a scratch directory first and deletes it on drop.

use super::{BedrockError, Result};
use rusty_leveldb::{
    Compressor, CompressorId, CompressorList, DB, LdbIterator, Options, Status, StatusCode, WriteBatch,
};
use std::collections::BTreeMap;
use std::ops::Bound;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Compressor id of raw deflate (Mojang's `ZlibCompressorRaw`), what Bedrock writes.
pub const COMPRESSOR_RAW_DEFLATE: u8 = 4;
/// Compressor id of zlib (with header), found in some older worlds.
pub const COMPRESSOR_ZLIB: u8 = 2;
/// Largest block a decompressor will produce (a LevelDB block is far smaller; this stops decompression bombs).
const MAX_BLOCK_OUTPUT: u64 = 64 * 1024 * 1024;

fn compression_error(msg: String) -> Status {
    Status::new(StatusCode::CompressionError, &msg)
}

fn inflate(mut reader: impl Read) -> rusty_leveldb::Result<Vec<u8>> {
    let mut out = Vec::new();
    // One byte over the cap tells "exactly the cap" from "more than the cap".
    let read = reader.by_ref().take(MAX_BLOCK_OUTPUT + 1).read_to_end(&mut out);
    match read {
        Err(e) => Err(compression_error(format!("inflate failed: {e}"))),
        Ok(_) if out.len() as u64 > MAX_BLOCK_OUTPUT => Err(compression_error("block inflates to over 64 MB".into())),
        Ok(_) => Ok(out),
    }
}

/// Raw deflate, no header (compressor id 4).
#[derive(Clone, Copy, Debug, Default)]
pub struct RawDeflate;

impl CompressorId for RawDeflate {
    const ID: u8 = COMPRESSOR_RAW_DEFLATE;
}

impl Compressor for RawDeflate {
    fn encode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
        e.write_all(&block).map_err(|e| compression_error(e.to_string()))?;
        e.finish().map_err(|e| compression_error(e.to_string()))
    }

    fn decode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        inflate(flate2::read::DeflateDecoder::new(&block[..]))
    }
}

/// Zlib with header (compressor id 2).
#[derive(Clone, Copy, Debug, Default)]
pub struct Zlib;

impl CompressorId for Zlib {
    const ID: u8 = COMPRESSOR_ZLIB;
}

impl Compressor for Zlib {
    fn encode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        e.write_all(&block).map_err(|e| compression_error(e.to_string()))?;
        e.finish().map_err(|e| compression_error(e.to_string()))
    }

    fn decode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        inflate(flate2::read::ZlibDecoder::new(&block[..]))
    }
}

/// The compressors a Bedrock database can contain: none (0), snappy (1), zlib (2) and raw deflate (4).
pub fn compressor_list() -> CompressorList {
    let mut list = CompressorList::default(); // 0 and 1
    list.set(Zlib);
    list.set(RawDeflate);
    list
}

/// One operation of a [`Batch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchOp {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
}

/// An ordered set of writes applied together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    pub ops: Vec<BatchOp>,
}

impl Batch {
    pub fn new() -> Batch {
        Batch::default()
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) {
        self.ops.push(BatchOp::Put(key.to_vec(), value.to_vec()));
    }

    pub fn delete(&mut self, key: &[u8]) {
        self.ops.push(BatchOp::Delete(key.to_vec()));
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// The operations the converter needs from a world database.
///
/// Everything takes `&mut self` (the LevelDB handle is single-threaded) and returns [`BedrockError`], never panics.
pub trait BedrockDb {
    /// The value of a key, `None` if it is absent (or deleted).
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Visit the live entries with key >= `start` in key order until `visit` returns false.
    fn scan(&mut self, start: &[u8], visit: &mut dyn FnMut(&[u8], &[u8]) -> bool) -> Result<()>;

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()>;

    fn delete(&mut self, key: &[u8]) -> Result<()>;

    /// Apply a batch of writes.
    fn write_batch(&mut self, batch: &Batch) -> Result<()>;

    /// Make pending writes durable (the log).
    fn flush(&mut self) -> Result<()>;

    /// Write everything out as table files and compact them (what Chunker does when it finishes a world).
    fn compact(&mut self) -> Result<()>;

    /// All live keys in order, each once.
    fn iter_keys(&mut self) -> Result<Vec<Vec<u8>>> {
        let mut keys = Vec::new();
        self.scan(&[], &mut |k, _| {
            keys.push(k.to_vec());
            true
        })?;
        Ok(keys)
    }

    /// All live entries whose key starts with `prefix`.
    fn scan_prefix(&mut self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut out = Vec::new();
        self.scan(prefix, &mut |k, v| {
            if !k.starts_with(prefix) {
                return false;
            }
            out.push((k.to_vec(), v.to_vec()));
            true
        })?;
        Ok(out)
    }
}

/// An in-memory [`BedrockDb`] for tests of the layers above.
#[derive(Clone, Debug, Default)]
pub struct MemDb {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl MemDb {
    pub fn new() -> MemDb {
        MemDb::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl BedrockDb for MemDb {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.map.get(key).cloned())
    }

    fn scan(&mut self, start: &[u8], visit: &mut dyn FnMut(&[u8], &[u8]) -> bool) -> Result<()> {
        for (k, v) in self.map.range::<[u8], _>((Bound::Included(start), Bound::Unbounded)) {
            if !visit(k, v) {
                break;
            }
        }
        Ok(())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.map.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<()> {
        self.map.remove(key);
        Ok(())
    }

    fn write_batch(&mut self, batch: &Batch) -> Result<()> {
        for op in &batch.ops {
            match op {
                BatchOp::Put(k, v) => {
                    self.map.insert(k.clone(), v.clone());
                }
                BatchOp::Delete(k) => {
                    self.map.remove(k);
                }
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn compact(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A scratch directory under the system temp dir that is removed when dropped.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

impl ScratchDir {
    /// Create a fresh directory `<temp>/blockforge-<label>-<pid>-<n>-<nanos>`.
    pub fn new(label: &str) -> Result<ScratchDir> {
        Self::new_in(&std::env::temp_dir(), label)
    }

    /// Create a fresh scratch directory inside `parent`.
    pub fn new_in(parent: &Path, label: &str) -> Result<ScratchDir> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("blockforge-{label}-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(ScratchDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Settings of a [`LevelDb`].
#[derive(Clone, Debug)]
pub struct DbOptions {
    /// Compressor id used for blocks written (4 = raw deflate like Bedrock, 0 = none).
    pub compressor: u8,
    /// Bytes buffered in memory before they are written as a table file; also bounds the write-ahead log.
    pub write_buffer_size: usize,
    /// Table block size (Chunker uses 160 KB).
    pub block_size: usize,
}

impl Default for DbOptions {
    fn default() -> Self {
        DbOptions { compressor: COMPRESSOR_RAW_DEFLATE, write_buffer_size: 16 * 1024 * 1024, block_size: 160 * 1024 }
    }
}

impl DbOptions {
    fn to_options(&self, create: bool) -> Options {
        let mut o = Options::default();
        o.create_if_missing = create;
        o.compressor_list = Rc::new(compressor_list());
        o.compressor = self.compressor;
        o.write_buffer_size = self.write_buffer_size;
        o.block_size = self.block_size;
        o.block_cache_capacity_bytes = 8 * 1024 * 1024;
        o
    }
}

/// A Bedrock LevelDB on disk (single-threaded: the handle is `!Send`).
pub struct LevelDb {
    // Declared first so the database is closed before the scratch copy is removed.
    db: Option<DB>,
    read_only: bool,
    _scratch: Option<ScratchDir>,
}

impl LevelDb {
    /// Open a world's `db/` folder for reading. The folder is copied to a scratch directory first; the original is
    /// never touched. Missing `LOCK` and log files are fine.
    pub fn open_read(db_dir: &Path) -> Result<LevelDb> {
        Self::open_read_with(db_dir, &DbOptions::default(), None)
    }

    /// [`open_read`](Self::open_read) with explicit options and an optional parent for the scratch copy.
    pub fn open_read_with(db_dir: &Path, options: &DbOptions, scratch_parent: Option<&Path>) -> Result<LevelDb> {
        if !db_dir.join("CURRENT").is_file() {
            return Err(BedrockError::Db(format!("{} is not a LevelDB folder (no CURRENT file)", db_dir.display())));
        }
        let scratch = match scratch_parent {
            Some(p) => ScratchDir::new_in(p, "leveldb")?,
            None => ScratchDir::new("leveldb")?,
        };
        let target = scratch.path().join("db");
        std::fs::create_dir_all(&target)?;
        for entry in std::fs::read_dir(db_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            // The lock is a process artefact, and only plain files belong to a database.
            if name == "LOCK" || !entry.file_type()?.is_file() {
                continue;
            }
            std::fs::copy(entry.path(), target.join(&name))?;
        }
        let mut opts = options.to_options(false);
        opts.reuse_logs = false;
        let db = DB::open(&target, opts)?;
        Ok(LevelDb { db: Some(db), read_only: true, _scratch: Some(scratch) })
    }

    /// Create a new, empty database at `db_dir` (the folder must not hold a database yet).
    pub fn create_new(db_dir: &Path, options: &DbOptions) -> Result<LevelDb> {
        if db_dir.join("CURRENT").exists() {
            return Err(BedrockError::Db(format!("{} already holds a database", db_dir.display())));
        }
        std::fs::create_dir_all(db_dir)?;
        let db = DB::open(db_dir, options.to_options(true))?;
        Ok(LevelDb { db: Some(db), read_only: false, _scratch: None })
    }

    /// Open an existing database in place for reading and writing (used to reopen what [`create_new`] wrote).
    ///
    /// [`create_new`]: Self::create_new
    pub fn open_write(db_dir: &Path, options: &DbOptions) -> Result<LevelDb> {
        let db = DB::open(db_dir, options.to_options(false))?;
        Ok(LevelDb { db: Some(db), read_only: false, _scratch: None })
    }

    fn handle(&mut self) -> Result<&mut DB> {
        self.db.as_mut().ok_or_else(|| BedrockError::Db("the database is closed".into()))
    }

    fn writable(&mut self) -> Result<&mut DB> {
        if self.read_only {
            return Err(BedrockError::Db("the database was opened read-only".into()));
        }
        self.handle()
    }

    /// Flush, release the lock and close the database. Further calls fail with an error.
    pub fn close(&mut self) -> Result<()> {
        match self.db.take() {
            Some(mut db) => db.close().map_err(Into::into),
            None => Ok(()),
        }
    }
}

impl Drop for LevelDb {
    fn drop(&mut self) {
        if !self.read_only {
            if let Some(db) = self.db.as_mut() {
                let _ = db.close();
            }
        }
        self.db = None;
    }
}

impl BedrockDb for LevelDb {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let db = self.handle()?;
        let snapshot = db.get_snapshot();
        Ok(db.get_at(&snapshot, key)?.map(|b| b.to_vec()))
    }

    fn scan(&mut self, start: &[u8], visit: &mut dyn FnMut(&[u8], &[u8]) -> bool) -> Result<()> {
        let mut it = self.handle()?.new_iter()?;
        if start.is_empty() {
            it.seek_to_first();
        } else {
            it.seek(start);
        }
        while let Some((k, v)) = it.current() {
            if !visit(&k, &v) || !it.advance() {
                break;
            }
        }
        Ok(())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.writable()?.put(key, value).map_err(Into::into)
    }

    fn delete(&mut self, key: &[u8]) -> Result<()> {
        self.writable()?.delete(key).map_err(Into::into)
    }

    fn write_batch(&mut self, batch: &Batch) -> Result<()> {
        let mut wb = WriteBatch::default();
        for op in &batch.ops {
            match op {
                BatchOp::Put(k, v) => wb.put(k, v),
                BatchOp::Delete(k) => wb.delete(k),
            }
        }
        self.writable()?.write(wb, false).map_err(Into::into)
    }

    fn flush(&mut self) -> Result<()> {
        self.writable()?.flush().map_err(Into::into)
    }

    fn compact(&mut self) -> Result<()> {
        let db = self.writable()?;
        db.compact_range(&[], &[0xFF; 64])?;
        db.flush().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_dir(label: &str) -> ScratchDir {
        // Keep large scratch data out of /tmp when the caller points TMPDIR elsewhere.
        ScratchDir::new(label).expect("scratch dir")
    }

    #[test]
    fn compressors_round_trip() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        for c in [&RawDeflate as &dyn Compressor, &Zlib] {
            let enc = c.encode(data.clone()).unwrap();
            assert!(enc.len() < data.len());
            assert_eq!(c.decode(enc).unwrap(), data);
        }
        // The two are not interchangeable.
        let raw = RawDeflate.encode(data.clone()).unwrap();
        assert!(Zlib.decode(raw).is_err());
        assert_eq!(RawDeflate.decode(RawDeflate.encode(Vec::new()).unwrap()).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn truncated_or_garbage_input_is_an_error() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 255) as u8).collect();
        for c in [&RawDeflate as &dyn Compressor, &Zlib] {
            let enc = c.encode(data.clone()).unwrap();
            for cut in [1, enc.len() / 2, enc.len() - 1] {
                assert!(c.decode(enc[..cut].to_vec()).is_err(), "cut {cut}");
            }
        }
        assert!(RawDeflate.decode(vec![0xFF; 32]).is_err());
        assert!(Zlib.decode(vec![]).is_err());
    }

    #[test]
    fn compressor_list_has_the_bedrock_ids() {
        let l = compressor_list();
        for id in [0u8, 1, 2, 4] {
            assert!(l.is_set(id), "{id}");
        }
        assert!(!l.is_set(3));
    }

    #[test]
    fn mem_db_behaviour() {
        let mut db = MemDb::new();
        assert!(db.is_empty());
        db.put(b"b", b"2").unwrap();
        db.put(b"a", b"1").unwrap();
        db.put(b"ab", b"3").unwrap();
        db.put(b"b", b"22").unwrap();
        assert_eq!(db.get(b"b").unwrap().as_deref(), Some(&b"22"[..]));
        assert_eq!(db.get(b"zz").unwrap(), None);
        assert_eq!(db.iter_keys().unwrap(), vec![b"a".to_vec(), b"ab".to_vec(), b"b".to_vec()]);
        let p = db.scan_prefix(b"a").unwrap();
        assert_eq!(p, vec![(b"a".to_vec(), b"1".to_vec()), (b"ab".to_vec(), b"3".to_vec())]);
        db.delete(b"a").unwrap();
        db.delete(b"never-there").unwrap();
        let mut batch = Batch::new();
        batch.put(b"c", b"x");
        batch.delete(b"b");
        batch.put(b"d", b"y");
        assert_eq!(batch.len(), 3);
        db.write_batch(&batch).unwrap();
        assert_eq!(db.iter_keys().unwrap(), vec![b"ab".to_vec(), b"c".to_vec(), b"d".to_vec()]);
        let mut seen = 0;
        db.scan(b"c", &mut |_, _| {
            seen += 1;
            false
        })
        .unwrap();
        assert_eq!(seen, 1);
        db.flush().unwrap();
        db.compact().unwrap();
    }

    #[test]
    fn write_then_read_2000_keys() {
        let dir = work_dir("dbtest");
        let path = dir.path().join("db");
        let opts = DbOptions { write_buffer_size: 256 * 1024, ..DbOptions::default() };
        {
            let mut db = LevelDb::create_new(&path, &opts).unwrap();
            for i in 0..2000u32 {
                let key = format!("key-{i:05}");
                let value = format!("value-{i}-{}", "x".repeat((i % 200) as usize));
                db.put(key.as_bytes(), value.as_bytes()).unwrap();
            }
            // A batch with an overwrite and a delete.
            let mut b = Batch::new();
            b.put(b"key-00000", b"replaced");
            b.delete(b"key-00001");
            db.write_batch(&b).unwrap();
            db.compact().unwrap();
        }
        assert!(path.join("CURRENT").is_file());
        // Reopen in place for writing, then read-only through a scratch copy.
        {
            let mut db = LevelDb::open_write(&path, &opts).unwrap();
            assert!(db.get(b"key-01999").unwrap().is_some());
        }
        let mut db = LevelDb::open_read(&path).unwrap();
        assert_eq!(db.get(b"key-00000").unwrap().as_deref(), Some(&b"replaced"[..]));
        assert_eq!(db.get(b"key-00001").unwrap(), None);
        for i in 2..2000u32 {
            let key = format!("key-{i:05}");
            let value = format!("value-{i}-{}", "x".repeat((i % 200) as usize));
            assert_eq!(db.get(key.as_bytes()).unwrap().as_deref(), Some(value.as_bytes()), "{key}");
        }
        let keys = db.iter_keys().unwrap();
        assert_eq!(keys.len(), 1999);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(db.scan_prefix(b"key-0001").unwrap().len(), 10);
        // Writes through a read handle are refused.
        assert!(db.put(b"k", b"v").is_err());
        // The tables are written with raw deflate: the index block's trailer byte is the compressor id.
        let mut tables = 0;
        for e in std::fs::read_dir(&path).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ldb") {
                let bytes = std::fs::read(&p).unwrap();
                let footer = &bytes[bytes.len() - 48..];
                let mut pos = 0;
                let mut varint = || {
                    let (mut v, mut shift) = (0u64, 0);
                    loop {
                        let b = footer[pos];
                        pos += 1;
                        v |= u64::from(b & 0x7F) << shift;
                        shift += 7;
                        if b & 0x80 == 0 {
                            return v as usize;
                        }
                    }
                };
                let (_meta_off, _meta_size, index_off, index_size) = (varint(), varint(), varint(), varint());
                assert_eq!(bytes[index_off + index_size], COMPRESSOR_RAW_DEFLATE);
                tables += 1;
            }
        }
        assert!(tables > 0, "compaction should have produced a table file");
    }

    #[test]
    fn open_read_does_not_touch_the_original() {
        let dir = work_dir("dbro");
        let path = dir.path().join("db");
        {
            let mut db = LevelDb::create_new(&path, &DbOptions::default()).unwrap();
            db.put(b"a", b"1").unwrap();
            db.compact().unwrap();
        }
        let snapshot = |p: &Path| {
            let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(p)
                .unwrap()
                .map(|e| e.unwrap())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()))
                .collect();
            v.sort();
            v
        };
        let before = snapshot(&path);
        {
            let mut db = LevelDb::open_read(&path).unwrap();
            assert_eq!(db.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
        }
        assert_eq!(snapshot(&path), before);
        assert!(LevelDb::open_read(&dir.path().join("missing")).is_err());
        assert!(LevelDb::create_new(&path, &DbOptions::default()).is_err());
    }

    #[test]
    fn scratch_dir_is_removed_on_drop() {
        let p = {
            let s = ScratchDir::new("droptest").unwrap();
            std::fs::write(s.path().join("f"), b"x").unwrap();
            assert!(s.path().is_dir());
            s.path().to_path_buf()
        };
        assert!(!p.exists());
    }
}
