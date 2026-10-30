//! Settings: the same options, wording and defaults as the Java edition's Settings page, in groups with the ones
//! changed most often first and the rarely needed ones folded under "Advanced". A search at the top finds a setting
//! by its name, its explanation or a few words it is known by, and a jump list goes straight to a group. Every
//! change is saved as it is made, and a small "Saved" says so for a moment.

use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::App;
use super::icons::{self, Icon};
use super::motion;
use super::theme::{self, *};
use super::widgets::*;

/// The widest the page's content gets; a wider window leaves the rest empty rather than stretching the rows.
const PAGE_WIDTH: f32 = 720.0;
/// The list of sections at the left of a wide window, and the room between it and the settings.
const NAV_W: f32 = 204.0;
const NAV_GAP: f32 = 24.0;
/// The least room the settings keep beside the list; a narrower window shows the sections as chips above instead.
const NAV_CONTENT_MIN: f32 = 460.0;

/// How wide the drop-downs and text fields of the rows are, so they line up down a card.
pub(super) const CONTROL_W: f32 = 300.0;
/// The width of a drop-down in a row of [`CONTROL_W`] (its button leaves a little room at the edge).
pub(super) const COMBO_W: f32 = 290.0;
/// The room a switch takes at the right of a row.
const SWITCH_W: f32 = 44.0;
/// The least room the title and hint of a row keep beside the control; with less, the control goes under them.
const TEXT_MIN: f32 = 240.0;

/// How long "Saved" stays on the title row after a setting was saved, and how much of that it takes to fade out.
const SAVED_FOR: Duration = Duration::from_millis(2000);
const SAVED_FADE: Duration = Duration::from_millis(400);
/// The room "Saved" always takes, shown or not, so nothing on the title row moves.
const SAVED_WIDTH: f32 = 76.0;

fn saved_at_id() -> egui::Id {
    egui::Id::unique("settings-saved-at")
}

/// Note that the settings were just saved: the title row of the Settings page shows "Saved" for a moment.
pub(super) fn mark_saved(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(saved_at_id(), Instant::now()));
    ctx.request_repaint();
}

/// How much of "Saved" shows `since` it was saved: all of it for the first part of its time, then fading out, then
/// nothing (`None`).
fn saved_alpha(since: Duration) -> Option<f32> {
    if since >= SAVED_FOR {
        return None;
    }
    let left = SAVED_FOR - since;
    Some((left.as_secs_f32() / SAVED_FADE.as_secs_f32()).min(1.0))
}

/// "Saved" in a fixed slot at the right of the title row, there for two seconds after a save and then gone: a small
/// pill in the brand tint with a check mark.
fn saved_stamp(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SAVED_WIDTH, 24.0), egui::Sense::hover());
    let Some(at) = ui.data(|d| d.get_temp::<Instant>(saved_at_id())) else { return };
    let Some(alpha) = saved_alpha(at.elapsed()) else { return };
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::same(12), c_accent_soft().gamma_multiply(alpha));
    super::discover::paint_check(p, egui::pos2(rect.left() + 17.0, rect.center().y), 11.0, ink.gamma_multiply(alpha));
    p.text(
        egui::pos2(rect.left() + 28.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Saved",
        super::fonts::medium(FS_META),
        ink.gamma_multiply(alpha),
    );
    ui.ctx().request_repaint_after(Duration::from_millis(50));
}

// ---- the groups and the settings in them, as the search and the jump list know them

/// The sections of the Settings page, in the order the list at the left shows them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Group {
    Appearance,
    Game,
    Java,
    Network,
    Backups,
    Launcher,
    Privacy,
}

impl Group {
    const ALL: [Group; 7] =
        [Group::Appearance, Group::Game, Group::Java, Group::Network, Group::Backups, Group::Launcher, Group::Privacy];

    fn title(self) -> &'static str {
        match self {
            Group::Appearance => "Appearance",
            Group::Game => "Game",
            Group::Java => "Java & performance",
            Group::Network => "Downloads and network",
            Group::Backups => "Backups",
            Group::Launcher => "Launcher",
            Group::Privacy => "Privacy",
        }
    }

    /// The short name in the list at the left.
    fn short(self) -> &'static str {
        match self {
            Group::Network => "Downloads",
            g => g.title(),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Group::Appearance => "How the launcher looks",
            Group::Game => "Memory, the console and how games start",
            Group::Java => "Helpers that make games open sooner and run smoother, and the Java they run on",
            Group::Network => "Connection changes apply the next time BlockForge starts",
            Group::Backups => "Automatic backups of instances and where they are kept",
            Group::Launcher => "Updates, the tray and where files are kept",
            Group::Privacy => "Off unless you turn it on",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Group::Appearance => Icon::Layers,
            Group::Game => Icon::Play,
            Group::Java => Icon::Sliders,
            Group::Network => Icon::Download,
            Group::Backups => Icon::Folder,
            Group::Launcher => Icon::Home,
            Group::Privacy => Icon::Info,
        }
    }
}

/// One setting as the search knows it: its title and explanation (the row shows these, unless its explanation
/// changes with the setting), and a few more words it may be looked for by.
pub(super) struct Entry {
    key: &'static str,
    group: Group,
    /// Folded under "Advanced" at the end of its group.
    advanced: bool,
    title: &'static str,
    hint: &'static str,
    words: &'static str,
}

const fn entry(
    key: &'static str,
    group: Group,
    advanced: bool,
    title: &'static str,
    hint: &'static str,
    words: &'static str,
) -> Entry {
    Entry { key, group, advanced, title, hint, words }
}

use Group as G;

/// Every setting of the page, section by section in the order they are listed.
const ENTRIES: [Entry; 32] = [
    entry(
        "theme",
        G::Appearance,
        false,
        "Theme",
        "Dark, light or the same as your system, and the colour themes the Java edition knows",
        "dark light system colour color look material night",
    ),
    entry(
        "zoom",
        G::Appearance,
        false,
        "Interface size",
        "How big text and buttons are. Ctrl + and Ctrl \u{2212} change it too",
        "zoom scale font text size dpi bigger smaller percent",
    ),
    entry(
        "artwork",
        G::Appearance,
        false,
        "Show background artwork",
        "A faint pattern of blocks behind the pages",
        "pattern background blocks wallpaper decoration picture",
    ),
    entry(
        "title-bar",
        G::Appearance,
        false,
        "Use the system's title bar",
        "The desktop's own frame around the window instead of BlockForge's title bar",
        "title bar window frame decorations border minimize maximize close gnome wayland",
    ),
    entry(
        "light-mode",
        G::Appearance,
        false,
        "Light mode for slower PCs",
        "Fewer animations and a smaller picture cache, for computers with 4 GB of memory or less",
        "animations motion reduce slow low memory",
    ),
    entry(
        "memory",
        G::Game,
        false,
        "Memory",
        "How much memory a game may use: picked for each game and this computer, or set by hand",
        "ram gb mb heap xmx maximum automatic",
    ),
    entry(
        "keep-open",
        G::Game,
        false,
        "Keep the launcher open",
        "Leave BlockForge open while Minecraft is running",
        "close hide minimise minimize window",
    ),
    entry(
        "console",
        G::Game,
        false,
        "Show the console",
        "Open the log window next to the launcher",
        "log logs output crash debug",
    ),
    entry(
        "game-options",
        G::Game,
        false,
        "Pick game settings for this PC on first launch",
        "New games start with graphics that suit this computer",
        "options graphics video automatic first",
    ),
    entry(
        "gpu",
        G::Game,
        false,
        "Graphics card",
        "For laptops with two graphics cards. Applies the next time the game starts.",
        "gpu nvidia amd intel integrated dedicated power saving high performance",
    ),
    entry(
        "compiler",
        G::Java,
        true,
        "Java compiler",
        "Automatic uses the light one on computers with few cores, for much quicker starts. Full performance can give higher frame rates in long sessions.",
        "jit c1 c2 startup mode light full",
    ),
    entry(
        "java-path",
        G::Java,
        true,
        "Java path",
        "For games set not to use the Java that comes with Minecraft. Leave empty for automatic",
        "jdk jre runtime folder executable",
    ),
    entry(
        "java-args",
        G::Java,
        true,
        "Java arguments",
        "Extra options passed to the Java virtual machine",
        "jvm flags parameters options gc g1 aikar",
    ),
    entry(
        "fast-start",
        G::Java,
        false,
        "Fast start",
        super::performance::FAST_START_HINT,
        "speed quick open boost agent",
    ),
    entry(
        "smoothness",
        G::Java,
        false,
        "Smoothness helper",
        super::performance::LITE_HINT,
        "lite fps lag stutter fixes patches",
    ),
    entry(
        "low-end",
        G::Java,
        false,
        "Low-end mode",
        super::performance::LOW_END_HINT,
        "slow weak potato fps graphics view distance",
    ),
    entry(
        "class-cache",
        G::Java,
        false,
        "Speed up game start (class cache)",
        "Makes the next start of a game quicker (Minecraft 1.18 and newer)",
        "cds appcds speed quick",
    ),
    entry(
        "warm-up",
        G::Java,
        false,
        "Read game files ahead of Play",
        "Loads a game's files into memory while the launcher is idle so Play starts quicker",
        "preload cache disk speed warm",
    ),
    entry(
        "connections",
        G::Network,
        false,
        "Files downloaded at once",
        "More is faster on a good connection (1 to 100)",
        "concurrent downloads parallel threads speed",
    ),
    entry(
        "timeout",
        G::Network,
        false,
        "Connection timeout",
        "Seconds to wait for a server (1 to 600)",
        "slow network wait seconds internet",
    ),
    entry(
        "proxy",
        G::Network,
        false,
        "Use a proxy",
        "Send downloads through a proxy server",
        "socks http host port internet",
    ),
    entry(
        "modrinth-key",
        G::Network,
        true,
        "Modrinth API key",
        "Optional. Raises the limits Modrinth puts on requests",
        "token rate limit",
    ),
    entry(
        "downloads-folder",
        G::Network,
        true,
        "Downloads folder",
        "Where support bundles are saved. Empty uses your Downloads folder.",
        "directory location path support bundle",
    ),
    entry(
        "auto-backup",
        G::Backups,
        false,
        "Back up after playing",
        "Makes a backup every time a game closes",
        "automatic backup save worlds",
    ),
    entry(
        "backups-keep",
        G::Backups,
        false,
        "Backups to keep",
        "Per instance; the oldest go when a new one is made. 0 keeps them all",
        "count number limit old delete",
    ),
    entry(
        "backup-contents",
        G::Backups,
        false,
        "Backup contents",
        "What the automatic backup holds",
        "mode worlds config mods",
    ),
    entry(
        "backups-folder",
        G::Backups,
        true,
        "Backups folder",
        "Where instance backups are saved. Empty uses the backups folder in the data folder.",
        "directory location path",
    ),
    entry(
        "updates",
        G::Launcher,
        false,
        "Check for BlockForge updates",
        "Look on GitHub for a newer version and offer to install it",
        "update version upgrade new release",
    ),
    entry(
        "tray",
        G::Launcher,
        false,
        "Tray icon",
        "Show a tray menu while the launcher runs",
        "system tray menu notification area",
    ),
    entry(
        "data-folder",
        G::Launcher,
        false,
        "Data folder",
        "Where instances, mods and logs are stored",
        "files directory location open path",
    ),
    entry(
        "analytics",
        G::Privacy,
        false,
        "Share anonymous usage statistics",
        "Which screens are used and how often. Off unless you turn it on",
        "telemetry tracking data",
    ),
    entry(
        "pack-stats",
        G::Privacy,
        false,
        "Report pack installs and play time",
        "Counts for public modpacks, so authors can see how they are used. Off unless you turn it on. The network checker in Tools needs this on",
        "logs statistics telemetry modpack",
    ),
];

