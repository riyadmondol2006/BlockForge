//! Putting a downloaded update in place and starting the new copy.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use super::plan::Kind;

/// What happened when the update was started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// A new copy (or the installer) is starting; this launcher should close now.
    Exit,
    /// This launcher stays open. `message` tells the player what to do next; `command` is something they can run.
    Stay { message: String, command: Option<String> },
}

/// Applies the downloaded `file` for this kind of install. Blocks, so call it off the window's thread.
pub fn install(file: &Path, kind: Kind) -> Result<Installed> {
    crate::logs::info(format!("Installing the launcher update from {}", file.display()));
    match kind {
        Kind::AppImage => {
            let target = std::env::var_os("APPIMAGE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .context("Couldn't find the AppImage file BlockForge runs from")?;
            replace_executable(file, &target)?;
            relaunch(&target)?;
            Ok(Installed::Exit)
        }
        Kind::WindowsPortable => {
            let exe = std::env::current_exe().context("Couldn't find the BlockForge program file")?;
            swap_portable_zip(file, &exe)?;
            relaunch(&exe)?;
            Ok(Installed::Exit)
        }
        Kind::WindowsInstaller => {
            // the setup program replaces this install, so this copy has to close right after it starts
            Command::new(file).spawn().with_context(|| format!("Couldn't start {}", file.display()))?;
            Ok(Installed::Exit)
        }
        Kind::MacApp => {
            Command::new("open").arg(file).spawn().context("Couldn't open the downloaded disk image")?;
            Ok(Installed::Exit)
        }
        Kind::Deb | Kind::Rpm => install_package(file, kind),
        Kind::PageOnly => bail!("This copy of BlockForge can't be updated automatically"),
    }
}

/// Closes this process after a short wait, in case the window didn't manage to (the installer needs us gone).
pub fn exit_soon() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        std::process::exit(0);
    });
}

/// Starts `program` as a separate process that outlives this one.
pub fn relaunch(program: &Path) -> Result<()> {
    let mut command = crate::platform::command(program);
    // a program started from inside an AppImage must not think it is that AppImage
    command.env_remove("APPDIR").env_remove("APPIMAGE").env_remove("ARGV0").env_remove("OWD");
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().with_context(|| format!("Couldn't start the new BlockForge at {}", program.display()))?;
    Ok(())
}

