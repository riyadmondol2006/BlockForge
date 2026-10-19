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

/// The engine in use.
pub fn backend() -> Arc<dyn Backend> {
    Arc::new(NotReady)
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
        let b = backend();
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
}
