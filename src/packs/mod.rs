//! Modpacks from ATLauncher's own list and from Technic, installed the way the Java edition installs them, into the
//! same `instance.json` format.
//!
//! * [`atlauncher`] the pack list, pack codes, update checks and the checks before an install
//! * [`technic`] Technic search, modpack details, Solder builds and updates
//! * [`json`] the pack version definition (`Configs.json`) both are turned into
//! * [`choose`] which optional mods get installed
//! * [`engine`] the installer: downloads, placing files, loaders, actions, configs, `instance.json`
//! * [`loader`] getting a pack's loader (Forge, Fabric, Quilt, NeoForge, Legacy Fabric) in place
//! * [`dl`], [`fsx`], [`snapshot`] downloading, file helpers and the backup made before an update

pub mod atlauncher;
pub mod choose;
pub mod dl;
pub mod engine;
pub mod fsx;
pub mod json;
pub mod loader;
pub mod place;
pub mod server;
pub mod snapshot;
pub mod technic;

use std::path::{Path, PathBuf};

use crate::paths::Dirs;

/// ATLauncher's download server for packs, configs, images and files that are not on the pack makers' sites.
pub const DOWNLOAD_SERVER: &str = "https://download.nodecdn.net/containers/atl";

/// Letters and digits only. Pack and version names become folder names and addresses this way.
pub fn safe_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Whether a path below an instance is the player's own data that a pack must never delete or overwrite: worlds
/// (`saves`, and `world…`/`DIM…` for old versions and servers) and `instance.json`. The first part of the path decides,
/// ignoring case.
pub fn is_player_data(rel: &std::path::Path) -> bool {
    let Some(first) = rel.components().next() else { return true };
    let first = first.as_os_str().to_string_lossy().to_ascii_lowercase();
    first == "saves" || first.starts_with("world") || first.starts_with("dim") || first == "instance.json"
}

/// `<data>/downloads`: every mod file fetched for a pack is kept here, so installing again does not download it
/// again. Same place as the Java edition.
pub fn downloads_dir(dirs: &Dirs) -> PathBuf {
    dirs.base.join("downloads")
}

/// `<data>/downloads/technic`
pub fn technic_downloads_dir(dirs: &Dirs) -> PathBuf {
    downloads_dir(dirs).join("technic")
}

/// `<data>/servers`
pub fn servers_dir(dirs: &Dirs) -> PathBuf {
    dirs.base.join("servers")
}

/// `<data>/configs/common`: files copied into every pack instance after it is installed.
pub fn common_dir(dirs: &Dirs) -> PathBuf {
    dirs.configs.join("common")
}

/// The folder a browser puts downloads in. A value from the settings (`customDownloadsPath`) wins.
pub fn user_downloads_dir(custom: Option<&str>) -> Option<PathBuf> {
    if let Some(c) = custom.filter(|c| !c.is_empty()) {
        return Some(PathBuf::from(c));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)?;
    if cfg!(target_os = "linux") {
        // the folder the desktop calls Downloads, which may have another name or place
        if let Ok(text) = std::fs::read_to_string(home.join(".config").join("user-dirs.dirs")) {
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("XDG_DOWNLOAD_DIR=") {
                    let p = rest.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
                    if !p.is_empty() {
                        return Some(PathBuf::from(p));
                    }
                }
            }
        }
    }
    Some(home.join("Downloads"))
}

/// Write through a temporary file so a crash never leaves half a file behind.
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The game jar to start an instance with. Very old packs (and Technic packs that bring their own Forge) ship
/// `bin/modpack.jar`, which has to be joined with Mojang's game jar into `bin/minecraft.jar`; that is what the Java
/// edition builds before every start. Returns `client_jar` unchanged for every other instance. The joined jar is
/// only rebuilt when one of its parts is newer.
pub fn custom_game_jar(root: &Path, client_jar: &Path) -> anyhow::Result<PathBuf> {
    let modpack = root.join("bin").join("modpack.jar");
    if !modpack.is_file() {
        return Ok(client_jar.to_path_buf());
    }
    let out = root.join("bin").join("minecraft.jar");
    let newest_input =
        [&modpack, client_jar].iter().filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).max();
    let built = std::fs::metadata(&out).and_then(|m| m.modified()).ok();
    if let (Some(built), Some(newest)) = (built, newest_input) {
        if built >= newest {
            return Ok(out);
        }
    }
    let _ = std::fs::remove_file(&out);
    fsx::combine_jars(client_jar, &modpack, &out)?;
    Ok(out)
}

/// Where the Java edition's `Mod.getFile()` of a downloaded mod lives, for tests and callers.
pub fn downloaded_file(dirs: &Dirs, file: &str) -> PathBuf {
    downloads_dir(dirs).join(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names_keep_letters_and_digits() {
        assert_eq!(safe_name("Magicality: The Next Dimension"), "MagicalityTheNextDimension");
        assert_eq!(safe_name("Hot & Steamy"), "HotSteamy");
        assert_eq!(safe_name("1.21.11-1"), "121111");
    }

    #[test]
    fn folders_sit_under_the_data_folder() {
        let d = Dirs::at("/data/bf");
        assert_eq!(downloads_dir(&d), Path::new("/data/bf/downloads"));
        assert_eq!(technic_downloads_dir(&d), Path::new("/data/bf/downloads/technic"));
        assert_eq!(common_dir(&d), Path::new("/data/bf/configs/common"));
        assert_eq!(user_downloads_dir(Some("/x/y")), Some(PathBuf::from("/x/y")));
    }
}
