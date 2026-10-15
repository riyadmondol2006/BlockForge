//! The launcher's own log, in the Java edition's layout: `logs/blockforge.log` for this run, and the logs of
//! earlier runs in `logs/old/<yyyy-MM-dd-HH-mm-ss>.log`, which are deleted after 14 days.
//!
//! Anything in the program can call [`info`], [`warn`], [`error`] or [`debug`]. Lines that arrive before
//! [`init`] has opened the file are kept and written as soon as it has, so nothing from start-up is lost.
//! Each line looks like the Java edition's: `12:34:56.789 [main] INFO  - message`, followed by a blank line
//! (the Java edition's messages end in a newline and the layout adds another).

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use chrono::Local;

use crate::paths::Dirs;

/// Old logs are kept this long, like the Java edition's log4j rule (`IfLastModified age="14d"`).
const KEEP_OLD_LOGS: Duration = Duration::from_secs(14 * 24 * 3600);
/// Lines waiting for the file to be opened, and lines kept for a log view in the window.
const QUEUE_CAPACITY: usize = 16384;
const RECENT_CAPACITY: usize = 2000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

/// One line of the launcher log.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Counts up from 1; a log view remembers the last one it showed and asks for what came after.
    pub seq: u64,
    pub time: String,
    pub thread: String,
    pub level: LogLevel,
    pub text: String,
}

impl Entry {
    /// The line exactly as it is written to the file.
    pub fn file_line(&self) -> String {
        let body = if self.text.ends_with('\n') { self.text.clone() } else { format!("{}\n", self.text) };
        format!("{} [{}] {:<5} - {}\n", self.time, self.thread, self.level.name(), body)
    }
}

#[derive(Default)]
struct State {
    file: Option<File>,
    waiting: VecDeque<Entry>,
    recent: VecDeque<Entry>,
}

static STATE: Mutex<State> = Mutex::new(State { file: None, waiting: VecDeque::new(), recent: VecDeque::new() });
static SEQ: AtomicU64 = AtomicU64::new(0);
static DEBUG: AtomicBool = AtomicBool::new(false);
static DEBUG_LEVEL: AtomicU8 = AtomicU8::new(0);
static BASE_DIR: OnceLock<String> = OnceLock::new();
static PERF: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

pub fn log_file(dirs: &Dirs) -> PathBuf {
    dirs.logs.join("blockforge.log")
}

pub fn old_logs_dir(dirs: &Dirs) -> PathBuf {
    dirs.logs.join("old")
}

/// Whether debug lines are written. Also the switch that stops the data folder's path being hidden in the log
/// (the Java edition shows `**USERSDIR**` instead, unless it runs in debug mode).
pub fn show_debug() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

pub fn set_debug(on: bool) {
    DEBUG.store(on, Ordering::Relaxed);
}

/// How chatty `debug_at` is: 0 is off, 1 is the usual debug mode, up to 5 for everything (`--debug-level`).
pub fn debug_level() -> u8 {
    DEBUG_LEVEL.load(Ordering::Relaxed)
}

pub fn set_debug_level(level: u8) {
    DEBUG_LEVEL.store(level, Ordering::Relaxed);
}

/// Start the log for this run: move the last run's log into `old/`, remove logs older than 14 days and open a
/// fresh `blockforge.log`. Lines logged before this call are written first.
pub fn init(dirs: &Dirs, debug: bool) -> std::io::Result<()> {
    let _ = BASE_DIR.set(dirs.base.to_string_lossy().to_string());
    if debug {
        set_debug(true);
        set_debug_level(debug_level().max(1));
    }
    std::fs::create_dir_all(&dirs.logs)?;
    roll_over(dirs);
    let file = File::create(log_file(dirs))?;
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.file = Some(file);
    let waiting: Vec<Entry> = state.waiting.drain(..).collect();
    for entry in waiting {
        write_entry(&mut state, &entry);
    }
    Ok(())
}

/// The previous run's log becomes `old/<date>.log` and anything older than 14 days goes.
fn roll_over(dirs: &Dirs) {
    let current = log_file(dirs);
    let old = old_logs_dir(dirs);
    if std::fs::metadata(&current).map(|m| m.len() > 0).unwrap_or(false) {
        let _ = std::fs::create_dir_all(&old);
        let name = format!("{}.log", Local::now().format("%Y-%m-%d-%H-%M-%S"));
        let mut target = old.join(&name);
        let mut n = 1;
        while target.exists() {
            target = old.join(format!("{}-{n}.log", name.trim_end_matches(".log")));
            n += 1;
        }
        if std::fs::rename(&current, &target).is_err() {
            // a different drive, or the file is held open: copy it instead
            if std::fs::copy(&current, &target).is_ok() {
                let _ = std::fs::remove_file(&current);
            }
        }
    }
    delete_older_than(&old, KEEP_OLD_LOGS);
}

