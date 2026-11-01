//! Leases: operating-system file locks that say who is using what, and that the system drops the moment their owner
//! ends, however it ends, so a crash can never leave a prefix or a build "in use".
//!
//! Ported from BedrockOnLinux `bol/prefix.py` (`prefix_operation_lock`, `shared_assets_lock`, `launch_lock`), with the
//! same shape as Horizon's (`src/horizon/lock.rs`):
//!
//! - **game**: one Bedrock game at a time on this computer (exclusive, `locks/game.lease`).
//! - **prefix**: the profile's launch lock (exclusive): setup, repair and play of that prefix exclude each other.
//! - **assets**: the engine and the game build in use (shared): nothing may replace or delete them meanwhile.
//!
//! The supervisor keeps its leases for the whole session and lets the game's wrapper inherit them, so a supervisor
//! that is killed leaves them held for as long as the game still runs.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Exclusive,
    Shared,
}

/// Why a lease could not be taken.
#[derive(Debug)]
pub enum LeaseError {
    /// someone else holds it (the path says which lease)
    Busy(PathBuf),
    /// the wait was cancelled
    Cancelled,
    Io(PathBuf, io::Error),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeaseError::Busy(p) => write!(f, "{} is in use by another Bedrock session or job", p.display()),
            LeaseError::Cancelled => f.write_str("cancelled"),
            LeaseError::Io(p, e) => write!(f, "can't use lease {}: {e}", p.display()),
        }
    }
}

impl std::error::Error for LeaseError {}

/// A lease that is let go when this is dropped.
#[derive(Debug)]
pub struct Lease {
    file: File,
}

fn open(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    Ok(file)
}

impl Lease {
    /// Take the lease now or say it's busy.
    pub fn try_take(path: &Path, kind: Kind) -> Result<Lease, LeaseError> {
        let file = open(path).map_err(|e| LeaseError::Io(path.to_path_buf(), e))?;
        let r = match kind {
            Kind::Exclusive => file.try_lock(),
            Kind::Shared => file.try_lock_shared(),
        };
        match r {
            Ok(()) => Ok(Lease { file }),
            Err(TryLockError::WouldBlock) => Err(LeaseError::Busy(path.to_path_buf())),
            Err(TryLockError::Error(e)) => Err(LeaseError::Io(path.to_path_buf(), e)),
        }
    }

