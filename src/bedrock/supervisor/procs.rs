//! Which processes are ours, and stopping them without ever touching anyone else's.
//!
//! Ported from BedrockOnLinux `bol/prefix.py` (`prefix_processes`, `stop_prefix_procs`) and
//! `bol/launch.py` (`_prefix_stably_idle_after_wrapper`), with the ownership made stricter:
//!
//! - A process is ours when it belongs to this player (same user) **and** its environment carries exactly this
//!   session's `WINEPREFIX`. A different prefix, even one that only starts the same way (`pfx` and `pfx2`), is never
//!   ours. Nothing is ever signalled by name (`pkill wine`) or by process group alone.
//! - An identity is the process number **and** its start time (field 22 of `/proc/<pid>/stat`). The numbers are
//!   recycled; a recorded identity whose start time no longer matches is a stranger now, and is skipped.
//! - The identity is checked again, with the process pinned by a process descriptor, immediately before each signal.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A process number with the time it started (clock ticks after boot): the only thing that identifies a process
/// for longer than an instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ProcId {
    pub pid: u32,
    pub start: u64,
}

/// What `/proc` says about one process.
#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub id: ProcId,
    pub ppid: u32,
    /// `Z` for a zombie, which holds a number but no resources and is not "running"
    pub state: char,
    pub uid: u32,
    pub comm: String,
    pub cmdline: Vec<String>,
}

impl ProcInfo {
    pub fn is_zombie(&self) -> bool {
        self.state == 'Z' || self.state == 'X'
    }
}

/// How signals reach processes. Real: process descriptors. Tests: a recorder.
pub trait Signals: Send + Sync {
    /// Send `sig` to `id` if `still_ours` holds once the process is pinned. `Ok(true)` when it was sent.
    fn send(&self, id: ProcId, sig: i32, still_ours: &mut dyn FnMut() -> bool) -> std::io::Result<bool>;
}

#[cfg(target_os = "linux")]
pub struct RealSignals;

#[cfg(target_os = "linux")]
impl Signals for RealSignals {
    fn send(&self, id: ProcId, sig: i32, still_ours: &mut dyn FnMut() -> bool) -> std::io::Result<bool> {
        super::sys::signal_pinned(id.pid, sig, still_ours)
    }
}

/// Reads process facts from a `/proc`-shaped directory (the real one, or a made-up one in a test).
#[derive(Clone, Debug)]
pub struct ProcTable {
    root: PathBuf,
    /// processes of this user only (whose environment we may read, and whose we may signal)
    uid: u32,
    /// never reported: the supervisor itself
    myself: u32,
}

/// The user id of the files in a directory (a process's `/proc/<pid>` is owned by its user).
#[cfg(unix)]
fn owner_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.uid())
}