fn delete_older_than(dir: &Path, age: Duration) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("log") {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|a| a > age);
        if old {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn write_entry(state: &mut State, entry: &Entry) {
    if let Some(file) = state.file.as_mut() {
        let _ = file.write_all(entry.file_line().as_bytes());
        let _ = file.flush();
    }
    state.recent.push_back(entry.clone());
    while state.recent.len() > RECENT_CAPACITY {
        state.recent.pop_front();
    }
}

/// Hide the data folder's path, like the Java edition does unless it is in debug mode.
fn scrub(text: &str) -> String {
    if show_debug() {
        return text.to_string();
    }
    match BASE_DIR.get() {
        Some(base) if !base.is_empty() => text.replace(base.as_str(), "**USERSDIR**"),
        _ => text.to_string(),
    }
}

pub fn log(level: LogLevel, text: &str) {
    let thread = std::thread::current().name().unwrap_or("worker").to_string();
    let entry = Entry {
        seq: SEQ.fetch_add(1, Ordering::Relaxed) + 1,
        time: Local::now().format("%H:%M:%S%.3f").to_string(),
        thread,
        level,
        text: scrub(text),
    };
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.file.is_some() {
        write_entry(&mut state, &entry);
    } else {
        state.recent.push_back(entry.clone());
        while state.recent.len() > RECENT_CAPACITY {
            state.recent.pop_front();
        }
        if state.waiting.len() < QUEUE_CAPACITY {
            state.waiting.push_back(entry);
        }
    }
}

pub fn info(text: impl AsRef<str>) {
    log(LogLevel::Info, text.as_ref());
}

pub fn warn(text: impl AsRef<str>) {
    log(LogLevel::Warn, text.as_ref());
}

pub fn error(text: impl AsRef<str>) {
    log(LogLevel::Error, text.as_ref());
}

/// Written only in debug mode.
pub fn debug(text: impl AsRef<str>) {
    if show_debug() {
        log(LogLevel::Debug, text.as_ref());
    }
}

/// Written only when the debug level is at least `level`.
pub fn debug_at(level: u8, text: impl AsRef<str>) {
    if debug_level() >= level {
        debug(text);
    }
}

/// The equivalent of the Java edition's `logStackTrace(message, e)`: the message, then the whole chain of causes.
pub fn error_chain(context: &str, err: &anyhow::Error) {
    error(context);
    error(format!("{err:#}"));
}

/// Lines logged after number `after` (0 for all that are still remembered), oldest first.
pub fn recent(after: u64) -> Vec<Entry> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.recent.iter().filter(|e| e.seq > after).cloned().collect()
}

/// Delete the logs of earlier runs (the Tools page's "Clear logs"). The log of this run stays.
pub fn clear_old_logs(dirs: &Dirs) -> usize {
    let mut deleted = 0;
    if let Ok(rd) = std::fs::read_dir(old_logs_dir(dirs)) {
        for e in rd.flatten() {
            let path = e.path();
            let ok = if path.is_dir() {
                std::fs::remove_dir_all(&path).is_ok()
            } else {
                std::fs::remove_file(&path).is_ok()
            };
            if ok {
                deleted += 1;
            }
        }
    }
    deleted
}

// ---------------------------------------------------------------------------------------------------------
// The Java edition's PerformanceManager: with debug on, measure how long a named step takes.

pub fn perf_start(name: &str) {
    if show_debug() {
        PERF.lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(name.to_string(), Instant::now());
    }
}

pub fn perf_end(name: &str) {
    if !show_debug() {
        return;
    }
    let started = PERF.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(name));
    if let Some(t) = started {
        debug_at(5, format!("{name} took {} ms", t.elapsed().as_millis()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dirs(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("blockforge-logs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        Dirs::at(base)
    }

    #[test]
    fn lines_look_like_the_java_edition() {
        let e = Entry {
            seq: 1,
            time: "00:29:12.924".into(),
            thread: "main".into(),
            level: LogLevel::Info,
            text: "Organising filesystem".into(),
        };
        assert_eq!(e.file_line(), "00:29:12.924 [main] INFO  - Organising filesystem\n\n");
        let w = Entry { level: LogLevel::Error, text: "bad\n".into(), ..e };
        assert_eq!(w.file_line(), "00:29:12.924 [main] ERROR - bad\n\n");
    }

    #[test]
    fn last_run_is_rolled_into_old_and_stale_logs_go() {
        let dirs = temp_dirs("roll");
        std::fs::create_dir_all(old_logs_dir(&dirs)).unwrap();
        std::fs::write(log_file(&dirs), "previous run\n").unwrap();
        let stale = old_logs_dir(&dirs).join("2020-01-01-00-00-00.log");
        std::fs::write(&stale, "ancient").unwrap();
        let f = File::options().write(true).open(&stale).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(30 * 24 * 3600)).unwrap();
        drop(f);

        roll_over(&dirs);

        assert!(!log_file(&dirs).exists());
        assert!(!stale.exists(), "a log older than 14 days is deleted");
        let rolled: Vec<_> = std::fs::read_dir(old_logs_dir(&dirs)).unwrap().flatten().collect();
        assert_eq!(rolled.len(), 1);
        let name = rolled[0].file_name().to_string_lossy().to_string();
        assert!(name.ends_with(".log") && name.len() == "2026-10-16-12-00-00.log".len(), "{name}");
        assert_eq!(std::fs::read_to_string(rolled[0].path()).unwrap(), "previous run\n");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn clearing_removes_only_old_logs() {
        let dirs = temp_dirs("clear");
        std::fs::create_dir_all(old_logs_dir(&dirs)).unwrap();
        std::fs::write(old_logs_dir(&dirs).join("a.log"), "x").unwrap();
        std::fs::write(old_logs_dir(&dirs).join("b.log"), "x").unwrap();
        std::fs::write(log_file(&dirs), "current").unwrap();
        assert_eq!(clear_old_logs(&dirs), 2);
        assert!(log_file(&dirs).exists());
        let _ = std::fs::remove_dir_all(&dirs.base);
    }
}
