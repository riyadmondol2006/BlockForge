// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The database thread of the world reader.
//!
//! `rusty-leveldb` handles are `!Send` and take `&mut self` for every read, so a [`LevelDb`](super::db::LevelDb) can
//! neither sit in a `Mutex` shared between threads nor be cloned. The reader therefore runs the database on ONE
//! thread that owns it for its whole life and serves requests over a channel: the workers send "give me the records
//! of this column" and get back plain byte vectors, then decode them (NBT, palettes, block resolution, which is
//! where the time goes) on their own thread in parallel. A request costs one channel round trip, a few microseconds,
//! and the thread never holds more than one column of raw bytes per request, so memory stays bounded.
//!
//! Chunker reads a column with point `get`s for every sub-chunk y from -64 to 63; here one prefix scan finds the
//! sub-chunk records in a single seek (keys sort bytewise, so y 0..63 come before -64..-1).

use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

use crate::convert::bedrock::db::BedrockDb;
use crate::convert::bedrock::keys::{ChunkTag, ColumnPos, DimensionId, chunk_key, enumerate_columns};
use crate::convert::bedrock::{BedrockError, Result};

/// A boxed database, created on the database thread.
pub(super) type DbOpener = Box<dyn FnOnce() -> Result<Box<dyn BedrockDb>> + Send>;

/// What to fetch for a column besides its sub-chunks.
#[derive(Clone, Copy, Debug)]
pub(super) struct Want {
    /// Read Data3D (and Data2D when there is none, or when `legacy_biomes` is set).
    pub biomes: bool,
    /// Read the `BlockEntity` record.
    pub block_entities: bool,
    /// Data3D may exist (1.17.30 and on); otherwise only Data2D is read.
    pub data3d: bool,
}

/// The raw records of one column.
#[derive(Clone, Debug, Default)]
pub(super) struct ColumnRecords {
    /// `(key y, value)` of every sub-chunk, in key byte order.
    pub sub_chunks: Vec<(i8, Vec<u8>)>,
    pub data3d: Option<Vec<u8>>,
    pub data2d: Option<Vec<u8>>,
    pub block_entities: Option<Vec<u8>>,
    /// The chunk `Version` byte record.
    pub version: Option<Vec<u8>>,
    /// The `FinalizedState` record.
    pub finalized: Option<Vec<u8>>,
}

impl ColumnRecords {
    /// Whether the column has no record that matters to the reader.
    pub fn is_empty(&self) -> bool {
        self.sub_chunks.is_empty()
            && self.data3d.is_none()
            && self.data2d.is_none()
            && self.block_entities.is_none()
    }
}

type Pairs = Vec<(Vec<u8>, Vec<u8>)>;

enum Request {
    Column { dim: DimensionId, x: i32, z: i32, want: Want, reply: Sender<Result<ColumnRecords>> },
    Get { key: Vec<u8>, reply: Sender<Result<Option<Vec<u8>>>> },
    Prefix { prefix: Vec<u8>, reply: Sender<Result<Pairs>> },
    Shutdown,
}

/// The owner of the database thread. It is `Sync`: any number of threads may call its methods at once.
pub(super) struct DbHandle {
    tx: Sender<Request>,
    thread: Option<JoinHandle<()>>,
}

fn gone() -> BedrockError {
    BedrockError::Db("the database thread is gone".into())
}

fn io_pending(tag: &str) -> BedrockError {
    BedrockError::Db(format!("the database thread did not answer a {tag} request"))
}

