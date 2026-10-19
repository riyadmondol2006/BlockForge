//! The narrow interface between the converter page and the conversion engine.
//!
//! The page (`gui/convert.rs`) only knows this module: it asks a [`Backend`] what a world is, which versions can be
//! written and runs a [`ConvertJob`] on a background thread, listening to [`Event`]s. [`backend`] hands out the
//! engine in use; until the engine is finished that is [`NotReady`], which says so plainly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::convert::bedrock::detect::detect_bedrock;
use crate::convert::bedrock::reader::BedrockWorldReader;
use crate::convert::java::dirs::detect_java;
use crate::convert::level::encoding::{EncodingType, TargetVersion, VersionTriple};
use crate::convert::level::{ConversionSettings, ConvertRequest, ProgressSink};
use crate::convert::pipeline::convert_world;

/// Which game a world belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edition {
    Java,
    Bedrock,
}

impl Edition {
    /// The name shown to the player.
    pub fn label(self) -> &'static str {
        match self {
            Edition::Java => "Java Edition",
            Edition::Bedrock => "Bedrock Edition",
        }
    }

    /// The edition a world is usually converted to.
    pub fn other(self) -> Edition {
        match self {
            Edition::Java => Edition::Bedrock,
            Edition::Bedrock => Edition::Java,
        }
    }
}

/// An edition and a game version in the usual notation (`"1.21.4"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub edition: Edition,
    pub version: String,
}

/// Which dimensions to convert. Names are the ones [`Detected::dimensions`] lists.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum DimensionSelection {
    /// Every dimension the world has.
    #[default]
    All,
    /// Only these.
    Only(Vec<String>),
}

impl DimensionSelection {
    /// Whether the dimension called `name` is part of the selection.
    pub fn includes(&self, name: &str) -> bool {
        match self {
            DimensionSelection::All => true,
            DimensionSelection::Only(list) => list.iter().any(|d| d == name),
        }
    }
}

/// One conversion to run.
#[derive(Clone, Debug)]
pub struct ConvertJob {
    /// A world folder, or a `.mcworld` / `.zip` file.
    pub input: PathBuf,
    /// The folder to write the new world to. The engine never writes into `input`.
    pub output: PathBuf,
    pub target: Target,
    pub dimensions: DimensionSelection,
    /// Keep data that has no equivalent in the other edition (stored as it was) instead of dropping it.
    pub keep_unknown_nbt: bool,
}

/// What a world turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub edition: Edition,
    pub version: String,
    /// The world's name (from `level.dat`, `levelname.txt` or its folder).
    pub name: String,
    /// The dimensions found: `"overworld"`, `"nether"`, `"end"`.
    pub dimensions: Vec<String>,
    pub size_bytes: u64,
}

/// What a conversion reports while it runs. A run ends with exactly one of `Done`, `Failed` or `Cancelled`.
#[derive(Clone, Debug)]
pub enum Event {
    /// What is happening now ("Reading chunks", "Writing the database").
    Stage(String),
    /// How far along the whole job is, from 0 to 1.
    Progress(f32),
    /// Something was not perfect but the run goes on.
    Warning(String),
    Done(Summary),
    Failed(String),
    Cancelled,
}

/// What a finished conversion did.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Columns of blocks (chunks) written.
    pub columns: u64,
    /// Blocks with no equivalent in the target and how many there were of each, most frequent first.
    pub unmapped_blocks: Vec<(String, u64)>,
    pub warnings: Vec<String>,
    pub elapsed: Duration,
    /// Where the new world is.
    pub output: PathBuf,
}

/// The conversion engine, as far as the page needs it.
pub trait Backend: Send + Sync {
    /// Looks at a world folder or archive. The error is a sentence for the player.
    fn detect(&self, path: &Path) -> Result<Detected, String>;
    /// The versions a world can be converted to in `edition`, newest first.
    fn target_versions(&self, edition: Edition) -> Vec<String>;
    /// Converts `job`, sending [`Event`]s to `events`, until it is done or `cancel` is set. Blocks: call it from a
    /// background thread.
    fn run(&self, job: ConvertJob, events: Sender<Event>, cancel: Arc<AtomicBool>);
}

/// The text the stand-in engine answers with.
pub const NOT_READY: &str = "The converter is not finished yet. Worlds cannot be converted in this version.";

/// The engine that is not there yet: it detects nothing and fails every job with a clear message.
pub struct NotReady;

impl Backend for NotReady {
    fn detect(&self, _path: &Path) -> Result<Detected, String> {
        Err(NOT_READY.to_string())
    }

    fn target_versions(&self, _edition: Edition) -> Vec<String> {
        Vec::new()
    }

    fn run(&self, _job: ConvertJob, events: Sender<Event>, _cancel: Arc<AtomicBool>) {
        let _ = events.send(Event::Failed(NOT_READY.to_string()));
    }
}

/// The real engine: the Rust port of Chunker. Today it converts Bedrock worlds to Java Edition.
pub struct Engine;

