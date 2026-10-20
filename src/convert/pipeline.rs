// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The conversion itself: detect the input, read it column by column, write the other edition.
//!
//! Chunker drives this with a task tree and a column pre-transform handler that waits for neighbouring columns. Here
//! a pool of worker threads reads columns in region order (so the writer keeps few region files busy), runs each
//! column's pre-transform with its neighbours (read through a small shared cache), encodes the Java chunk and hands
//! the bytes to the single writer thread over a bounded channel, which bounds memory. Only the Bedrock to Java
//! direction exists (by decision); other requests say so plainly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::convert::bedrock::detect::detect_bedrock;
use crate::convert::bedrock::reader::BedrockWorldReader;
use crate::convert::java::dirs::detect_java;
use crate::convert::java::writer::{JavaWorldWriter, PreparedColumn};
use crate::convert::level::encoding::EncodingType;
use crate::convert::level::task::CancelToken;
use crate::convert::level::{ConvertError, ConvertRequest, ConvertSummary, Dimension, ProgressSink};
use crate::convert::model::{Column, Neighbours};
use crate::convert::util::Version;

/// Most threads that read and encode columns.
const MAX_WORKERS: usize = 8;
/// How many encoded columns may wait for the writer.
const QUEUE: usize = 48;
/// Neighbour columns kept for the pre-transforms before the cache starts over.
const NEIGHBOUR_CACHE: usize = 1536;

