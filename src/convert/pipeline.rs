// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The conversion itself: detect the input, read it column by column, write the other edition.
//!
//! Chunker drives this with a task tree and a column pre-transform handler that waits for neighbouring columns. Here
//! a pool of worker threads reads columns in region order (so the writer keeps few region files busy), runs each
//! column's pre-transform with its neighbours (read through a small shared cache), encodes the Java chunk and hands
//! the bytes to the single writer thread over a bounded channel, which bounds memory. Only the Bedrock to Java
//! direction exists (by decision); other requests say so plainly.
//!
//! Before the columns, the level is made ready: the level setting overrides are applied
//! ([`ConversionSettings::apply_to_level_settings`]) and the players are chosen and their items converted
//! (`pipeline_players`). Map links need nothing here: the reader gives items the index of their map and the writer's
//! item hooks ([`JavaWorldWriter::item_hooks`]) turn it into the map's Java id wherever an item is written.
//!
//! The entities of a dimension are read before its columns (`BedrockWorldReader::entities`): an entity goes to the
//! column it stands in, which is not always the one Bedrock stored it with.

#[path = "pipeline_players.rs"]
mod players;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::convert::bedrock::detect::detect_bedrock;
use crate::convert::bedrock::reader::BedrockWorldReader;
use crate::convert::bedrock::reader::ColumnEntities;
use crate::convert::java::dirs::detect_java;
use crate::convert::java::writer::{JavaWorldWriter, PreparedColumn};
use crate::convert::level::encoding::EncodingType;
use crate::convert::level::task::CancelToken;
use crate::convert::level::{
    ConversionSettings, ConvertError, ConvertRequest, ConvertSummary, Dimension, ProgressSink,
};
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
    convert_opened(&reader, req, sink, cancel, started)
}

