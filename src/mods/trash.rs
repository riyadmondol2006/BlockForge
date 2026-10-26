//! Deleting files the way the `useRecycleBin` setting asks: into the system's recycle bin when it is on, for good
//! when it is off. If the bin can't be used the file is deleted, as the Java edition does, so removing a mod
//! always removes it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Deletes `path` (a file or a folder). With `recycle` the system's recycle bin is tried first.
/// Symbolic links are never followed or removed: a link might point at something the player cares about.
pub fn delete(path: &Path, recycle: bool) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).with_context(|| format!("{} doesn't exist", path.display()))?;
    if meta.file_type().is_symlink() {
        bail!("Not deleting {} because it is a link", path.display());
    }
    if recycle && move_to_trash(path).is_ok() {
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::remove_dir_all(path).with_context(|| format!("deleting {}", path.display()))
    } else {
        std::fs::remove_file(path).with_context(|| format!("deleting {}", path.display()))
    }
}

/// Moves `path` into the system's recycle bin.
pub fn move_to_trash(path: &Path) -> Result<()> {
    let abs = std::fs::canonicalize(path).with_context(|| format!("looking up {}", path.display()))?;
    if cfg!(target_os = "windows") {
        windows_recycle(&abs)
    } else if cfg!(target_os = "macos") {
        let home = home_dir().context("no home folder")?;
        let trash = home.join(".Trash");
        std::fs::create_dir_all(&trash)?;
        move_unique(&abs, &trash).map(|_| ())
    } else {
        freedesktop_trash(&abs)
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// The XDG trash: `files/` holds the file and `info/` a small note saying where it came from, so a file manager
/// can put it back.
fn freedesktop_trash(abs: &Path) -> Result<()> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".local").join("share")))
        .context("no home folder")?;
    let trash = data.join("Trash");
    let (files, info) = (trash.join("files"), trash.join("info"));
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = abs.file_name().context("no file name")?.to_string_lossy().to_string();
    let target_name = unique_name(&name, |n| files.join(n).exists() || info.join(format!("{n}.trashinfo")).exists());
    let note = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(&abs.to_string_lossy()),
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
    );
    std::fs::write(info.join(format!("{target_name}.trashinfo")), note)?;
    if let Err(e) = move_path(abs, &files.join(&target_name)) {
        let _ = std::fs::remove_file(info.join(format!("{target_name}.trashinfo")));
        return Err(e);
    }
    Ok(())
}

fn windows_recycle(abs: &Path) -> Result<()> {
    let call = if abs.is_dir() { "DeleteDirectory" } else { "DeleteFile" };
    // the path goes in through the environment, never into the script's text: PowerShell also ends a quoted
    // string at the curly quotes (’ ‘ ‚ ‛), so a file called `a’;Start-Process calc;’.zip` would otherwise run code
    let script = format!(
        "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::{call}($env:BLOCKFORGE_RECYCLE_PATH, 'OnlyErrorDialogs', 'SendToRecycleBin')"
    );
    let status = crate::platform::command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .env("BLOCKFORGE_RECYCLE_PATH", without_verbatim_prefix(&abs.to_string_lossy()))
        .status()?;
    if status.success() { Ok(()) } else { bail!("the recycle bin refused {}", abs.display()) }
}

/// `C:\x` for `\\?\C:\x` and `\\server\share` for `\\?\UNC\server\share`: Windows' `canonicalize` adds that
/// prefix, and the recycle bin doesn't take paths that have it.
fn without_verbatim_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

fn move_unique(from: &Path, dir: &Path) -> Result<PathBuf> {
    let name = from.file_name().context("no file name")?.to_string_lossy().to_string();
    let target = dir.join(unique_name(&name, |n| dir.join(n).exists()));
    move_path(from, &target)?;
    Ok(target)
}

/// Rename, or copy and delete when the two places are on different drives.
fn move_path(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_dir(from, to)?;
        std::fs::remove_dir_all(from)?;
    } else {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// "mod.jar" becomes "mod (2).jar", "mod (3).jar" and so on until the name is free.
pub fn unique_name(file_name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(file_name) {
        return file_name.to_string();
    }
    let (base, ext) = match file_name.rfind('.') {
        Some(dot) if dot > 0 => (&file_name[..dot], &file_name[dot..]),
        _ => (file_name, ""),
    };
    for i in 2..1000 {
        let candidate = format!("{base} ({i}){ext}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{base} ({}){ext}", chrono::Utc::now().timestamp_millis())
}

/// Percent-encodes a path for a `.trashinfo` file: everything except letters, digits, `/` and `-_.~`.
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_count_up() {
        assert_eq!(unique_name("sodium.jar", |_| false), "sodium.jar");
        assert_eq!(unique_name("sodium.jar", |n| n == "sodium.jar"), "sodium (2).jar");
        assert_eq!(unique_name("sodium.jar", |n| n == "sodium.jar" || n == "sodium (2).jar"), "sodium (3).jar");
        assert_eq!(unique_name("README", |n| n == "README"), "README (2)");
    }

    #[test]
    fn paths_are_percent_encoded() {
        assert_eq!(percent_encode("/home/a b/ü.jar"), "/home/a%20b/%C3%BC.jar");
    }

    #[test]
    fn deleting_for_good_removes_files_and_folders_but_not_links() {
        let dir = std::env::temp_dir().join(format!("blockforge-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.jar"), "x").unwrap();
        std::fs::write(dir.join("sub/b.jar"), "x").unwrap();
        delete(&dir.join("a.jar"), false).unwrap();
        delete(&dir.join("sub"), false).unwrap();
        assert!(!dir.join("a.jar").exists() && !dir.join("sub").exists());
        assert!(delete(&dir.join("missing"), false).is_err());
        #[cfg(unix)]
        {
            std::fs::write(dir.join("real"), "x").unwrap();
            std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
            assert!(delete(&dir.join("link"), false).is_err());
            assert!(dir.join("real").exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_freedesktop_trash_keeps_a_note_for_restoring() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("blockforge-trash2-{}", std::process::id()));
        let data = dir.join("data");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&data).unwrap();
        let file = dir.join("gone.jar");
        std::fs::write(&file, "x").unwrap();
        // XDG_DATA_HOME is process wide; this is the only test that sets it
        unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
        move_to_trash(&file).unwrap();
        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        assert!(!file.exists());
        assert!(data.join("Trash/files/gone.jar").exists());
        let note = std::fs::read_to_string(data.join("Trash/info/gone.jar.trashinfo")).unwrap();
        assert!(note.starts_with("[Trash Info]\nPath=") && note.contains("DeletionDate="));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