#[cfg(not(unix))]
fn owner_of(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
pub fn current_uid() -> u32 {
    owner_of(Path::new("/proc/self")).unwrap_or(u32::MAX)
}

#[cfg(not(unix))]
pub fn current_uid() -> u32 {
    u32::MAX
}

/// Parse the contents of `/proc/<pid>/stat`: `(state, ppid, pgrp, starttime)`. The command name is in parentheses
/// and may hold spaces and parentheses itself, so the fields are counted from the last `)`.
pub fn parse_stat(text: &str) -> Option<(char, u32, i32, u64)> {
    let rest = text.rsplit_once(')')?.1;
    let f: Vec<&str> = rest.split_whitespace().collect();
    // f[0] state, f[1] ppid, f[2] pgrp, ... f[19] starttime
    let state = f.first()?.chars().next()?;
    Some((state, f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?, f.get(19)?.parse().ok()?))
}

/// Whether a NUL-separated environment holds exactly `WINEPREFIX=<prefix>` (one trailing slash does not matter).
pub fn environ_has_prefix(environ: &[u8], prefix: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let want = trim_slash(prefix.as_os_str().as_bytes());
    environ.split(|b| *b == 0).any(|entry| entry.strip_prefix(b"WINEPREFIX=").is_some_and(|v| trim_slash(v) == want))
}

fn trim_slash(v: &[u8]) -> &[u8] {
    if v.len() > 1 { v.strip_suffix(b"/").unwrap_or(v) } else { v }
}

impl ProcTable {
    pub fn system() -> Self {
        Self { root: PathBuf::from("/proc"), uid: current_uid(), myself: std::process::id() }
    }

    /// A table over another directory, for tests. `myself` is a number that is never reported.
    #[cfg(test)]
    pub fn at(root: &Path, uid: u32, myself: u32) -> Self {
        Self { root: root.to_path_buf(), uid, myself }
    }

    pub fn info(&self, pid: u32) -> Option<ProcInfo> {
        let dir = self.root.join(pid.to_string());
        let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
        let (state, ppid, _pgrp, start) = parse_stat(&stat)?;
        let uid = owner_of(&dir).unwrap_or(u32::MAX);
        let comm = std::fs::read_to_string(dir.join("comm")).unwrap_or_default().trim().to_string();
        let cmdline = std::fs::read(dir.join("cmdline"))
            .map(|b| {
                b.split(|c| *c == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect()
            })
            .unwrap_or_default();
        Some(ProcInfo { id: ProcId { pid, start }, ppid, state, uid, comm, cmdline })
    }

    /// Whether `info` is a live (not zombie) process of the user this table is for.
    pub fn is_live_and_ours(&self, info: &ProcInfo) -> bool {
        !info.is_zombie() && info.uid == self.uid
    }

    /// Whether `id` is still that process and still ours under `scope`.
    pub fn is_ours(&self, id: ProcId, scope: &Scope) -> bool {
        let Some(info) = self.info(id.pid) else { return false };
        if info.id.start != id.start || info.is_zombie() || info.uid != self.uid {
            return false;
        }
        if std::fs::read(self.root.join(id.pid.to_string()).join("environ"))
            .is_ok_and(|e| environ_has_prefix(&e, scope.prefix))
        {
            return true;
        }
        scope.root.is_some_and(|root| self.is_descendant(id.pid, root))
    }

    /// Whether `pid` is below `root` in the process tree (a process that cleared its environment is still the
    /// game's own child when its parents lead back to the supervisor; a subreaper adopts the orphans).
    pub fn is_descendant(&self, pid: u32, root: u32) -> bool {
        let mut at = pid;
        for _ in 0..64 {
            let Some(info) = self.info(at) else { return false };
            if info.ppid == root {
                return true;
            }
            if info.ppid <= 1 {
                return false;
            }
            at = info.ppid;
        }
        false
    }

    /// Everything ours under `scope`: the processes using the prefix and the supervisor's own descendants.
    pub fn in_scope(&self, scope: &Scope) -> Vec<ProcInfo> {
        let mut all = self.in_prefix(scope.prefix);
        if let Some(root) = scope.root {
            for d in self.descendants(root) {
                if !all.iter().any(|a| a.id == d.id) {
                    all.push(d);
                }
            }
        }
        all.sort_by_key(|i| i.id.pid);
        all
    }

    /// Every live process of ours that uses `prefix` (zombies are not "using" anything).
    pub fn in_prefix(&self, prefix: &Path) -> Vec<ProcInfo> {
        let Ok(rd) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut found = Vec::new();
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if pid == self.myself {
                continue;
            }
            let Some(info) = self.info(pid) else { continue };
            if info.is_zombie() || info.uid != self.uid {
                continue;
            }
            // unreadable environment (another user's, or a process that ended meanwhile): not ours to claim
            let Ok(environ) = std::fs::read(self.root.join(pid.to_string()).join("environ")) else { continue };
            if environ_has_prefix(&environ, prefix) {
                found.push(info);
            }
        }
        found.sort_by_key(|i| i.id.pid);
        found
    }

    /// Every process under `parent` (children, grandchildren ...) by following parent numbers. Used for the
    /// wrapper's own tree, which has no prefix variable to find it by.
    pub fn descendants(&self, parent: u32) -> Vec<ProcInfo> {
        let Ok(rd) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let all: Vec<ProcInfo> = rd
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()))
            .filter(|pid| *pid != self.myself)
            .filter_map(|pid| self.info(pid))
            .filter(|i| !i.is_zombie() && i.uid == self.uid)
            .collect();
        let mut by_parent: HashMap<u32, Vec<&ProcInfo>> = HashMap::new();
        for i in &all {
            by_parent.entry(i.ppid).or_default().push(i);
        }
        let mut out = Vec::new();
        let mut queue = vec![parent];
        let mut seen = HashSet::new();
        while let Some(p) = queue.pop() {
            if !seen.insert(p) {
                continue;
            }
            for child in by_parent.get(&p).into_iter().flatten() {
                out.push((*child).clone());
                queue.push(child.id.pid);
            }
        }
        out
    }
}

