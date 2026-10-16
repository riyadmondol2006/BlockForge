//! The fast-start helper: a small Java agent (`packaging/boost`) that the game is started with (`-javaagent`). It
//! works for vanilla and every mod loader on every Minecraft version, because it isn't a mod: it only gets things
//! ready on spare processor cores that the game would otherwise do one after the other on its own thread.
//!
//! It is on by default. A player can switch it off for all games (the `fastStart` setting) or for one instance (the
//! `fastStart` entry of the instance's launcher settings, which wins). If a game stops soon after it started while the
//! helper was on, [`may_be_the_cause`] says so, and the window can offer to turn the helper off.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::Settings;
use crate::instances::Instance;
use crate::paths::Dirs;

/// The built agent. It is part of the program, so there is nothing to install or to go missing.
static AGENT_JAR: &[u8] = include_bytes!("../../packaging/boost/blockforge-boost.jar");

/// Every line the agent prints starts with this, so it can be found in the game's log.
pub const MARKER: &str = "[BlockForge Boost]";

/// A game that stops with an error sooner than this after it started is blamed on the helper, as one of the
/// possible causes. A game that ran longer has been through the part where the helper works.
pub const EARLY_EXIT: Duration = Duration::from_secs(25);

/// What the JVM itself says when it can't use an agent.
const AGENT_FAILURES: [&str; 6] = [
    "Error opening zip file or JAR manifest missing",
    "Failed to find Premain-Class manifest attribute",
    "processing of -javaagent failed",
    "agent library failed to init: instrument",
    "Failed to load Java agent",
    "Could not find agent",
];

/// Whether the helper is on for this instance: the instance's own `fastStart` entry, otherwise the launcher-wide
/// setting.
pub fn enabled(settings: &Settings, instance: &Instance) -> bool {
    instance.file.launcher.extras.get("fastStart").and_then(|v| v.as_bool()).unwrap_or(settings.fast_start)
}

/// Whether the helper was given an instance's own switch (as opposed to following the launcher-wide setting).
pub fn instance_override(instance: &Instance) -> Option<bool> {
    instance.file.launcher.extras.get("fastStart").and_then(|v| v.as_bool())
}

/// Where the agent file is kept.
pub fn jar_path(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("blockforge-boost.jar")
}

/// Writes the agent file if it isn't there or is another version, and returns where it is.
pub fn ensure_jar(dirs: &Dirs) -> Result<PathBuf> {
    let path = jar_path(dirs);
    if std::fs::read(&path).ok().as_deref() != Some(AGENT_JAR) {
        std::fs::create_dir_all(&dirs.cache).with_context(|| format!("making {}", dirs.cache.display()))?;
        // write beside it and rename, so a game that is starting never sees half a file
        let temp = path.with_extension(format!("jar.{}", std::process::id()));
        std::fs::write(&temp, AGENT_JAR)?;
        std::fs::rename(&temp, &path)?;
    }
    Ok(path)
}

/// The file the agent keeps the names of the classes to load ahead in, one per instance.
pub fn class_list(dirs: &Dirs, instance_key: &str) -> PathBuf {
    dirs.cache.join("boost").join(format!("{instance_key}.classes"))
}

/// The agent's options. The agent splits them at commas, so a path with one means no class list is used.
pub fn options(class_list: &Path) -> String {
    if class_list.to_string_lossy().contains(',') {
        return "quiet".to_string();
    }
    format!("preload={}", class_list.display())
}

/// The Java option that loads the agent.
pub fn java_argument(jar: &Path, options: &str) -> String {
    format!("-javaagent:{}={}", jar.display(), options)
}

/// If a game that has ended could have been stopped by the helper. `code` is the exit code (None when the game was
/// killed from outside, which is the player's doing), `ran_for` how long it ran, `log` its output lines.
///
/// True when the JVM complained about the agent, when the agent printed a problem and the game then failed, or when
/// the game failed soon after it started (the helper is only one of the possible reasons, so the window words it as
/// "try turning it off").
pub fn may_be_the_cause(was_on: bool, code: Option<i32>, ran_for: Duration, log: &[String]) -> bool {
    if !was_on {
        return false;
    }
    if log.iter().any(|l| AGENT_FAILURES.iter().any(|f| l.contains(f))) {
        return true;
    }
    let failed = code.is_some_and(|c| c != 0);
    if !failed {
        return false;
    }
    let agent_problem = log.iter().any(|l| l.contains(MARKER) && l.contains("problem"));
    agent_problem || ran_for < EARLY_EXIT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_agent_is_embedded_and_is_a_jar() {
        // a zip file starts with PK
        assert!(AGENT_JAR.starts_with(b"PK"));
        assert!(AGENT_JAR.len() > 1000);
    }

    #[test]
    fn options_skip_paths_with_commas() {
        assert_eq!(options(Path::new("/x/a.classes")), "preload=/x/a.classes");
        assert!(!options(Path::new("/x,y/a.classes")).contains("preload"));
        assert_eq!(java_argument(Path::new("/a/b.jar"), "preload=/x"), "-javaagent:/a/b.jar=preload=/x");
    }

    #[test]
    fn the_jar_is_written_once_and_replaced_when_different() {
        let base = std::env::temp_dir().join(format!("bf-boost-{}", std::process::id()));
        let dirs = Dirs::at(&base);
        let path = ensure_jar(&dirs).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), AGENT_JAR);
        std::fs::write(&path, b"old").unwrap();
        ensure_jar(&dirs).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), AGENT_JAR);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn early_failures_are_blamed_only_while_the_helper_was_on() {
        let quiet = lines(&["[main/INFO]: Loading Minecraft"]);
        let short = Duration::from_secs(6);
        let long = Duration::from_secs(600);
        assert!(may_be_the_cause(true, Some(1), short, &quiet));
        assert!(!may_be_the_cause(false, Some(1), short, &quiet));
        // a normal end, or a long game that crashed later, isn't the helper's doing
        assert!(!may_be_the_cause(true, Some(0), short, &quiet));
        assert!(!may_be_the_cause(true, Some(1), long, &quiet));
        // killed from outside: the player closed it
        assert!(!may_be_the_cause(true, None, short, &quiet));
    }

    #[test]
    fn jvm_agent_errors_and_agent_problems_are_recognised() {
        let long = Duration::from_secs(600);
        let jvm = lines(&["Error opening zip file or JAR manifest missing : /x/blockforge-boost.jar"]);
        assert!(may_be_the_cause(true, Some(1), long, &jvm));
        assert!(may_be_the_cause(true, None, long, &jvm));
        assert!(!may_be_the_cause(false, Some(1), long, &jvm));
        let problem = lines(&["[BlockForge Boost] problem: opening jar files: java.lang.Error"]);
        assert!(may_be_the_cause(true, Some(1), long, &problem));
        // a problem the agent coped with doesn't blame it for a game that ended well
        assert!(!may_be_the_cause(true, Some(0), long, &problem));
        let ok = lines(&[
            "[BlockForge Boost] on (preload=/x)",
            "[BlockForge Boost] loaded 9000 of 9100 classes ahead in 900 ms",
        ]);
        assert!(!may_be_the_cause(true, Some(1), long, &ok));
    }
}
