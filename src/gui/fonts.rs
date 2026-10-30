//! The app's typeface: Inter (SIL Open Font Licence 1.1, see `assets/fonts/OFL.txt`) in three weights.
//!
//! The files are static, Latin-only subsets with tabular figures baked in as the default digits, so numbers in
//! columns and counters line up. egui's own fonts stay behind Inter as the fallback chain for emoji and symbols, and
//! a font of the system that covers Chinese, Japanese and Korean (see [`system_fallback`]) comes after them, found
//! after the window opens, so mod and modpack names and descriptions in those scripts don't turn into boxes. Nothing
//! that large is bundled: every Windows and macOS has such a font, and so does a Linux set up for those languages.

use egui::{FontData, FontDefinitions, FontFamily, FontId, RichText};
use std::sync::Arc;

const REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");

const MEDIUM_FAMILY: &str = "medium";
const SEMIBOLD_FAMILY: &str = "semibold";

/// Loads Inter into the egui context. Call once, where the context is created. A font of the system for Chinese,
/// Japanese and Korean is looked for in the background and joins the fallbacks when found.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions(None));
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("fonts".into()).spawn(move || {
        if let Some((_, bytes)) = system_fallback() {
            ctx.set_fonts(definitions(Some(bytes)));
            ctx.request_repaint();
        }
    });
}

/// Inter in its three weights, then egui's own fonts, then `cjk` (the bytes of a system font) when there is one.
fn definitions(cjk: Option<Vec<u8>>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in [("Inter-Regular", REGULAR), ("Inter-Medium", MEDIUM), ("Inter-SemiBold", SEMIBOLD)] {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    if let Some(bytes) = cjk {
        fonts.font_data.insert(CJK.to_owned(), Arc::new(FontData::from_owned(bytes)));
        fallbacks.push(CJK.to_owned());
        if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
            mono.push(CJK.to_owned());
        }
    }
    for (family, face) in [
        (FontFamily::Proportional, "Inter-Regular"),
        (FontFamily::Name(MEDIUM_FAMILY.into()), "Inter-Medium"),
        (FontFamily::Name(SEMIBOLD_FAMILY.into()), "Inter-SemiBold"),
    ] {
        let mut chain = vec![face.to_owned()];
        chain.extend(fallbacks.iter().cloned());
        fonts.families.insert(family, chain);
    }
    fonts
}

const CJK: &str = "system-cjk";

/// The fonts that cover Chinese, Japanese and Korean, best first, as each system ships them. On Linux the usual
/// folders are searched for these file names; the first one found is used (the first face of a collection).
fn candidates() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    if cfg!(windows) {
        let fonts =
            std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows")).join("Fonts");
        ["msyh.ttc", "msyh.ttf", "YuGothR.ttc", "meiryo.ttc", "malgun.ttf", "simsun.ttc"]
            .iter()
            .map(|f| fonts.join(f))
            .collect()
    } else if cfg!(target_os = "macos") {
        [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    } else {
        const NAMES: [&str; 8] = [
            "NotoSansCJK-Regular.ttc",
            "NotoSansCJKsc-Regular.otf",
            "SourceHanSans-Regular.ttc",
            "SourceHanSansSC-Regular.otf",
            "NotoSansCJK-VF.ttc",
            "wqy-microhei.ttc",
            "wqy-zenhei.ttc",
            "DroidSansFallbackFull.ttf",
        ];
        let mut roots = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(&home).join(".local/share/fonts"));
            roots.push(PathBuf::from(home).join(".fonts"));
        }
        let mut found: Vec<(usize, PathBuf)> = Vec::new();
        for root in roots {
            find_named(&root, &NAMES, 3, &mut found);
        }
        found.sort_by_key(|(rank, _)| *rank);
        found.into_iter().map(|(_, p)| p).collect()
    }
}