impl DbHandle {
    /// Start the thread, open the database on it and list the columns it holds (Chunker's scan with
    /// [`is_column_key`](super::keys::is_column_key)).
    pub fn start(opener: DbOpener) -> Result<(DbHandle, Vec<ColumnPos>)> {
        let (tx, rx) = channel::<Request>();
        let (ready_tx, ready_rx) = channel::<Result<Vec<ColumnPos>>>();
        let thread = std::thread::Builder::new()
            .name("bedrock-db".into())
            .spawn(move || {
                let mut db = match opener() {
                    Ok(db) => db,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let columns = enumerate_columns(db.as_mut());
                let failed = columns.is_err();
                let _ = ready_tx.send(columns);
                if failed {
                    return;
                }
                while let Ok(request) = rx.recv() {
                    match request {
                        Request::Column { dim, x, z, want, reply } => {
                            let _ = reply.send(read_column_records(db.as_mut(), dim, x, z, want));
                        }
                        Request::Get { key, reply } => {
                            let _ = reply.send(db.get(&key));
                        }
                        Request::Prefix { prefix, reply } => {
                            let _ = reply.send(db.scan_prefix(&prefix));
                        }
                        Request::Shutdown => break,
                    }
                }
                // Dropping the database here closes it and removes the scratch copy.
            })
            .map_err(|e| BedrockError::Io(format!("could not start the database thread: {e}")))?;
        match ready_rx.recv() {
            Ok(Ok(columns)) => Ok((DbHandle { tx, thread: Some(thread) }, columns)),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(gone()),
        }
    }

    /// The raw records of a column.
    pub fn column(&self, dim: DimensionId, x: i32, z: i32, want: Want) -> Result<ColumnRecords> {
        let (reply, answer) = channel();
        self.tx.send(Request::Column { dim, x, z, want, reply }).map_err(|_| gone())?;
        answer.recv().map_err(|_| io_pending("column"))?
    }

    /// One value by key.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let (reply, answer) = channel();
        self.tx.send(Request::Get { key: key.to_vec(), reply }).map_err(|_| gone())?;
        answer.recv().map_err(|_| io_pending("get"))?
    }

    /// All entries whose key starts with `prefix`.
    pub fn prefix(&self, prefix: &[u8]) -> Result<Pairs> {
        let (reply, answer) = channel();
        self.tx.send(Request::Prefix { prefix: prefix.to_vec(), reply }).map_err(|_| gone())?;
        answer.recv().map_err(|_| io_pending("scan"))?
    }
}