    /// Wait up to `wait` for the lease (the previous holder may be a moment from letting go), polling `cancelled`.
    pub fn take_within(
        path: &Path,
        kind: Kind,
        wait: Duration,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Lease, LeaseError> {
        let deadline = Instant::now() + wait;
        loop {
            match Lease::try_take(path, kind) {
                Err(LeaseError::Busy(p)) => {
                    if cancelled() {
                        return Err(LeaseError::Cancelled);
                    }
                    if Instant::now() >= deadline {
                        return Err(LeaseError::Busy(p));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                other => return other,
            }
        }
    }

    /// Let the game's wrapper inherit this lease (and so hold it for as long as it runs, whatever happens to us).
    #[cfg(target_os = "linux")]
    pub fn inherit_into_children(&self) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        super::sys::keep_across_exec(self.file.as_raw_fd())
    }
}

/// Everything a session holds, taken in an order that can't deadlock (game, prefix, then assets, each by path).
#[derive(Debug, Default)]
pub struct Held {
    pub leases: Vec<Lease>,
}

/// What a session needs.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LeaseSet {
    /// one game at a time: `locks/game.lease`
    pub game: Option<PathBuf>,
    /// the prefix's launch lock
    pub prefix: Option<PathBuf>,
    /// shared leases on the engine and the build (and anything else read during the game)
    pub assets: Vec<PathBuf>,
}

impl LeaseSet {
    /// Take them all, waiting up to `wait` for each. If any can't be taken, none stay taken. The error names what was
    /// busy first, so a second game says "another game is running" and not something about files.
    pub fn take(&self, wait: Duration, cancelled: &dyn Fn() -> bool) -> Result<Held, LeaseError> {
        let mut held = Held::default();
        if let Some(p) = &self.game {
            held.leases.push(Lease::take_within(p, Kind::Exclusive, wait, cancelled)?);
        }
        if let Some(p) = &self.prefix {
            held.leases.push(Lease::take_within(p, Kind::Exclusive, wait, cancelled)?);
        }
        let mut assets = self.assets.clone();
        assets.sort();
        assets.dedup();
        for p in &assets {
            held.leases.push(Lease::take_within(p, Kind::Shared, wait, cancelled)?);
        }
        Ok(held)
    }
}

impl Held {
    #[cfg(target_os = "linux")]
    pub fn inherit_into_children(&self) {
        for l in &self.leases {
            let _ = l.inherit_into_children();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    fn never() -> bool {
        false
    }

    /// Take a lease that should be free, allowing for a process another test is starting at this instant (forked, not
    /// yet executed), which holds a copy of every open lock for a few milliseconds.
    fn soon(path: &Path, kind: Kind) -> Lease {
        Lease::take_within(path, kind, Duration::from_secs(5), &never).unwrap()
    }

    #[test]
    fn exclusive_excludes_everything_and_shared_shares() {
        let dir = scratch("lease-kinds");
        let f = dir.join("a.lease");
        let first = soon(&f, Kind::Exclusive);
        assert!(matches!(Lease::try_take(&f, Kind::Exclusive), Err(LeaseError::Busy(_))));
        assert!(matches!(Lease::try_take(&f, Kind::Shared), Err(LeaseError::Busy(_))));
        drop(first);
        let s1 = soon(&f, Kind::Shared);
        let s2 = soon(&f, Kind::Shared);
        assert!(matches!(Lease::try_take(&f, Kind::Exclusive), Err(LeaseError::Busy(_))));
        drop((s1, s2));
        let _ = soon(&f, Kind::Exclusive);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_second_game_is_refused_and_a_cancelled_preparation_gives_everything_back() {
        let dir = scratch("lease-set");
        let set = LeaseSet {
            game: Some(dir.join("locks/game.lease")),
            prefix: Some(dir.join("p/locks/launch.lease")),
            assets: vec![dir.join("locks/engine.lease"), dir.join("locks/build.lease"), dir.join("locks/engine.lease")],
        };
        let first = set.take(Duration::from_secs(5), &never).unwrap();
        assert_eq!(first.leases.len(), 4, "duplicate asset leases are taken once");
        // another profile wants to play at the same time: the single-game lease says no
        let other = LeaseSet { game: set.game.clone(), prefix: Some(dir.join("q/locks/launch.lease")), assets: vec![] };
        match other.take(Duration::ZERO, &never) {
            Err(LeaseError::Busy(p)) => assert!(p.ends_with("game.lease")),
            e => panic!("{e:?}"),
        }
        // the other profile's prefix lease was not left behind by the failed attempt
        assert!(
            Lease::take_within(&dir.join("q/locks/launch.lease"), Kind::Exclusive, Duration::from_secs(5), &never)
                .is_ok()
        );
        drop(first);
        // a preparation that was cancelled while waiting
        // (a process another test is starting at this instant still holds a copy of the lock for a few milliseconds)
        let blocker =
            Lease::take_within(set.prefix.as_ref().unwrap(), Kind::Exclusive, Duration::from_secs(5), &never).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let waited = std::thread::scope(|s| {
            let h = s.spawn(|| set.take(Duration::from_secs(30), &|| cancel.load(std::sync::atomic::Ordering::SeqCst)));
            std::thread::sleep(Duration::from_millis(100));
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            h.join().unwrap()
        });
        assert!(matches!(waited, Err(LeaseError::Cancelled)));
        // the game lease it had already taken is free again, only the prefix's blocker holds
        assert!(
            Lease::take_within(set.game.as_ref().unwrap(), Kind::Exclusive, Duration::from_secs(5), &never).is_ok()
        );
        drop(blocker);
        assert!(set.take(Duration::from_secs(5), &never).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_lease_is_given_back_when_its_process_dies() {
        // the kernel drops the lock with the process: a crash leaves nothing "in use"
        let dir = scratch("lease-death");
        let f = dir.join("x.lease");
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args([
                "--exact",
                "bedrock::supervisor::lease::tests::lease_holder_role",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BFB_LEASE_ROLE", &f)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // wait until it says it holds the lease
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            use std::io::BufRead;
            assert!(out.read_line(&mut line).unwrap() > 0, "holder ended early");
            if line.contains("HELD") {
                break;
            }
        }
        assert!(matches!(Lease::try_take(&f, Kind::Exclusive), Err(LeaseError::Busy(_))));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(Lease::take_within(&f, Kind::Exclusive, Duration::from_secs(5), &never).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Not a test: the process the one above starts (a test binary run again, told by the environment).
    #[test]
    fn lease_holder_role() {
        let Some(path) = std::env::var_os("BFB_LEASE_ROLE") else { return };
        let _lease = Lease::try_take(Path::new(&path), Kind::Exclusive).unwrap();
        println!("HELD");
        std::thread::sleep(Duration::from_secs(120));
    }
}