/// The setting called `key` (a name used in this file and services.rs).
fn find(key: &str) -> &'static Entry {
    ENTRIES.iter().find(|e| e.key == key).unwrap_or_else(|| panic!("no setting called {key}"))
}

/// The words of a search, in lower case. Only letters and digits count: "-Xmx" is looked for as "xmx".
fn search_terms(query: &str) -> Vec<String> {
    words(query).map(str::to_lowercase).collect()
}

/// The words of `text`: its runs of letters and digits.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// Whether every word of a search starts a word in one of `texts`, in any case ("mem" finds "Memory", but "ram"
/// doesn't find "frame"). No words match everything.
fn matches(terms: &[String], texts: &[&str]) -> bool {
    terms.iter().all(|t| texts.iter().any(|x| words(x).any(|w| w.to_lowercase().starts_with(t.as_str()))))
}

/// Whether `e` is found by a search: by its group's name, its title, its explanation or its other words.
fn entry_matches(terms: &[String], e: &Entry) -> bool {
    matches(terms, &[e.group.title(), e.title, e.hint, e.words])
}

/// Which settings the page shows: all of them, or the ones a search finds. Settings this computer has no use for
/// (`hidden`) are never shown.
pub(super) struct View {
    searching: bool,
    shown: Vec<&'static str>,
}

impl View {
    fn new(query: &str, hidden: &[&str]) -> View {
        let terms = search_terms(query);
        let shown =
            ENTRIES.iter().filter(|e| !hidden.contains(&e.key) && entry_matches(&terms, e)).map(|e| e.key).collect();
        View { searching: !terms.is_empty(), shown }
    }

    fn shows(&self, key: &str) -> bool {
        self.shown.contains(&key)
    }

    fn group_shown(&self, g: Group) -> bool {
        ENTRIES.iter().any(|e| e.group == g && self.shows(e.key))
    }

    /// How many of the folded settings of `g` are shown.
    fn advanced_count(&self, g: Group) -> usize {
        ENTRIES.iter().filter(|e| e.group == g && e.advanced && self.shows(e.key)).count()
    }

    #[cfg(test)]
    fn groups(&self) -> Vec<Group> {
        Group::ALL.into_iter().filter(|g| self.group_shown(*g)).collect()
    }
}

/// The section list's state, kept between frames: the section that is open.
#[derive(Clone, Copy)]
pub(super) struct Nav {
    open: Group,
}

impl Default for Nav {
    fn default() -> Nav {
        Nav { open: Group::Appearance }
    }
}

impl Nav {
    fn id() -> egui::Id {
        egui::Id::unique("settings-nav")
    }
}

/// Which sections the page draws: the open one, or while searching every one with a setting that matches.
fn drawn(view: &View, nav: &Nav, group: Group) -> bool {
    if view.searching { view.group_shown(group) } else { group == nav.open }
}

// ---- the parts of the page

/// The title of a block of settings and one muted line about it, above its card.
pub(super) fn section_head(ui: &mut Ui, title: &str, hint: &str) {
    ui.add_space(20.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(title).size(FS_SECTION).semibold());
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(FS_META).color(c_muted()));
        }
    });
    ui.add_space(8.0);
}

/// The small rounded tile of a group's icon: the icon in the brand colour on the brand tint.
pub(super) fn icon_tile(ui: &mut Ui, icon: Icon, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint_icon_tile(ui.painter(), rect, icon);
}

/// [`icon_tile`] painted at `rect`.
pub(super) fn paint_icon_tile(p: &egui::Painter, rect: egui::Rect, icon: Icon) {
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    p.rect(
        rect,
        egui::CornerRadius::same(R_CONTROL),
        c_accent_soft(),
        egui::Stroke::new(1.0, c_accent().gamma_multiply(0.22)),
        egui::StrokeKind::Inside,
    );
    icons::paint(p, icon, rect.center(), (rect.width() * 0.46).round(), ink);
}

/// The icon tile and the two lines of a group's head, without the room around it.
fn group_title(ui: &mut Ui, icon: Icon, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        icon_tile(ui, icon, 34.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).size(FS_SECTION).semibold());
            if !hint.is_empty() {
                ui.add(egui::Label::new(RichText::new(hint).size(FS_META).color(c_muted())).truncate());
            }
        });
    });
}

/// The rules between the rows of one card: call [`Rows::gap`] before each row.
pub(super) struct Rows {
    first: bool,
}

/// The room above and below the hairline between two rows.
const ROW_AIR: f32 = 12.0;

impl Rows {
    pub(super) fn new() -> Rows {
        Rows { first: true }
    }

    /// Nothing before the first row, a rule before every other.
    pub(super) fn gap(&mut self, ui: &mut Ui) {
        if std::mem::take(&mut self.first) {
            return;
        }
        // the same room above and below the line, whatever the spacing of the card is
        let pad = (ROW_AIR - ui.spacing().item_spacing.y).max(0.0);
        ui.add_space(pad);
        rule(ui);
        ui.add_space(pad);
    }
}

/// "Reset" after a row's title, for a setting that isn't at its default: a small dot in the brand colour that says
/// it was changed, and a quiet word that puts the default back. `default` names the default, on hover.
fn reset_link(ui: &mut Ui, title: &str, default: &str) -> egui::Response {
    let font = super::fonts::medium(FS_META);
    let galley = ui.painter().layout_no_wrap("Reset".to_string(), font, egui::Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 22.0, 18.0), egui::Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    let word = egui::Rect::from_min_max(egui::pos2(rect.left() + 9.0, rect.top()), rect.max);
    if t > 0.0 {
        ui.painter().rect_filled(word, egui::CornerRadius::same(R_TAG), c_hover().gamma_multiply(t));
    }
    ui.painter().circle_filled(egui::pos2(rect.left() + 3.0, rect.center().y), 3.0, ink);
    let fg = motion::mix(c_muted(), ink, t);
    ui.painter().galley(word.center() - galley.size() / 2.0, galley, fg);
    focus_ring(ui, &resp, word, R_TAG);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Reset {title}")));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Changed from the default. Reset puts back: {default}"))
}

/// The title of a row, followed by [`reset_link`] when `reset` names a default the setting isn't at. Returns true
/// when Reset was pressed.
pub(super) fn row_title(ui: &mut Ui, title: &str, reset: Option<&str>) -> bool {
    let Some(default) = reset else {
        ui.label(RichText::new(title).semibold());
        return false;
    };
    let mut pressed = false;
    ui.scope(|ui| {
        // as tall as the title, so the explanation sits as close under it as under a title without Reset
        ui.spacing_mut().interact_size.y = 0.0;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(title).semibold());
            pressed = reset_link(ui, title, default).clicked();
        });
    });
    pressed
}

/// A setting row: the title (with Reset after it when `reset` names a default the setting isn't at) and its
/// explanation on the left, the control at the right in `control_width`. Controls of one kind take one width, so they
/// line up down a card; on a narrow page the control goes under the text. Returns true when Reset was pressed.
pub(super) fn option_row(
    ui: &mut Ui,
    title: &str,
    hint: &str,
    control_width: f32,
    reset: Option<&str>,
    control: impl FnOnce(&mut Ui),
) -> bool {
    let room = ui.available_width();
    let mut pressed = false;
    let text = |ui: &mut Ui, pressed: &mut bool| {
        ui.spacing_mut().item_spacing.y = 2.0;
        *pressed = row_title(ui, title, reset);
        if !hint.is_empty() {
            small_muted(ui, hint);
        }
    };
    if room < control_width + TEXT_MIN {
        ui.vertical(|ui| text(ui, &mut pressed));
        ui.add_space(4.0);
        ui.horizontal_wrapped(control);
        ui.add_space(2.0);
        return pressed;
    }
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(room - control_width - 24.0);
            text(ui, &mut pressed);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    pressed
}

