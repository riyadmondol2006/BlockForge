//! Where the game's files come from: a jar on disk, or a map in memory (for tests).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Read access to the files of one game jar.
pub trait FileSource {
    /// The content of `path` (like `data/minecraft/recipe/stick.json`), if the jar has it.
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Every file name in the jar.
    fn names(&self) -> Vec<String>;
    fn exists(&self, path: &str) -> bool;
}

pub struct ZipSource {
    archive: zip::ZipArchive<std::io::BufReader<File>>,
}

impl ZipSource {
    pub fn open(path: &Path) -> Result<ZipSource, String> {
        let file = File::open(path).map_err(|e| format!("Can't open {}: {e}", path.display()))?;
        let archive = zip::ZipArchive::new(std::io::BufReader::new(file))
            .map_err(|e| format!("{} is not a valid jar: {e}", path.display()))?;
        Ok(ZipSource { archive })
    }
}

impl FileSource for ZipSource {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut f = self.archive.by_name(path).ok()?;
        let mut out = Vec::with_capacity(f.size().min(1 << 24) as usize);
        f.read_to_end(&mut out).ok()?;
        Some(out)
    }
    fn names(&self) -> Vec<String> {
        self.archive.file_names().filter_map(|n| n.ok().map(|c| c.to_string())).collect()
    }
    fn exists(&self, path: &str) -> bool {
        self.archive.index_for_name(path).is_some()
    }
}

/// Files held in memory, for tests.
#[derive(Default)]
pub struct MemSource(pub BTreeMap<String, Vec<u8>>);

impl MemSource {
    pub fn with(mut self, path: &str, content: &str) -> MemSource {
        self.0.insert(path.to_string(), content.as_bytes().to_vec());
        self
    }
}

impl FileSource for MemSource {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.0.get(path).cloned()
    }
    fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
    fn exists(&self, path: &str) -> bool {
        self.0.contains_key(path)
    }
}
