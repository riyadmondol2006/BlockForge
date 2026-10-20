// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Loaders for Chunker's `cli/data` files (vanilla block, state and item lists per game version).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::parse_dir_version;
use crate::convert::mapping::{StateMap, StateValue, Version};

/// A game version folder: its name and parsed version.
#[derive(Clone, Debug)]
pub(crate) struct VersionDir {
    pub name: String,
    pub version: Version,
    pub path: PathBuf,
}

/// The version folders under `data/<edition>` (`java` or `bedrock`), oldest first.
pub(crate) fn version_dirs(data: &Path, edition: &str) -> Vec<VersionDir> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(data.join(edition)) else { return out };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if let (true, Some(version)) = (path.is_dir(), parse_dir_version(&name)) {
            out.push(VersionDir { name, version, path });
        }
    }
    out.sort_by(|a, b| a.version.cmp(&b.version).then_with(|| a.name.cmp(&b.name)));
    out
}

/// All states of the blocks of one version: id -> state maps (Java: also the default state).
#[derive(Debug, Default)]
pub(crate) struct BlockData {
    pub states: BTreeMap<String, Vec<StateMap>>,
    pub defaults: BTreeMap<String, StateMap>,
    pub allowed: HashMap<String, HashSet<StateMap>>,
}

impl BlockData {
    fn push(&mut self, id: &str, states: StateMap) {
        self.allowed.entry(id.to_string()).or_default().insert(states.clone());
        self.states.entry(id.to_string()).or_default().push(states);
    }
}

#[derive(Deserialize)]
struct JavaBlock {
    #[serde(default)]
    states: Vec<JavaState>,
}

#[derive(Deserialize)]
struct JavaState {
    #[serde(default)]
    properties: BTreeMap<String, String>,
    #[serde(default)]
    default: bool,
}

/// `java/<v>/blocks.json` (the vanilla `blocks` report).
pub(crate) fn load_java_blocks(dir: &VersionDir) -> Result<BlockData, String> {
    let text = std::fs::read(dir.path.join("blocks.json")).map_err(|e| e.to_string())?;
    let blocks: BTreeMap<String, JavaBlock> = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    let mut data = BlockData::default();
    for (id, block) in blocks {
        for s in block.states {
            let map: StateMap = s.properties.iter().map(|(k, v)| (k.as_str().into(), StateValue::str(v))).collect();
            if s.default {
                data.defaults.insert(id.clone(), map.clone());
            }
            data.push(&id, map);
        }
    }
    Ok(data)
}

/// All ids of `java/<v>/items.json` (block items included).
pub(crate) fn load_java_all_items(dir: &VersionDir) -> Result<BTreeSet<String>, String> {
    let text = std::fs::read(dir.path.join("items.json")).map_err(|e| e.to_string())?;
    let items: BTreeMap<String, serde::de::IgnoredAny> = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    Ok(items.into_keys().collect())
}

#[derive(Deserialize)]
struct BedrockFile {
    blocks: Vec<BedrockBlock>,
}

#[derive(Deserialize)]
struct BedrockBlock {
    name: String,
    #[serde(default)]
    states: Option<Value>,
}

/// `bedrock/<v>/block_states.json`. Every state combination is entered twice, `waterlogged` false and true (the
/// pseudo state Chunker uses to pick the liquid layer).
pub(crate) fn load_bedrock_blocks(dir: &VersionDir) -> Result<BlockData, String> {
    let text = std::fs::read(dir.path.join("block_states.json")).map_err(|e| e.to_string())?;
    let file: BedrockFile = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    let mut data = BlockData::default();
    for b in file.blocks {
        let mut map = StateMap::new();
        if let Some(Value::Array(states)) = &b.states {
            for s in states {
                let name = s.get("name").and_then(Value::as_str).ok_or("state without a name")?;
                let value = &s["value"];
                let v = match s.get("type").and_then(Value::as_str) {
                    Some("int") => StateValue::Int(value.as_i64().ok_or("int state without a number")? as i32),
                    Some("byte") => StateValue::Bool(value.as_i64() == Some(1)),
                    Some("string") => StateValue::str(value.as_str().ok_or("string state without a string")?),
                    other => return Err(format!("unknown state type {other:?}")),
                };
                map.insert(name.into(), v);
            }
        }
        let mut dry = map.clone();
        dry.insert("waterlogged".into(), StateValue::Bool(false));
        map.insert("waterlogged".into(), StateValue::Bool(true));
        // The Java test keeps a set per id, so duplicates collapse; keep that behaviour.
        if !data.allowed.get(&b.name).is_some_and(|s| s.contains(&dry)) {
            data.push(&b.name, dry);
        }
        if !data.allowed.get(&b.name).is_some_and(|s| s.contains(&map)) {
            data.push(&b.name, map);
        }
    }
    Ok(data)
}

/// `bedrock/<v>/item_names.json`: ids of real items (entries flagged `item: false`, or numeric ids below 256, are
/// blocks), without the `item.` prefix.
pub(crate) fn load_bedrock_items(dir: &VersionDir) -> Result<BTreeSet<String>, String> {
    let text = std::fs::read(dir.path.join("item_names.json")).map_err(|e| e.to_string())?;
    let items: BTreeMap<String, Value> = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    let mut out = BTreeSet::new();
    for (k, v) in items {
        let is_item = match &v {
            Value::Object(o) => o.get("item").and_then(Value::as_bool).unwrap_or(false),
            other => other.as_i64().unwrap_or(0) >= 256,
        };
        if is_item {
            out.insert(k.replace("item.", ""));
        }
    }
    Ok(out)
}