/// A setting row for a control wider than [`setting_row`] leaves room for (a drop-down, a text field, a segmented
/// control): the explanation keeps clear of the control, and on a narrow page the control goes under the text.
pub(super) fn wide_row(ui: &mut Ui, title: &str, hint: &str, control_width: f32, control: impl FnOnce(&mut Ui)) {
    option_row(ui, title, hint, control_width, None, control);
}

/// The card of one group as its rows are drawn: which rows the search shows, and the rules between them.
pub(super) struct Card<'v> {
    view: &'v View,
    rows: Rows,
}

impl Card<'_> {
    pub(super) fn shows(&self, key: &str) -> bool {
        self.view.shows(key)
    }

    /// The rule before a row that isn't one of [`Card::row`]'s (a line under a row about it, for one).
    pub(super) fn gap(&mut self, ui: &mut Ui) {
        self.rows.gap(ui);
    }

    /// The row of the setting `key` with its own title and explanation, when the search shows it. Returns true when
    /// its Reset was pressed.
    pub(super) fn row(
        &mut self,
        ui: &mut Ui,
        key: &str,
        width: f32,
        reset: Option<&str>,
        control: impl FnOnce(&mut Ui),
    ) -> bool {
        self.row_hint(ui, key, find(key).hint, width, reset, control)
    }

    /// [`Card::row`] with an explanation that says how the setting is right now.
    pub(super) fn row_hint(
        &mut self,
        ui: &mut Ui,
        key: &str,
        hint: &str,
        width: f32,
        reset: Option<&str>,
        control: impl FnOnce(&mut Ui),
    ) -> bool {
        if !self.shows(key) {
            return false;
        }
        self.rows.gap(ui);
        option_row(ui, find(key).title, hint, width, reset, control)
    }

    /// The row of an on/off setting whose default is `default`. Returns true when it changed (by its switch or by
    /// Reset).
    pub(super) fn switch(&mut self, ui: &mut Ui, key: &str, on: &mut bool, default: bool) -> bool {
        let reset = (*on != default).then_some(if default { "On" } else { "Off" });
        let mut changed = false;
        if self.row(ui, key, SWITCH_W, reset, |ui| changed = toggle(ui, on).changed()) {
            *on = default;
            changed = true;
        }
        changed
    }

    /// A one-line text setting, saved when the field is left or Enter is pressed: `(the typed text then, whether
    /// Reset was pressed)`.
    pub(super) fn text(
        &mut self,
        ui: &mut Ui,
        key: &str,
        placeholder: &str,
        value: &str,
        reset: Option<&str>,
    ) -> (Option<String>, bool) {
        let mut committed = None;
        let pressed = self.row(ui, key, CONTROL_W, reset, |ui| {
            let width = ui.available_width().clamp(160.0, CONTROL_W);
            committed = draft_text(ui, key, value, placeholder, [width, 32.0], egui::Margin::symmetric(10, 7));
        });
        (committed, pressed)
    }
}

/// Which part of a group's rows to draw: the ones always there, or the ones folded under "Advanced".
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Main,
    Advanced,
}

/// A section of the Settings page, when it is the open one (or, while searching, when the search finds any of it): its
/// head, then one card of glass holding the rows (`rows` draws them, once for each [`Part`]). The advanced ones are folded under "Advanced" at the end of the card,
/// which says how many of them were changed (`changed`); while searching, they are shown like the others.
pub(super) fn section(
    ui: &mut Ui,
    view: &View,
    nav: &Nav,
    group: Group,
    changed: usize,
    mut rows: impl FnMut(&mut Ui, &mut Card, Part),
) {
    if !view.group_shown(group) || !drawn(view, nav, group) {
        return;
    }
    // the first section starts at the top of the page; the room above a head is for between two sections
    let first = Group::ALL.into_iter().find(|g| view.group_shown(*g) && drawn(view, nav, *g)) == Some(group);
    ui.add_space(if first { 0.0 } else { 26.0 });
    group_title(ui, group.icon(), group.title(), group.hint());
    ui.add_space(10.0);
    card(ui, |ui| {
        // the title of a row sits close to its explanation
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut c = Card { view, rows: Rows::new() };
        rows(ui, &mut c, Part::Main);
        let folded = view.advanced_count(group);
        if folded == 0 {
            return;
        }
        if view.searching {
            rows(ui, &mut c, Part::Advanced);
        } else {
            advanced_fold(ui, group, folded, changed, &mut c, |ui, c| rows(ui, c, Part::Advanced));
        }
    });
}

/// The fold at the end of a group's card that holds its rarely needed settings: a line with a chevron that turns,
/// "Advanced", how many settings are in it and how many of them were changed. The line opens and closes it, by
/// pointer or by keyboard, and the rows slide open under it.
fn advanced_fold(
    ui: &mut Ui,
    group: Group,
    count: usize,
    changed: usize,
    c: &mut Card,
    add: impl FnOnce(&mut Ui, &mut Card),
) {
    let id = egui::Id::unique(("settings-advanced", group.title()));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    c.gap(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    if resp.clicked() {
        state.toggle(ui);
    }
    let open = state.openness(ui.ctx());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let p = ui.painter();
    if t > 0.0 {
        p.rect_filled(
            rect.expand2(egui::vec2(6.0, 0.0)),
            egui::CornerRadius::same(R_CONTROL),
            c_hover().gamma_multiply(t),
        );
    }
    // the chevron points right when folded and down when open
    let cx = egui::pos2(rect.left() + 6.0, rect.center().y);
    let a = open * std::f32::consts::FRAC_PI_2;
    let rot = |x: f32, y: f32| cx + egui::vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
    let fg = motion::mix(c_muted(), c_text(), t.max(open));
    p.add(egui::Shape::line(vec![rot(-2.0, -4.5), rot(2.5, 0.0), rot(-2.0, 4.5)], egui::Stroke::new(1.6, fg)));
    let title = p.layout_no_wrap("Advanced".to_string(), super::fonts::medium(FS_BODY), fg);
    let title_w = title.size().x;
    p.galley(egui::pos2(rect.left() + 20.0, rect.center().y - title.size().y / 2.0), title, fg);
    let more = if count == 1 { "1 setting".to_string() } else { format!("{count} settings") };
    let mut x = rect.left() + 20.0 + title_w + 10.0;
    let meta = p.layout_no_wrap(more, super::fonts::medium(FS_META), c_faint());
    let meta_w = meta.size().x;
    p.galley(egui::pos2(x, rect.center().y - meta.size().y / 2.0), meta, c_faint());
    if changed > 0 {
        // changed settings that are folded away still say so
        x += meta_w + 10.0;
        let ink = if is_dark() { c_accent() } else { c_accent_hover() };
        p.circle_filled(egui::pos2(x + 3.0, rect.center().y), 3.0, ink);
        let text = if changed == 1 { "1 changed".to_string() } else { format!("{changed} changed") };
        let g = p.layout_no_wrap(text, super::fonts::medium(FS_META), ink);
        p.galley(egui::pos2(x + 10.0, rect.center().y - g.size().y / 2.0), g, ink);
    }
    focus_ring(ui, &resp, rect.expand2(egui::vec2(6.0, 0.0)), R_CONTROL);
    let is_open = state.is_open();
    let label = format!("Advanced {} settings", group.title());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, is_open, &label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    state.show_body_unindented(ui, |ui| add(ui, c));
}

/// The search field at the top of the page: a magnifier at its left, and once something is typed, a cross at its
/// right that clears it (Escape does too). Ctrl+F (Cmd+F) goes to it.
fn search_box(ui: &mut Ui, query: &mut String, width: f32) {
    let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F);
    // the shortcut is named in the field when there is room for it
    let hint = if width >= 280.0 {
        format!("Search settings  ({})", ui.ctx().format_shortcut(&shortcut))
    } else {
        "Search settings".to_string()
    };
    let r = ui.add_sized(
        [width, 36.0],
        egui::TextEdit::singleline(query)
            .id(egui::Id::unique("settings-search"))
            .hint_text(RichText::new(hint).color(c_faint()))
            .margin(egui::Margin { left: 34, right: 34, top: 8, bottom: 8 })
            .background_color(c_field()),
    );
    let fg = if r.has_focus() { c_text() } else { c_muted() };
    icons::paint(ui.painter(), Icon::Search, egui::pos2(r.rect.left() + 18.0, r.rect.center().y), 15.0, fg);
    if ui.input_mut(|i| i.consume_shortcut(&shortcut)) {
        r.request_focus();
    }
    if (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        query.clear();
    }
    if query.is_empty() {
        return;
    }
    let x = egui::Rect::from_center_size(egui::pos2(r.rect.right() - 18.0, r.rect.center().y), egui::vec2(24.0, 24.0));
    let clear = ui.interact(x, r.id.with("clear"), egui::Sense::click());
    let t = motion::hover(ui, clear.id, clear.hovered());
    if t > 0.0 {
        ui.painter().rect_filled(x, egui::CornerRadius::same(R_TAG), c_hover().gamma_multiply(t));
    }
    let ink = egui::Stroke::new(1.5, motion::mix(c_muted(), c_text(), t));
    let (c, s) = (x.center(), 4.0);
    ui.painter().line_segment([c + egui::vec2(-s, -s), c + egui::vec2(s, s)], ink);
    ui.painter().line_segment([c + egui::vec2(-s, s), c + egui::vec2(s, -s)], ink);
    focus_ring(ui, &clear, x, R_TAG);
    clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Clear search"));
    if clear.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Clear search").clicked() {
        query.clear();
        r.request_focus();
    }
}