/// How long each stage of a stop lasts. The defaults are BedrockOnLinux's.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// after the polite request (SIGTERM) before it gets serious
    pub term_grace: Duration,
    /// after SIGKILL before giving up
    pub kill_grace: Duration,
    /// how often the prefix is looked at again
    pub poll: Duration,
    /// how long the prefix may take to become, and stay, empty after the wrapper is gone
    pub idle_timeout: Duration,
    /// empty looks in a row that count as idle (a service started during shutdown shows up in between)
    pub idle_confirmations: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            term_grace: Duration::from_secs(5),
            kill_grace: Duration::from_secs(2),
            poll: Duration::from_millis(100),
            idle_timeout: Duration::from_secs(10),
            idle_confirmations: 3,
        }
    }
}

/// What a stop did, for the log.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StopReport {
    /// processes seen using the prefix during the stop
    pub seen: usize,
    /// of which needed SIGKILL
    pub killed: usize,
    /// processes left when the time ran out (the prefix is not idle)
    pub left: Vec<ProcId>,
}

/// Where the stop currently is, for the supervisor's state and events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Term,
    Kill,
}

/// Signals only what is ours. Remembers every identity it saw, so a stranger that took a recycled number is never hit.
/// What counts as ours: processes using this prefix, and (when set) everything below the supervisor itself.
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    pub prefix: &'a Path,
    pub root: Option<u32>,
}

impl<'a> Scope<'a> {
    pub fn prefix_only(prefix: &'a Path) -> Self {
        Scope { prefix, root: None }
    }
}

pub struct Reaper<'a> {
    pub table: &'a ProcTable,
    pub signals: &'a dyn Signals,
    pub scope: Scope<'a>,
}

