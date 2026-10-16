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

/// Writes the agent file if it isn't there or is another version, and returns where it is. An error when its path has
/// an `=` or a `,`: `-javaagent:` ends the path at the first `=`, and the JVM then doesn't start at all; a `,` would also
/// be in the class list's path, which the agent can't be given (see [`options`]), so it would only cost time.
pub fn ensure_jar(dirs: &Dirs) -> Result<PathBuf> {
    let path = jar_path(dirs);
    if path.to_string_lossy().contains(['=', ',']) {
        anyhow::bail!("its folder's path has an '=' or ',' in it, which Java can't take ({})", path.display());
    }
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

/// The agent's options. The agent splits them at commas, so a path with one means no class list is used. `key`
/// describes what the agent can't see itself (see [`setup_key`]); a list made with another key isn't used.
/// `spinning_disk`: the game's files are on a hard disk (see `disk`). Otherwise the agent is told so, and doesn't time
/// reads to find out (a busy SSD can look slow for a moment); on a hard disk the launcher reads the files once before
/// the start, and the agent checks for itself.
pub fn options(class_list: &Path, key: &str, spinning_disk: bool) -> String {
    if class_list.to_string_lossy().contains(',') {
        return "quiet".to_string();
    }
    let mut out = format!("preload={}", class_list.display());
    let key: String = key.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !key.is_empty() {
        out.push_str(&format!(",key={key}"));
    }
    if !spinning_disk {
        out.push_str(",disk=ssd");
    }
    out
}

/// A short description of the instance's mods (names, sizes and dates of the files in `mods`). The agent checks the
/// Java and the class path itself; mods change which library classes a game loads without changing either, so a new
/// key makes the next start write a fresh class list.
pub fn setup_key(game_dir: &Path) -> String {
    let mut entries: Vec<String> = std::fs::read_dir(game_dir.join("mods"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs());
                    Some(format!("{}:{}:{modified}", e.file_name().to_string_lossy(), meta.len()))
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    // FNV-1a: stable across runs and versions of Rust, unlike the standard hasher
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in entries.join("\n").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("m{h:016x}")
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
        assert_eq!(options(Path::new("/x/a.classes"), "", true), "preload=/x/a.classes");
        assert_eq!(options(Path::new("/x/a.classes"), "m12ab", true), "preload=/x/a.classes,key=m12ab");
        assert_eq!(options(Path::new("/x/a.classes"), "m12ab", false), "preload=/x/a.classes,key=m12ab,disk=ssd");
        // nothing in the key can break the agent's option list
        assert_eq!(options(Path::new("/x/a.classes"), "a,b=c d", true), "preload=/x/a.classes,key=abcd");
        assert!(!options(Path::new("/x,y/a.classes"), "m1", false).contains("preload"));
        assert_eq!(java_argument(Path::new("/a/b.jar"), "preload=/x"), "-javaagent:/a/b.jar=preload=/x");
        // Windows paths go through as they are: the agent doesn't treat backslashes, spaces or colons specially
        assert_eq!(
            options(Path::new(r"C:\Users\Jo Ann\AppData\Roaming\BlockForge\cache\boost\a.classes"), "m1", true),
            r"preload=C:\Users\Jo Ann\AppData\Roaming\BlockForge\cache\boost\a.classes,key=m1"
        );
    }

    #[test]
    fn the_setup_key_follows_the_mods() {
        let base = std::env::temp_dir().join(format!("bf-boost-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // no mods folder at all
        let none = setup_key(&base);
        assert_eq!(none, setup_key(&base));
        std::fs::create_dir_all(base.join("mods")).unwrap();
        std::fs::write(base.join("mods").join("a.jar"), b"one").unwrap();
        let one = setup_key(&base);
        assert_ne!(one, none);
        assert_eq!(one, setup_key(&base));
        assert!(one.chars().all(|c| c.is_ascii_alphanumeric()));
        std::fs::write(base.join("mods").join("b.jar"), b"two").unwrap();
        assert_ne!(setup_key(&base), one);
        std::fs::remove_file(base.join("mods").join("b.jar")).unwrap();
        assert_eq!(setup_key(&base), one);
        std::fs::write(base.join("mods").join("a.jar"), b"other").unwrap();
        assert_ne!(setup_key(&base), one);
        let _ = std::fs::remove_dir_all(base);
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
    fn a_path_java_cant_take_is_refused() {
        // the JVM reads "-javaagent:/a=b/x.jar" as the jar "/a" with options "b/x.jar", and doesn't start
        for name in ["bf-boost-a=b", "bf-boost-a,b"] {
            let base = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            let dirs = Dirs::at(&base);
            let e = ensure_jar(&dirs).unwrap_err();
            assert!(e.to_string().contains("'='"));
            assert!(!jar_path(&dirs).exists());
            let _ = std::fs::remove_dir_all(base);
        }
        // spaces and other characters are fine
        let base = std::env::temp_dir().join(format!("bf boost ä-{}", std::process::id()));
        assert!(ensure_jar(&Dirs::at(&base)).is_ok());
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