/// The files under `dir` (at most `depth` folders down) whose name is one of `names`, with their place in it.
fn find_named(dir: &std::path::Path, names: &[&str], depth: u32, found: &mut Vec<(usize, std::path::PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                find_named(&path, names, depth - 1, found);
            }
        } else if let Some(rank) = names.iter().position(|n| e.file_name().to_str() == Some(n)) {
            found.push((rank, path));
        }
    }
}

/// The first system font of [`candidates`] that can be read, with its path.
fn system_fallback() -> Option<(std::path::PathBuf, Vec<u8>)> {
    candidates().into_iter().find_map(|p| std::fs::read(&p).ok().filter(|b| b.len() > 1024).map(|b| (p, b)))
}

/// `text.semibold()` for the weight that stands in for `strong()`: real Inter SemiBold, not just a brighter colour.
pub trait TextExt {
    fn semibold(self) -> RichText;
    fn medium_weight(self) -> RichText;
}

impl TextExt for RichText {
    fn semibold(self) -> RichText {
        self.family(FontFamily::Name(SEMIBOLD_FAMILY.into()))
    }
    fn medium_weight(self) -> RichText {
        self.family(FontFamily::Name(MEDIUM_FAMILY.into()))
    }
}

/// Inter Medium (500) at `size` points.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM_FAMILY.into()))
}

/// The monospace face at `size` points: versions, paths, logs and other technical facts.
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

#[allow(dead_code)]
/// Inter SemiBold (600) at `size` points.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD_FAMILY.into()))
}

#[allow(dead_code)]
/// Text in Inter SemiBold at the size of the style it ends up in.
pub fn semibold_text(text: impl Into<String>) -> RichText {
    RichText::new(text).family(FontFamily::Name(SEMIBOLD_FAMILY.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context) {
        // fonts are applied at the start of a frame
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    }

    fn width(ctx: &egui::Context, text: &str, font: FontId) -> f32 {
        ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE).size().x)
    }

    #[test]
    fn digits_are_tabular_in_every_weight() {
        let ctx = egui::Context::default();
        install(&ctx);
        frame(&ctx);
        for font in [FontId::proportional(14.0), medium(14.0), semibold(14.0)] {
            let narrow = width(&ctx, "1111", font.clone());
            let wide = width(&ctx, "0000", font.clone());
            assert!((narrow - wide).abs() < 0.01, "{font:?}: {narrow} vs {wide}");
            // a letter is not tabular, so the face really is Inter and not a fallback with equal digits
            assert!(width(&ctx, "iiii", font.clone()) < wide);
        }
    }

    #[test]
    fn a_system_font_for_cjk_is_found_by_its_file_name_and_joins_the_fallbacks() {
        let dir = crate::testutil::scratch("fonts");
        std::fs::create_dir_all(dir.join("google-noto-sans-cjk/sub")).unwrap();
        std::fs::write(dir.join("google-noto-sans-cjk/sub/wqy-microhei.ttc"), b"x").unwrap();
        std::fs::write(dir.join("google-noto-sans-cjk/NotoSansCJK-Regular.ttc"), b"x").unwrap();
        std::fs::write(dir.join("other.ttf"), b"x").unwrap();
        let mut found = Vec::new();
        find_named(&dir, &["NotoSansCJK-Regular.ttc", "wqy-microhei.ttc"], 3, &mut found);
        found.sort();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, 0, "the better font ranks first");
        let defs = definitions(Some(REGULAR.to_vec()));
        let chain = &defs.families[&FontFamily::Proportional];
        assert_eq!(chain.first().map(String::as_str), Some("Inter-Regular"));
        assert_eq!(chain.last().map(String::as_str), Some(CJK), "after Inter and egui's own fonts");
        assert!(definitions(None).families[&FontFamily::Proportional].iter().all(|f| f != CJK));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn weights_differ() {
        let ctx = egui::Context::default();
        install(&ctx);
        frame(&ctx);
        let regular = width(&ctx, "Create your first instance", FontId::proportional(14.0));
        let semi = width(&ctx, "Create your first instance", semibold(14.0));
        assert!(semi > regular);
    }
}
