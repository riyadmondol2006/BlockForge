//! Colours and look: "the Workbench". A deep blue-black ground with a faint pattern of blocks, surfaces in three
//! steps (surface, elevated, what floats), one mint primary for Play and the main actions, and a warm amber secondary
//! for what wants a look (favourites, updates). Technical facts (versions, paths, logs) are set in a monospace face.
//! There is a dark and a light variant; the theme the player picks decides which one is in use, and every colour is
//! looked up when it is drawn so a change shows at once.

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

// The palette. Grounds with a little blue in them, each layer one step lighter than the one under it (ground,
// sidebar glass, cards, fields, what floats). Colour is the brand gradient and the states (danger, warning); the rest
// is ink on ground.

/// Window background, under the soft glow drawn behind the pages.
pub fn c_bg() -> Color32 {
    pick(0x0B1016, 0xF2F4F7)
}
/// The sidebar: a pane of glass over the ground.
pub fn c_surface() -> Color32 {
    pick(0x0D131B, 0xF8F9FB)
}
/// Cards and panels.
pub fn c_card() -> Color32 {
    pick(0x151D29, 0xFFFFFF)
}
/// Inputs and tonal buttons.
pub fn c_field() -> Color32 {
    pick(0x1A2433, 0xFFFFFF)
}
/// The fill of a row, a nav item or a ghost button under the pointer.
pub fn c_hover() -> Color32 {
    pick(0x1E2939, 0xEBEFF4)
}
/// The open nav item, a pressed button, a selected segment, an empty cell of the play strip.
pub fn c_active() -> Color32 {
    pick(0x243247, 0xDFE5EE)
}
/// Old names for the hover fill, kept so older code reads the same.
pub fn c_card_hover() -> Color32 {
    c_hover()
}
pub fn c_field_hover() -> Color32 {
    pick(0x202B3C, 0xF2F5F9)
}
/// Menus, dialogs and tooltips: what floats sits one step above the cards.
pub fn c_raised() -> Color32 {
    pick(0x1A2332, 0xFFFFFF)
}
/// The line around cards and panels: barely there.
pub fn c_border() -> Color32 {
    pick(0x212D3F, 0xDDE2E9)
}
/// The line between rows inside one card or list.
pub fn c_rule() -> Color32 {
    pick(0x1A2433, 0xEBEEF3)
}
/// The line around things you can press: one step stronger than a card's, and at least 3:1 against the card, the
/// field and the page behind it (WCAG 1.4.11), since in the light look a field is white on a white card and the line
/// is all that shows where it is.
pub fn c_control() -> Color32 {
    pick(0x66748C, 0x8A93A3)
}
pub fn c_text() -> Color32 {
    pick(0xE8EDF4, 0x0D141C)
}
pub fn c_muted() -> Color32 {
    pick(0xA3AEBF, 0x4F5A69)
}
/// Meta text, placeholders and the like (4.5:1 on the cards). Never on `c_active` or `c_selection`: use `c_muted` there.
pub fn c_faint() -> Color32 {
    pick(0x8893A6, 0x5C6776)
}
/// A selected list row or combo item, and selected text.
pub fn c_selection() -> Color32 {
    pick(0x27364D, 0xD9E2EC)
}
/// The brand colour, where the gradient starts: Play, the main action, progress, what is switched on.
pub fn c_accent() -> Color32 {
    pick(0x2DD4AA, 0x0A7A62)
}
/// Where the brand gradient ends.
pub fn c_accent2() -> Color32 {
    pick(0x5EE6C6, 0x0F766E)
}
pub fn c_accent_hover() -> Color32 {
    pick(0x6EEBCB, 0x07614E)
}
pub fn c_on_accent() -> Color32 {
    pick(0x03261D, 0xFFFFFF)
}
/// The tint behind a highlighted banner, a selected chip or a file dropped on the window.
pub fn c_accent_soft() -> Color32 {
    pick(0x0D2E29, 0xD3F4EA)
}
/// The warm secondary: a favourite's star, an update waiting, a highlight that is not the main action.
pub fn c_secondary() -> Color32 {
    pick(0xF3B56B, 0x9A5A06)
}
/// The tint behind something marked in the secondary colour.
pub fn c_secondary_soft() -> Color32 {
    pick(0x2C2316, 0xFCEFD9)
}
pub fn c_danger() -> Color32 {
    pick(0xFF8A8A, 0xC02626)
}
pub fn c_danger_bg() -> Color32 {
    pick(0x2A1519, 0xFCE8E8)
}
pub fn c_danger_hover() -> Color32 {
    pick(0x3A1D22, 0xF8D5D5)
}
pub fn c_danger_line() -> Color32 {
    pick(0x52262D, 0xEBB1B1)
}
pub fn c_warn() -> Color32 {
    pick(0xF5C451, 0x8F6200)
}
pub fn c_warn_bg() -> Color32 {
    pick(0x2A2312, 0xFFF6DB)
}