/// [`convert_world`] once the input is open: the level, the players, every column, the level files.
fn convert_opened(
    reader: &BedrockWorldReader,
    req: &ConvertRequest,
    sink: &dyn ProgressSink,
    cancel: &CancelToken,
    started: Instant,
) -> Result<ConvertSummary, ConvertError> {
    let mut level = reader.level()?;
    level.settings = ConversionSettings::apply_to_level_settings(&req.settings, &level.settings)
        .map_err(|e| ConvertError::Unsupported(format!("the changed level settings do not fit: {e}")))?;
    let players = players::prepare(reader, &mut level, &req.settings)?;
    let target = match req.target.version {
        Some(t) => Version::new(t.major() as i32, t.minor() as i32, t.patch() as i32),
        None => crate::convert::java::version::latest().1,
    };
    sink.stage("Preparing the new world");
    let mut writer = JavaWorldWriter::create(&req.output, target, &level, &req.settings)?;
    writer.set_player_items(players.items);
    writer.set_ender_items(players.ender_items);
    for guest in players.guests {
        writer.add_guest_player(guest);
    }
    writer.set_entity_source(reader.entity_source());

    match run(reader, &mut writer, sink, cancel) {
        Ok(columns) => {
            sink.stage("Finishing");
            let mut blocks: HashMap<String, u64> = reader.unmapped_blocks();
            for (name, n) in writer.unmapped_blocks() {
                *blocks.entry(name).or_default() += n;
            }
            let mut warnings = reader.warnings();
            warnings.extend(players.warnings);
            let mut unmapped_entities: Vec<(String, u64)> = reader.unmapped_entities().into_iter().collect();
            if !unmapped_entities.is_empty() {
                unmapped_entities.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                let list: Vec<String> =
                    unmapped_entities.iter().take(10).map(|(id, n)| format!("{id} ({n})")).collect();
                warnings.push(format!("Entities with no Java Edition equivalent were left out: {}", list.join(", ")));
            }
            let written = writer.finish_with_summary();
            let _ = &written;
            match written {
                Ok(written) => {
                    if written.dropped_entities > 0 {
                        warnings.push(format!(
                            "{} entities could not be written for this Java version (it has no such entity).",
                            written.dropped_entities
                        ));
                    }
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
    let entities = Mutex::new(reader.entities(dim)?);
    let (tx, rx) = sync_channel::<Result<Option<PreparedColumn>, ConvertError>>(QUEUE);
    let mut written = 0usize;
    let mut failure: Option<ConvertError> = None;
    let shared = std::sync::RwLock::new(&mut *writer);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (next, stop, cache, shared, entities) = (&next, &stop, &cache, &shared, &entities);
            scope.spawn(move || {
                loop {
                    if stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(x, z)) = columns.get(i) else { break };
                    let item = prepare(reader, shared, cache, entities, dim, x, z);
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
    entities: &Mutex<ColumnEntities>,
    dim: &Dimension,
    x: i32,
    z: i32,
) -> Result<Option<PreparedColumn>, ConvertError> {
    let Some(mut column) = reader.read_column(dim, x, z)? else { return Ok(None) };
    if let Some(list) = entities.lock().unwrap_or_else(|e| e.into_inner()).remove(&(x, z)) {
        column.entities.extend(list);
    }
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

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;

#[cfg(test)]
mod entity_tests {
    use super::*;
    use crate::convert::bedrock::db::{BedrockDb, DbOptions, LevelDb, ScratchDir};
    use crate::convert::bedrock::keys::{self, ChunkTag, DimensionId};
    use crate::convert::bedrock::level_dat::LevelDat;
    use crate::convert::bedrock::palette::{
        BlockStateRaw, RawPalette, RawSubChunk, encode_nbt_concat, encode_sub_chunk,
    };
    use crate::convert::java::region::RegionFile;
    use crate::convert::level::{NullSink, TargetVersion, VersionTriple};
    use crate::convert::nbt::{Compound, Tag};

    fn actor(identifier: &str, unique_id: i64, pos: [f32; 3]) -> Compound {
        Compound::new()
            .with("identifier", identifier)
            .with("UniqueID", unique_id)
            .with("Pos", Tag::List(pos.iter().map(|v| Tag::Float(*v)).collect()))
            .with("Rotation", Tag::List(vec![Tag::Float(0.0), Tag::Float(0.0)]))
    }

    /// A Bedrock world with stone in columns 0,0 and 1,0, a cow in the legacy store (standing in column 1,0), a cat
    /// of the host in the digp store and an item frame block with its block entity.
    fn make_world(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        let version = Tag::List([1, 21, 50, 0, 0].into_iter().map(Tag::Int).collect());
        let root = Compound::new().with("LevelName", "Entities").with("lastOpenedWithVersion", version);
        LevelDat { storage_version: 10, root_name: String::new(), root }.write(&dir.join("level.dat")).unwrap();
        let mut db = LevelDb::create_new(&dir.join("db"), &DbOptions::default()).unwrap();
        let ow = DimensionId::OVERWORLD;
        let stone = BlockStateRaw::new("minecraft:stone", Compound::new(), 18168865);
        for x in [0, 1] {
            let sub = RawSubChunk { version: 9, y: 0, layers: vec![RawPalette::uniform(stone.clone())] };
            db.put(&keys::sub_chunk_key(ow, x, 0, 0), &encode_sub_chunk(&sub).unwrap()).unwrap();
        }
        let frame = BlockStateRaw::new("minecraft:frame", Compound::new().with("facing_direction", 1), 18168865);
        let sub = RawSubChunk { version: 9, y: 4, layers: vec![RawPalette::uniform(frame)] };
        db.put(&keys::sub_chunk_key(ow, 0, 0, 4), &encode_sub_chunk(&sub).unwrap()).unwrap();
        let item = Compound::new().with("Name", "minecraft:diamond").with("Count", 1i8).with("Damage", 0i16);
        let be = Compound::new().with("id", "ItemFrame").with("x", 3).with("y", 64).with("z", 3).with("Item", item);
        db.put(&keys::chunk_key(ow, 0, 0, ChunkTag::BlockEntity), &encode_nbt_concat(&[Tag::Compound(be)]).unwrap())
            .unwrap();
        let cow = actor("minecraft:cow", 7, [20.0, 64.0, 4.0]);
        db.put(&keys::chunk_key(ow, 0, 0, ChunkTag::Entity), &encode_nbt_concat(&[Tag::Compound(cow)]).unwrap())
            .unwrap();
        let mut cat = actor("minecraft:cat", 8, [5.0, 64.0, 5.0]);
        cat.put("OwnerNew", crate::convert::bedrock::resolvers::entity::HOST_UNIQUE_ID);
        cat.put("IsTamed", 1i8);
        cat.put("Variant", 5);
        let key = keys::actor_storage_key(8);
        db.put(&keys::actor_key(key), &encode_nbt_concat(&[Tag::Compound(cat)]).unwrap()).unwrap();
        db.put(&keys::digp_key(ow, 0, 0), &key).unwrap();
        db.compact().unwrap();
    }

    fn entity_ids(chunk: &Compound) -> Vec<String> {
        let list = chunk.get("Entities").and_then(Tag::as_list).cloned().unwrap_or_default();
        let mut ids: Vec<String> =
            list.iter().filter_map(|e| e.as_compound()?.get_str("id").map(str::to_owned)).collect();
        ids.sort();
        ids
    }

    #[test]
    fn a_world_keeps_its_entities() {
        let tmp = ScratchDir::new("pipeline-entities").unwrap();
        let world = tmp.path().join("world");
        make_world(&world);
        for (version, separate) in [("26.3", true), ("1.16.5", false)] {
            let out = tmp.path().join(version);
            let owner = uuid::Uuid::from_u128(0x1234);
            let request = ConvertRequest {
                input: world.clone(),
                output: out.clone(),
                target: TargetVersion::new(EncodingType::Java, VersionTriple::from_string(version).unwrap()),
                settings: crate::convert::level::ConversionSettings::builder().pet_owner_uuid(Some(owner)).build(),
            };
            let cancel = CancelToken::default();
            convert_world(&request, &NullSink, &cancel).unwrap();
            let read = |dir: &str, x: u8| {
                let path = out.join(dir).join("r.0.0.mca");
                RegionFile::open_read(&path).unwrap().read_chunk(x, 0).unwrap()
            };
            if separate {
                let dir = "dimensions/minecraft/overworld/entities";
                let first = read(dir, 0).expect("entity chunk 0,0");
                assert_eq!(entity_ids(&first), ["minecraft:cat", "minecraft:item_frame"]);
                let cat = first.get("Entities").and_then(Tag::as_list).unwrap().iter().filter_map(Tag::as_compound);
                let cat = cat.clone().find(|e| e.get_str("id") == Some("minecraft:cat")).unwrap();
                let ints = crate::convert::java::resolvers::entity::uuid_of_unique_id(8);
                assert!(cat.contains_key("UUID") && ints.get_version_num() == 3);
                let owner_ints: Vec<i32> =
                    (0..4).map(|i| ((0x1234u128 >> (96 - 32 * i)) & 0xffff_ffff) as i32).collect();
                assert_eq!(cat.get("Owner").and_then(Tag::as_int_array), Some(&owner_ints));
                // The cow stands in the next column.
                assert_eq!(entity_ids(&read(dir, 1).expect("entity chunk 1,0")), ["minecraft:cow"]);
            } else {
                let root = read("region", 0).unwrap();
                let level = root.get("Level").and_then(Tag::as_compound).unwrap();
                assert_eq!(entity_ids(level), ["minecraft:cat", "minecraft:item_frame"]);
                let next = read("region", 1).unwrap();
                assert_eq!(entity_ids(next.get("Level").and_then(Tag::as_compound).unwrap()), ["minecraft:cow"]);
            }
            // For the game's DataFixer harness: the entity region file of the newest target.
            if let Some(dump) = std::env::var_os("BF_PIPELINE_DUMP")
                && separate
            {
                let dump = std::path::PathBuf::from(dump);
                std::fs::create_dir_all(&dump).unwrap();
                let file = out.join("dimensions/minecraft/overworld/entities/r.0.0.mca");
                std::fs::copy(file, dump.join("r.0.0.mca")).unwrap();
            }
        }
    }
}