impl Reaper<'_> {
    /// Send `sig` to `id` if it is still the process we recorded and still uses the prefix. `true` when sent.
    pub fn signal(&self, id: ProcId, sig: i32) -> bool {
        let (table, scope) = (self.table, self.scope);
        let mut check = || table.is_ours(id, &scope);
        // a failed signal is not a problem here: the process went away, or was never ours
        self.signals.send(id, sig, &mut check).unwrap_or(false)
    }

    /// Stop everything using the prefix, including what starts while it shuts down: SIGTERM at once, a rescan every
    /// `poll` through `term_grace` (new processes get SIGTERM as they appear), then SIGKILL for whatever remains
    /// through `kill_grace`. `force` skips the polite stage.
    pub fn stop(
        &self,
        timing: Timing,
        force: bool,
        mut on_stage: impl FnMut(Stage),
        mut sleep: impl FnMut(Duration),
    ) -> StopReport {
        let mut seen: HashSet<ProcId> = HashSet::new();
        let mut termed: HashSet<ProcId> = HashSet::new();
        let mut killed: HashSet<ProcId> = HashSet::new();
        let live = |seen: &mut HashSet<ProcId>| -> Vec<ProcInfo> {
            let mut v = self.table.in_scope(&self.scope);
            v.retain(|i| i.uid == self.table.uid);
            seen.extend(v.iter().map(|i| i.id));
            v
        };
        if !force {
            on_stage(Stage::Term);
            let deadline = Instant::now() + timing.term_grace;
            loop {
                let now_live = live(&mut seen);
                if now_live.is_empty() {
                    return StopReport { seen: seen.len(), killed: 0, left: Vec::new() };
                }
                for p in &now_live {
                    if termed.insert(p.id) {
                        self.signal(p.id, super::sys::SIGTERM);
                    }
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                sleep(timing.poll.min(remaining));
            }
        }
        on_stage(Stage::Kill);
        let deadline = Instant::now() + timing.kill_grace;
        loop {
            let now_live = live(&mut seen);
            if now_live.is_empty() {
                return StopReport { seen: seen.len(), killed: killed.len(), left: Vec::new() };
            }
            for p in &now_live {
                if self.signal(p.id, super::sys::SIGKILL) {
                    killed.insert(p.id);
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            sleep(timing.poll.min(remaining));
        }
        let left: Vec<ProcId> = self.table.in_scope(&self.scope).into_iter().map(|i| i.id).collect();
        StopReport { seen: seen.len(), killed: killed.len(), left }
    }

    /// Wait until nothing uses the prefix for `idle_confirmations` looks in a row, or `idle_timeout` passes.
    /// Returns what was last seen (empty means idle).
    pub fn wait_idle(&self, timing: Timing, mut sleep: impl FnMut(Duration)) -> Vec<ProcId> {
        let deadline = Instant::now() + timing.idle_timeout;
        let mut empty = 0;
        loop {
            let live: Vec<ProcId> = self.table.in_scope(&self.scope).into_iter().map(|i| i.id).collect();
            if live.is_empty() {
                empty += 1;
                if empty >= timing.idle_confirmations.max(1) {
                    return live;
                }
            } else {
                empty = 0;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return live;
            }
            sleep(timing.poll.min(remaining));
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A made-up `/proc` for tests: directories with `stat`, `environ`, `comm` and `cmdline` files.
    use super::*;
    use std::sync::Mutex;

    pub fn stat_line(pid: u32, comm: &str, state: char, ppid: u32, pgrp: i32, start: u64) -> String {
        // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime cutime cstime
        // priority nice threads itrealvalue starttime ...
        format!("{pid} ({comm}) {state} {ppid} {pgrp} {pgrp} 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 {start} 1000 100")
    }

    pub fn add(root: &Path, pid: u32, comm: &str, state: char, ppid: u32, start: u64, env: &[(&str, &str)]) {
        let d = root.join(pid.to_string());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("stat"), stat_line(pid, comm, state, ppid, pid as i32, start)).unwrap();
        std::fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
        let mut environ = Vec::new();
        for (k, v) in env {
            environ.extend_from_slice(format!("{k}={v}").as_bytes());
            environ.push(0);
        }
        std::fs::write(d.join("environ"), environ).unwrap();
        std::fs::write(d.join("cmdline"), format!("{comm}\0")).unwrap();
    }

    pub fn remove(root: &Path, pid: u32) {
        let _ = std::fs::remove_dir_all(root.join(pid.to_string()));
    }

    /// Records the signals it is asked to send; with `kill_on` it also removes the process from the fake `/proc`.
    pub struct Recorder {
        pub root: PathBuf,
        pub sent: Mutex<Vec<(u32, i32)>>,
        /// a signal that ends the process (the fake dies when it gets one)
        pub fatal: Vec<i32>,
        /// processes that ignore SIGTERM
        pub stubborn: Vec<u32>,
    }

    impl Signals for Recorder {
        fn send(&self, id: ProcId, sig: i32, still_ours: &mut dyn FnMut() -> bool) -> std::io::Result<bool> {
            if !still_ours() {
                return Ok(false);
            }
            self.sent.lock().unwrap().push((id.pid, sig));
            let immune = sig == 15 && self.stubborn.contains(&id.pid);
            if self.fatal.contains(&sig) && !immune {
                remove(&self.root, id.pid);
            }
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;
    use crate::testutil::scratch;
    use std::sync::Mutex;

    fn table(root: &Path) -> ProcTable {
        ProcTable::at(root, current_uid(), 1)
    }

    const PFX: &str = "/data/profiles/a/compat/pfx";

    #[test]
    fn stat_is_read_from_the_last_parenthesis() {
        let line = stat_line(42, "evil) S 1 (name", 'S', 7, 42, 987654);
        assert_eq!(parse_stat(&line), Some(('S', 7, 42, 987654)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn only_the_exact_prefix_is_ours() {
        let root = scratch("procs-exact");
        add(&root, 100, "wineserver", 'S', 1, 10, &[("WINEPREFIX", PFX), ("HOME", "/h")]);
        add(&root, 101, "wine64", 'S', 100, 11, &[("WINEPREFIX", "/data/profiles/a/compat/pfx2")]);
        add(&root, 102, "other", 'S', 1, 12, &[("WINEPREFIXX", PFX)]);
        add(&root, 103, "trailing", 'S', 1, 13, &[("WINEPREFIX", "/data/profiles/a/compat/pfx/")]);
        add(&root, 104, "parent", 'S', 1, 14, &[("WINEPREFIX", "/data/profiles/a/compat")]);
        let t = table(&root);
        let found: Vec<u32> = t.in_prefix(Path::new(PFX)).iter().map(|i| i.id.pid).collect();
        assert_eq!(found, vec![100, 103]);
        // and the supervisor itself is never reported
        let me = ProcTable::at(&root, current_uid(), 100);
        assert_eq!(me.in_prefix(Path::new(PFX)).len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_reused_number_is_a_stranger() {
        let root = scratch("procs-reuse");
        add(&root, 200, "wineserver", 'S', 1, 500, &[("WINEPREFIX", PFX)]);
        let t = table(&root);
        let recorded = t.info(200).unwrap().id;
        assert!(t.is_ours(recorded, &Scope::prefix_only(Path::new(PFX))));
        // the process ends and an unrelated one takes the number (later start time, even the same prefix text)
        remove(&root, 200);
        add(&root, 200, "firefox", 'S', 1, 900, &[("HOME", "/h")]);
        assert!(!t.is_ours(recorded, &Scope::prefix_only(Path::new(PFX))));
        let rec = Recorder { root: root.clone(), sent: Mutex::new(vec![]), fatal: vec![9], stubborn: vec![] };
        let reaper = Reaper { table: &t, signals: &rec, scope: Scope::prefix_only(Path::new(PFX)) };
        assert!(!reaper.signal(recorded, 15));
        assert!(!reaper.signal(recorded, 9));
        assert!(rec.sent.lock().unwrap().is_empty(), "nothing was signalled");
        // same start time but the process stopped using our prefix: also a stranger
        remove(&root, 200);
        add(&root, 200, "other", 'S', 1, 500, &[("WINEPREFIX", "/elsewhere")]);
        assert!(!t.is_ours(recorded, &Scope::prefix_only(Path::new(PFX))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zombies_do_not_use_the_prefix() {
        let root = scratch("procs-zombie");
        add(&root, 300, "wine64", 'Z', 1, 5, &[("WINEPREFIX", PFX)]);
        assert!(table(&root).in_prefix(Path::new(PFX)).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_stop_signals_in_order_and_picks_up_late_children() {
        let root = scratch("procs-stop");
        add(&root, 400, "wineserver", 'S', 1, 5, &[("WINEPREFIX", PFX)]);
        add(&root, 401, "game", 'S', 400, 6, &[("WINEPREFIX", PFX)]);
        // someone else's wine, same machine
        add(&root, 410, "wineserver", 'S', 1, 7, &[("WINEPREFIX", "/other/pfx")]);
        let rec = Recorder { root: root.clone(), sent: Mutex::new(vec![]), fatal: vec![15, 9], stubborn: vec![401] };
        let t = table(&root);
        let reaper = Reaper { table: &t, signals: &rec, scope: Scope::prefix_only(Path::new(PFX)) };
        let fast = Timing {
            term_grace: Duration::from_millis(200),
            kill_grace: Duration::from_millis(200),
            poll: Duration::from_millis(10),
            idle_timeout: Duration::from_millis(300),
            idle_confirmations: 2,
        };
        // a service that starts while the others are shutting down
        let late = std::cell::Cell::new(false);
        let mut stages = Vec::new();
        let report = reaper.stop(
            fast,
            false,
            |s| stages.push(s),
            |d| {
                if !late.replace(true) {
                    add(&root, 402, "services", 'S', 400, 8, &[("WINEPREFIX", PFX)]);
                }
                std::thread::sleep(d);
            },
        );
        assert!(report.left.is_empty(), "{report:?}");
        assert_eq!(stages, vec![Stage::Term, Stage::Kill]);
        let sent = rec.sent.lock().unwrap().clone();
        // every one of ours was asked politely, the stubborn one was then killed, nobody else was touched
        for pid in [400, 401, 402] {
            assert!(sent.contains(&(pid, 15)), "{pid} was never asked: {sent:?}");
        }
        assert!(sent.contains(&(401, 9)));
        assert!(sent.iter().all(|(pid, _)| *pid != 410), "{sent:?}");
        assert!(report.killed >= 1 && report.seen == 3, "{report:?}");
        assert!(t.info(410).is_some());
        assert!(reaper.wait_idle(fast, std::thread::sleep).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn force_skips_the_polite_stage_and_reports_what_would_not_die() {
        let root = scratch("procs-force");
        add(&root, 500, "uninterruptible", 'D', 1, 5, &[("WINEPREFIX", PFX)]);
        // a signal that does nothing: the process stays
        let rec = Recorder { root: root.clone(), sent: Mutex::new(vec![]), fatal: vec![], stubborn: vec![] };
        let t = table(&root);
        let reaper = Reaper { table: &t, signals: &rec, scope: Scope::prefix_only(Path::new(PFX)) };
        let fast = Timing {
            term_grace: Duration::from_secs(60),
            kill_grace: Duration::from_millis(100),
            poll: Duration::from_millis(10),
            idle_timeout: Duration::from_millis(100),
            idle_confirmations: 2,
        };
        let mut stages = Vec::new();
        let report = reaper.stop(fast, true, |s| stages.push(s), std::thread::sleep);
        assert_eq!(stages, vec![Stage::Kill]);
        assert_eq!(report.left.len(), 1, "the stuck process is reported, not hidden");
        assert!(rec.sent.lock().unwrap().iter().all(|(_, s)| *s == 9));
        assert_eq!(reaper.wait_idle(fast, std::thread::sleep).len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn idle_needs_consecutive_empty_looks() {
        let root = scratch("procs-idle");
        add(&root, 600, "wineserver", 'S', 1, 5, &[("WINEPREFIX", PFX)]);
        let t = table(&root);
        let rec = Recorder { root: root.clone(), sent: Mutex::new(vec![]), fatal: vec![], stubborn: vec![] };
        let reaper = Reaper { table: &t, signals: &rec, scope: Scope::prefix_only(Path::new(PFX)) };
        let timing = Timing {
            idle_timeout: Duration::from_secs(5),
            poll: Duration::from_millis(5),
            idle_confirmations: 3,
            ..Timing::default()
        };
        let looks = std::cell::Cell::new(0);
        let left = reaper.wait_idle(timing, |d| {
            let n = looks.get() + 1;
            looks.set(n);
            match n {
                1 => remove(&root, 600),
                // a service appears in between two empty looks: the count starts over
                3 => add(&root, 601, "services", 'S', 1, 9, &[("WINEPREFIX", PFX)]),
                4 => remove(&root, 601),
                _ => {}
            }
            std::thread::sleep(d);
        });
        assert!(left.is_empty());
        assert!(looks.get() >= 6, "looked only {} times", looks.get());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn descendants_follow_parents() {
        let root = scratch("procs-tree");
        add(&root, 700, "wrapper", 'S', 1, 1, &[]);
        add(&root, 701, "child", 'S', 700, 2, &[]);
        add(&root, 702, "grandchild", 'S', 701, 3, &[]);
        add(&root, 703, "unrelated", 'S', 1, 4, &[]);
        let ids: Vec<u32> = {
            let mut v: Vec<u32> = table(&root).descendants(700).iter().map(|i| i.id.pid).collect();
            v.sort();
            v
        };
        assert_eq!(ids, vec![701, 702]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
