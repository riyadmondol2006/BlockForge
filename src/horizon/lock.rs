//! Locks between BlockForge processes (the app and the command line can run at once, on the same data folder). The
//! in-process `BUSY` mutex only keeps one process from installing or preparing twice; these files keep processes off
//! each other, and keep a build that a game plays from where it is:
//!
//! - `horizon/.lock`, exclusive, held around installing, preparing, taking resources from another build and pruning.
//! - `<build>/.in-use`, shared, held by every start that plays from the build until the game is over. Taking a build's
//!   resources or removing it first asks for the same file exclusively, which only works when no game plays from it.
//!
//! The operating system drops a lock when its owner ends, however it ends, so a crash never leaves a build "in use".

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};

use super::release;
use crate::install::Progress;
use crate::paths::Dirs;

/// A lock that is let go when this is dropped. Holds nothing when the file system has no locks: the in-process mutex
/// and the games BlockForge itself knows of still protect then, as before.
#[derive(Debug)]
pub struct Held {
    _file: Option<File>,
}

fn open(path: &Path, create: bool) -> std::io::Result<File> {
    if create {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    OpenOptions::new().read(true).write(true).create(create).truncate(false).open(path)
}

fn exclusive_file(dirs: &Dirs) -> PathBuf {
    release::horizon_dir(dirs).join(".lock")
}

fn in_use_file(build: &Path) -> PathBuf {
    build.join(".in-use")
}

/// Wait for the right to install and prepare: no other BlockForge process does either until this is dropped. Gives up
/// when the player cancels.
pub async fn exclusive(dirs: &Dirs, progress: &Progress) -> Result<Held> {
    let path = exclusive_file(dirs);
    let file = match open(&path, true) {
        Ok(f) => f,
        Err(e) => {
            crate::logs::warn(format!("Horizon: couldn't open {}: {e}", path.display()));
            return Ok(Held { _file: None });
        }
    };
    let mut told = false;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Held { _file: Some(file) }),
            Err(TryLockError::WouldBlock) => {
                if !told {
                    told = true;
                    progress.stage(format!("Waiting for another BlockForge to finish with {}", super::NAME));
                }
                if progress.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    bail!("cancelled");
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(TryLockError::Error(e)) => {
                crate::logs::warn(format!("Horizon: couldn't lock {}: {e}", path.display()));
                return Ok(Held { _file: None });
            }
        }
    }
}

/// Mark `build` as played from, for as long as the result is held (the game's lifetime). Taken while holding
/// `exclusive`, so no clean-up can be between choosing the build and this.
pub fn use_build(build: &Path) -> Held {
    let path = in_use_file(build);
    let file = open(&path, true).and_then(|f| f.lock_shared().map(|()| f));
    match file {
        Ok(f) => Held { _file: Some(f) },
        Err(e) => {
            crate::logs::warn(format!("Horizon: couldn't mark {} as in use: {e}", build.display()));
            Held { _file: None }
        }
    }
}

/// Claim `build` for moving its resources or removing it: `None` when a game plays from it (or that can't be told).
/// The claim holds until dropped.
pub fn claim_unused(build: &Path) -> Option<Held> {
    match open(&in_use_file(build), false) {
        // never played from
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Held { _file: None }),
        Err(_) => None,
        Ok(file) => match file.try_lock() {
            Ok(()) => Some(Held { _file: Some(file) }),
            Err(_) => None,
        },
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::scratch;

    /// `claim_unused`, given a moment: a game started by another test at this instant (forked, not yet executed) still
    /// holds a copy of the lock, for a few milliseconds.
    pub(crate) fn claim_unused_soon(build: &Path) -> Option<Held> {
        for _ in 0..100 {
            if let Some(held) = claim_unused(build) {
                return Some(held);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn a_build_played_from_cannot_be_claimed_until_the_game_is_over() {
        let build = scratch("hz-lock-use");
        // never played from, and a folder that isn't a build
        assert!(claim_unused(&build).is_some());
        assert!(claim_unused(&build.join("missing")).is_some());
        let first = use_build(&build);
        let second = use_build(&build);
        assert!(claim_unused(&build).is_none());
        drop(first);
        assert!(claim_unused(&build).is_none(), "the second game still plays from it");
        drop(second);
        let claim = claim_unused_soon(&build).expect("free again");
        // a claim keeps a game from starting meanwhile on the same file, but the file stays for the next start
        assert!(claim_unused(&build).is_none());
        drop(claim);
        assert!(claim_unused_soon(&build).is_some());
        let _ = std::fs::remove_dir_all(build);
    }

    #[test]
    fn only_one_holds_the_exclusive_lock_and_waiting_ends_on_cancel() {
        let base = scratch("hz-lock-excl");
        let dirs = Dirs::at(&base);
        crate::testutil::runtime().block_on(async {
            let progress = Progress::new();
            let held = exclusive(&dirs, &progress).await.unwrap();
            let waiting = Progress::new();
            let other = {
                let (dirs, waiting) = (dirs.clone(), waiting.clone());
                tokio::spawn(async move { exclusive(&dirs, &waiting).await.map(|_| ()) })
            };
            tokio::time::sleep(Duration::from_millis(600)).await;
            assert!(!other.is_finished(), "got the lock while it was held");
            assert!(waiting.stage_text().contains("Waiting"));
            waiting.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(other.await.unwrap().is_err());
            // let go: the next one gets it
            drop(held);
            exclusive(&dirs, &Progress::new()).await.unwrap();
        });
        let _ = std::fs::remove_dir_all(base);
    }
}
