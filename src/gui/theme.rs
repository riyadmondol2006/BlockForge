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

// The palette. Neutral greys with the faintest green in them; the green accent is kept for the one main action on a
// screen, for progress and for what is switched on. Everything else (selection, tabs, filters) is ink on ground, so
// colour means something when it shows.

/// Window background; the sidebar sits on the same ground.
pub fn c_bg() -> Color32 {
    pick(0x0E0F0F, 0xF5F6F5)
}
/// The sidebar. The same ground as the window, divided from it by one line.
pub fn c_surface() -> Color32 {
    c_bg()
}
/// Cards and panels.
pub fn c_card() -> Color32 {
    pick(0x151716, 0xFFFFFF)
}
pub fn c_card_hover() -> Color32 {
    pick(0x1B1D1C, 0xF0F2F1)
}
/// Inputs and tonal buttons.
pub fn c_field() -> Color32 {
    pick(0x1B1D1C, 0xEDEFEE)
}
pub fn c_field_hover() -> Color32 {
    pick(0x242726, 0xE2E5E3)
}
/// Menus, dialogs and tooltips: what floats sits one step above the cards.
pub fn c_raised() -> Color32 {
    pick(0x1C1E1D, 0xFFFFFF)
}
/// The line around cards and panels: barely there.
pub fn c_border() -> Color32 {
    pick(0x242726, 0xE2E5E3)
}
/// The line around things you can press: one step stronger than a card's.
pub fn c_control() -> Color32 {
    pick(0x353937, 0xCBD0CD)
}
pub fn c_text() -> Color32 {
    pick(0xECEEED, 0x121413)
}
pub fn c_muted() -> Color32 {
    pick(0xA0A6A2, 0x59605C)
}
/// Meta text, placeholders and the like (still 4.5:1 on the cards).
pub fn c_faint() -> Color32 {
    pick(0x898F8B, 0x6A716D)
}
/// The ink fill of a selected filter or tab: the text colour turned into a surface.
pub fn c_ink() -> Color32 {
    pick(0xECEEED, 0x1B1E1C)
}
pub fn c_on_ink() -> Color32 {
    pick(0x0E0F0F, 0xF5F6F5)
}
pub fn c_accent() -> Color32 {
    pick(0x5FD97A, 0x167A36)
}
pub fn c_accent_hover() -> Color32 {
    pick(0x7BE892, 0x1D9141)
}
pub fn c_on_accent() -> Color32 {
    pick(0x04210C, 0xFFFFFF)
}
/// Text selection, and the faint tint behind a highlighted banner.
pub fn c_accent_soft() -> Color32 {
    pick(0x173020, 0xDCEFE1)
}
pub fn c_danger() -> Color32 {
    pick(0xFF8A80, 0xC02626)
}
pub fn c_danger_bg() -> Color32 {
    pick(0x2B1816, 0xFCE8E6)
}
pub fn c_danger_hover() -> Color32 {
    pick(0x3A201D, 0xF8D5D1)
}
pub fn c_danger_line() -> Color32 {
    pick(0x4F2A27, 0xEBB1AB)
}
pub fn c_warn() -> Color32 {
    pick(0xF5C451, 0x8F6200)
}

/// Corner radii, used everywhere so the shapes agree: cards and panels, things you press, small tags.
pub const R_CARD: u8 = 12;
pub const R_CONTROL: u8 = 8;
pub const R_TAG: u8 = 6;

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
    v.window_fill = c_raised();
    v.extreme_bg_color = c_field();
    v.faint_bg_color = c_card();
    v.window_stroke = Stroke::new(1.0, c_control());
    v.window_corner_radius = CornerRadius::same(R_CARD + 4);
    v.menu_corner_radius = CornerRadius::same(R_CONTROL + 2);
    v.override_text_color = Some(c_text());
    v.hyperlink_color = c_accent();
    v.selection.bg_fill = c_accent_soft();
    v.selection.stroke = Stroke::new(1.0, c_accent());
    // shadows only on what floats (menus, dialogs, tooltips)
    let shadow = Color32::from_black_alpha(if is_dark() { 140 } else { 36 });
    v.window_shadow = egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: shadow };
    v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: shadow };

    let r = CornerRadius::same(R_CONTROL);
    v.widgets.noninteractive.bg_fill = c_card();
    v.widgets.noninteractive.weak_bg_fill = c_card();
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, c_border());
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, c_text());
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = c_field();
    v.widgets.inactive.weak_bg_fill = c_field();
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, c_control());
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, c_text());
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = c_field_hover();
    v.widgets.hovered.weak_bg_fill = c_field_hover();
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, c_control());
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, c_text());
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = c_field_hover();
    v.widgets.active.weak_bg_fill = c_field_hover();
    v.widgets.active.bg_stroke = Stroke::new(1.0, c_text());
    v.widgets.active.fg_stroke = Stroke::new(1.5, c_text());
    v.widgets.active.corner_radius = r;
    v.widgets.open = v.widgets.active;
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.scroll.bar_width = 8.0;
        s.animation_time = if reduced_motion { 0.0 } else { 1.0 / 10.0 };
        s.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
    });
}
