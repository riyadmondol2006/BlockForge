//! Pictures from the web (pack logos, mod icons): fetched once, turned into a small PNG that always decodes, and
//! kept in `cache/icons/` so the next start shows them at once, even without a connection.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::paths::Dirs;

/// The longest side kept. Lists show these at 44-60 points, so this is sharp even on a 3x screen.
const MAXIMUM_SIDE: u32 = 192;
/// A cached picture that wasn't asked for in this long is deleted at the next start.
const KEEP: Duration = Duration::from_secs(60 * 24 * 3600);

fn cache_file(dirs: &Dirs, url: &str) -> PathBuf {
    dirs.cache.join("icons").join(format!("{}.png", crate::http::sha1_hex(url.as_bytes())))
}

/// Turn whatever the server sent (PNG, JPEG, WebP, GIF) into a square PNG of at most [`MAXIMUM_SIDE`] points on a
/// side (a wide or tall picture is centred on transparent space, so a list row never changes shape), or `None` when
/// it isn't a picture that can be read (an SVG, an error page).
pub fn normalise(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    if img.width() == 0 || img.height() == 0 {
        return None;
    }
    let img =
        if img.width().max(img.height()) > MAXIMUM_SIDE { img.thumbnail(MAXIMUM_SIDE, MAXIMUM_SIDE) } else { img };
    let img = img.to_rgba8();
    let side = img.width().max(img.height());
    let mut square = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut square,
        &img,
        i64::from((side - img.width()) / 2),
        i64::from((side - img.height()) / 2),
    );
    let mut out = std::io::Cursor::new(Vec::new());
    square.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The picture at `url` as PNG bytes: from the cache folder when it is there, otherwise downloaded (with the
/// usual retries) and then saved there.
pub async fn fetch(dirs: &Dirs, url: &str) -> Option<Vec<u8>> {
    let file = cache_file(dirs, url);
    if let Ok(png) = std::fs::read(&file) {
        if png.len() > 8 {
            return Some(png);
        }
    }
    let bytes = crate::http::get_bytes(url).await.ok()?;
    let png = tokio::task::spawn_blocking(move || normalise(&bytes)).await.ok()??;
    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
        // written under another name first, so a half-written file is never read
        let part = file.with_extension("part");
        if std::fs::write(&part, &png).is_ok() {
            let _ = std::fs::rename(&part, &file);
        }
    }
    Some(png)
}

/// Delete pictures nobody asked for in a long time, so the folder doesn't grow for ever.
pub fn prune(dirs: &Dirs) {
    let Ok(entries) = std::fs::read_dir(dirs.cache.join("icons")) else { return };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.accessed().or_else(|_| m.modified()))
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(format: image::ImageFormat, w: u32, h: u32) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([200, 40, 90])));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn every_kind_of_picture_becomes_a_png() {
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg, image::ImageFormat::Gif] {
            let png = normalise(&picture(format, 64, 64)).unwrap_or_else(|| panic!("{format:?}"));
            assert_eq!(image::guess_format(&png).unwrap(), image::ImageFormat::Png);
        }
    }

    #[test]
    fn big_pictures_are_shrunk_and_sit_on_a_square() {
        let png = normalise(&picture(image::ImageFormat::Png, 1024, 512)).unwrap();
        let small = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!((small.width(), small.height()), (192, 192));
        // the picture is in the middle, the space above and below is empty
        assert_eq!(small.get_pixel(96, 96).0[3], 255);
        assert_eq!(small.get_pixel(96, 5).0[3], 0);
        let png = normalise(&picture(image::ImageFormat::Png, 40, 30)).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().width(), 40);
    }

    #[test]
    fn things_that_are_not_pictures_are_refused() {
        assert!(normalise(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").is_none());
        assert!(normalise(b"<html>404 Not Found</html>").is_none());
        assert!(normalise(&[]).is_none());
    }
}
