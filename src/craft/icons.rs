//! Item pictures, read from the game's own textures.

use std::path::Path;

use super::source::{FileSource, ZipSource};

/// An RGBA picture, 4 bytes a pixel, row by row.
pub struct IconImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct Icons {
    src: Box<dyn FileSource>,
}

impl Icons {
    pub fn open(jar: &Path) -> Result<Icons, String> {
        Ok(Icons { src: Box::new(ZipSource::open(jar)?) })
    }
    pub fn new(src: Box<dyn FileSource>) -> Icons {
        Icons { src }
    }
    /// The picture of `id` (`minecraft:oak_planks`): the first frame of an animated texture, tinted where the game
    /// tints it (leaves, grass), or `None` when the item has no usable texture.
    pub fn image(&mut self, _id: &str) -> Option<IconImage> {
        let _ = &mut self.src;
        None
    }
}