/// One line of the list at the left: an icon and a name, tinted when it is the open section. `count` is how many
/// settings of it a search found.
fn nav_item(ui: &mut Ui, icon: Icon, label: &str, current: bool, count: Option<usize>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    let s = motion::toward(ui.ctx(), resp.id.with("on"), current, motion::MEDIUM);
    let p = ui.painter();
    let shape = egui::CornerRadius::same(R_CONTROL);
    if t > 0.0 {
        p.rect_filled(rect, shape, c_hover().gamma_multiply(t * (1.0 - s)));
    }
    if s > 0.0 {
        p.rect_filled(rect, shape, c_accent_soft().gamma_multiply(s));
        // a short bar in the brand gradient at the left edge grows in as the section opens
        let h = 18.0 * s;
        let bar = egui::Rect::from_center_size(egui::pos2(rect.left() + 1.5, rect.center().y), egui::vec2(3.0, h));
        motion::brand_rect(p, bar, 2);
    }
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    let icon_c = motion::mix(c_muted(), ink, s);
    icons::paint(p, icon, egui::pos2(rect.left() + 20.0, rect.center().y), 15.0, icon_c);
    let fg = motion::mix(c_muted(), c_text(), t.max(s));
    let g = p.layout_no_wrap(label.to_string(), super::fonts::medium(FS_BODY), fg);
    p.with_clip_rect(rect.shrink(2.0)).galley(
        egui::pos2(rect.left() + 38.0, rect.center().y - g.size().y / 2.0),
        g,
        fg,
    );
    if let Some(n) = count {
        let g = p.layout_no_wrap(n.to_string(), super::fonts::mono(FS_META), c_muted());
        let pill = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 8.0 - (g.size().x + 12.0) / 2.0, rect.center().y),
            egui::vec2(g.size().x + 12.0, 20.0),
        );
        p.rect_filled(pill, egui::CornerRadius::same(10), c_field());
        p.galley(pill.center() - g.size() / 2.0, g, c_muted());
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, current, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What a press in the list at the left asks for.
enum NavPick {
    Section(Group),
    Page(super::Page),
}

/// The list of sections down the left of a wide window (a row of chips over the settings in a narrow one), and
/// under it the two other pages of the account corner. While searching, each section says how many settings it
/// found.
fn nav_list(ui: &mut Ui, view: &View, nav: &Nav, side: bool, here: super::Page) -> Option<NavPick> {
    let on_settings = here == super::Page::Settings;
    let mut pick = None;
    let count = |g: Group| view.searching.then(|| ENTRIES.iter().filter(|e| e.group == g && view.shows(e.key)).count());
    if side {
        ui.spacing_mut().item_spacing.y = 2.0;
        for g in Group::ALL {
            let n = count(g);
            if n == Some(0) {
                continue;
            }
            if nav_item(ui, g.icon(), g.short(), on_settings && !view.searching && nav.open == g, n).clicked() {
                pick = Some(NavPick::Section(g));
            }
        }
        ui.add_space(8.0);
        rule(ui);
        ui.add_space(8.0);
        if nav_item(ui, Icon::User, "Accounts", here == super::Page::Accounts, None).clicked() {
            pick = Some(NavPick::Page(super::Page::Accounts));
        }
        if nav_item(ui, Icon::Info, "About", here == super::Page::About, None).clicked() {
            pick = Some(NavPick::Page(super::Page::About));
        }
        return pick;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for g in Group::ALL {
            if count(g) == Some(0) {
                continue;
            }
            if chip(ui, g.short(), on_settings && !view.searching && nav.open == g).clicked() {
                pick = Some(NavPick::Section(g));
            }
        }
        if chip(ui, "Accounts", here == super::Page::Accounts).clicked() {
            pick = Some(NavPick::Page(super::Page::Accounts));
        }
        if chip(ui, "About", here == super::Page::About).clicked() {
            pick = Some(NavPick::Page(super::Page::About));
        }
    });
    pick
}

/// What the page shows when the search finds nothing. Returns true when "Clear search" was pressed.
fn no_matches(ui: &mut Ui, query: &str) -> bool {
    let mut clear = false;
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        icon_tile(ui, Icon::Search, 44.0);
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("No settings match \u{201C}{}\u{201D}", query.trim())).size(FS_SECTION).semibold(),
        );
        ui.add_space(4.0);
        muted(ui, "Try another word, such as memory, Java, theme or backup.");
        ui.add_space(14.0);
        clear = secondary(ui, "Clear search").clicked();
    });
    clear
}

fn query_id() -> egui::Id {
    egui::Id::unique("settings-search-query")
}

/// The interface sizes the Appearance row offers.
const ZOOMS: [(f32, &str); 5] = [(0.9, "90%"), (1.0, "100%"), (1.1, "110%"), (1.25, "125%"), (1.5, "150%")];

/// The offered size closest to `zoom` (Ctrl and + can land between two of them).
fn nearest_zoom(zoom: f32) -> usize {
    (0..ZOOMS.len()).min_by(|&a, &b| (ZOOMS[a].0 - zoom).abs().total_cmp(&(ZOOMS[b].0 - zoom).abs())).unwrap_or(1)
}

pub(super) fn memory_text(mb: u32) -> String {
    if mb % 1024 == 0 { format!("{} GB", mb / 1024) } else { format!("{:.1} GB", mb as f32 / 1024.0) }
}

/// The computer's memory (total, free) in MB, read again at most every few seconds rather than on every frame.
pub(super) fn memory_now() -> (u64, u64) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<(Instant, (u64, u64))>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    match *last {
        Some((at, mb)) if at.elapsed() < Duration::from_secs(3) => mb,
        _ => {
            let mb = crate::platform::memory_mb();
            *last = Some((Instant::now(), mb));
            mb
        }
    }
}

/// Whether `home` holds a Java (`bin/java`), remembered for the last path asked about so the file isn't looked at
/// on every frame.
fn java_found(home: &str) -> bool {
    use std::sync::Mutex;
    static LAST: Mutex<Option<(String, bool)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, found)) = last.as_ref().filter(|(p, _)| p == home) {
        return *found;
    }
    let found = crate::java::java_binary(std::path::Path::new(home)).is_file();
    *last = Some((home.to_string(), found));
    found
}

/// A one-line text field for a setting. What is typed is kept between frames (the field shows `value` again once it
/// is left) and handed back when the field is left or Enter is pressed: `Some(text)` then, `None` otherwise.
pub(super) fn draft_text(
    ui: &mut Ui,
    id_salt: &str,
    value: &str,
    placeholder: &str,
    size: [f32; 2],
    margin: egui::Margin,
) -> Option<String> {
    let id = egui::Id::unique(("settings-draft", id_salt));
    let stored = ui.data(|d| d.get_temp::<String>(id));
    // egui can report the focus as lost in the frame after Enter too; only a field that was being edited answers
    let editing = stored.is_some();
    let mut draft = stored.unwrap_or_else(|| value.to_string());
    let r = ui.add_sized(
        size,
        egui::TextEdit::singleline(&mut draft)
            .id_salt(id_salt)
            .hint_text(placeholder)
            .margin(margin)
            .background_color(c_field()),
    );
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        editing.then_some(draft)
    } else if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, draft));
        None
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
        // a draft left over without the field ever saying it was left: the page was switched (the sidebar is drawn
        // before the page, so the field wasn't drawn again to notice). What was typed is saved now, not dropped.
        (editing && draft != value).then_some(draft)
    }
}

/// The warning under the Java path row when the folder typed there has no Java in it.
fn java_path_warning(custom: Option<&str>) -> Option<&'static str> {
    custom.filter(|p| !java_found(p)).map(|_| {
        "No Java was found in that folder, so the automatic one is used. Enter the Java folder, the one with bin in it."
    })
}

/// The "Graphics card" row for computers with two cards. `gpus` is `None` while they are being looked for. Returns
/// true when the choice changed.
fn graphics_card_row(ui: &mut Ui, c: &mut Card, setting: &mut String, gpus: Option<&[crate::gpu::Gpu]>) -> bool {
    use crate::gpu::{self, Choice};
    let list = gpus.unwrap_or(&[]);
    let choice = Choice::parse(setting);
    let two = list.len() >= 2;
    let name_of = |kind: &Choice| match gpu::resolve(kind, list) {
        Some(g) => format!(" ({})", g.name),
        None => String::new(),
    };
    // the drop-down's own text stays short; the list shows which card each choice means
    let short = |c: &Choice| match c {
        Choice::Auto => "Automatic".to_string(),
        Choice::Integrated => "Power saving".to_string(),
        Choice::Dedicated => "High performance".to_string(),
        Choice::Specific(id) => match list.iter().find(|g| &g.id == id) {
            Some(g) => g.name.clone(),
            None => "A card that isn't here".to_string(),
        },
    };
    let long = |c: &Choice| match c {
        Choice::Integrated | Choice::Dedicated => format!("{}{}", short(c), name_of(c)),
        _ => short(c),
    };
    let hint = match (gpus, list.len()) {
        (None, _) => "Looking for graphics cards\u{2026}".to_string(),
        (Some(_), 0) => "Couldn't find out which graphics cards this computer has".to_string(),
        (Some(_), 1) => format!("Only one graphics card was found ({}), so there is nothing to choose", list[0].name),
        _ => match gpu::resolve(&choice, list) {
            Some(g) => format!("The game uses {}. Applies the next time it starts.", g.name),
            None => find("gpu").hint.to_string(),
        },
    };
    let reset = (choice != Choice::Auto).then_some("Automatic");
    let mut changed = false;
    let pressed = c.row_hint(ui, "gpu", &hint, CONTROL_W, reset, |ui| {
        ui.add_enabled_ui(two, |ui| {
            egui::ComboBox::from_id_salt("gpu").selected_text(short(&choice)).width(COMBO_W).show_ui(ui, |ui| {
                let mut options = vec![Choice::Auto, Choice::Integrated, Choice::Dedicated];
                if gpu::can_pick_by_name() {
                    options.extend(list.iter().map(|g| Choice::Specific(g.id.clone())));
                }
                for o in options {
                    if ui.selectable_label(choice == o, long(&o)).clicked() && choice != o {
                        *setting = o.as_setting();
                        changed = true;
                    }
                }
            });
        });
    });
    if pressed {
        *setting = Choice::Auto.as_setting();
        changed = true;
    }
    changed
}

