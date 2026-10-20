//! The app's typeface: Inter (SIL Open Font Licence 1.1, see `assets/fonts/OFL.txt`) in three weights.
//!
//! The files are static, Latin-only subsets with tabular figures baked in as the default digits, so numbers in
//! columns and counters line up. egui's own fonts stay behind Inter as the fallback chain, so CJK, emoji and
//! symbols that Inter does not carry still draw.

use egui::{FontData, FontDefinitions, FontFamily, FontId, RichText};
use std::sync::Arc;

const REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");

const MEDIUM_FAMILY: &str = "medium";
const SEMIBOLD_FAMILY: &str = "semibold";

/// Loads Inter into the egui context. Call once, where the context is created.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in [("Inter-Regular", REGULAR), ("Inter-Medium", MEDIUM), ("Inter-SemiBold", SEMIBOLD)] {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
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
    ctx.set_fonts(fonts);
}

/// `text.semibold()` for the weight that stands in for `strong()`: real Inter SemiBold, not just a brighter colour.
pub trait TextExt {
    fn semibold(self) -> RichText;
}

impl TextExt for RichText {
    fn semibold(self) -> RichText {
        self.family(FontFamily::Name(SEMIBOLD_FAMILY.into()))
    }
}

/// Inter Medium (500) at `size` points.
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM_FAMILY.into()))
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
    fn weights_differ() {
        let ctx = egui::Context::default();
        install(&ctx);
        frame(&ctx);
        let regular = width(&ctx, "Create your first instance", FontId::proportional(14.0));
        let semi = width(&ctx, "Create your first instance", semibold(14.0));
        assert!(semi > regular);
    }
}