/// Convert the world of `req` and report to `sink`. Stops with [`ConvertError::Cancelled`] when `cancel` is set.
/// The input is never written to; a failed or cancelled run removes what it wrote to the output.
pub fn convert_world(
    req: &ConvertRequest,
    sink: &dyn ProgressSink,
    cancel: &CancelToken,
) -> Result<ConvertSummary, ConvertError> {
    let started = Instant::now();
    if req.target.edition != EncodingType::Java {
        return Err(ConvertError::Unsupported(
            "Converting to Bedrock Edition is not available yet; only Bedrock to Java works for now.".into(),
        ));
    }
    sink.stage("Reading the world");
    let Some(info) = detect_bedrock(&req.input) else {
        return Err(if detect_java(&req.input).is_some() {
            ConvertError::Unsupported(
                "This is a Java Edition world. Only Bedrock to Java conversion works for now.".into(),
            )
        } else {
            ConvertError::Malformed("This does not look like a Minecraft world.".into())
        });
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(ConvertError::Cancelled);
    }
    let reader = BedrockWorldReader::open(&info, &req.settings)?;
    let level = reader.level()?;
    let target = match req.target.version {
        Some(t) => Version::new(t.major() as i32, t.minor() as i32, t.patch() as i32),
        None => crate::convert::java::version::latest().1,
    };
    sink.stage("Preparing the new world");
    let mut writer = JavaWorldWriter::create(&req.output, target, &level, &req.settings)?;

    match run(&reader, &mut writer, sink, cancel) {
        Ok(columns) => {
            sink.stage("Finishing");
            let mut blocks: HashMap<String, u64> = reader.unmapped_blocks();
            for (name, n) in writer.unmapped_blocks() {
                *blocks.entry(name).or_default() += n;
            }
            let mut warnings = reader.warnings();
            let written = writer.finish_with_summary();
            let _ = &written;
            match written {
                Ok(_) => {
                    let mut unmapped: Vec<(String, u64)> = blocks.into_iter().collect();
                    unmapped.sort();
                    warnings.dedup();
                    sink.progress(1.0);
                    Ok(ConvertSummary { columns, blocks_unmapped: unmapped, warnings, elapsed: started.elapsed() })
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => {
            writer.abort();
            Err(e)
        }
    }
}

/// A small cache of columns read only to be looked at by a neighbour's pre-transform.
struct NeighbourCache {
    columns: Mutex<HashMap<(i32, i32), Option<Arc<Column>>>>,
}

impl NeighbourCache {
    fn get(
        &self,
        reader: &BedrockWorldReader,
        dim: &Dimension,
        x: i32,
        z: i32,
    ) -> Result<Option<Arc<Column>>, ConvertError> {
        if let Some(found) = self.columns.lock().unwrap_or_else(|e| e.into_inner()).get(&(x, z)) {
            return Ok(found.clone());
        }
        let read = reader.read_column(dim, x, z)?.map(Arc::new);
        let mut map = self.columns.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= NEIGHBOUR_CACHE {
            map.clear();
        }
        map.insert((x, z), read.clone());
        Ok(read)
    }
}

fn run(
    reader: &BedrockWorldReader,
    writer: &mut JavaWorldWriter,
    sink: &dyn ProgressSink,
    cancel: &CancelToken,
) -> Result<u64, ConvertError> {
    let dims: Vec<Dimension> = reader.dimensions().into_iter().filter(|d| writer.supports_dimension(d)).collect();
    let mut plan: Vec<(Dimension, Vec<(i32, i32)>)> = Vec::new();
    for dim in dims {
        let mut columns = reader.columns(&dim);
        // region by region, so the writer keeps few region files busy at a time
        columns.sort_by_key(|&(x, z)| (z >> 5, x >> 5, z, x));
        if !columns.is_empty() {
            plan.push((dim, columns));
        }
    }
    let total: usize = plan.iter().map(|(_, c)| c.len()).sum();
    if total == 0 {
        return Err(ConvertError::Malformed("The world has no chunks to convert.".into()));
    }
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, MAX_WORKERS);
    let mut done = 0usize;
    for (dim, columns) in &plan {
        sink.stage(&format!("Converting {}", dim_label(dim)));
        done += convert_dimension(reader, writer, dim, columns, workers, sink, cancel, done, total)?;
        writer.finish_dimension(dim)?;
    }
    Ok(done as u64)
}

fn dim_label(dim: &Dimension) -> String {
    let id = dim.identifier();
    id.strip_prefix("minecraft:").unwrap_or(id).replace('_', " ")
}

#[allow(clippy::too_many_arguments)]
fn convert_dimension(
    reader: &BedrockWorldReader,
    writer: &mut JavaWorldWriter,
    dim: &Dimension,
    columns: &[(i32, i32)],
    workers: usize,
    sink: &dyn ProgressSink,
    cancel: &CancelToken,
    already: usize,
    total: usize,
) -> Result<usize, ConvertError> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let cache = NeighbourCache { columns: Mutex::new(HashMap::new()) };
    let (tx, rx) = sync_channel::<Result<Option<PreparedColumn>, ConvertError>>(QUEUE);
    let mut written = 0usize;
    let mut failure: Option<ConvertError> = None;
    let shared = std::sync::RwLock::new(&mut *writer);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (next, stop, cache, shared) = (&next, &stop, &cache, &shared);
            scope.spawn(move || {
                loop {
                    if stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(x, z)) = columns.get(i) else { break };
                    let item = prepare(reader, shared, cache, dim, x, z);
                    let failed = item.is_err();
                    if tx.send(item).is_err() || failed {
                        break;
                    }
                }
            });
        }
        drop(tx);
        // the writer thread is this one: the writer is not shared
        let mut counted = 0usize;
        for item in rx {
            counted += 1;
            match item {
                Ok(Some(prepared)) => {
                    if failure.is_none() {
                        let result = shared.write().unwrap_or_else(|e| e.into_inner()).write_prepared(dim, prepared);
                        if let Err(e) = result {
                            failure = Some(e);
                            stop.store(true, Ordering::Relaxed);
                        } else {
                            written += 1;
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    if failure.is_none() {
                        failure = Some(e);
                    }
                    stop.store(true, Ordering::Relaxed);
                }
            }
            if counted % 16 == 0 {
                sink.progress((already + counted).min(total) as f64 / total as f64 * 0.98);
            }
        }
    });
    if let Some(e) = failure {
        return Err(e);
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(ConvertError::Cancelled);
    }
    Ok(written)
}

/// Read one column, run its pre-transform, encode it for Java.
fn prepare(
    reader: &BedrockWorldReader,
    writer: &std::sync::RwLock<&mut JavaWorldWriter>,
    cache: &NeighbourCache,
    dim: &Dimension,
    x: i32,
    z: i32,
) -> Result<Option<PreparedColumn>, ConvertError> {
    let Some(mut column) = reader.read_column(dim, x, z)? else { return Ok(None) };
    if column.has_pre_transform() {
        let mut held: Vec<(crate::convert::model::Edge, Arc<Column>)> = Vec::new();
        for edge in column.required_edges().iter() {
            let (dx, dz) = edge.offset();
            if let Some(neighbour) = cache.get(reader, dim, x + dx, z + dz)? {
                held.push((edge, neighbour));
            }
        }
        let mut neighbours = Neighbours::new();
        for (edge, c) in &held {
            neighbours.set(*edge, Some(c.as_ref()));
        }
        column.pre_transform(&neighbours);
    }
    let guard = writer.read().unwrap_or_else(|e| e.into_inner());
    guard.prepare_column(dim, &column)
}
