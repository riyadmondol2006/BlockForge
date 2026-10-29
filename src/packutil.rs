//! Small helpers shared by the FTB installer, the importers and the exporters: making a ready-to-run
//! Minecraft + loader, safe paths from files other people wrote, and folder copying.

use std::io::{self, Read, Write};
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

/// Whether a file name from someone else (a mod site, a modpack) is one plain name in every operating system: no
/// folders, no `..`, no drive letter or Windows stream (`C:x.jar`, `a.jar:evil`), no control characters.
pub fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
        && !name.chars().any(char::is_control)
        && Path::new(name).components().all(|c| matches!(c, Component::Normal(_)))
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

/// Copy a file out of a zip someone else made (a modpack, an export) without holding its CRC-32 against it: some
/// packs (RLCraft's Technic zip) list out-of-date checksums for jars that are whole, and the Java launchers never
/// check them. A file that comes out shorter or longer than the zip says is still an error.
pub fn copy_zip_entry<R: Read + ?Sized, W: Write + ?Sized>(
    entry: &mut zip::read::ZipFile<'_, R>,
    out: &mut W,
) -> io::Result<u64> {
    let size = entry.size();
    let mut counting = Counting { inner: out, written: 0 };
    match io::copy(entry, &mut counting) {
        // the zip crate says so only after the last byte, so everything has been written by then
        Err(e) if counting.written == size && is_bad_checksum(&e) => Ok(size),
        r => r,
    }
}

/// A file out of a zip someone else made, as bytes, its CRC-32 not checked (see [`copy_zip_entry`]).
pub fn zip_entry_bytes<R: Read + ?Sized>(entry: &mut zip::read::ZipFile<'_, R>) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(entry.size().min(64 << 20) as usize);
    copy_zip_entry(entry, &mut out)?;
    Ok(out)
}

/// A text file out of a zip someone else made, its CRC-32 not checked (see [`copy_zip_entry`]).
pub fn zip_entry_text<R: Read + ?Sized>(entry: &mut zip::read::ZipFile<'_, R>) -> io::Result<String> {
    String::from_utf8(zip_entry_bytes(entry)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn is_bad_checksum(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::InvalidData && e.to_string() == "Invalid checksum"
}

struct Counting<'a, W: Write + ?Sized> {
    inner: &'a mut W,
    written: u64,
}

impl<W: Write + ?Sized> Write for Counting<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod zip_entry_tests {
    use std::io::{Cursor, Read, Write};

    use super::*;

    #[test]
    fn a_file_with_an_out_of_date_checksum_is_still_read() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        w.start_file("mods/a.jar", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&b"the whole jar, every byte of it ".repeat(64)).unwrap();
        let mut bytes = w.finish().unwrap().into_inner();
        let crc = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap().by_index(0).unwrap().crc32().to_le_bytes();
        // every copy of the checksum (the file's own header and the zip's index) now says something else
        let mut from = 0;
        while let Some(at) = bytes[from..].windows(4).position(|w| w == crc) {
            bytes[from + at] ^= 0xff;
            from += at + 4;
        }
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let err = z.by_index(0).unwrap().read_to_end(&mut Vec::new()).unwrap_err();
        assert!(is_bad_checksum(&err), "{err}");
        assert_eq!(
            zip_entry_bytes(&mut z.by_index(0).unwrap()).unwrap(),
            b"the whole jar, every byte of it ".repeat(64)
        );
    }
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

#[cfg(test)]
mod file_name_tests {
    use super::*;

    #[test]
    fn file_names_from_mod_sites_must_be_one_plain_name() {
        for ok in ["sodium-fabric-0.5.8+mc1.20.1.jar", "Complementary Shaders r5.zip", "日本語.jar", "a (2).jar"] {
            assert!(safe_file_name(ok), "{ok}");
        }
        for bad in ["", ".", "..", "../x.jar", "a/b.jar", "a\\b.jar", "C:x.jar", "x.jar:ads", "a\0b", "a\nb.jar", "/x"]
        {
            assert!(!safe_file_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn archive_paths_cannot_reach_outside_on_any_system() {
        for bad in ["/Users/Public/evil.bat", "\\evil.bat", "..\\..\\x", "a/../../x", "./../x", "//server/share/x"] {
            assert!(safe_relative(bad).is_none(), "{bad}");
        }
        assert_eq!(safe_relative("overrides/../mods/a.jar"), None);
        assert_eq!(safe_relative("mods/./a.jar"), Some(PathBuf::from("mods/a.jar")));
    }
}
