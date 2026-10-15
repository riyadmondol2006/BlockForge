//! Launching the oldest Minecraft versions (1.5.2 and older) the way the Java edition does: through a tiny wrapper
//! program that starts the game as an applet inside a window, which is how those versions find their game folder and
//! login. The wrapper is the Java edition's `legacy-launch` project, compiled; it is built into this program and
//! copied next to the libraries when needed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::instances::Instance;
use crate::paths::Dirs;

/// The compiled `legacy-launch` project (the file the Java edition keeps as `legacy-launch-jar`).
static LEGACY_LAUNCH_JAR: &[u8] = include_bytes!("../../assets/legacy-launch.jar");

/// The class the game's Java is started with instead of Minecraft's own main class.
pub const MAIN_CLASS: &str = "com.blockforge.launcher.mclauncher.legacy.LegacyMCLauncher";

/// Where the wrapper lives (`libraries/launcher/legacy-launch.jar`, as in the Java edition).
pub fn jar_path(dirs: &Dirs) -> PathBuf {
    dirs.libraries.join("launcher").join("legacy-launch.jar")
}

/// Puts the wrapper in place, replacing a copy of another size (an older one, or a damaged one).
pub fn ensure_jar(dirs: &Dirs) -> Result<PathBuf> {
    let path = jar_path(dirs);
    let current = std::fs::metadata(&path).map(|m| m.len()).ok();
    if current != Some(LEGACY_LAUNCH_JAR.len() as u64) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, LEGACY_LAUNCH_JAR)
            .with_context(|| format!("copying the legacy launcher to {}", path.display()))?;
    }
    Ok(path)
}

/// Compares dotted version numbers by their leading digits: `1.5.2` against `1.5` compares only as far as `1.5`, so
/// every 1.5.x counts as equal to 1.5. Versions with `_` or without a leading number (`b1.7.3`, `a1.2.6`, `rd-132211`)
/// never match. This is the Java edition's `Utils.matchVersion`.
pub fn match_version(version: &str, matches: &str, less_than: bool, equal: bool) -> bool {
    if version.contains('_') {
        return false;
    }
    let (Some(v), Some(m)) = (numeric_parts(version), numeric_parts(matches)) else { return false };
    let compare = compare_parts(&v, &m, m.len());
    if equal && compare == 0 {
        return true;
    }
    if less_than { compare < 0 } else { compare > 0 }
}

/// `version >= matches` or `version > matches` as whole versions (`Utils.matchWholeVersion`).
pub fn match_whole_version(version: &str, matches: &str, equal: bool) -> bool {
    let (Some(v), Some(m)) = (numeric_parts(version), numeric_parts(matches)) else { return false };
    let compare = compare_parts(&v, &m, v.len().max(m.len()));
    if equal { compare >= 0 } else { compare > 0 }
}

fn numeric_parts(version: &str) -> Option<Vec<u32>> {
    let mut parts = Vec::new();
    for part in version.split('.') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        parts.push(digits.parse().ok()?);
    }
    Some(parts)
}

fn compare_parts(a: &[u32], b: &[u32], length: usize) -> i32 {
    for i in 0..length {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    0
}

/// If this Minecraft version is old enough to need the wrapper (1.5.2 and older releases).
pub fn should_use_legacy_launch(minecraft_version: &str) -> bool {
    match_version(minecraft_version, "1.5", true, true)
}

/// If this instance is started through the wrapper: an old release, with the setting not turned off, and not on
/// Legacy Fabric (which brings its own way of starting).
pub fn uses_legacy_launch(instance: &Instance, disable_legacy_launching: bool) -> bool {
    let is_legacy_fabric = instance.loader().is_some_and(|(kind, _)| kind.eq_ignore_ascii_case("LegacyFabric"));
    if instance.file.version.kind != "release" || disable_legacy_launching || is_legacy_fabric {
        return false;
    }
    should_use_legacy_launch(instance.mc_version())
}

/// The arguments after the wrapper's main class: where the game lives, who plays, the window title and size.
pub fn wrapper_arguments(
    instance: &Instance,
    username: &str,
    session: &str,
    width: u32,
    height: u32,
    maximise: bool,
) -> Vec<String> {
    vec![
        instance.root.to_string_lossy().to_string(),
        username.to_string(),
        session.to_string(),
        format!("{} - {}", crate::NAME, instance.name()),
        width.to_string(),
        height.to_string(),
        maximise.to_string(),
    ]
}

/// Whether a path is the wrapper (for tests and the log).
pub fn is_wrapper(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == "legacy-launch.jar")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrapper_is_a_real_jar() {
        assert!(LEGACY_LAUNCH_JAR.starts_with(b"PK"));
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(LEGACY_LAUNCH_JAR)).unwrap();
        assert!(zip.by_name("com/blockforge/launcher/mclauncher/legacy/LegacyMCLauncher.class").is_ok());
        assert!(zip.by_name("net/minecraft/Launcher.class").is_ok());
    }

    #[test]
    fn version_matching_follows_the_java_edition() {
        // every 1.5.x is "1.5 or older"
        for v in ["1.5", "1.5.1", "1.5.2", "1.4.7", "1.2.5", "1.0", "1.5.2.7"] {
            assert!(should_use_legacy_launch(v), "{v}");
        }
        for v in ["1.6", "1.6.4", "1.7.10", "1.8.9", "1.12.2", "1.21.1", "26.1"] {
            assert!(!should_use_legacy_launch(v), "{v}");
        }
        // old alpha/beta/classic names and snapshots never match
        for v in ["b1.7.3", "a1.2.6", "rd-132211", "c0.30_01c", "13w16a", "1.5_01"] {
            assert!(!should_use_legacy_launch(v), "{v}");
        }
        assert!(match_whole_version("3.3.3", "3.3.1", true));
        assert!(match_whole_version("3.3.1", "3.3.1", true));
        assert!(!match_whole_version("3.3.1", "3.3.1", false));
        assert!(!match_whole_version("3.2.9", "3.3.1", true));
        assert!(match_whole_version("3.10", "3.3.1", true));
    }

    #[test]
    fn the_jar_is_copied_once() {
        let dir = std::env::temp_dir().join(format!("bf-legacy-{}", std::process::id()));
        let dirs = Dirs::at(&dir);
        let path = ensure_jar(&dirs).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), LEGACY_LAUNCH_JAR.len() as u64);
        // a damaged copy is replaced
        std::fs::write(&path, b"junk").unwrap();
        ensure_jar(&dirs).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), LEGACY_LAUNCH_JAR);
        assert!(is_wrapper(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