impl Engine {
    fn size_of(path: &Path) -> u64 {
        fn walk(path: &Path, depth: u32) -> u64 {
            let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
            if meta.is_file() {
                return meta.len();
            }
            if !meta.is_dir() || depth > 12 {
                return 0;
            }
            std::fs::read_dir(path).map(|rd| rd.flatten().map(|e| walk(&e.path(), depth + 1)).sum()).unwrap_or(0)
        }
        walk(path, 0)
    }
}

fn dimension_name(identifier: &str) -> String {
    match identifier {
        "minecraft:overworld" => "overworld".to_string(),
        "minecraft:the_nether" => "nether".to_string(),
        "minecraft:the_end" => "end".to_string(),
        other => other.to_string(),
    }
}

fn dimension_identifier(name: &str) -> String {
    match name {
        "overworld" => "minecraft:overworld".to_string(),
        "nether" => "minecraft:the_nether".to_string(),
        "end" => "minecraft:the_end".to_string(),
        other => other.to_string(),
    }
}

/// Sends what the conversion reports to the page.
struct EventSink {
    events: Sender<Event>,
}

impl ProgressSink for EventSink {
    fn progress(&self, fraction: f64) {
        let _ = self.events.send(Event::Progress(fraction.clamp(0.0, 1.0) as f32));
    }
    fn stage(&self, text: &str) {
        let _ = self.events.send(Event::Stage(text.to_string()));
    }
    fn warning(&self, message: &str) {
        let _ = self.events.send(Event::Warning(message.to_string()));
    }
}

impl Backend for Engine {
    fn detect(&self, path: &Path) -> Result<Detected, String> {
        if let Some(info) = detect_bedrock(path) {
            let version = info
                .game_version
                .map(|v| format!("{}.{}.{}", v[0], v[1], v[2]))
                .unwrap_or_else(|| info.version.name().to_string());
            let dimensions = BedrockWorldReader::open(&info, &ConversionSettings::default())
                .map(|r| r.dimensions().iter().map(|d| dimension_name(d.identifier())).collect())
                .unwrap_or_else(|_| vec!["overworld".to_string()]);
            return Ok(Detected {
                edition: Edition::Bedrock,
                version,
                name: info.name.clone(),
                dimensions,
                size_bytes: Engine::size_of(path),
            });
        }
        if detect_java(path).is_some() {
            return Err("This is a Java Edition world. BlockForge converts Bedrock worlds to Java Edition, not the other way round.".into());
        }
        Err("This does not look like a Minecraft world (no level.dat with a Bedrock db or a Java region folder)."
            .into())
    }

    fn target_versions(&self, edition: Edition) -> Vec<String> {
        match edition {
            Edition::Java => {
                let mut seen = std::collections::BTreeSet::new();
                let mut out = Vec::new();
                for (_, v) in crate::convert::java::version::DATA_VERSIONS.iter().rev() {
                    if (v.major, v.minor) < (1, 13) {
                        continue;
                    }
                    let text = format!("{}.{}.{}", v.major, v.minor, v.patch);
                    let text = text.strip_suffix(".0").map(str::to_string).unwrap_or(text);
                    if seen.insert(text.clone()) {
                        out.push(text);
                    }
                }
                out
            }
            Edition::Bedrock => Vec::new(),
        }
    }

    fn run(&self, job: ConvertJob, events: Sender<Event>, cancel: Arc<AtomicBool>) {
        let started = std::time::Instant::now();
        let sink = EventSink { events: events.clone() };
        let Ok(version) = VersionTriple::from_string(&job.target.version) else {
            let _ = events.send(Event::Failed(format!("{} is not a version.", job.target.version)));
            return;
        };
        let edition = match job.target.edition {
            Edition::Java => EncodingType::Java,
            Edition::Bedrock => EncodingType::Bedrock,
        };
        let mut settings = ConversionSettings::builder().build();
        if let DimensionSelection::Only(names) = &job.dimensions {
            settings.dimension_mapping =
                Some(names.iter().map(|n| (dimension_identifier(n), dimension_identifier(n))).collect());
        }
        settings.allow_nbt_copying = job.keep_unknown_nbt;
        let request = ConvertRequest {
            input: job.input.clone(),
            output: job.output.clone(),
            target: TargetVersion::new(edition, version),
            settings,
        };
        match convert_world(&request, &sink, &cancel) {
            Ok(summary) => {
                let mut unmapped = summary.blocks_unmapped;
                unmapped.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                let _ = events.send(Event::Done(Summary {
                    columns: summary.columns,
                    unmapped_blocks: unmapped,
                    warnings: summary.warnings,
                    elapsed: started.elapsed(),
                    output: job.output,
                }));
            }
            Err(e) if e.is_cancelled() => {
                let _ = events.send(Event::Cancelled);
            }
            Err(e) => {
                let _ = events.send(Event::Failed(e.to_string()));
            }
        }
    }
}

