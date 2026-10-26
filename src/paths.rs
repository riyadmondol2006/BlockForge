//! Where everything lives. Matches the Java edition's installed layout so both can share one folder.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Dirs {
    pub base: PathBuf,
    pub instances: PathBuf,
    pub libraries: PathBuf,
    pub assets: PathBuf,
    pub runtimes: PathBuf,
    pub configs: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub temp: PathBuf,
    pub loaders: PathBuf,
}

impl Dirs {
    pub fn at(base: impl Into<PathBuf>) -> Dirs {
        let base = base.into();
        Dirs {
            instances: base.join("instances"),
            libraries: base.join("libraries"),
            assets: base.join("assets"),
            runtimes: base.join("runtimes").join("minecraft"),
            configs: base.join("configs"),
            cache: base.join("cache"),
            logs: base.join("logs"),
            temp: base.join("temp"),
            loaders: base.join("loaders"),
            base,
        }
    }

    /// `BLOCKFORGE_DIR` if set, otherwise the per-user folder the Java edition's installers use.
    pub fn detect() -> Dirs {
        if let Some(dir) = std::env::var_os("BLOCKFORGE_DIR").filter(|v| !v.is_empty()) {
            return Dirs::at(dir);
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let base = if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData").join("Roaming"))
                .join("BlockForge")
        } else if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support").join("BlockForge")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local").join("share"))
                .join("blockforge")
        };
        Dirs::at(base)
    }

    /// Create every folder BlockForge needs. A failure names the folder, because "File exists" alone says nothing (it
    /// happens when a shortcut in the data folder points somewhere that is gone, such as an unplugged drive).
    pub fn ensure(&self) -> std::io::Result<()> {
        let make = |dir: &Path| {
            std::fs::create_dir_all(dir)
                .map_err(|e| std::io::Error::new(e.kind(), format!("can't use the folder {}: {e}", dir.display())))
        };
        for d in [
            &self.base,
            &self.instances,
            &self.libraries,
            &self.assets,
            &self.runtimes,
            &self.configs,
            &self.cache,
            &self.logs,
            &self.temp,
            &self.loaders,
        ] {
            make(d)?;
        }
        make(&self.assets.join("indexes"))?;
        make(&self.assets.join("objects"))?;
        make(&self.assets.join("log_configs"))?;
        Ok(())
    }

    pub fn settings_file(&self) -> PathBuf {
        self.configs.join("BlockForge.json")
    }

    pub fn accounts_file(&self) -> PathBuf {
        self.configs.join("accounts.json")
    }

    pub fn instance_dir(&self, folder: &str) -> PathBuf {
        self.instances.join(folder)
    }
}

/// The longest instance folder name, in characters. Names are kept well under the 255 bytes most file systems allow
/// (a letter outside ASCII can take 3 or 4 bytes), and short enough that Windows' 260-character paths still fit the
/// game's own files below the folder.
const MAX_FOLDER_CHARS: usize = 64;

/// Folder name for an instance called `name`: letters and digits only, the way the Java edition names them. Names
/// Windows keeps for devices (`CON`, `NUL`, `COM1` ...) get a `_` added, and very long names are cut short.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| c.is_alphanumeric()).take(MAX_FOLDER_CHARS).collect();
    if cleaned.is_empty() {
        "Instance".to_string()
    } else if is_windows_device_name(&cleaned) {
        format!("{cleaned}_")
    } else {
        cleaned
    }
}

/// Whether Windows refuses `name` as a file or folder name because it names a device, whatever the case and with
/// or without an extension (`con`, `NUL.txt`, `com1`).
pub fn is_windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches([' ']).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// A folder name based on `name` that doesn't exist yet in `parent`.
pub fn unique_folder(parent: &Path, name: &str) -> String {
    let base = folder_name(name);
    if !parent.join(&base).exists() {
        return base;
    }
    (2..).map(|n| format!("{base}{n}")).find(|candidate| !parent.join(candidate).exists()).unwrap()
}

/// Make a new, empty folder based on `name` in `parent` and return its name and path. Unlike [`unique_folder`] the
/// folder is created here, so two instances made at the same moment can't end up sharing one.
pub fn claim_unique_folder(parent: &Path, name: &str) -> std::io::Result<(String, PathBuf)> {
    std::fs::create_dir_all(parent)?;
    let base = folder_name(name);
    for n in 1u32.. {
        let candidate = if n == 1 { base.clone() } else { format!("{base}{n}") };
        let path = parent.join(&candidate);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok((candidate, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(std::io::Error::new(e.kind(), format!("can't make the folder {}: {e}", path.display())));
            }
        }
    }
    unreachable!("ran out of folder numbers")
}

/// `name` as a single, harmless path component: unchanged when it already is one (a version id such as `1.20.1` or
/// `1.20.1-forge-47.2.0`), otherwise with separators and `..` replaced, so a file from a modpack or a server can't
/// name a place outside the folder it is joined to.
pub fn safe_component(name: &str) -> String {
    let plain = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !(cfg!(windows) && name.contains(':'));
    if plain {
        return name.to_string();
    }
    let replaced: String = name.chars().map(|c| if matches!(c, '/' | '\\' | '\0' | ':') { '_' } else { c }).collect();
    match replaced.as_str() {
        "" => "_".to_string(),
        "." | ".." => replaced.replace('.', "_"),
        _ => replaced,
    }
}

/// `rel` joined to `base`, only when it stays inside `base`: no absolute paths, drive letters or `..`. For paths
/// read from files someone else wrote (version JSONs, modpacks, archives).
pub fn join_inside(base: &Path, rel: &str) -> Option<PathBuf> {
    crate::packutil::safe_relative(rel).map(|r| base.join(r))
}

