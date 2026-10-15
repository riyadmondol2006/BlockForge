//! Small helpers shared by the FTB installer, the importers and the exporters: making a ready-to-run
//! Minecraft + loader, safe paths from files other people wrote, and folder copying.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::install::{self, Progress};
use crate::instances::{self, LoaderVersion};
use crate::loaders::{self, Loader};
use crate::model::VersionJson;
use crate::paths::Dirs;

/// Download Minecraft `mc` and the loader, ready to be written into an `instance.json`. This is the same first
/// half that [`instances::create`] does, for callers that already have an instance folder (updates, re-imports).
pub async fn prepare_version(
    dirs: &Dirs,
    mc: &str,
    loader: Loader,
    loader_version: Option<&str>,
    progress: &Arc<Progress>,
) -> Result<(VersionJson, Option<LoaderVersion>)> {
    progress.stage(format!("Reading Minecraft {mc}"));
    let vanilla = install::vanilla_json(dirs, mc).await?;
    let (mut merged, info) = loaders::install(dirs, loader, mc, loader_version, &vanilla, progress).await?;
    if let Some(jv) = &merged.java_version {
        if !jv.component.is_empty() {
            crate::java::ensure_runtime(dirs, &jv.component, progress).await?;
        }
    }
    install::ensure_game_files(dirs, &merged, progress, false).await?;
    merged.id = mc.to_string();
    Ok((merged, info))
}

/// The loader version from `list` (what [`loaders::versions`] returned) that `wanted` names, if there is one.
/// Other launchers write Forge as `47.2.0`; the list has `1.20.1-47.2.0`.
pub fn match_loader_version(loader: Loader, mc: &str, wanted: &str, list: &[String]) -> Option<String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    list.iter()
        .find(|v| {
            v.as_str() == wanted
                || (loader == Loader::Forge
                    && v.strip_prefix(&format!("{mc}-"))
                        .is_some_and(|rest| rest == wanted || rest.starts_with(&format!("{wanted}-"))))
        })
        .cloned()
}

/// A path written by someone else (a modpack, an archive), kept only if it stays inside the folder it is joined to.
pub fn safe_relative(path: &str) -> Option<PathBuf> {
    let cleaned = path.replace('\\', "/");
    let p = Path::new(cleaned.trim_start_matches("./"));
    if cleaned.is_empty() || p.is_absolute() || cleaned.starts_with('/') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() { None } else { Some(out) }
}

/// A name for a new instance that no existing instance has: `wanted`, or `wanted (2)`, `wanted (3)` ...
/// A name with no letters or digits is replaced by "Imported instance" (the Java edition's rule).
pub fn unique_name(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = if wanted.chars().any(|c| c.is_ascii_alphanumeric()) {
        wanted.trim().to_string()
    } else {
        "Imported instance".to_string()
    };
    if !taken(&base) {
        return base;
    }
    (2..).map(|i| format!("{base} ({i})")).find(|c| !taken(c)).unwrap_or(base)
}

/// Whether an instance with this name (ignoring case) or the same folder name exists.
pub fn instance_name_taken(dirs: &Dirs, name: &str) -> bool {
    if dirs.instances.join(crate::paths::folder_name(name)).exists() {
        return true;
    }
    instances::load_all(dirs).iter().any(|i| i.name().eq_ignore_ascii_case(name))
}

/// Copy `from` into `to`, merging with what is there. Symbolic links are skipped, never followed.
pub fn copy_dir_merge(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))?.flatten() {
        let ty = e.file_type()?;
        let dest = to.join(e.file_name());
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            copy_dir_merge(&e.path(), &dest)?;
        } else if ty.is_file() {
            std::fs::copy(e.path(), &dest).with_context(|| format!("copying {}", e.path().display()))?;
        }
    }
    Ok(())
}

/// Remove `dir` and its parents up to (not including) `root` while they are empty.
pub fn remove_empty_parents(root: &Path, dir: &Path) {
    let mut cur = dir.to_path_buf();
    while cur != root && cur.starts_with(root) {
        let empty = std::fs::read_dir(&cur).map(|mut r| r.next().is_none()).unwrap_or(false);
        if !empty || std::fs::remove_dir(&cur).is_err() {
            break;
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => break,
        }
    }
}

/// The player's downloads folder, where browser downloads of manually-fetched files end up.
pub fn downloads_dir() -> PathBuf {
    let home =
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    home.join("Downloads")
}

/// Size in bytes of everything under `dir` (files only).
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_paths_stay_inside() {
        assert_eq!(safe_relative("./mods/a.jar"), Some(PathBuf::from("mods/a.jar")));
        assert_eq!(safe_relative("config\\x\\y.toml"), Some(PathBuf::from("config/x/y.toml")));
        assert_eq!(safe_relative("../evil"), None);
        assert_eq!(safe_relative("a/../../evil"), None);
        assert_eq!(safe_relative("/etc/passwd"), None);
        assert_eq!(safe_relative(""), None);
    }

    #[test]
    fn loader_versions_are_matched_to_the_list() {
        let forge =
            vec!["1.20.1-47.2.0".to_string(), "1.20.1-47.1.0".to_string(), "1.7.10-10.13.4.1614-1.7.10".to_string()];
        assert_eq!(match_loader_version(Loader::Forge, "1.20.1", "47.2.0", &forge).as_deref(), Some("1.20.1-47.2.0"));
        assert_eq!(
            match_loader_version(Loader::Forge, "1.20.1", "1.20.1-47.1.0", &forge).as_deref(),
            Some("1.20.1-47.1.0")
        );
        assert_eq!(
            match_loader_version(Loader::Forge, "1.7.10", "10.13.4.1614", &forge).as_deref(),
            Some("1.7.10-10.13.4.1614-1.7.10")
        );
        assert_eq!(match_loader_version(Loader::Forge, "1.20.1", "99.0.0", &forge), None);
        let fabric = vec!["0.16.0".to_string()];
        assert_eq!(match_loader_version(Loader::Fabric, "1.21", "0.16.0", &fabric).as_deref(), Some("0.16.0"));
        assert_eq!(match_loader_version(Loader::Fabric, "1.21", "", &fabric), None);
    }

    #[test]
    fn names_are_made_unique() {
        let taken = |n: &str| ["My Pack", "My Pack (2)"].contains(&n);
        assert_eq!(unique_name("Other", taken), "Other");
        assert_eq!(unique_name("My Pack", taken), "My Pack (3)");
        assert_eq!(unique_name("***", taken), "Imported instance");
    }
}