/// The top of the memory slider: half of the computer's memory in steps of 256 MB, at least 2 GB.
fn memory_slider_max(total_mb: u64) -> u32 {
    let half = if total_mb > 0 { total_mb / 2 / 256 * 256 } else { 8192 };
    (half.min(u32::MAX as u64) as u32).max(2048)
}

/// Where the settings keep "Show background artwork" (on unless it was turned off), among the launcher's extras.
const ARTWORK_KEY: &str = "backgroundArtwork";
/// Where the settings keep that the look follows the system's, among the launcher's extras.
const FOLLOW_SYSTEM_KEY: &str = "themeFollowsSystem";
/// The light theme the Light card picks (the Dark card picks the default theme).
const LIGHT_THEME: &str = "com.blockforge.launcher.themes.Material3Light";

/// Whether the faint pattern of blocks is drawn behind the pages (Settings: "Show background artwork").
pub(super) fn artwork_on(s: &crate::config::Settings) -> bool {
    s.extras.get(ARTWORK_KEY).and_then(|v| v.as_bool()).unwrap_or(true)
}

/// Turn the background artwork on or off (on, the default, is not written).
fn set_artwork(s: &mut crate::config::Settings, on: bool) {
    if on {
        s.extras.remove(ARTWORK_KEY);
    } else {
        s.extras.insert(ARTWORK_KEY.into(), false.into());
    }
}

/// Whether the look follows the system's (the System card).
fn follows_system(s: &crate::config::Settings) -> bool {
    s.extras.get(FOLLOW_SYSTEM_KEY).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Whether the launcher is drawn dark: as the system is when it follows the system (and the system says), else as
/// the colour theme is.
pub(super) fn look_is_dark(s: &crate::config::Settings, system_dark: Option<bool>) -> bool {
    match system_dark.filter(|_| follows_system(s)) {
        Some(dark) => dark,
        None => theme::theme_is_dark(&s.theme),
    }
}

/// Pick one of the theme cards: 0 dark, 1 light, 2 the system's. A colour theme that is already of the right kind is
/// kept, so a player's Dracula stays when Dark is pressed again; the system's look also sets a theme of its kind, for
/// the Java edition, which reads the same file.
fn set_look(s: &mut crate::config::Settings, card: usize, system_dark: Option<bool>) {
    let dark = match card {
        0 => true,
        1 => false,
        _ => system_dark.unwrap_or_else(|| theme::theme_is_dark(&s.theme)),
    };
    if card == 2 {
        s.extras.insert(FOLLOW_SYSTEM_KEY.into(), true.into());
    } else {
        s.extras.remove(FOLLOW_SYSTEM_KEY);
    }
    if theme::theme_is_dark(&s.theme) != dark {
        s.theme = if dark { crate::config::DEFAULT_THEME } else { LIGHT_THEME }.to_string();
    }
}

/// The three cards of the Theme row, Dark, Light and System, each with a small picture of the launcher in that look.
/// Returns the card that was pressed.
fn theme_cards(ui: &mut Ui, current: usize, system_dark: Option<bool>) -> Option<usize> {
    const GAP: f32 = 12.0;
    let w = ((ui.available_width() - 2.0 * GAP) / 3.0).clamp(96.0, 220.0).floor();
    let preview_h = (w * 0.56).round();
    let mut pressed = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let system = match system_dark {
            Some(true) => "System (dark now)",
            Some(false) => "System (light now)",
            None => "System",
        };
        for (i, label) in ["Dark", "Light", system].into_iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, preview_h + 30.0), egui::Sense::click());
            let on = i == current;
            let t = motion::hover(ui, resp.id, resp.hovered());
            let s = motion::toward(ui.ctx(), resp.id.with("on"), on, motion::MEDIUM);
            let preview = egui::Rect::from_min_size(rect.min, egui::vec2(w, preview_h));
            let p = ui.painter();
            match i {
                0 => paint_preview(p, preview, true),
                1 => paint_preview(p, preview, false),
                _ => {
                    // half and half: the system decides
                    paint_preview(&p.with_clip_rect(preview.with_max_x(preview.center().x)), preview, true);
                    paint_preview(&p.with_clip_rect(preview.with_min_x(preview.center().x)), preview, false);
                }
            }
            let edge = motion::mix(motion::mix(c_border(), c_control(), t), c_accent(), s);
            p.rect_stroke(
                preview.expand(if s > 0.0 { 2.0 } else { 0.0 }),
                egui::CornerRadius::same(R_CONTROL + 2),
                egui::Stroke::new(1.0 + s, edge),
                egui::StrokeKind::Inside,
            );
            if s > 0.0 {
                // a check in the brand colour at the picture's corner
                let c = egui::pos2(preview.right() - 12.0, preview.top() + 12.0);
                p.circle_filled(c, 8.0, c_accent().gamma_multiply(s));
                super::discover::paint_check(p, c, 9.0, c_on_accent().gamma_multiply(s));
            }
            let fg = motion::mix(c_muted(), c_text(), t.max(s));
            let font = if on { super::fonts::semibold(FS_BODY) } else { super::fonts::medium(FS_BODY) };
            p.text(egui::pos2(rect.center().x, preview.bottom() + 16.0), egui::Align2::CENTER_CENTER, label, font, fg);
            focus_ring(ui, &resp, preview, R_CONTROL + 2);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, label));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                pressed = Some(i);
            }
        }
    });
    pressed
}

/// A small picture of the launcher in the dark or the light look: the sidebar with its places, a title, a card with
/// a Play button and two more cards. Drawn in that look's own colours, whatever the window's look is now.
fn paint_preview(p: &egui::Painter, rect: egui::Rect, dark: bool) {
    let rgb = |v: u32| egui::Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    let (ground, side, card, line, ink, faint, accent) = if dark {
        (rgb(0x0B1016), rgb(0x0D131B), rgb(0x151D29), rgb(0x212D3F), rgb(0xE8EDF4), rgb(0x3A4658), rgb(0x2DD4AA))
    } else {
        (rgb(0xF2F4F7), rgb(0xF8F9FB), rgb(0xFFFFFF), rgb(0xDDE2E9), rgb(0x0D141C), rgb(0xC9D0DA), rgb(0x0A7A62))
    };
    let r = egui::CornerRadius::same(R_CONTROL + 2);
    p.rect_filled(rect, r, ground);
    let u = rect.width() / 100.0;
    let at = |x: f32, y: f32, w: f32, h: f32| {
        egui::Rect::from_min_size(rect.min + egui::vec2(x * u, y * u), egui::vec2(w * u, h * u))
    };
    // the sidebar and its places, the first one open
    p.rect_filled(
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 26.0 * u, rect.bottom())),
        egui::CornerRadius { nw: R_CONTROL + 2, sw: R_CONTROL + 2, ne: 0, se: 0 },
        side,
    );
    p.rect_filled(at(4.0, 6.0, 7.0, 7.0), 2.0, accent);
    for k in 0..4 {
        let y = 18.0 + k as f32 * 7.0;
        let fill = if k == 0 { accent.gamma_multiply(0.35) } else { faint };
        p.rect_filled(at(4.0, y, 18.0, 3.5), 1.5, fill);
    }
    // the page: a title, a wide card with Play, two small cards
    p.rect_filled(at(31.0, 7.0, 30.0, 4.5), 1.5, ink);
    p.rect_filled(at(31.0, 14.0, 44.0, 2.5), 1.0, faint);
    let big = at(31.0, 20.0, 63.0, 15.0);
    p.rect_filled(big, 3.0, card);
    p.rect_stroke(big, 3.0, egui::Stroke::new(1.0, line), egui::StrokeKind::Inside);
    p.rect_filled(at(35.0, 24.0, 22.0, 3.0), 1.0, ink);
    p.rect_filled(at(35.0, 29.5, 14.0, 2.5), 1.0, faint);
    p.rect_filled(at(78.0, 24.5, 12.0, 6.0), 2.0, accent);
    for k in 0..2 {
        let c = at(31.0 + k as f32 * 32.5, 39.0, 30.5, 13.0);
        p.rect_filled(c, 3.0, card);
        p.rect_stroke(c, 3.0, egui::Stroke::new(1.0, line), egui::StrokeKind::Inside);
        p.rect_filled(at(34.0 + k as f32 * 32.5, 43.0, 16.0, 2.5), 1.0, faint);
    }
}

