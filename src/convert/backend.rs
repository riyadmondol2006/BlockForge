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
use crate::convert::level::{
    ConversionSettings, ConvertError, ConvertRequest, EntityFilter, LevelOverrides, PlayerOptions, ProgressSink,
};
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

/// What happens to the world's entities (the entity options of `ConversionSettings`, with the same defaults: all
/// on). The account that owns the host's pets comes from the player options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityOptions {
    /// Convert entities at all (mobs, pets, villagers, armor stands, paintings, minecarts, boats, item frames).
    pub convert: bool,
    /// Which mobs to keep.
    pub filter: EntityFilter,
    /// Leave out dropped items, experience orbs, falling blocks and projectiles.
    pub drop_transient: bool,
    /// Bedrock item frames become Java item frames, with their item.
    pub item_frames: bool,
}

impl Default for EntityOptions {
    fn default() -> Self {
        EntityOptions { convert: true, filter: EntityFilter::All, drop_transient: true, item_frames: true }
    }
}

impl EntityOptions {
    /// Put the options into the settings of a conversion.
    pub fn apply(&self, settings: &mut ConversionSettings) {
        settings.process_entities = self.convert;
        settings.entity_filter = self.filter;
        settings.drop_transient_entities = self.drop_transient;
        settings.convert_item_frames = self.item_frames;
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
    /// Who owns the converted world, the ender chest and stats, whose inventory and the other players' profiles
    /// ([`ConversionSettings::player`]; the default keeps everything of the host and writes the other profiles).
    pub player: PlayerOptions,
    /// Difficulty, game mode, cheats and hardcore to force ([`ConversionSettings::level_overrides`]; the default
    /// keeps the world's own).
    pub level_overrides: LevelOverrides,
    /// Mobs, pets, item frames and the other entities.
    pub entities: EntityOptions,
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
    /// The other players of a Bedrock world (besides the host), whose inventory the owner can be given.
    pub players: Vec<WorldPlayer>,
}

/// One of the other players of a Bedrock world (`player_server_<uuid>`). Bedrock keeps no gamertag, so a player is
/// told apart by what they have: their level, how many items they carry and where they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldPlayer {
    /// The profile's uuid (the key without `player_server_`), the way
    /// [`InventorySource`](crate::convert::level::InventorySource) names it.
    pub id: String,
    /// A player signed in to an account owned it.
    pub signed_in: bool,
    pub level: i32,
    /// Stacks in the inventory and armor.
    pub items: usize,
    /// The block the player stands on.
    pub position: [i32; 3],
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
    /// A failure trying again can't fix: the world's data is damaged, or it is a world that can't be converted.
    FailedForGood(String),
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

/// What a Java Edition world is told: only Bedrock worlds are converted.
pub const JAVA_WORLD: &str =
    "This is a Java Edition world. BlockForge converts Bedrock worlds to Java Edition, not the other way round.";

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
            let reader = BedrockWorldReader::open(&info, &ConversionSettings::default());
            let dimensions = reader
                .as_ref()
                .map(|r| r.dimensions().iter().map(|d| dimension_name(d.identifier())).collect())
                .unwrap_or_else(|_| vec!["overworld".to_string()]);
            // the other players, to choose whose inventory the owner gets; a world that can't list them has none
            let players = reader
                .as_ref()
                .ok()
                .and_then(|r| r.player_profiles().ok())
                .unwrap_or_default()
                .into_iter()
                .map(|p| WorldPlayer {
                    signed_in: p.signed_in(),
                    level: p.player.stats.xp_level,
                    items: p.player.inventory.len(),
                    position: p.player.position.map(|v| v.floor() as i32),
                    id: p.id,
                })
                .collect();
            return Ok(Detected {
                edition: Edition::Bedrock,
                version,
                name: info.name.clone(),
                dimensions,
                size_bytes: Engine::size_of(path),
                players,
            });
        }
        if detect_java(path).is_some() {
            return Err(JAVA_WORLD.to_string());
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
        settings.player = job.player.clone();
        settings.level_overrides = job.level_overrides;
        job.entities.apply(&mut settings);
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
                let message = explain(&e, &job.output);
                let for_good = matches!(e, ConvertError::Malformed(_) | ConvertError::Unsupported(_));
                let _ = events.send(if for_good { Event::FailedForGood(message) } else { Event::Failed(message) });
            }
        }
    }
}

/// A failed conversion in words a player can act on: what went wrong and, where it helps, which folder.
fn explain(e: &ConvertError, output: &Path) -> String {
    use std::io::ErrorKind;
    match e {
        ConvertError::Io(io) if io.kind() == ErrorKind::PermissionDenied => format!(
            "BlockForge isn't allowed to write in {}. Choose another place to save the world.",
            output.parent().unwrap_or(output).display()
        ),
        ConvertError::Io(io) if io.kind() == ErrorKind::StorageFull => {
            "The disk is full. Free some space, or save the world on another drive.".to_string()
        }
        ConvertError::Io(io) if io.kind() == ErrorKind::NotFound => {
            format!("A file of the world went missing while it was read ({io}). Was the world moved or deleted?")
        }
        ConvertError::Io(io) => format!("Reading or writing a file failed: {io}"),
        ConvertError::Malformed(msg) => format!(
            "The world's data is damaged or isn't what a Bedrock world holds ({msg}). Trying again won't help: open \
             the world in Minecraft once, let it save, and convert it again."
        ),
        ConvertError::Unsupported(msg) => format!("This world can't be converted: {msg}."),
        other => other.to_string(),
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
            player: PlayerOptions::default(),
            level_overrides: LevelOverrides::default(),
            entities: EntityOptions::default(),
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
            player: PlayerOptions::default(),
            level_overrides: LevelOverrides::default(),
            entities: EntityOptions::default(),
        }
    }

    #[test]
    fn entity_options_reach_the_settings() {
        let mut settings = ConversionSettings::default();
        EntityOptions::default().apply(&mut settings);
        assert!(settings.process_entities && settings.drop_transient_entities && settings.convert_item_frames);
        assert_eq!(settings.entity_filter, EntityFilter::All);
        let off = EntityOptions {
            convert: false,
            filter: EntityFilter::NoHostile,
            drop_transient: false,
            item_frames: false,
        };
        off.apply(&mut settings);
        assert!(!settings.process_entities && !settings.drop_transient_entities && !settings.convert_item_frames);
        assert_eq!(settings.entity_filter, EntityFilter::NoHostile);
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
        assert!(
            matches!(events.last(), Some(Event::Failed(m) | Event::FailedForGood(m)) if m.contains("Bedrock")),
            "{events:?}"
        );
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
        assert!(matches!(rx.iter().last(), Some(Event::Failed(_) | Event::FailedForGood(_))));
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
