//! Colours and look. The values are the Java edition's palette, which has a dark and a light variant; the theme the
//! player picks decides which one is in use, and every colour is looked up when it is drawn so a change shows at once.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

static DARK: AtomicBool = AtomicBool::new(true);

pub fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

fn pick(dark: u32, light: u32) -> Color32 {
    let v = if is_dark() { dark } else { light };
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Window background.
pub fn c_bg() -> Color32 {
    pick(0x0E1210, 0xF3F6EF)
}
/// Sidebar background.
pub fn c_surface() -> Color32 {
    pick(0x121712, 0xE8EEE3)
}
/// Cards and raised surfaces.
pub fn c_card() -> Color32 {
    pick(0x1A201B, 0xFFFFFF)
}
pub fn c_card_hover() -> Color32 {
    pick(0x222A23, 0xEEF4E9)
}
/// Inputs and tonal buttons.
pub fn c_field() -> Color32 {
    pick(0x232B24, 0xE2E9DC)
}
pub fn c_field_hover() -> Color32 {
    pick(0x2C362D, 0xD5DECE)
}
pub fn c_border() -> Color32 {
    pick(0x2E382F, 0xD3DCCC)
}
pub fn c_text() -> Color32 {
    pick(0xE7ECE4, 0x141A15)
}
pub fn c_muted() -> Color32 {
    pick(0x9BA89C, 0x586558)
}
pub fn c_accent() -> Color32 {
    pick(0x5FD97A, 0x1B8A3C)
}
pub fn c_accent_hover() -> Color32 {
    pick(0x7BE892, 0x23A04A)
}
pub fn c_on_accent() -> Color32 {
    pick(0x04210C, 0xFFFFFF)
}
/// Selected navigation item, highlighted banners.
pub fn c_accent_soft() -> Color32 {
    pick(0x1D3824, 0xD3EED9)
}
pub fn c_danger() -> Color32 {
    pick(0xFF8A80, 0xC62828)
}
pub fn c_danger_bg() -> Color32 {
    pick(0x3A1F1D, 0xFBE3E1)
}
pub fn c_danger_hover() -> Color32 {
    pick(0x4A2725, 0xF6CFCB)
}
pub fn c_danger_line() -> Color32 {
    pick(0x5A2E2B, 0xE9A6A0)
}
pub fn c_warn() -> Color32 {
    pick(0xF5C451, 0x9A6A00)
}

pub const RADIUS: u8 = 12;

/// The themes the Java edition offers: id, label, and whether it is one of the dark ones.
pub const THEMES: [(&str, &str, bool); 18] = [
    ("com.blockforge.launcher.themes.Material3Dark", "Material 3 Dark (default)", true),
    ("com.blockforge.launcher.themes.Material3Light", "Material 3 Light", false),
    ("com.blockforge.launcher.themes.Dark", "BlockForge Dark", true),
    ("com.blockforge.launcher.themes.Light", "BlockForge Light", false),
    ("com.blockforge.launcher.themes.MonokaiPro", "Monokai Pro", true),
    ("com.blockforge.launcher.themes.DraculaContrast", "Dracula Contrast", true),
    ("com.blockforge.launcher.themes.HiberbeeDark", "Hiberbee Dark", true),
    ("com.blockforge.launcher.themes.Vuesion", "Vuesion", true),
    ("com.blockforge.launcher.themes.MaterialPalenightContrast", "Material Palenight Contrast", true),
    ("com.blockforge.launcher.themes.ArcOrange", "Arc Orange", false),
    ("com.blockforge.launcher.themes.CyanLight", "Cyan Light", false),
    ("com.blockforge.launcher.themes.HighTechDarkness", "High Tech Darkness", true),
    ("com.blockforge.launcher.themes.OneDark", "One Dark", true),
    ("com.blockforge.launcher.themes.Tokyonight", "TokyoNight", true),
    ("com.blockforge.launcher.themes.CatppuccinLatte", "Catppuccin Latte", false),
    ("com.blockforge.launcher.themes.CatppuccinFrappe", "Catppuccin Frapp\u{e9}", true),
    ("com.blockforge.launcher.themes.CatppuccinMacchiato", "Catppuccin Macchiato", true),
    ("com.blockforge.launcher.themes.CatppuccinMocha", "Catppuccin Mocha", true),
];

pub fn theme_is_dark(id: &str) -> bool {
    THEMES.iter().find(|t| t.0 == id).map(|t| t.2).unwrap_or(true)
}

pub fn theme_label(id: &str) -> &'static str {
    THEMES.iter().find(|t| t.0 == id).map(|t| t.1).unwrap_or("Material 3 Dark (default)")
}

/// Set the look of every standard widget from the current palette. Call again after the theme changes.
pub fn apply(ctx: &egui::Context, reduced_motion: bool) {
    let mut v = if is_dark() { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = c_bg();
    v.window_fill = c_surface();
    v.extreme_bg_color = c_field();
    v.faint_bg_color = c_card();
    v.window_stroke = Stroke::new(1.0, c_border());
    v.window_corner_radius = CornerRadius::same(RADIUS + 4);
    v.menu_corner_radius = CornerRadius::same(RADIUS);
    v.override_text_color = Some(c_text());
    v.hyperlink_color = c_accent();
    v.selection.bg_fill = c_accent_soft();
    v.selection.stroke = Stroke::new(1.0, c_accent());
    let shadow = Color32::from_black_alpha(if is_dark() { 120 } else { 50 });
    v.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: shadow };
    v.popup_shadow = v.window_shadow;

    let r = CornerRadius::same(10);
    v.widgets.noninteractive.bg_fill = c_card();
    v.widgets.noninteractive.weak_bg_fill = c_card();
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c_border());
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, c_text());
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = c_card_hover();
    v.widgets.inactive.weak_bg_fill = c_card_hover();
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, c_border());
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, c_text());
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = c_field_hover();
    v.widgets.hovered.weak_bg_fill = c_field_hover();
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, c_accent());
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, c_text());
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = c_accent_soft();
    v.widgets.active.weak_bg_fill = c_accent_soft();
    v.widgets.active.bg_stroke = Stroke::new(1.0, c_accent());
    v.widgets.active.fg_stroke = Stroke::new(1.5, c_text());
    v.widgets.active.corner_radius = r;
    v.widgets.open = v.widgets.active;
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(14.0, 8.0);
        s.spacing.interact_size.y = 34.0;
        s.spacing.scroll.bar_width = 8.0;
        s.animation_time = if reduced_motion { 0.0 } else { 1.0 / 12.0 };
        s.text_styles = [
            (TextStyle::Heading, FontId::new(26.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.5, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
    });
}