impl App {
    pub(super) fn settings_ui(&mut self, ui: &mut Ui) {
        // the page's room less the scroll bar's strip at the window's edge. A wide window has the list of sections
        // down the left and the open section beside it; a narrow one has the sections as chips over it.
        let room = ui.available_width() - 14.0;
        let side = room >= NAV_W + NAV_GAP + NAV_CONTENT_MIN;
        let content_w = if side { (room - NAV_W - NAV_GAP).min(PAGE_WIDTH) } else { room.min(PAGE_WIDTH) };
        let full_w = if side { NAV_W + NAV_GAP + content_w } else { content_w };
        // the title, and at the right of the same row a "Saved" that shows for a moment after each save
        ui.scope(|ui| {
            ui.set_max_width(full_w);
            PageHeader::new("Settings").subtitle("Every change is saved as soon as it is made.").show(ui, saved_stamp);
        });

        let mut query = ui.data(|d| d.get_temp::<String>(query_id())).unwrap_or_default();
        let mut nav = ui.data(|d| d.get_temp::<Nav>(Nav::id())).unwrap_or_default();
        // the graphics card can only be chosen on some systems
        let hidden: &[&str] = if crate::gpu::supported() { &[] } else { &["gpu"] };

        let mut dirty = false;
        let mut dragged = false;
        let mut theme_changed = false;
        let mut open_data = false;
        let mut go: Option<super::Page> = None;
        let (total_mb, free_mb) = memory_now();
        let max_mb = memory_slider_max(total_mb);
        let example = self.automatic_memory_example(total_mb, free_mb);
        let self_gpus = self.gpus.clone();
        let data_dir = self.dirs.base.display().to_string();
        let perf = super::performance::SettingsView::new(self, ui.ctx());
        let system_dark = ui.ctx().system_theme().map(|t| t == egui::Theme::Dark);

        let area = ui.available_rect_before_wrap();
        let main = if side {
            egui::Rect::from_min_max(egui::pos2(area.left() + NAV_W + NAV_GAP, area.top()), area.max)
        } else {
            area
        };
        // the search and the list of sections come first for the keyboard (Tab), wherever they are on the page
        let view = if side {
            let list = egui::Rect::from_min_size(area.min, egui::vec2(NAV_W, area.height()));
            let mut list = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
            search_box(&mut list, &mut query, NAV_W);
            list.add_space(12.0);
            let view = View::new(&query, hidden);
            match nav_list(&mut list, &view, &nav, true, super::Page::Settings) {
                Some(NavPick::Section(g)) => {
                    nav.open = g;
                    query.clear();
                }
                Some(NavPick::Page(p)) => {
                    go = Some(p);
                    query.clear();
                }
                None => {}
            }
            View::new(&query, hidden)
        } else {
            let mut view = View::new(&query, hidden);
            ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
                search_box(ui, &mut query, content_w);
                ui.add_space(10.0);
                ui.scope(|ui| {
                    ui.set_max_width(content_w);
                    view = View::new(&query, hidden);
                    match nav_list(ui, &view, &nav, false, super::Page::Settings) {
                        Some(NavPick::Section(g)) => {
                            nav.open = g;
                            query.clear();
                        }
                        Some(NavPick::Page(p)) => {
                            go = Some(p);
                            query.clear();
                        }
                        None => {}
                    }
                });
            });
            View::new(&query, hidden)
        };
        let top = if side { main.top() } else { ui.cursor().top() + 4.0 };
        let main = egui::Rect::from_min_max(egui::pos2(main.left(), top), main.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
            ScrollArea::vertical()
                .id_salt(("settings-page", nav.open.title(), view.searching))
                .auto_shrink(false)
                .show(ui, |ui| {
                    // at most PAGE_WIDTH wide, but never wider than the window (set_max_width alone would widen a
                    // narrow page)
                    ui.set_max_width(content_w.min(ui.available_width()));
                    if view.searching {
                        if view.shown.is_empty() {
                            if no_matches(ui, &query) {
                                query.clear();
                            }
                        } else {
                            let n = view.shown.len();
                            let found =
                                if n == 1 { "1 setting found".to_string() } else { format!("{n} settings found") };
                            ui.label(
                                RichText::new(format!("{found} for \u{201C}{}\u{201D}", query.trim()))
                                    .size(FS_SECTION)
                                    .semibold(),
                            );
                            ui.add_space(14.0);
                        }
                    }
                    let s = &mut self.settings;
                    section(ui, &view, &nav, Group::Appearance, 0, |ui, c, part| {
                        if part != Part::Main {
                            return;
                        }
                        let follows = follows_system(s);
                        if c.shows("theme") {
                            c.gap(ui);
                            row_title(ui, find("theme").title, None);
                            small_muted(ui, find("theme").hint);
                            ui.add_space(8.0);
                            let current = if follows {
                                2
                            } else if theme::theme_is_dark(&s.theme) {
                                0
                            } else {
                                1
                            };
                            if let Some(i) = theme_cards(ui, current, system_dark) {
                                if i != current {
                                    set_look(s, i, system_dark);
                                    theme_changed = true;
                                    dirty = true;
                                }
                            }
                            ui.add_space(10.0);
                            // the whole list of colour themes the Java edition knows, which this look follows as dark or
                            // light
                            let reset = (s.theme != crate::config::DEFAULT_THEME || follows)
                                .then(|| theme::theme_label(crate::config::DEFAULT_THEME));
                            let pressed = option_row(
                                ui,
                                "Colour theme",
                                "Shared with the Java edition; BlockForge draws it dark or light",
                                CONTROL_W,
                                reset,
                                |ui| {
                                    egui::ComboBox::from_id_salt("theme")
                                        .selected_text(theme::theme_label(&s.theme))
                                        .width(COMBO_W)
                                        .show_ui(ui, |ui| {
                                            for (id, label, _) in theme::THEMES {
                                                if ui.selectable_label(s.theme == id, label).clicked() && s.theme != id
                                                {
                                                    s.theme = id.to_string();
                                                    s.extras.remove(FOLLOW_SYSTEM_KEY);
                                                    theme_changed = true;
                                                    dirty = true;
                                                }
                                            }
                                        });
                                },
                            );
                            if pressed {
                                s.theme = crate::config::DEFAULT_THEME.to_string();
                                s.extras.remove(FOLLOW_SYSTEM_KEY);
                                theme_changed = true;
                                dirty = true;
                            }
                        }
                        let zoom = ui.ctx().zoom_factor();
                        let reset = ((zoom - 1.0).abs() > 0.001).then_some("100%");
                        let pressed = c.row(ui, "zoom", CONTROL_W, reset, |ui| {
                            let labels: Vec<&str> = ZOOMS.iter().map(|z| z.1).collect();
                            if let Some(i) = segmented(ui, &labels, nearest_zoom(zoom)) {
                                // the window applies it (and keeps it for the next start) on the next frame
                                ui.ctx().set_zoom_factor(ZOOMS[i].0);
                            }
                        });
                        if pressed {
                            ui.ctx().set_zoom_factor(1.0);
                        }
                        let mut artwork = artwork_on(s);
                        if c.switch(ui, "artwork", &mut artwork, true) {
                            set_artwork(s, artwork);
                            dirty = true;
                        }
                        if cfg!(target_os = "linux") {
                            let mut system = !super::titlebar::own(s);
                            if c.switch(ui, "title-bar", &mut system, false) {
                                super::titlebar::set_system(s, system);
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Decorations(system));
                                dirty = true;
                            }
                        }
                        let low_end = super::low_end_pc();
                        let hint = if low_end {
                            "Fewer animations and a smaller picture cache. On by default for this computer"
                        } else {
                            find("light-mode").hint
                        };
                        let mut on = s.light_mode.unwrap_or(low_end);
                        // the default follows the computer
                        let reset = (on != low_end).then_some(if low_end { "On, for this computer" } else { "Off" });
                        let pressed = c.row_hint(ui, "light-mode", hint, SWITCH_W, reset, |ui| {
                            if toggle(ui, &mut on).changed() {
                                s.light_mode = Some(on);
                                theme_changed = true;
                                dirty = true;
                            }
                        });
                        if pressed {
                            s.light_mode = None;
                            theme_changed = true;
                            dirty = true;
                        }
                        // no Language row: the window is in English only for now, so a choice there would change
                        // nothing. `language` stays in the settings file for the Java edition.
                    });
                    section(ui, &view, &nav, Group::Game, 0, |ui, c, part| match part {
                        Part::Main => {
                            let mut auto = s.automatic_memory != Some(false);
                            let hint = if auto {
                                example.clone()
                            } else {
                                format!(
                                    "Set by hand, for every game without its own setting: 1 GB to {}",
                                    memory_text(max_mb)
                                )
                            };
                            let reset = (!auto).then_some("Automatic");
                            let width = if auto { 140.0 } else { 400.0 };
                            let pressed = c.row_hint(ui, "memory", &hint, width, reset, |ui| {
                                if toggle(ui, &mut auto).changed() {
                                    s.automatic_memory = Some(auto);
                                    dirty = true;
                                }
                                ui.label("Automatic");
                                if !auto {
                                    // the row lays out from the right: the amount, then the slider to its left
                                    ui.add_space(16.0);
                                    let label = ui.label(
                                        RichText::new(memory_text(s.maximum_memory))
                                            .font(super::fonts::medium(FS_BODY)),
                                    );
                                    if s.maximum_memory > max_mb {
                                        // set elsewhere (an imported setting, the file): more than the slider goes to
                                        label.on_hover_text(format!(
                                            "More than the slider goes to ({}, half of this computer's memory). \
                                             Drag it to pick less.",
                                            memory_text(max_mb)
                                        ));
                                    }
                                    ui.scope(|ui| {
                                        // a rail that can be seen, filled in the brand colour up to the handle (like
                                        // a switch that is on)
                                        ui.visuals_mut().widgets.inactive.bg_fill = c_active();
                                        ui.visuals_mut().selection.bg_fill = c_accent();
                                        ui.spacing_mut().slider_width = 200.0;
                                        let mut mb = s.maximum_memory.clamp(1024, max_mb);
                                        let slider = egui::Slider::new(&mut mb, 1024..=max_mb)
                                            .step_by(256.0)
                                            .show_value(false)
                                            .trailing_fill(true);
                                        if ui.add(slider).changed() {
                                            s.maximum_memory = mb;
                                            // written once the drag is over, not on every frame of it
                                            dragged = true;
                                        }
                                    });
                                }
                            });
                            if pressed {
                                s.automatic_memory = Some(true);
                                dirty = true;
                            }
                            dirty |= c.switch(ui, "keep-open", &mut s.keep_launcher_open, true);
                            dirty |= c.switch(ui, "console", &mut s.enable_console, false);
                            dirty |= c.switch(ui, "game-options", &mut s.auto_game_options, true);
                            dirty |= graphics_card_row(ui, c, &mut s.gpu, self_gpus.as_deref());
                        }
                        Part::Advanced => {}
                    });
                    let custom_java = s.using_custom_java_path.then(|| s.java_path.clone()).filter(|p| !p.is_empty());
                    let java_changed = [
                        s.startup_mode != "auto",
                        custom_java.is_some(),
                        s.java_parameters != crate::config::DEFAULT_JAVA_ARGS,
                    ]
                    .into_iter()
                    .filter(|c| *c)
                    .count();
                    section(ui, &view, &nav, Group::Java, java_changed, |ui, c, part| match part {
                        Part::Main => {
                            dirty |= perf.rows(ui, c, s);
                            dirty |= c.switch(ui, "class-cache", &mut s.class_data_cache, true);
                            let mut warm = s.extras.get("warmUpGameFiles").and_then(|v| v.as_bool()).unwrap_or(true);
                            if c.switch(ui, "warm-up", &mut warm, true) {
                                s.extras.insert("warmUpGameFiles".into(), warm.into());
                                dirty = true;
                            }
                            // what the smoothness helper did in the last game, under its rows
                            if c.shows("smoothness") {
                                c.gap(ui);
                                perf.details_fold(ui);
                            }
                        }
                        Part::Advanced => {
                            const MODES: [(&str, &str); 3] =
                                [("auto", "Automatic"), ("fast", "Light"), ("full", "Full performance")];
                            let reset = (s.startup_mode != "auto").then_some("Automatic");
                            let pressed = c.row(ui, "compiler", CONTROL_W, reset, |ui| {
                                let labels = MODES.map(|(_, label)| label);
                                let now =
                                    MODES.iter().position(|(mode, _)| *mode == s.startup_mode).unwrap_or(usize::MAX);
                                if let Some(i) = segmented(ui, &labels, now).filter(|&i| i != now) {
                                    s.startup_mode = MODES[i].0.to_string();
                                    dirty = true;
                                }
                            });
                            if pressed {
                                s.startup_mode = "auto".to_string();
                                dirty = true;
                            }
                            // an empty path means automatic. The game only falls back to this Java when it doesn't use
                            // the one that comes with Minecraft (an instance's own Java setting), as in the Java
                            // edition.
                            let custom =
                                s.using_custom_java_path.then(|| s.java_path.clone()).filter(|p| !p.is_empty());
                            let (typed, pressed) = c.text(
                                ui,
                                "java-path",
                                "Automatic (recommended)",
                                custom.as_deref().unwrap_or(""),
                                custom.is_some().then_some("Automatic"),
                            );
                            if let Some(path) = typed {
                                let path = path.trim().trim_matches('"').to_string();
                                if custom.as_deref().unwrap_or("") != path {
                                    // a cleared path is cleared for real, or the old Java would still be used at launch
                                    s.using_custom_java_path = !path.is_empty();
                                    s.java_path = path;
                                    dirty = true;
                                }
                            }
                            if pressed {
                                s.using_custom_java_path = false;
                                s.java_path.clear();
                                dirty = true;
                            }
                            if let Some(warning) = java_path_warning(custom.as_deref()).filter(|_| c.shows("java-path"))
                            {
                                ui.add(egui::Label::new(RichText::new(warning).color(c_warn()).size(FS_META)).wrap());
                            }
                            let own_args = s.java_parameters != crate::config::DEFAULT_JAVA_ARGS;
                            let (typed, pressed) = c.text(
                                ui,
                                "java-args",
                                "",
                                &s.java_parameters,
                                own_args.then_some("the recommended arguments"),
                            );
                            if let Some(args) = typed {
                                let args = args.trim().to_string();
                                if args != s.java_parameters {
                                    s.java_parameters = args;
                                    dirty = true;
                                }
                            }
                            if pressed {
                                s.java_parameters = crate::config::DEFAULT_JAVA_ARGS.to_string();
                                dirty = true;
                            }
                        }
                    });
                    section(ui, &view, &nav, Group::Launcher, 0, |ui, c, part| {
                        if part != Part::Main {
                            return;
                        }
                        dirty |= c.switch(ui, "updates", &mut s.check_for_launcher_updates, true);
                        dirty |= c.switch(ui, "tray", &mut s.enable_tray_menu, true);
                        let hint = format!("{}: {data_dir}", find("data-folder").hint);
                        c.row_hint(ui, "data-folder", &hint, 100.0, None, |ui| {
                            if secondary_icon(ui, super::icons::Icon::Folder, "Open").clicked() {
                                open_data = true;
                            }
                        });
                    });
                    self.services_settings_ui(ui, &view, &nav);
                    let s = &mut self.settings;
                    section(ui, &view, &nav, Group::Privacy, 0, |ui, c, part| {
                        if part != Part::Main {
                            return;
                        }
                        dirty |= c.switch(ui, "analytics", &mut s.enable_analytics, false);
                        dirty |= c.switch(ui, "pack-stats", &mut s.enable_logs, false);
                    });
                    ui.add_space(24.0);
                });
        });
        ui.data_mut(|d| {
            d.insert_temp(query_id(), query);
            d.insert_temp(Nav::id(), nav);
        });
        if let Some(page) = go {
            self.go(page);
        }
        if open_data {
            self.open_folder(&self.dirs.base.clone());
        }
        if theme_changed {
            theme::set_dark(look_is_dark(&self.settings, system_dark));
            theme::apply(ui.ctx(), self.settings.light_mode.unwrap_or_else(super::low_end_pc));
        }
        if dirty {
            super::motion::set_pattern(artwork_on(&self.settings));
            self.save_settings();
            // the player is looking: a tray that can't start now says why
            self.tray_quiet = false;
            self.sync_tray();
            mark_saved(ui.ctx());
        } else if dragged {
            self.save_settings_soon();
            mark_saved(ui.ctx());
        }
    }

    /// The frame of Accounts and About, the other two pages of the account corner: their title over the search and
    /// the list of Settings' sections at the left (as on Settings), and the page beside it, at most `max_w` wide.
    /// Typing in the search or picking a section goes to Settings.
    pub(super) fn account_corner(
        &mut self,
        ui: &mut Ui,
        here: super::Page,
        title: &str,
        subtitle: &str,
        max_w: f32,
        content: impl FnOnce(&mut App, &mut Ui),
    ) {
        let room = ui.available_width() - 14.0;
        let side = room >= NAV_W + NAV_GAP + NAV_CONTENT_MIN;
        let content_w = if side { (room - NAV_W - NAV_GAP).min(max_w) } else { room.min(max_w) };
        ui.scope(|ui| {
            ui.set_max_width(if side { NAV_W + NAV_GAP + content_w } else { content_w });
            PageHeader::new(title).subtitle(subtitle).show(ui, |_| {});
        });
        // a search left on Settings means nothing here: the field starts empty, and what is typed in it goes to
        // Settings
        let mut query = String::new();
        let mut nav = ui.data(|d| d.get_temp::<Nav>(Nav::id())).unwrap_or_default();
        let hidden: &[&str] = if crate::gpu::supported() { &[] } else { &["gpu"] };
        let mut go = None;
        let area = ui.available_rect_before_wrap();
        let main = if side {
            let list = egui::Rect::from_min_size(area.min, egui::vec2(NAV_W, area.height()));
            let mut list = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(Layout::top_down(Align::Min)));
            search_box(&mut list, &mut query, NAV_W);
            list.add_space(12.0);
            let view = View::new("", hidden);
            match nav_list(&mut list, &view, &nav, true, here) {
                Some(NavPick::Section(g)) => {
                    nav.open = g;
                    go = Some(super::Page::Settings);
                }
                Some(NavPick::Page(p)) => go = Some(p),
                None => {}
            }
            egui::Rect::from_min_max(egui::pos2(area.left() + NAV_W + NAV_GAP, area.top()), area.max)
        } else {
            ui.scope(|ui| {
                ui.set_max_width(content_w);
                let view = View::new("", hidden);
                match nav_list(ui, &view, &nav, false, here) {
                    Some(NavPick::Section(g)) => {
                        nav.open = g;
                        go = Some(super::Page::Settings);
                    }
                    Some(NavPick::Page(p)) => go = Some(p),
                    None => {}
                }
            });
            ui.add_space(14.0);
            egui::Rect::from_min_max(egui::pos2(area.left(), ui.cursor().top()), area.max)
        };
        let main = main.with_max_x(main.left() + content_w + 14.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| content(self, ui));
        if !query.is_empty() {
            // a search is a search of the settings
            go = Some(super::Page::Settings);
        }
        ui.data_mut(|d| {
            d.insert_temp(query_id(), query);
            d.insert_temp(Nav::id(), nav);
        });
        if let Some(page) = go.filter(|p| *p != here) {
            self.go(page);
        }
    }

    /// For the screenshot tour: Settings on a section (`settings-java`), with a search (`settings-search`), or on its
    /// first section.
    pub(super) fn shot_settings(&mut self, ctx: &egui::Context, variant: &str) {
        self.page = super::Page::Settings;
        let open = if variant == "settings-java" { Group::Java } else { Group::Appearance };
        let query = if variant == "settings-search" { "java" } else { "" };
        ctx.data_mut(|d| {
            d.insert_temp(query_id(), query.to_string());
            d.insert_temp(Nav::id(), Nav { open });
        });
    }

    /// Follow the system's look when the theme is set to: called every frame, it changes the look when the system's
    /// does (and does nothing otherwise, or when the system doesn't say).
    pub(super) fn follow_system_theme(&mut self, ctx: &egui::Context) {
        if !follows_system(&self.settings) {
            return;
        }
        let Some(dark) = ctx.system_theme().map(|t| t == egui::Theme::Dark) else { return };
        if dark != theme::is_dark() {
            theme::set_dark(dark);
            theme::apply(ctx, self.settings.light_mode.unwrap_or_else(super::low_end_pc));
        }
    }

    /// "Automatic: 4 GB for ..." for the game played last, shown under the memory setting.
    fn automatic_memory_example(&self, total_mb: u64, free_mb: u64) -> String {
        match self.instances.iter().find(|i| i.last_played() > 0).or(self.instances.first()) {
            Some(i) => {
                let mb = crate::config::auto_memory_mb(i.mod_count, total_mb, free_mb);
                format!("Automatic: {} for {}", memory_text(mb), i.name())
            }
            None => "Automatic: picked for each game and this PC".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame with `events`, drawing a draft field that shows `value`. Returns what it handed back and the
    /// field's place.
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, value: &str) -> (Option<String>, egui::Rect) {
        let mut out = (None, egui::Rect::NOTHING);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut full = ctx.run_ui(input, |ui| {
            let r = ui.scope(|ui| draft_text(ui, "test-field", value, "", [300.0, 40.0], egui::Margin::same(4)));
            out = (r.inner, r.response.rect);
        });
        full.textures_delta.clear();
        out
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn typing_in_a_setting_field_is_kept_until_it_is_left() {
        let ctx = egui::Context::default();
        let (_, rect) = frame(&ctx, vec![], "old");
        let at = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&ctx, vec![egui::Event::PointerMoved(at), press(true)], "old");
        frame(&ctx, vec![press(false)], "old");
        // select everything, then type over it over two frames; the saved value stays "old" meanwhile
        assert_eq!(frame(&ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], "old").0, None);
        assert_eq!(frame(&ctx, vec![egui::Event::Text("/opt/".into())], "old").0, None);
        assert_eq!(frame(&ctx, vec![egui::Event::Text("jdk".into())], "old").0, None);
        assert_eq!(frame(&ctx, vec![], "old").0, None, "nothing is saved while typing");
        let entered = frame(&ctx, vec![key(egui::Key::Enter, Default::default())], "old").0;
        assert_eq!(entered.as_deref(), Some("/opt/jdk"), "Enter hands the typed text back");
        // once left, the field shows the saved value again
        assert_eq!(frame(&ctx, vec![], "saved").0, None);
    }

    #[test]
    fn typing_is_saved_when_the_page_is_left_with_the_field_still_focused() {
        let ctx = egui::Context::default();
        let (_, rect) = frame(&ctx, vec![], "old");
        let at = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&ctx, vec![egui::Event::PointerMoved(at), press(true)], "old");
        frame(&ctx, vec![press(false)], "old");
        frame(&ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], "old");
        frame(&ctx, vec![egui::Event::Text("-Xss4M".into())], "old");
        // the sidebar switches the page: the field isn't drawn for a few frames
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
        }
        // back on the page, what was typed is handed back once, not thrown away
        assert_eq!(frame(&ctx, vec![], "old").0.as_deref(), Some("-Xss4M"));
        assert_eq!(frame(&ctx, vec![], "-Xss4M").0, None);
    }

    #[test]
    fn saved_shows_for_two_seconds_and_fades_out() {
        assert_eq!(saved_alpha(Duration::ZERO), Some(1.0));
        assert_eq!(saved_alpha(Duration::from_millis(1500)), Some(1.0));
        let fading = saved_alpha(Duration::from_millis(1800)).unwrap();
        assert!(fading > 0.0 && fading < 1.0, "fading at 1.8 s: {fading}");
        assert_eq!(saved_alpha(Duration::from_millis(2000)), None);
        assert_eq!(saved_alpha(Duration::from_secs(60)), None);
    }

    #[test]
    fn the_stamp_is_painted_once_settings_were_saved() {
        let ctx = egui::Context::default();
        let texts = |ctx: &egui::Context| {
            let mut found = Vec::new();
            let mut out = ctx.run_ui(egui::RawInput::default(), saved_stamp);
            for clipped in &out.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    found.push(t.galley.text().to_string());
                }
            }
            out.textures_delta.clear();
            found
        };
        assert!(!texts(&ctx).iter().any(|t| t == "Saved"), "nothing was saved yet");
        mark_saved(&ctx);
        assert!(texts(&ctx).iter().any(|t| t == "Saved"), "shown right after a save");
    }

    #[test]
    fn the_interface_size_row_marks_the_nearest_size() {
        assert_eq!(ZOOMS[nearest_zoom(1.0)].1, "100%");
        assert_eq!(ZOOMS[nearest_zoom(1.2)].1, "125%", "Ctrl and + twice");
        assert_eq!(ZOOMS[nearest_zoom(0.3)].1, "90%");
        assert_eq!(ZOOMS[nearest_zoom(3.0)].1, "150%");
    }

    #[test]
    fn the_memory_slider_stops_at_half_the_memory() {
        assert_eq!(memory_slider_max(16_000), 7936);
        assert_eq!(memory_slider_max(0), 8192, "unknown memory");
        assert_eq!(memory_slider_max(2_000), 2048, "never less than 2 GB");
        assert_eq!(memory_slider_max(u64::MAX), u32::MAX, "no overflow");
    }

    #[test]
    fn every_word_of_a_search_must_start_a_word() {
        let terms = search_terms("  Java   PATH ");
        assert_eq!(terms, ["java", "path"]);
        assert!(matches(&terms, &["Java path", "For games set not to use..."]));
        assert!(matches(&terms, &["Java", "the folder path"]), "the words may be in different texts");
        assert!(!matches(&terms, &["Java arguments", "Extra options"]));
        assert!(matches(&search_terms(""), &["anything"]), "no words match everything");
        assert!(matches(&search_terms("MEM"), &["Memory"]), "any case, the start of a word");
        assert!(!matches(&search_terms("ram"), &["higher frame rates"]), "not the middle of a word");
        assert_eq!(search_terms("-Xmx, GB"), ["xmx", "gb"], "punctuation is left out");
        assert!(matches(&search_terms("gb"), &["4 GB of memory"]));
    }

    #[test]
    fn a_search_finds_settings_by_title_hint_words_or_group() {
        let view = View::new("ram", &[]);
        assert_eq!(view.shown, ["memory"], "by a word it is known by");
        assert!(view.searching);
        assert_eq!(view.groups(), [Group::Game]);
        let view = View::new("privacy", &[]);
        assert_eq!(view.shown, ["analytics", "pack-stats"], "by the name of its group");
        let view = View::new("proxy server", &[]);
        assert_eq!(view.shown, ["proxy"], "by its explanation");
        let view = View::new("folder", &[]);
        assert!(view.shows("data-folder") && view.shows("backups-folder") && view.shows("downloads-folder"));
        assert_eq!(view.advanced_count(Group::Backups), 1);
        let view = View::new("zzzz", &[]);
        assert!(view.shown.is_empty() && view.groups().is_empty(), "nothing found");
    }

    #[test]
    fn without_a_search_every_setting_shows_except_the_hidden_ones() {
        let view = View::new("", &[]);
        assert!(!view.searching);
        assert_eq!(view.shown.len(), ENTRIES.len());
        assert_eq!(view.groups(), Group::ALL);
        let view = View::new("", &["gpu"]);
        assert!(!view.shows("gpu") && view.shows("memory"));
        assert!(View::new("graphics card", &["gpu"]).shown.is_empty(), "a hidden setting isn't found either");
    }

    #[test]
    fn the_page_shows_the_open_section_or_every_section_a_search_finds() {
        let nav = Nav { open: Group::Backups };
        let all = View::new("", &[]);
        let drawn_all: Vec<Group> = Group::ALL.into_iter().filter(|g| drawn(&all, &nav, *g)).collect();
        assert_eq!(drawn_all, [Group::Backups], "one section at a time");
        let found = View::new("folder", &[]);
        let drawn_found: Vec<Group> =
            Group::ALL.into_iter().filter(|g| found.group_shown(*g) && drawn(&found, &nav, *g)).collect();
        assert_eq!(
            drawn_found,
            [Group::Java, Group::Network, Group::Backups, Group::Launcher],
            "a search looks everywhere"
        );
        assert!(found.shows("data-folder"));
    }

    #[test]
    fn the_theme_cards_pick_a_look_and_keep_a_theme_of_its_kind() {
        let mut s = crate::config::Settings::default();
        assert!(look_is_dark(&s, Some(false)), "the default theme is dark, whatever the system is");
        set_look(&mut s, 1, None);
        assert_eq!(s.theme, LIGHT_THEME);
        assert!(!look_is_dark(&s, Some(true)));
        // a dark theme of the player's own stays when Dark is picked
        s.theme = "com.blockforge.launcher.themes.CatppuccinMocha".into();
        set_look(&mut s, 0, None);
        assert_eq!(s.theme, "com.blockforge.launcher.themes.CatppuccinMocha");
        // following the system: the look is the system's, and the theme follows it for the Java edition
        set_look(&mut s, 2, Some(false));
        assert!(follows_system(&s) && s.theme == LIGHT_THEME);
        assert!(look_is_dark(&s, Some(true)) && !look_is_dark(&s, Some(false)));
        assert!(!look_is_dark(&s, None), "a system that doesn't say: the theme decides");
        set_look(&mut s, 0, Some(false));
        assert!(!follows_system(&s) && look_is_dark(&s, Some(false)));
    }

    #[test]
    fn background_artwork_is_on_unless_turned_off() {
        let mut s = crate::config::Settings::default();
        assert!(artwork_on(&s));
        set_artwork(&mut s, false);
        assert!(!artwork_on(&s) && s.extras.get(ARTWORK_KEY) == Some(&serde_json::Value::Bool(false)));
        set_artwork(&mut s, true);
        assert!(artwork_on(&s) && !s.extras.contains_key(ARTWORK_KEY), "the default isn't written");
    }

    #[test]
    fn every_setting_has_a_name_of_its_own_and_every_group_has_settings() {
        for (i, e) in ENTRIES.iter().enumerate() {
            assert!(ENTRIES[..i].iter().all(|o| o.key != e.key), "{} twice", e.key);
            assert!(!e.title.is_empty() && !e.hint.is_empty());
        }
        for g in Group::ALL {
            assert!(ENTRIES.iter().any(|e| e.group == g && !e.advanced), "{g:?} has no settings");
        }
        // the groups are listed in the order they are shown
        let order: Vec<usize> =
            ENTRIES.iter().map(|e| Group::ALL.iter().position(|g| *g == e.group).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]));
    }
}