/// The engine in use.
pub fn backend() -> Arc<dyn Backend> {
    Arc::new(Engine)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;

    #[test]
    fn editions_swap() {
        assert_eq!(Edition::Java.other(), Edition::Bedrock);
        assert_eq!(Edition::Bedrock.other().other(), Edition::Bedrock);
        assert_ne!(Edition::Java.label(), Edition::Bedrock.label());
    }

    #[test]
    fn a_selection_knows_its_dimensions() {
        assert!(DimensionSelection::All.includes("nether"));
        let only = DimensionSelection::Only(vec!["overworld".into(), "end".into()]);
        assert!(only.includes("end"));
        assert!(!only.includes("nether"));
        assert!(!DimensionSelection::Only(Vec::new()).includes("overworld"));
        assert_eq!(DimensionSelection::default(), DimensionSelection::All);
    }

    #[test]
    fn the_stand_in_says_it_is_not_finished() {
        let b = NotReady;
        let err = b.detect(Path::new("/nowhere")).unwrap_err();
        assert!(err.contains("not finished"), "{err}");
        assert!(b.target_versions(Edition::Java).is_empty());
        let (tx, rx) = channel();
        let job = ConvertJob {
            input: "/a".into(),
            output: "/b".into(),
            target: Target { edition: Edition::Java, version: "1.21.4".into() },
            dimensions: DimensionSelection::All,
            keep_unknown_nbt: false,
        };
        b.run(job, tx, Arc::new(AtomicBool::new(false)));
        let events: Vec<Event> = rx.iter().collect();
        assert!(matches!(events.as_slice(), [Event::Failed(m)] if m == NOT_READY));
    }

    fn test_world() -> Option<PathBuf> {
        let p =
            PathBuf::from(std::env::var_os("HOME")?).join("Downloads").join("Mace, Spear, Sword PvP Map! V4.mcworld");
        p.exists().then_some(p)
    }

    fn job(input: &Path, output: &Path, edition: Edition, version: &str) -> ConvertJob {
        ConvertJob {
            input: input.into(),
            output: output.into(),
            target: Target { edition, version: version.into() },
            dimensions: DimensionSelection::All,
            keep_unknown_nbt: false,
        }
    }

    #[test]
    fn the_engine_lists_java_versions_newest_first() {
        let versions = Engine.target_versions(Edition::Java);
        assert!(versions.len() > 20);
        assert!(versions.contains(&"1.21.4".to_string()) && versions.contains(&"1.13.2".to_string()));
        assert!(!versions.iter().any(|v| v.starts_with("1.12")));
        assert!(Engine.target_versions(Edition::Bedrock).is_empty());
    }

    #[test]
    fn the_engine_refuses_what_it_cannot_do_with_a_clear_message() {
        let dir = std::env::temp_dir().join(format!("bf-engine-refuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Engine.detect(&dir).is_err());
        let (tx, rx) = channel();
        Engine.run(job(&dir, &dir.join("out"), Edition::Bedrock, "1.21.50"), tx, Arc::new(AtomicBool::new(false)));
        let events: Vec<Event> = rx.iter().collect();
        assert!(matches!(events.last(), Some(Event::Failed(m)) if m.contains("Bedrock")), "{events:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_engine_converts_the_test_world() {
        let Some(world) = test_world() else { return };
        let detected = Engine.detect(&world).unwrap();
        assert_eq!(detected.edition, Edition::Bedrock);
        assert_eq!(detected.version, "1.26.51");
        assert!(detected.name.contains("Mace"));
        assert!(detected.dimensions.contains(&"overworld".to_string()));
        let out = std::env::temp_dir().join(format!("bf-engine-convert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let (tx, rx) = channel();
        Engine.run(job(&world, &out, Edition::Java, "1.21.4"), tx, Arc::new(AtomicBool::new(false)));
        let events: Vec<Event> = rx.iter().collect();
        let Some(Event::Done(summary)) = events.last() else { panic!("{events:?}") };
        assert_eq!(summary.columns, 46);
        assert!(summary.unmapped_blocks.is_empty(), "{:?}", summary.unmapped_blocks);
        assert!(out.join("level.dat").is_file() && out.join("region").is_dir());
        assert!(events.iter().any(|e| matches!(e, Event::Progress(p) if *p >= 0.99)));
        // the same folder again must be refused (it is not empty), and a cancelled run leaves nothing behind
        let (tx, rx) = channel();
        Engine.run(job(&world, &out, Edition::Java, "1.21.4"), tx, Arc::new(AtomicBool::new(false)));
        assert!(matches!(rx.iter().last(), Some(Event::Failed(_))));
        let out2 = std::env::temp_dir().join(format!("bf-engine-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out2);
        let (tx, rx) = channel();
        Engine.run(job(&world, &out2, Edition::Java, "1.21.4"), tx, Arc::new(AtomicBool::new(true)));
        assert!(matches!(rx.iter().last(), Some(Event::Cancelled)));
        assert!(!out2.exists() || std::fs::read_dir(&out2).map(|mut d| d.next().is_none()).unwrap_or(true));
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&out2);
    }
}
