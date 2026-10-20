// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! A map item's data (`ChunkerMap`).

use std::path::Path;

use image::imageops::FilterType;

use super::dimension::Dimension;
use crate::convert::nbt::Compound;

/// A filled map: its position and the pixels as RGBA bytes (`width * height * 4`).
#[derive(Clone, Debug, PartialEq)]
pub struct LevelMap {
    /// The id the map had in the input world (`None` for maps supplied by the user).
    pub original_id: Option<i64>,
    pub id: i64,
    pub width: i32,
    pub height: i32,
    pub scale: i8,
    pub dimension: Dimension,
    pub x_center: i32,
    pub z_center: i32,
    pub unlimited_tracking: bool,
    pub locked: bool,
    /// RGBA pixels, row by row; alpha is 0 or 255.
    pub bytes: Option<Vec<u8>>,
    /// The map's original NBT, kept when copying NBT.
    pub original_nbt: Option<Compound>,
}

impl LevelMap {
    /// A map of the standard 128 x 128 size with no pixels.
    pub fn new(id: i64, dimension: Dimension) -> LevelMap {
        LevelMap {
            original_id: None,
            id,
            width: 128,
            height: 128,
            scale: 0,
            dimension,
            x_center: 0,
            z_center: 0,
            unlimited_tracking: false,
            locked: false,
            bytes: None,
            original_nbt: None,
        }
    }

    /// Orders maps by id (`BY_ID_COMPARATOR`).
    pub fn cmp_by_id(a: &LevelMap, b: &LevelMap) -> std::cmp::Ordering {
        a.id.cmp(&b.id)
    }

    /// Loads the pixels from an image file, resizing (bicubic) to `width` x `height` when needed; alpha above 128
    /// becomes 255, otherwise 0 (`loadImage`).
    pub fn load_image(&mut self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Err(format!(
                "Could not find map file {}",
                path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ));
        }
        let image = image::open(path).map_err(|e| format!("could not read map image {}: {e}", path.display()))?;
        let (w, h) = (self.width.max(0) as u32, self.height.max(0) as u32);
        let mut rgba = if image.width() != w || image.height() != h {
            image.resize_exact(w, h, FilterType::CatmullRom).to_rgba8()
        } else {
            image.to_rgba8()
        };
        for pixel in rgba.pixels_mut() {
            pixel.0[3] = if pixel.0[3] > 128 { 255 } else { 0 };
        }
        self.bytes = Some(rgba.into_raw());
        Ok(())
    }

    /// The map as a JSON object for a UI (without pixels and NBT, as Chunker's transient fields).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "originalId": self.original_id,
            "id": self.id,
            "width": self.width,
            "height": self.height,
            "scale": self.scale,
            "dimension": self.dimension.identifier(),
            "xCenter": self.x_center,
            "zCenter": self.z_center,
            "unlimitedTracking": self.unlimited_tracking,
            "locked": self.locked,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_order() {
        let mut a = LevelMap::new(3, Dimension::nether());
        a.original_id = Some(9);
        let b = LevelMap::new(1, Dimension::overworld());
        assert_eq!(LevelMap::cmp_by_id(&a, &b), std::cmp::Ordering::Greater);
        let json = a.to_json();
        assert_eq!(json["dimension"], "minecraft:the_nether");
        assert_eq!(json["originalId"], 9);
        assert_eq!(json["width"], 128);
    }

    #[test]
    fn load_image_resizes_and_thresholds_alpha() {
        let dir = std::env::temp_dir().join(format!("bf-map-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.png");
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([10, 20, 30, 255]));
        img.put_pixel(1, 0, image::Rgba([10, 20, 30, 100]));
        img.put_pixel(0, 1, image::Rgba([10, 20, 30, 129]));
        img.put_pixel(1, 1, image::Rgba([10, 20, 30, 0]));
        img.save(&path).unwrap();
        let mut map = LevelMap::new(0, Dimension::overworld());
        map.width = 2;
        map.height = 2;
        map.load_image(&path).unwrap();
        let bytes = map.bytes.clone().unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[0..4], &[10, 20, 30, 255]);
        assert_eq!(bytes[7], 0);
        assert_eq!(bytes[11], 255);
        assert_eq!(bytes[15], 0);
        // Resizing keeps the length contract.
        map.width = 4;
        map.height = 4;
        map.load_image(&path).unwrap();
        assert_eq!(map.bytes.as_ref().unwrap().len(), 64);
        assert!(map.load_image(&dir.join("missing.png")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
