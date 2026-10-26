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
    pick(0x0E100F, 0xF4F5F4)
}
/// The sidebar. The same ground as the window, divided from it by one line.
pub fn c_surface() -> Color32 {
    c_bg()
}
/// Cards and panels.
pub fn c_card() -> Color32 {
    pick(0x171A18, 0xFFFFFF)
}
/// Inputs and tonal buttons.
pub fn c_field() -> Color32 {
    pick(0x1D201E, 0xFFFFFF)
}
/// The fill of a row, a nav item or a ghost button under the pointer.
pub fn c_hover() -> Color32 {
    pick(0x222624, 0xEAEDEB)
}
/// The open nav item, a pressed button, a selected segment, an empty cell of the play strip.
pub fn c_active() -> Color32 {
    pick(0x2A2E2C, 0xDEE2DF)
}
/// Old names for the hover fill, kept so older code reads the same.
pub fn c_card_hover() -> Color32 {
    c_hover()
}
pub fn c_field_hover() -> Color32 {
    c_hover()
}
/// Menus, dialogs and tooltips: what floats sits one step above the cards.
pub fn c_raised() -> Color32 {
    pick(0x1F2321, 0xFFFFFF)
}
/// The line around cards and panels: barely there.
pub fn c_border() -> Color32 {
    pick(0x272B29, 0xE2E5E3)
}
/// The line between rows inside one card or list.
pub fn c_rule() -> Color32 {
    pick(0x202422, 0xEDEFEE)
}
/// The line around things you can press: one step stronger than a card's, and at least 3:1 against the card, the
/// field and the page behind it (WCAG 1.4.11), since in the light look a field is white on a white card and the line
/// is all that shows where it is.
pub fn c_control() -> Color32 {
    pick(0x666C68, 0x8A908C)
}
pub fn c_text() -> Color32 {
    pick(0xECEEED, 0x121413)
}
pub fn c_muted() -> Color32 {
    pick(0xA7ADA9, 0x565D59)
}
/// Meta text, placeholders and the like (4.5:1 on the cards). Never on `c_active` or `c_selection`: use `c_muted` there.
pub fn c_faint() -> Color32 {
    pick(0x8C928E, 0x616864)
}
/// The ink fill: the main action of a screen, a selected filter, the tab indicator, a switch that is on.
pub fn c_ink() -> Color32 {
    pick(0xECEEED, 0x1B1E1C)
}
pub fn c_ink_hover() -> Color32 {
    pick(0xFFFFFF, 0x2E322F)
}
pub fn c_on_ink() -> Color32 {
    pick(0x0E100F, 0xFFFFFF)
}
/// A selected list row or combo item, and selected text.
pub fn c_selection() -> Color32 {
    pick(0x303633, 0xDCE1DE)
}
/// Green means Play: the Play button, a running game and the progress of getting one started.
pub fn c_accent() -> Color32 {
    pick(0x5FD97A, 0x167A36)
}
pub fn c_accent_hover() -> Color32 {
    pick(0x7BE892, 0x12692E)
}
pub fn c_on_accent() -> Color32 {
    pick(0x04210C, 0xFFFFFF)
}
/// The tint behind a highlighted banner or a file dropped on the window.
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
pub fn c_warn_bg() -> Color32 {
    pick(0x2A2410, 0xFFF6DB)
}

/// Corner radii, used everywhere so the shapes agree: cards and panels, things you press, small tags, dialogs.
pub const R_CARD: u8 = 8;
pub const R_CONTROL: u8 = 6;
pub const R_TAG: u8 = 4;
pub const R_DIALOG: u8 = 12;

/// The type scale: nothing else is used.
pub const FS_TITLE: f32 = 22.0;
pub const FS_FIGURE: f32 = 20.0;
pub const FS_SECTION: f32 = 16.0;
pub const FS_BODY: f32 = 14.0;
pub const FS_META: f32 = 12.0;

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
    v.window_corner_radius = CornerRadius::same(R_DIALOG);
    v.menu_corner_radius = CornerRadius::same(R_CARD);
    v.override_text_color = Some(c_text());
    v.hyperlink_color = c_text();
    v.selection.bg_fill = c_selection();
    v.selection.stroke = Stroke::new(1.0, c_text());
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
    v.widgets.hovered.bg_fill = c_hover();
    v.widgets.hovered.weak_bg_fill = c_hover();
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, c_control());
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, c_text());
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = c_active();
    v.widgets.active.weak_bg_fill = c_active();
    v.widgets.active.fg_stroke = Stroke::new(1.5, c_text());
    v.widgets.active.corner_radius = r;
    v.widgets.open = v.widgets.active;
    v.widgets.open.bg_stroke = Stroke::new(1.0, c_control());
    // egui draws a widget with keyboard focus in the "active" style: the same ring as the launcher's own widgets
    // (see `focus_ring`), so the focus can be seen when moving with Tab
    v.widgets.active.bg_stroke = Stroke::new(2.0, c_accent());
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.scroll.bar_width = 8.0;
        // the floating scroll bar gets its own strip so it never sits on the edge of a card
        s.spacing.scroll.floating_allocated_width = 10.0;
        s.animation_time = if reduced_motion { 0.0 } else { 1.0 / 10.0 };
        s.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A button reached with Tab is drawn with the 2px accent ring, so a keyboard user can see where they are; the
    /// same button without the focus has the thin control outline.
    #[test]
    fn a_button_reached_with_tab_shows_the_focus_ring() {
        let ctx = egui::Context::default();
        apply(&ctx, true);
        let frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let _ = ui.button("Play as");
            });
            out.textures_delta.clear();
            out.shapes
                .into_iter()
                .filter_map(|c| match c.shape {
                    egui::Shape::Rect(r) => Some(r.stroke),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let ring = |strokes: &[Stroke]| strokes.iter().any(|s| s.width >= 2.0 && s.color == c_accent());
        assert!(!ring(&frame(vec![])), "no focus yet: no ring");
        let tab = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![tab]);
        assert!(ctx.memory(|m| m.focused()).is_some(), "Tab gave the button the focus");
        assert!(ring(&frame(vec![])), "the focused button is drawn with the accent ring");
    }
}