impl Drop for DbHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read the records of a column from a database (runs on the database thread; public to the module for tests).
pub(super) fn read_column_records(
    db: &mut dyn BedrockDb,
    dim: DimensionId,
    x: i32,
    z: i32,
    want: Want,
) -> Result<ColumnRecords> {
    let mut records = ColumnRecords::default();

    // Every sub-chunk of the column in one seek: the key is the 9/13 byte column key plus the y byte.
    let prefix = chunk_key(dim, x, z, ChunkTag::SubChunkPrefix);
    db.scan(&prefix, &mut |key, value| {
        if !key.starts_with(&prefix) {
            return false;
        }
        if key.len() == prefix.len() + 1 {
            records.sub_chunks.push((key[prefix.len()] as i8, value.to_vec()));
        }
        true
    })?;

    if want.biomes {
        if want.data3d {
            records.data3d = db.get(&chunk_key(dim, x, z, ChunkTag::Data3D))?;
        }
        if records.data3d.is_none() {
            records.data2d = db.get(&chunk_key(dim, x, z, ChunkTag::Data2D))?;
        }
    }
    if want.block_entities {
        records.block_entities = db.get(&chunk_key(dim, x, z, ChunkTag::BlockEntity))?;
    }
    records.version = db.get(&chunk_key(dim, x, z, ChunkTag::Version))?;
    records.finalized = db.get(&chunk_key(dim, x, z, ChunkTag::FinalizedState))?;
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::bedrock::db::MemDb;
    use crate::convert::bedrock::keys::sub_chunk_key;

    fn db() -> MemDb {
        let mut db = MemDb::new();
        for y in [-4i8, -1, 0, 3, 19] {
            db.put(&sub_chunk_key(DimensionId::OVERWORLD, 2, -3, y), &[y as u8, 1]).unwrap();
        }
        // Neighbours that must not be picked up.
        db.put(&sub_chunk_key(DimensionId::OVERWORLD, 2, -4, 0), &[9]).unwrap();
        db.put(&sub_chunk_key(DimensionId::NETHER, 2, -3, 0), &[8]).unwrap();
        db.put(&chunk_key(DimensionId::OVERWORLD, 2, -3, ChunkTag::Data3D), &[1, 2, 3]).unwrap();
        db.put(&chunk_key(DimensionId::OVERWORLD, 2, -3, ChunkTag::Data2D), &[4]).unwrap();
        db.put(&chunk_key(DimensionId::OVERWORLD, 2, -3, ChunkTag::BlockEntity), &[5]).unwrap();
        db.put(&chunk_key(DimensionId::OVERWORLD, 2, -3, ChunkTag::Version), &[40]).unwrap();
        db
    }

    #[test]
    fn column_records_are_found_by_prefix() {
        let mut db = db();
        let want = Want { biomes: true, block_entities: true, data3d: true };
        let r = read_column_records(&mut db, DimensionId::OVERWORLD, 2, -3, want).unwrap();
        let mut ys: Vec<i8> = r.sub_chunks.iter().map(|s| s.0).collect();
        ys.sort();
        assert_eq!(ys, [-4, -1, 0, 3, 19]);
        for (y, v) in &r.sub_chunks {
            assert_eq!(v[0], *y as u8);
        }
        assert_eq!(r.data3d, Some(vec![1, 2, 3]));
        assert!(r.data2d.is_none(), "Data2D is only read when there is no Data3D");
        assert_eq!(r.block_entities, Some(vec![5]));
        assert_eq!(r.version, Some(vec![40]));
        assert!(r.finalized.is_none());
        assert!(!r.is_empty());
    }

    #[test]
    fn wants_are_honoured() {
        let mut db = db();
        let none = Want { biomes: false, block_entities: false, data3d: true };
        let r = read_column_records(&mut db, DimensionId::OVERWORLD, 2, -3, none).unwrap();
        assert!(r.data3d.is_none() && r.data2d.is_none() && r.block_entities.is_none());
        let legacy = Want { biomes: true, block_entities: false, data3d: false };
        let r = read_column_records(&mut db, DimensionId::OVERWORLD, 2, -3, legacy).unwrap();
        assert!(r.data3d.is_none());
        assert_eq!(r.data2d, Some(vec![4]));
    }

    #[test]
    fn missing_columns_are_empty() {
        let mut db = db();
        let want = Want { biomes: true, block_entities: true, data3d: true };
        let r = read_column_records(&mut db, DimensionId::END, 2, -3, want).unwrap();
        assert!(r.is_empty());
        let r = read_column_records(&mut db, DimensionId::NETHER, 2, -3, want).unwrap();
        assert_eq!(r.sub_chunks.len(), 1);
    }

    #[test]
    fn handle_serves_many_threads_and_shuts_down() {
        let opener: DbOpener = Box::new(|| Ok(Box::new(db()) as Box<dyn BedrockDb>));
        let (handle, columns) = DbHandle::start(opener).unwrap();
        assert!(columns.iter().any(|c| c.x == 2 && c.z == -3));
        let want = Want { biomes: true, block_entities: true, data3d: true };
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..50 {
                        let r = handle.column(DimensionId::OVERWORLD, 2, -3, want).unwrap();
                        assert_eq!(r.sub_chunks.len(), 5);
                    }
                });
            }
        });
        assert_eq!(handle.get(&chunk_key(DimensionId::OVERWORLD, 2, -3, ChunkTag::Version)).unwrap(), Some(vec![40]));
        assert_eq!(handle.prefix(&chunk_key(DimensionId::OVERWORLD, 2, -3, ChunkTag::SubChunkPrefix)).unwrap().len(), 5);
        drop(handle);
    }

    #[test]
    fn open_failure_is_reported() {
        let opener: DbOpener = Box::new(|| Err(BedrockError::Db("nope".into())));
        let Err(e) = DbHandle::start(opener) else { panic!("expected an error") };
        assert!(e.to_string().contains("nope"));
    }
}