/// Maven coordinates (`group:artifact:version[:classifier][@ext]`) to a repository path. `None` when the
/// coordinates are incomplete or a part of them would lead out of the libraries folder (`..`, a slash).
pub fn maven_path(coords: &str) -> Option<String> {
    let (coords, ext) = match coords.split_once('@') {
        Some((c, e)) => (c, e),
        None => (coords, "jar"),
    };
    let mut parts = coords.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    let classifier = parts.next();
    let plain = |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', '\0']);
    let classifier_ok = classifier.is_none_or(plain);
    if !(plain(artifact) && plain(version) && plain(ext) && classifier_ok && group.split('.').all(plain)) {
        return None;
    }
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!("{}/{artifact}/{version}/{file}", group.replace('.', "/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_paths() {
        assert_eq!(maven_path("org.ow2.asm:asm:9.11").unwrap(), "org/ow2/asm/asm/9.11/asm-9.11.jar");
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-linux").unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar"
        );
        assert_eq!(maven_path("a.b:c:1@zip").unwrap(), "a/b/c/1/c-1.zip");
    }

    #[test]
    fn folder_names() {
        assert_eq!(folder_name("Minecraft 26.3 with Fabric"), "Minecraft263withFabric");
        assert_eq!(folder_name("!!!"), "Instance");
    }

    #[test]
    fn folder_names_windows_cannot_make_get_a_suffix() {
        for name in ["CON", "con", "Nul", "aux", "PRN", "COM1", "lpt9", "C O N"] {
            let f = folder_name(name);
            assert!(f.ends_with('_'), "{name} -> {f}");
            assert!(!is_windows_device_name(&f), "{name} -> {f}");
        }
        // only the exact device names: longer names and COM0 are fine
        for name in ["Console", "Null", "COM10", "COM0", "LPT", "Auxiliary"] {
            assert!(!folder_name(name).ends_with('_'), "{name}");
        }
        assert!(is_windows_device_name("nul.txt"));
        assert!(is_windows_device_name("CON "));
    }

    #[test]
    fn very_long_names_are_cut_to_a_size_every_file_system_takes() {
        let long = "a".repeat(5000);
        assert_eq!(folder_name(&long).chars().count(), MAX_FOLDER_CHARS);
        // letters outside ASCII count as one each and stay whole
        let wide = "日本語".repeat(100);
        let f = folder_name(&wide);
        assert_eq!(f.chars().count(), MAX_FOLDER_CHARS);
        assert!(f.len() < 255, "{} bytes", f.len());
        // emoji and symbols are dropped like other punctuation
        assert_eq!(folder_name("🎮 My World! 🔥"), "MyWorld");
    }

    #[test]
    fn claimed_folders_are_never_shared() {
        let parent = std::env::temp_dir().join(format!("bf-claim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        let (a, pa) = claim_unique_folder(&parent, "Pack").unwrap();
        let (b, pb) = claim_unique_folder(&parent, "Pack").unwrap();
        let (c, _) = claim_unique_folder(&parent, "P.a.c.k").unwrap();
        assert_eq!((a.as_str(), b.as_str(), c.as_str()), ("Pack", "Pack2", "Pack3"));
        assert!(pa.is_dir() && pb.is_dir());
        // many threads asking for the same name at once each get their own folder
        let names: Vec<String> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..16).map(|_| s.spawn(|| claim_unique_folder(&parent, "Race").unwrap().0)).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), 16, "{names:?}");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn maven_coordinates_that_would_leave_the_folder_are_refused() {
        assert_eq!(maven_path("../../evil:x:1"), None);
        assert_eq!(maven_path("a.b:..:1"), None);
        assert_eq!(maven_path("a.b:c:../../../x"), None);
        assert_eq!(maven_path("a.b:c:1:..\\..\\x"), None);
        assert_eq!(maven_path("a.b:c:1@../sh"), None);
        assert_eq!(maven_path("/abs:c:1"), None);
        assert_eq!(maven_path("a..b:c:1"), None, "an empty group part");
        assert_eq!(maven_path("a.b:c"), None, "no version");
        assert_eq!(maven_path(""), None);
        // ordinary coordinates still work, including dots and dashes in versions
        assert_eq!(
            maven_path("net.minecraftforge:forge:1.20.1-47.2.0:universal").unwrap(),
            "net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-universal.jar"
        );
    }

    #[test]
    fn components_from_files_stay_one_component() {
        assert_eq!(safe_component("1.20.1"), "1.20.1");
        assert_eq!(safe_component("1.20.1-forge-47.2.0"), "1.20.1-forge-47.2.0");
        assert_eq!(safe_component("1.14 Pre-Release 1"), "1.14 Pre-Release 1");
        assert_eq!(safe_component(".."), "__");
        assert_eq!(safe_component("."), "_");
        assert_eq!(safe_component(""), "_");
        assert_eq!(safe_component("../../etc"), ".._.._etc");
        assert_eq!(safe_component("a\\b"), "a_b");
        let base = Path::new("/data/libs");
        for evil in ["..", "../../x", "/etc/passwd", "a/../../x", "..\\..\\x"] {
            let joined = base.join(safe_component(evil));
            assert_eq!(joined.parent(), Some(base), "{evil} -> {}", joined.display());
        }
    }

    #[test]
    fn joins_stay_inside_the_base() {
        let base = Path::new("/data/libs");
        assert_eq!(join_inside(base, "a/b.jar"), Some(base.join("a/b.jar")));
        assert_eq!(join_inside(base, "../x"), None);
        assert_eq!(join_inside(base, "/etc/passwd"), None);
        assert_eq!(join_inside(base, "a/../../x"), None);
        assert_eq!(join_inside(base, ""), None);
    }
}
