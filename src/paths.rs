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

/// Folder name for an instance called `name`: letters and digits only, the way the Java edition names them.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| c.is_alphanumeric()).collect();
    if cleaned.is_empty() { "Instance".to_string() } else { cleaned }
}

/// A folder name based on `name` that doesn't exist yet in `parent`.
pub fn unique_folder(parent: &Path, name: &str) -> String {
    let base = folder_name(name);
    if !parent.join(&base).exists() {
        return base;
    }
    (2..).map(|n| format!("{base}{n}")).find(|candidate| !parent.join(candidate).exists()).unwrap()
}

/// Maven coordinates (`group:artifact:version[:classifier][@ext]`) to a repository path.
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
}