/// The soft shadow under a card: depth without a hard edge.
pub fn card_shadow() -> egui::Shadow {
    // the light look keeps it to a breath: on white cards a darker shadow reads as grey shelves under every row
    let (a, blur) = if is_dark() { (90, 22) } else { (8, 14) };
    egui::Shadow { offset: [0, 6], blur, spread: 0, color: Color32::from_black_alpha(a) }
}

/// The shadow of a card under the pointer: it rises a little.
pub fn card_shadow_raised() -> egui::Shadow {
    let a = if is_dark() { 130 } else { 16 };
    egui::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(a) }
}

/// The faint light along the top edge of a glass surface.
pub fn c_glass_edge() -> Color32 {
    if is_dark() { Color32::from_white_alpha(14) } else { Color32::from_white_alpha(200) }
}

/// Corner radii, used everywhere so the shapes agree: cards and panels, things you press, small tags, dialogs.
pub const R_CARD: u8 = 12;
pub const R_CONTROL: u8 = 8;
pub const R_TAG: u8 = 6;
pub const R_DIALOG: u8 = 16;

/// The type scale: nothing else is used.
pub const FS_TITLE: f32 = 26.0;
pub const FS_FIGURE: f32 = 20.0;
pub const FS_SECTION: f32 = 17.0;
pub const FS_BODY: f32 = 14.0;
pub const FS_META: f32 = 12.0;

// The layout of the window: the sidebar and the room around a page.

/// The sidebar open, with the names beside the icons.
pub const SIDEBAR_W: f32 = 232.0;
/// The sidebar folded to a rail of icons.
pub const RAIL_W: f32 = 72.0;
/// Below this window width (in points, so a large interface size counts) the sidebar folds to its rail, so the
/// page keeps room for two columns of cards.
pub const RAIL_BELOW: f32 = 1024.0;
/// The widest any page's content gets: lists and grids use the whole width up to this, then sit in the middle.
pub const PAGE_MAX_W: f32 = 1680.0;

/// Whether the sidebar is folded to its rail in a window `window_w` points wide.
pub fn rail_folded(window_w: f32) -> bool {
    window_w < RAIL_BELOW
}

/// The room around a page in a page area `width` points wide: tighter in a small window, roomier on a big screen.
/// The right side is a little narrower because the scroll bar floats in it.
pub fn page_margin(width: f32) -> egui::Margin {
    let t = ((width - 700.0) / 800.0).clamp(0.0, 1.0);
    let side = (24.0 + 16.0 * t).round() as i8;
    egui::Margin { left: side, right: side - 8, top: 28, bottom: 20 }
}

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
    v.window_stroke = Stroke::new(1.0, c_border());
    v.window_corner_radius = CornerRadius::same(R_DIALOG);
    v.menu_corner_radius = CornerRadius::same(R_CARD);
    v.override_text_color = Some(c_text());
    v.hyperlink_color = c_text();
    v.selection.bg_fill = c_selection();
    v.selection.stroke = Stroke::new(1.0, c_text());
    // shadows only on what floats (menus, dialogs, tooltips)
    let shadow = Color32::from_black_alpha(if is_dark() { 170 } else { 40 });
    v.window_shadow = egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: shadow };
    v.popup_shadow = egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: shadow };

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
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size.y = 34.0;
        s.spacing.menu_margin = egui::Margin::same(8);
        s.spacing.scroll.bar_width = 8.0;
        // the floating scroll bar gets its own strip so it never sits on the edge of a card
        s.spacing.scroll.floating_allocated_width = 10.0;
        s.animation_time = if reduced_motion { 0.0 } else { 0.16 };
        s.text_styles = [
            (TextStyle::Heading, FontId::new(FS_TITLE, FontFamily::Proportional)),
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

    #[test]
    fn the_sidebar_folds_in_a_narrow_window_and_the_margins_grow_with_the_page() {
        assert!(rail_folded(900.0));
        assert!(rail_folded(1023.0));
        assert!(!rail_folded(1024.0));
        assert!(!rail_folded(1920.0));
        let narrow = page_margin(600.0);
        let wide = page_margin(2400.0);
        assert_eq!((narrow.left, narrow.right), (24, 16));
        assert_eq!((wide.left, wide.right), (40, 32));
        let mid = page_margin(1100.0);
        assert!(mid.left > narrow.left && mid.left < wide.left);
    }

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
