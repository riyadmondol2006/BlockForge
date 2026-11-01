//! The backend's shared state: the snapshot the window reads, the events it hears, and (to come) the jobs.

use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;

use super::{Event, Snapshot};
use crate::paths::Dirs;

pub struct Service {
    pub(super) root: std::path::PathBuf,
    pub(super) handle: tokio::runtime::Handle,
    snapshot: RwLock<Arc<Snapshot>>,
    events: broadcast::Sender<Event>,
}

impl Service {
    /// Only allocates: no folder is made or read here.
    pub fn new(dirs: &Dirs, handle: tokio::runtime::Handle) -> Self {
        Self {
            root: dirs.base.join("bedrock"),
            handle,
            snapshot: RwLock::new(Arc::new(Snapshot::default())),
            events: broadcast::channel(256).0,
        }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}
