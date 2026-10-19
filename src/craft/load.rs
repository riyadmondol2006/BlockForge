//! Reading the recipe book of one game version out of its client jar.

use std::path::{Path, PathBuf};

use super::source::{FileSource, ZipSource};
use super::types::*;
use crate::paths::Dirs;

/// A game version whose client jar is on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledVersion {
    pub version: String,
    pub jar: PathBuf,
}

/// The vanilla client jars in the shared libraries folder (`net/minecraft/client/<v>/client-<v>.jar`, only the
/// plain ones, not the loader-patched `-srg`/`-slim`/`-extra` copies), newest first.
pub fn installed_versions(_dirs: &Dirs) -> Vec<InstalledVersion> {
    Vec::new()
}

/// Read the recipes, tags, names and item list of the game in `src`.
pub fn load(_version: &str, _src: &mut dyn FileSource) -> Result<Dataset, String> {
    Err("not written yet".into())
}

pub fn load_jar(version: &str, jar: &Path) -> Result<Dataset, String> {
    let mut src = ZipSource::open(jar)?;
    load(version, &mut src)
}