/// Replaces the program file `target` with `new` in one step: the new file is copied next to it, made runnable and
/// then renamed over the old one, so there is never a half-written program.
pub fn replace_executable(new: &Path, target: &Path) -> Result<()> {
    let dir = target.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = target.file_name().context("The program file has no name")?.to_string_lossy().into_owned();
    let staged = dir.join(format!(".{name}.new"));
    let result = (|| -> Result<()> {
        std::fs::copy(new, &staged).with_context(|| {
            format!(
                "BlockForge can't write to {} (is the folder read-only?). Download the new version by hand instead.",
                dir.display()
            )
        })?;
        make_executable(&staged)?;
        std::fs::rename(&staged, target).with_context(|| format!("Couldn't replace {}", target.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("Couldn't make {} runnable", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Takes the program out of the portable zip and swaps it in for `current`. The running program can be renamed but
/// not deleted on Windows, so it is renamed to `<name>.old` (removed by the next start) and the new one takes its
/// name. Returns where the old copy went.
pub fn swap_portable_zip(zip_file: &Path, current: &Path) -> Result<PathBuf> {
    let staged = sibling(current, ".new");
    if let Err(e) = extract_program(zip_file, &staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    swap_in(&staged, current)
}

/// `<path><suffix>` in the same folder, like `blockforge.exe.old`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Moves `new` into the place of `current`, keeping the old copy as `<current>.old`. Puts the old one back when the
/// second step fails.
pub fn swap_in(new: &Path, current: &Path) -> Result<PathBuf> {
    let old = sibling(current, ".old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(current, &old).with_context(|| {
        format!("Couldn't move the old BlockForge out of the way ({}). Is the folder read-only?", current.display())
    })?;
    if let Err(e) = std::fs::rename(new, current) {
        let _ = std::fs::rename(&old, current);
        let _ = std::fs::remove_file(new);
        return Err(anyhow::Error::new(e).context("Couldn't put the new BlockForge in place"));
    }
    Ok(old)
}

/// Writes the program (the `.exe` inside the zip, preferring one named for BlockForge) to `dest`.
fn extract_program(zip_file: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(zip_file).with_context(|| format!("Couldn't open {}", zip_file.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("The downloaded zip is damaged")?;
    let mut best: Option<(usize, bool)> = None;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).context("The downloaded zip is damaged")?;
        let Ok(name) = entry.name().map(|n| n.to_ascii_lowercase()) else { continue };
        if !entry.is_file() || !name.ends_with(".exe") || name.contains("unins") {
            continue;
        }
        let file_name = name.rsplit(['/', '\\']).next().unwrap_or(&name);
        let named = file_name.contains("blockforge");
        if best.is_none_or(|(_, was_named)| named && !was_named) {
            best = Some((i, named));
        }
    }
    let (index, _) = best.context("The downloaded zip doesn't contain the BlockForge program")?;
    let mut entry = archive.by_index(index)?;
    let mut out = std::fs::File::create(dest).with_context(|| format!("Couldn't write {}", dest.display()))?;
    std::io::copy(&mut entry, &mut out).context("Couldn't unpack the new BlockForge")?;
    Ok(())
}

/// Whether a program of this name can be found on the PATH.
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The package manager command that installs `file`, as the words to run. `has` says which programs exist.
pub fn package_command(kind: Kind, file: &Path, has: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    let file = file.to_string_lossy().into_owned();
    let words: &[&str] = match kind {
        Kind::Deb if has("apt-get") => &["apt-get", "install", "-y"],
        Kind::Rpm if has("dnf") => &["dnf", "install", "-y"],
        Kind::Rpm if has("zypper") => &["zypper", "--non-interactive", "install", "--allow-unsigned-rpm"],
        Kind::Rpm if has("yum") => &["yum", "install", "-y"],
        _ => return None,
    };
    Some(words.iter().map(|w| w.to_string()).chain(std::iter::once(file)).collect())
}

/// The line a player can paste into a terminal to install the package themselves.
pub fn manual_command(kind: Kind, file: &Path, has: impl Fn(&str) -> bool) -> String {
    let words = package_command(kind, file, has).unwrap_or_else(|| {
        let tool = if kind == Kind::Deb { ["apt-get", "install", "-y"] } else { ["dnf", "install", "-y"] };
        tool.iter().map(|w| w.to_string()).chain(std::iter::once(file.to_string_lossy().into_owned())).collect()
    });
    let quoted: Vec<String> = words.iter().map(|w| shell_quote(w)).collect();
    format!("sudo {}", quoted.join(" "))
}

fn shell_quote(word: &str) -> String {
    let plain = !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:+@".contains(c));
    if plain { word.to_string() } else { format!("'{}'", word.replace('\'', "'\\''")) }
}

/// Installs a `.deb` or `.rpm` with the system's package manager, asking for the password through polkit. When that
/// isn't possible, opens the file in the software installer and shows the command to run by hand.
fn install_package(file: &Path, kind: Kind) -> Result<Installed> {
    let command = manual_command(kind, file, on_path);
    let words = match package_command(kind, file, on_path) {
        Some(words) if on_path("pkexec") => words,
        _ => return Ok(open_for_manual_install(file, command)),
    };
    let output = crate::platform::command("pkexec")
        .args(&words)
        .stdin(Stdio::null())
        .output()
        .context("Couldn't start the package installer")?;
    if output.status.success() {
        relaunch(Path::new("/usr/bin/blockforge"))?;
        return Ok(Installed::Exit);
    }
    let code = output.status.code();
    let tail = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = tail.lines().rev().take(3).collect();
    crate::logs::warn(format!(
        "The package installer stopped with {code:?}: {}",
        tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
    ));
    // pkexec uses 126 when the password prompt was dismissed and 127 when it couldn't ask
    let message = if matches!(code, Some(126 | 127)) {
        "The update wasn't installed because the password was not given. Run this in a terminal to install it:"
    } else {
        "The package manager couldn't install the update. Run this in a terminal to see why and install it:"
    };
    Ok(Installed::Stay { message: message.to_string(), command: Some(command) })
}

fn open_for_manual_install(file: &Path, command: String) -> Installed {
    let _ = open::that_detached(file);
    Installed::Stay {
        message: "The update was downloaded. Open it with your software installer, or run this in a terminal:"
            .to_string(),
        command: Some(command),
    }
}

/// Removes what an earlier update left behind: the `.old` copy of a portable program, a half-staged AppImage and the
/// downloaded files. Everything is best effort.
pub fn cleanup_leftovers(temp: &Path) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(sibling(&exe, ".old"));
        let _ = std::fs::remove_file(sibling(&exe, ".new"));
    }
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()).map(PathBuf::from) {
        if let Some(name) = appimage.file_name() {
            let mut hidden = std::ffi::OsString::from(".");
            hidden.push(name);
            hidden.push(".new");
            let _ = std::fs::remove_file(appimage.with_file_name(hidden));
        }
    }
    let _ = std::fs::remove_dir_all(update_folder(temp));
}

/// Where downloaded updates are kept.
pub fn update_folder(temp: &Path) -> PathBuf {
    temp.join("update")
}
