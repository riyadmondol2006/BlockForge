//! The Performance controls: Speed-up, Fast start, the smoothness helper and Low-end mode, for all games (Settings >
//! Java & performance) and for one instance (the switch beside Play, its menu, its Overview and its settings); the
//! hint that suggests Low-end mode on a slow computer; and what happens when a game closes early while a helper was on.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use egui::{RichText, Ui};

use super::App;
use super::instance_menu::Act;
use super::motion;
use super::settings::{Card, Rows, wide_row};
use super::theme::*;
use super::widgets::*;
use crate::config::Settings;
use crate::instances::Instance;
use crate::launch::lite::{self, Patch, PatchState};
use crate::launch::lowend::{self, Mode};
use crate::launch::speedup::{self, GcChoice};
use crate::launch::{Level, LogLine};
use crate::paths::Dirs;

/// A change to one instance's helper switches. `None` means "follow the launcher" (for the collector, `Launcher`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelperChange {
    FastStart(Option<bool>),
    Lite(Option<bool>),
    LowEnd(Option<Mode>),
    Speedup(Option<bool>),
    SpeedupThreads(Option<bool>),
    SpeedupMemory(Option<bool>),
    SpeedupGc(Option<GcChoice>),
    /// Lower the simulation distance in the instance's `options.txt` (the player pressed the button for it).
    SimulationDistance,
}

/// What an instance's own switch holds when the player turns it to `new`: nothing when that is what the launcher's
/// setting already says, so a switch that was turned back follows the launcher again instead of pinning the instance.
pub(super) fn pin(new: bool, launcher: bool) -> Option<bool> {
    (new != launcher).then_some(new)
}

pub(super) const LOW_END_HINT: &str = "Lowers the view distance, clouds, shadows, particles and smooth lighting so the \
    game runs smoother. It only ever lowers, and switching it off puts your own settings back.";

pub(super) const LITE_HINT: &str = "Small fixes inside the game that make it run smoother. Works with every Minecraft \
    version and mod loader (Fabric, Quilt, Forge, NeoForge). Switch it off if a game misbehaves.";

pub(super) const FAST_START_HINT: &str = "Uses spare processor cores while the game is opening. Turn it off if a game \
    closes as soon as it starts.";

pub(super) const SPEEDUP_HINT: &str =
    "Faster world entry and less CPU and memory use. Each instance can have its own setting.";

pub(super) const QUICKER_START_HINT: &str =
    "The first time you play, nothing changes. After that, the game opens faster.";

pub(super) const THREADS_HINT: &str = "Keeps a few processor threads free for drawing frames while new land is made. \
    New land takes slightly longer.";

pub(super) const MEMORY_HINT: &str = "Packs the game's data more tightly and gives unused memory back. Needs Java 25 or \
    newer for the biggest effect.";

pub(super) const GC_HINT: &str = "A garbage collector with much shorter pauses. It uses about 1.4 GB more memory, so \
    Automatic keeps the usual one unless this computer has plenty.";

pub(super) const UNSIGNED_JAR_HINT: &str = "Uses a copy of the game without Mojang's signature so more of it can be \
    cached. The game then shows itself as Modded on its title screen and in crash reports. Worlds and servers aren't \
    affected.";

/// What the switch beside Play says when pointed at.
const SPEEDUP_TIP: &str = "Faster world entry and less CPU and memory use. Worlds, saves and servers stay as they are. \
    Applies the next time the game starts.";

/// The first part of [`SPEEDUP_TIP`], for when the running game started the other way.
const SPEEDUP_TIP_CORE: &str =
    "Faster world entry and less CPU and memory use. Worlds, saves and servers stay as they are.";

/// The tooltip of the Speed-up switch: while a game runs that started with Speed-up the other way round than the
/// switch says now (`started_with`), it says that the change comes at the next start.
fn speedup_tip(started_with: Option<bool>, now: bool) -> String {
    match started_with.filter(|w| *w != now) {
        Some(was) => format!(
            "{SPEEDUP_TIP_CORE} The running game started with Speed-up {}, so this applies the next time it starts.",
            if was { "on" } else { "off" }
        ),
        None => SPEEDUP_TIP.to_string(),
    }
}

// ---- what is said when Speed-up can't do its work

/// Why Speed-up does nothing (or not yet) for an instance, said in a line where its switches are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Exception {
    /// The player's own Java arguments replace everything Speed-up would add.
    OwnArguments,
    /// The game's Java is older than the one that has the AOT cache and compact object headers.
    OldJava(u32),
    /// The class cache is being made after the last game closed.
    CacheBeingMade,
}

impl Exception {
    pub(super) fn text(&self) -> String {
        match self {
            Exception::OwnArguments => "Your Java arguments are in charge, so these do nothing.".to_string(),
            Exception::OldJava(n) => old_java_text(*n),
            Exception::CacheBeingMade => {
                "Getting this ready after this game closes. The start after that is the fast one.".to_string()
            }
        }
    }
}

fn old_java_text(n: u32) -> String {
    format!("Not used here: this game runs Java {n}. This needs Java {} or newer.", speedup::MODERN_JAVA)
}

/// The Java version of a game that is too old for the parts of Speed-up that need Java 25, when its version says so.
fn old_java(java: Option<u32>) -> Option<u32> {
    java.filter(|n| *n < speedup::MODERN_JAVA)
}

/// The one thing worth saying about Speed-up for an instance, most important first: the player's own arguments (said
/// whatever the switch is, as they make it useless), then, with Speed-up on, a game too old for it and a cache that is
/// being made. Nothing when all is as it should be.
fn exception(own_arguments: bool, on: bool, java: Option<u32>, making_cache: bool) -> Option<Exception> {
    if own_arguments {
        return Some(Exception::OwnArguments);
    }
    if !on {
        return None;
    }
    if let Some(n) = old_java(java) {
        return Some(Exception::OldJava(n));
    }
    making_cache.then_some(Exception::CacheBeingMade)
}

/// "Your Java arguments are in charge, so these do nothing." where the player can see it.
pub(super) fn own_arguments_line(ui: &mut Ui) {
    exception_line(ui, &Exception::OwnArguments);
}

/// A line that says why something does nothing, where it can be seen: amber for what needs the player's hand (their
/// own arguments), a spinner while something is being made, otherwise a quiet note.
fn exception_line(ui: &mut Ui, e: &Exception) {
    let text = e.text();
    match e {
        Exception::OwnArguments => {
            status_line(ui, Status::Warn, &text);
        }
        Exception::CacheBeingMade => {
            status_line(ui, Status::Busy, &text);
        }
        Exception::OldJava(_) => {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                super::icons::paint(ui.painter(), super::icons::Icon::Info, r.center(), 13.0, c_faint());
                ui.add(egui::Label::new(RichText::new(text).size(FS_META).color(c_muted())).wrap());
            });
        }
    }
}

pub(super) use pending::unsigned_jar;

/// What the lead swaps for the real thing at merge: `launch::speedup` and `config` don't have these yet.
mod pending {
    use crate::config::Settings;
    use crate::instances::Instance;

    /// Whether the class cache of this instance is being made right now, after its game closed.
    pub fn cache_being_made(_instance: &Instance) -> bool {
        false
    }

    /// The setting "Even quicker vanilla start" (`speedup_unsigned_jar`): off unless switched on.
    pub fn unsigned_jar(settings: &Settings) -> bool {
        settings.extras.get("speedupUnsignedJar").and_then(|v| v.as_bool()).unwrap_or(false)
    }

    pub fn set_unsigned_jar(settings: &mut Settings, on: bool) {
        settings.extras.insert("speedupUnsignedJar".into(), on.into());
    }
}

// ---- the simulation distance

/// "Simulation distance is 32. This is often the biggest load on the game." when `distance` is worth a word.
fn simulation_advice(distance: Option<u32>) -> Option<String> {
    distance
        .filter(|d| *d > speedup::HIGH_SIMULATION_DISTANCE)
        .map(|n| format!("Simulation distance is {n}. This is often the biggest load on the game."))
}

type DistanceKept = (String, Option<u32>, u64);

fn distance_id() -> egui::Id {
    egui::Id::unique("performance-simulation-distance")
}

/// Whether what was read on pass `seen` is still the same showing of the menu on pass `now`: a menu that stays open
/// is drawn on consecutive passes; one that was closed in between isn't.
fn menu_still_open(seen: u64, now: u64) -> bool {
    now.saturating_sub(seen) <= 1
}

/// The simulation distance of an instance, read from its `options.txt` when the Performance menu opens and kept while
/// it stays open, not read at every frame. While the instance's game runs it is not read at all: the game writes the
/// file when it closes, over a change made now.
fn simulation_distance_now(ctx: &egui::Context, inst: &Instance, playing: bool) -> Option<u32> {
    if playing {
        return None;
    }
    let now = ctx.cumulative_pass_nr();
    let kept: Option<DistanceKept> = ctx.data(|d| d.get_temp(distance_id()));
    let value = match kept {
        Some((folder, value, seen)) if folder == inst.folder && menu_still_open(seen, now) => value,
        _ => speedup::simulation_distance(inst),
    };
    ctx.data_mut(|d| d.insert_temp(distance_id(), (inst.folder.clone(), value, now)));
    value
}

// ---- is this computer a slow one? (found out once, off the window's thread)

static SLOW: OnceLock<Option<String>> = OnceLock::new();
static LOOKING: std::sync::Once = std::sync::Once::new();

/// Why this computer looks slow (`Some(Some(reason))`), that it doesn't (`Some(None)`), or `None` while it is being
/// found out.
fn slow_reason(dirs: &Dirs, settings: &Settings, ctx: &egui::Context) -> Option<Option<String>> {
    if let Some(answer) = SLOW.get() {
        return Some(answer.clone());
    }
    LOOKING.call_once(|| {
        let (dirs, settings, ctx) = (dirs.clone(), settings.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = SLOW.set(lowend::looks_weak(&lowend::this_computer(&dirs, &settings)));
            ctx.request_repaint();
        });
    });
    None
}

/// The smoothness helper's last report, read again at most every few seconds.
type Report = Option<(String, SystemTime, Vec<Patch>)>;
static REPORT: Mutex<Option<(Instant, Report)>> = Mutex::new(None);

fn last_report(dirs: &Dirs) -> Report {
    let mut cached = REPORT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, r)) = cached.as_ref() {
        if at.elapsed() < Duration::from_secs(5) {
            return r.clone();
        }
    }
    let r = lite::latest_report(dirs);
    *cached = Some((Instant::now(), r.clone()));
    r
}

fn ago(t: SystemTime) -> String {
    match t.elapsed().map(|d| d.as_secs()) {
        Ok(s) if s < 60 => "just now".into(),
        Ok(s) if s < 3600 => format!("{} min ago", s / 60),
        Ok(s) if s < 86400 => format!("{} h ago", s / 3600),
        Ok(s) => format!("{} days ago", s / 86400),
        Err(_) => "just now".into(),
    }
}

/// The words of [`App::speed_summary`]: `own` is the instance's own Low-end mode (`None`: it follows the launcher's).
fn speed_summary_text(speedup: bool, fast: bool, smooth: bool, own: Option<Mode>, launcher: Mode) -> String {
    let state = |on: bool| if on { "on" } else { "off" };
    let low_end = match own {
        Some(mode) => format!("Low-end mode {}", mode.label().to_lowercase()),
        None => format!("Low-end mode like the launcher ({})", launcher.label().to_lowercase()),
    };
    format!(
        "Speed-up {} \u{00B7} Fast start {} \u{00B7} Smoothness helper {} \u{00B7} {low_end}",
        state(speedup),
        state(fast),
        state(smooth)
    )
}

/// How Low-end mode stands, in sentences of their own (`whose` says whom "on" is for). `slow` is what was found out
/// about this computer (see [`slow_reason`]).
fn low_end_state(mode: Mode, slow: Option<&Option<String>>, whose: &str) -> String {
    match (mode, slow) {
        (Mode::Auto, None) => "Right now: checking this computer\u{2026}".to_string(),
        (Mode::Auto, Some(Some(why))) => format!("Right now it is on, because this computer looks slow ({why})."),
        (Mode::Auto, Some(None)) => "Right now it is off, because this computer doesn't look slow.".to_string(),
        (Mode::On, _) => format!("It is on for {whose}."),
        (Mode::Off, _) => "It is off, so graphics settings are left as they are.".to_string(),
    }
}

fn state_color(state: PatchState) -> egui::Color32 {
    match state {
        PatchState::On => c_text(),
        PatchState::Off | PatchState::Other => c_muted(),
        PatchState::Problem => c_danger(),
    }
}

/// Names in a sentence: "Fast start", "Fast start and Speed-up", "Fast start, the smoothness helper and Speed-up".
fn join_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The helpers that were on when a game closed early, as a sentence's end: "Fast start was on", "Fast start and the
/// smoothness helper were on". `None` when none were.
fn blame_text(fast: bool, lite: bool, speedup: bool) -> Option<String> {
    let names: Vec<&str> = [(fast, "Fast start"), (lite, "the smoothness helper"), (speedup, "Speed-up")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    match names.len() {
        0 => None,
        1 => Some(format!("{} was on", names[0])),
        _ => Some(format!("{} were on", join_names(&names))),
    }
}

/// What Settings needs to draw the Performance rows, gathered before the settings are borrowed for editing.
pub(super) struct SettingsView {
    slow: Option<Option<String>>,
    report: Report,
    names: Vec<(String, String)>,
    /// The launcher-wide Java arguments are the player's own.
    custom_arguments: bool,
}

impl SettingsView {
    pub(super) fn new(app: &App, ctx: &egui::Context) -> SettingsView {
        SettingsView {
            slow: slow_reason(&app.dirs, &app.settings, ctx),
            report: last_report(&app.dirs),
            names: app.instances.iter().map(|i| (lite::instance_key(i), i.name().to_string())).collect(),
            custom_arguments: crate::launch::jvmargs::is_custom(Some(&app.settings.java_parameters)),
        }
    }

    /// Whether the launcher-wide Java arguments are the player's own, which replace what Speed-up adds.
    pub(super) fn custom_arguments(&self) -> bool {
        self.custom_arguments
    }

    /// The Performance rows of Settings: Speed-up, Fast start, the smoothness helper and Low-end mode, each with a
    /// Reset when it isn't at its default. Returns true when a setting changed.
    pub(super) fn rows(&self, ui: &mut Ui, c: &mut Card, s: &mut Settings) -> bool {
        if self.custom_arguments && c.shows("speedup") {
            exception_line(ui, &Exception::OwnArguments);
            ui.add_space(6.0);
        }
        let mut dirty = c.switch(ui, "speedup", &mut s.speedup, true);
        dirty |= c.switch(ui, "fast-start", &mut s.fast_start, true);
        dirty |= c.switch(ui, "smoothness", &mut s.lite, true);
        let mode = Mode::parse(&s.low_end_mode);
        let state = low_end_state(mode, self.slow.as_ref(), "every game without its own setting");
        let hint = format!("{LOW_END_HINT} {state}");
        let reset = (mode != Mode::Auto).then_some(Mode::Auto.label());
        let pressed = c.row_hint(ui, "low-end", &hint, 220.0, reset, |ui| {
            let modes = [Mode::Auto, Mode::On, Mode::Off];
            let labels: Vec<&str> = modes.iter().map(|m| m.label()).collect();
            let chosen = modes.iter().position(|m| *m == mode).unwrap_or(0);
            if let Some(i) = segmented(ui, &labels, chosen) {
                if modes[i] != mode {
                    s.low_end_mode = modes[i].as_setting().to_string();
                    dirty = true;
                }
            }
        });
        if pressed {
            s.low_end_mode = Mode::Auto.as_setting().to_string();
            dirty = true;
        }
        dirty
    }

    /// The folded rows of Speed-up: what it does piece by piece. They are greyed while Speed-up is off.
    pub(super) fn advanced_rows(&self, ui: &mut Ui, c: &mut Card, s: &mut Settings) -> bool {
        let mut dirty = false;
        ui.add_enabled_ui(s.speedup, |ui| {
            dirty |= c.switch(ui, "speedup-threads", &mut s.speedup_threads, true);
            dirty |= c.switch(ui, "speedup-memory", &mut s.speedup_memory, true);
            let choice = GcChoice::parse(&s.speedup_gc);
            let reset = (choice != GcChoice::Auto).then_some("Automatic");
            let pressed = c.row(ui, "speedup-gc", 230.0, reset, |ui| {
                let choices = [GcChoice::Auto, GcChoice::On, GcChoice::Off];
                let labels = ["Automatic", "On", "Off"];
                let chosen = choices.iter().position(|g| *g == choice).unwrap_or(0);
                // an id of its own: Low-end mode's segments have the same labels
                if let Some(i) = ui.push_id("speedup-gc", |ui| segmented(ui, &labels, chosen)).inner {
                    if choices[i] != choice {
                        s.speedup_gc = choices[i].as_setting().to_string();
                        dirty = true;
                    }
                }
            });
            if pressed {
                s.speedup_gc = GcChoice::Auto.as_setting().to_string();
                dirty = true;
            }
            let mut unsigned = pending::unsigned_jar(s);
            if c.switch(ui, "speedup-unsigned-jar", &mut unsigned, false) {
                pending::set_unsigned_jar(s, unsigned);
                dirty = true;
            }
        });
        dirty
    }

    /// What the smoothness helper did in the last game, folded.
    pub(super) fn details_fold(&self, ui: &mut Ui) {
        egui::CollapsingHeader::new(RichText::new("What the smoothness helper did").color(c_muted()))
            .id_salt("perf-details")
            .default_open(false)
            .show(ui, |ui| self.details(ui));
    }

    /// What the smoothness helper did in the game that ran last.
    fn details(&self, ui: &mut Ui) {
        let Some((key, at, patches)) = &self.report else {
            small_muted(ui, "Nothing yet. Start a game with the smoothness helper on, and what it did shows here.");
            return;
        };
        let name = self.names.iter().find(|(k, _)| k == key).map(|(_, n)| n.as_str()).unwrap_or(key);
        small_muted(ui, format!("The smoothness helper in the last game ({name}, {})", ago(*at)));
        if patches.is_empty() {
            small_muted(ui, "It made no changes in that game.");
            return;
        }
        egui::Grid::new("perf-patches").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            for p in patches {
                ui.label(RichText::new(p.state.label()).color(state_color(p.state)).semibold());
                ui.vertical(|ui| {
                    // what it is, in plain words (its id only when it has none), then what happened in that game
                    let what = if p.description.is_empty() { &p.id } else { &p.description };
                    ui.label(RichText::new(what).size(FS_META));
                    if !p.detail.is_empty() && p.detail != p.description {
                        small_muted(ui, &p.detail);
                    }
                });
                ui.end_row();
            }
        });
    }
}

impl App {
    /// What an instance's helpers are doing, in one line, for its Overview (they are changed in its settings):
    /// "Speed-up on · Fast start on · Smoothness helper on · Low-end mode like the launcher (off)".
    pub(super) fn speed_summary(&self, inst: &Instance) -> String {
        speed_summary_text(
            speedup::enabled(&self.settings, inst),
            crate::launch::fast_start_enabled(&self.settings, inst),
            lite::enabled(&self.settings, inst),
            lowend::instance_override(inst),
            Mode::parse(&self.settings.low_end_mode),
        )
    }

    /// Why Speed-up does nothing (or not yet) for this instance, if it does not.
    fn speedup_exception(&self, inst: &Instance) -> Option<Exception> {
        exception(
            speedup::own_arguments(&self.settings, inst),
            speedup::enabled(&self.settings, inst),
            speedup::java_major(inst),
            pending::cache_being_made(inst),
        )
    }

    /// The second line of an instance's Overview summary: only when there is something to say about Speed-up.
    pub(super) fn speedup_exception_line(&self, ui: &mut Ui, inst: &Instance) {
        if let Some(e) = self.speedup_exception(inst) {
            ui.add_space(4.0);
            exception_line(ui, &e);
        }
    }

    /// The Low-end choices of an instance, as menu items.
    fn low_end_choices(&self, ui: &mut Ui, inst: &Instance) -> Option<Act> {
        let mut act = None;
        menu_heading(ui, "Low-end mode for this instance");
        let own = lowend::instance_override(inst);
        let launcher = Mode::parse(&self.settings.low_end_mode);
        let choices = [
            (None, format!("Like the launcher ({})", launcher.label().to_lowercase())),
            (Some(Mode::Auto), "Automatic".to_string()),
            (Some(Mode::On), "On".to_string()),
            (Some(Mode::Off), "Off".to_string()),
        ];
        for (value, label) in choices {
            if menu_choice(ui, own == value, &label).clicked() {
                if own != value {
                    act = Some(Act::Helper(inst.folder.clone(), HelperChange::LowEnd(value)));
                }
                ui.close();
            }
        }
        act
    }

    /// The "Performance" submenu of an instance's menu: the same switches as on its page.
    pub(super) fn helpers_menu(&self, ui: &mut Ui, inst: &Instance) -> Option<Act> {
        let mut act = None;
        ui.menu_button("Performance\u{2026}", |ui| {
            act = self.helpers_contents(ui, inst);
        });
        act
    }

    /// The Speed-up control beside Play: a switch with its state, and a chevron that opens the same switches as the
    /// Performance menu. A click on the switch turns Speed-up on or off for this instance, and back to following the
    /// launcher when that is what it now says.
    pub(super) fn speedup_control(&self, ui: &mut Ui, inst: &Instance, height: f32) -> Option<Act> {
        let on = speedup::enabled(&self.settings, inst);
        // what a game that is running now was started with: the switch can differ from it until the next start
        let started_with = self.games.values().find(|g| g.folder == inst.folder && g.active()).map(|g| g.speedup);
        let (main, more) = super::workspace::speedup_split(ui, on, height);
        let mut act = None;
        let main = main.on_hover_text(speedup_tip(started_with, on));
        if main.clicked() {
            act = Some(Act::Helper(inst.folder.clone(), HelperChange::Speedup(pin(!on, self.settings.speedup))));
        }
        let more = more.on_hover_text(self.speed_summary(inst));
        let picked =
            super::instance_menu::open_menu_closing(&more, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                let act = self.helpers_contents(ui, inst);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("Changes apply the next time the game starts.").size(FS_META).color(c_faint()),
                    );
                });
                ui.add_space(2.0);
                act
            });
        picked.flatten().or(act)
    }

    /// Speed-up, Fast start, the smoothness helper and Low-end mode of one instance, as switches and choices in a
    /// menu, and the advice about the simulation distance when it is worth one.
    fn helpers_contents(&self, ui: &mut Ui, inst: &Instance) -> Option<Act> {
        let mut act = None;
        let exception = self.speedup_exception(inst);
        if exception == Some(Exception::OwnArguments) {
            menu_note(ui, &Exception::OwnArguments);
        }
        let mut on = speedup::enabled(&self.settings, inst);
        if menu_switch(ui, &mut on, "Speed-up", SPEEDUP_TIP) {
            act = Some(Act::Helper(inst.folder.clone(), HelperChange::Speedup(pin(on, self.settings.speedup))));
        }
        if exception == Some(Exception::CacheBeingMade) {
            menu_note(ui, &Exception::CacheBeingMade);
        }
        menu_rule(ui);
        let mut fast = crate::launch::fast_start_enabled(&self.settings, inst);
        if menu_switch(ui, &mut fast, "Fast start", "Opens the game sooner") {
            act = Some(Act::FastStart(inst.folder.clone(), fast));
        }
        let mut smooth = lite::enabled(&self.settings, inst);
        if menu_switch(ui, &mut smooth, "Smoothness helper", LITE_HINT) {
            let change = HelperChange::Lite(pin(smooth, self.settings.lite));
            act = Some(Act::Helper(inst.folder.clone(), change));
        }
        menu_rule(ui);
        if let Some(a) = self.low_end_choices(ui, inst) {
            act = Some(a);
        }
        let playing = self.is_playing(&inst.folder);
        if let Some(advice) = simulation_advice(simulation_distance_now(ui.ctx(), inst, playing)) {
            menu_rule(ui);
            ui.scope(|ui| {
                ui.set_max_width(MENU_W - 20.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(advice).size(FS_META).color(c_muted())).wrap());
                        ui.add_space(4.0);
                        let label = format!("Set it to {}", speedup::SUGGESTED_SIMULATION_DISTANCE);
                        if secondary(ui, &label).clicked() {
                            act = Some(Act::Helper(inst.folder.clone(), HelperChange::SimulationDistance));
                        }
                    });
                });
            });
        }
        act
    }

    /// The rows of the Performance group of an instance's settings, in the card of `rows`. Their choices apply at once.
    pub(super) fn instance_performance_rows(
        &self,
        ui: &mut Ui,
        inst: &Instance,
        rows: &mut Rows,
    ) -> Option<HelperChange> {
        let mut change = None;
        let s = &self.settings;
        let own_arguments = speedup::own_arguments(s, inst);
        let on = speedup::enabled(s, inst);
        if own_arguments {
            exception_line(ui, &Exception::OwnArguments);
            ui.add_space(6.0);
        }
        rows.gap(ui);
        let hint = "Faster world entry and less CPU and memory use. Applies the next time the game starts.";
        wide_row(ui, "Speed-up", hint, 230.0, |ui| {
            if let Some(v) = tri_choice(ui, "speedup", speedup::instance_override(inst), s.speedup) {
                change = Some(HelperChange::Speedup(v));
            }
        });
        if !own_arguments && on && pending::cache_being_made(inst) {
            ui.add_space(4.0);
            exception_line(ui, &Exception::CacheBeingMade);
        }
        let fast_own = crate::launch::boost::instance_override(inst);
        rows.gap(ui);
        wide_row(ui, "Fast start", FAST_START_HINT, 230.0, |ui| {
            if let Some(v) = tri_choice(ui, "fast-start", fast_own, s.fast_start) {
                change = Some(HelperChange::FastStart(v));
            }
        });
        let lite_own = lite::instance_override(inst);
        rows.gap(ui);
        wide_row(ui, "Smoothness helper", "Small fixes inside the game that make it run smoother", 230.0, |ui| {
            if let Some(v) = tri_choice(ui, "smoothness", lite_own, s.lite) {
                change = Some(HelperChange::Lite(v));
            }
        });
        let own = lowend::instance_override(inst);
        let launcher = Mode::parse(&s.low_end_mode);
        let now = low_end_state(lowend::mode(s, inst), slow_reason(&self.dirs, s, ui.ctx()).as_ref(), "this game");
        let lowered = if inst.root.join(lowend::BACKUP_FILE).exists() {
            let n = lowend::Backup::load(&inst.root).changed.len();
            format!(" {n} settings of this game are lowered right now.")
        } else {
            String::new()
        };
        let hint =
            format!("{LOW_END_HINT} The launcher's setting is {}. {now}{lowered}", launcher.label().to_lowercase());
        rows.gap(ui);
        wide_row(ui, "Low-end mode", &hint, 310.0, |ui| {
            let choices =
                [("Launcher", None), ("Automatic", Some(Mode::Auto)), ("On", Some(Mode::On)), ("Off", Some(Mode::Off))];
            let labels: Vec<&str> = choices.iter().map(|c| c.0).collect();
            let chosen = choices.iter().position(|c| c.1 == own).unwrap_or(0);
            if let Some(i) = ui.push_id("low-end", |ui| segmented(ui, &labels, chosen)).inner {
                if choices[i].1 != own {
                    change = Some(HelperChange::LowEnd(choices[i].1));
                }
            }
        });
        // the pieces of Speed-up: greyed while it is off for this instance
        rows.gap(ui);
        ui.add_enabled_ui(on, |ui| {
            wide_row(ui, "Smoother chunk loading", THREADS_HINT, 230.0, |ui| {
                if let Some(v) = tri_choice(ui, "speedup-threads", speedup::threads_override(inst), s.speedup_threads) {
                    change = Some(HelperChange::SpeedupThreads(v));
                }
            });
        });
        rows.gap(ui);
        ui.add_enabled_ui(on, |ui| {
            wide_row(ui, "Use less memory", MEMORY_HINT, 230.0, |ui| {
                if let Some(v) = tri_choice(ui, "speedup-memory", speedup::memory_override(inst), s.speedup_memory) {
                    change = Some(HelperChange::SpeedupMemory(v));
                }
            });
        });
        if let Some(n) = old_java(speedup::java_major(inst)).filter(|_| on && !own_arguments) {
            ui.add_space(4.0);
            exception_line(ui, &Exception::OldJava(n));
        }
        rows.gap(ui);
        let gc_own = speedup::gc_override(inst);
        let gc_launcher = GcChoice::parse(&s.speedup_gc);
        let hint = format!("{GC_HINT} The launcher's setting is {}.", gc_label(gc_launcher).to_lowercase());
        ui.add_enabled_ui(on, |ui| {
            wide_row(ui, "Shorter pauses", &hint, 310.0, |ui| {
                let choices = [
                    ("Launcher", None),
                    ("Automatic", Some(GcChoice::Auto)),
                    ("On", Some(GcChoice::On)),
                    ("Off", Some(GcChoice::Off)),
                ];
                let labels: Vec<&str> = choices.iter().map(|c| c.0).collect();
                let chosen = choices.iter().position(|c| c.1 == gc_own).unwrap_or(0);
                if let Some(i) = ui.push_id("speedup-gc", |ui| segmented(ui, &labels, chosen)).inner {
                    if choices[i].1 != gc_own {
                        change = Some(HelperChange::SpeedupGc(choices[i].1));
                    }
                }
            });
        });
        change
    }

    /// Applies a change to an instance's helper switches (kept in the instance's own file).
    pub(super) fn apply_helper_change(&mut self, folder: &str, change: HelperChange) {
        if change == HelperChange::SimulationDistance {
            self.lower_simulation_distance(folder);
            return;
        }
        let Some(inst) = self.instances.iter_mut().find(|i| i.folder == folder) else { return };
        if matches!(
            change,
            HelperChange::Lite(Some(true))
                | HelperChange::Lite(None)
                | HelperChange::Speedup(Some(true))
                | HelperChange::Speedup(None)
        ) {
            // switched on again by the player: the count of early closes starts over
            lite::record_start(&self.dirs, &lite::instance_key(inst), false);
        }
        let extras = &mut inst.file.launcher.extras;
        let text = |v: String| Some(serde_json::Value::String(v));
        let (key, value) = match change {
            HelperChange::FastStart(v) => ("fastStart", v.map(serde_json::Value::Bool)),
            HelperChange::Lite(v) => ("lite", v.map(serde_json::Value::Bool)),
            HelperChange::LowEnd(v) => ("lowEndMode", v.and_then(|m| text(m.as_setting().into()))),
            HelperChange::Speedup(v) => ("speedup", v.map(serde_json::Value::Bool)),
            HelperChange::SpeedupThreads(v) => ("speedupThreads", v.map(serde_json::Value::Bool)),
            HelperChange::SpeedupMemory(v) => ("speedupMemory", v.map(serde_json::Value::Bool)),
            HelperChange::SpeedupGc(v) => ("speedupGc", v.and_then(|g| text(g.as_setting().into()))),
            HelperChange::SimulationDistance => return,
        };
        match value {
            Some(v) => {
                extras.insert(key.to_string(), v);
            }
            None => {
                extras.remove(key);
            }
        }
        if let Err(e) = inst.save() {
            self.toast_err(format!("Couldn't save the setting: {e}"));
        }
    }

    /// The button of the Performance menu: the simulation distance of the instance goes down to what is suggested.
    fn lower_simulation_distance(&mut self, folder: &str) {
        let Some(inst) = self.instance(folder).cloned() else { return };
        if self.is_playing(folder) {
            self.toast_err("Close the game first: it would write its own simulation distance back when it closes.");
            return;
        }
        match speedup::set_simulation_distance(&inst, speedup::SUGGESTED_SIMULATION_DISTANCE) {
            Ok(()) => {
                // read again the next time the menu shows
                self.ctx.data_mut(|d| d.remove_temp::<DistanceKept>(distance_id()));
                self.toast(format!(
                    "Simulation distance is now {}. It applies the next time the game starts.",
                    speedup::SUGGESTED_SIMULATION_DISTANCE
                ));
            }
            Err(e) => self.toast_err(format!("Couldn't change the simulation distance: {e}")),
        }
    }

    /// A one-time hint on the home page: this computer looks slow and Low-end mode is off.
    pub(super) fn low_end_hint(&mut self, ui: &mut Ui) {
        if self.settings.low_end_hint_dismissed || Mode::parse(&self.settings.low_end_mode) != Mode::Off {
            return;
        }
        let Some(Some(why)) = slow_reason(&self.dirs, &self.settings, ui.ctx()) else { return };
        let (mut use_it, mut dismiss) = (false, false);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 260.0).max(220.0));
                    ui.label(RichText::new("This computer looks slow").semibold());
                    small_muted(
                        ui,
                        format!(
                            "It has {why}. Low-end mode lowers the graphics settings of games so they run smoother, \
                             and puts your own settings back when you switch it off."
                        ),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary(ui, "No thanks").clicked() {
                        dismiss = true;
                    }
                    if primary(ui, "Use Low-end mode").clicked() {
                        use_it = true;
                    }
                });
            });
        });
        ui.add_space(8.0);
        if use_it || dismiss {
            if use_it {
                self.settings.low_end_mode = Mode::Auto.as_setting().to_string();
                self.toast("Low-end mode is on (automatic). Settings > Performance changes it.");
            }
            self.settings.low_end_hint_dismissed = true;
            self.save_settings();
        }
    }

    /// A game has ended: if a helper may have stopped it, offer to turn the helpers off; after two early failures in a
    /// row Speed-up switches itself off for that instance, and after two more the smoothness helper does.
    pub(super) fn helpers_after_exit(&mut self, key: &str, code: Option<i32>) {
        let Some(g) = self.games.get(key) else { return };
        if !g.fast_start && !g.lite && !g.speedup {
            return;
        }
        let log: Vec<String> = g.lines.iter().map(|l| l.text.clone()).collect();
        // Speed-up changes how Java starts, as Fast start does: a start it was part of is judged the same way
        let early = lite::may_be_the_cause(g.fast_start || g.speedup, g.lite, code, g.started.elapsed(), &log);
        let (folder, lite_was_on, fast_was_on, speedup_was_on) = (g.folder.clone(), g.lite, g.fast_start, g.speedup);
        let Some(inst) = self.instance(&folder).cloned() else { return };
        let failures = lite::record_start(&self.dirs, &lite::instance_key(&inst), early);
        if !early {
            return;
        }
        if speedup_was_on && lite::should_turn_off(failures) && speedup::enabled(&self.settings, &inst) {
            self.apply_helper_change(&folder, HelperChange::Speedup(Some(false)));
            // the others get their own turn: the count starts over
            lite::record_start(&self.dirs, &lite::instance_key(&inst), false);
            let text = format!(
                "[BlockForge] Speed-up is now off for this instance: the game closed early {failures} times in a \
                 row with it on. It can be switched on again beside Play or in the instance's settings."
            );
            if let Some(g) = self.games.get_mut(key) {
                g.lines.push(LogLine { text, level: Level::Warn });
            }
            self.toast_err(format!(
                "Speed-up was switched off for {}: the game closed early twice in a row.",
                inst.name()
            ));
            return;
        }
        if lite_was_on && lite::should_turn_off(failures) && lite::enabled(&self.settings, &inst) {
            self.apply_helper_change(&folder, HelperChange::Lite(Some(false)));
            let text = format!(
                "[BlockForge] The smoothness helper is now off for this instance: the game closed early {failures} \
                 times in a row with it on. It can be switched on again on the instance's card or in its settings."
            );
            if let Some(g) = self.games.get_mut(key) {
                g.lines.push(LogLine { text, level: Level::Warn });
            }
            self.toast_err(format!(
                "The smoothness helper was switched off for {}: the game closed early twice in a row.",
                inst.name()
            ));
            if !fast_was_on {
                return;
            }
        }
        self.boost_blame = Some(folder);
    }

    /// The question after a game closed early with a helper on.
    pub(super) fn helpers_blame_ui(&mut self, ctx: &egui::Context) {
        let Some(folder) = self.boost_blame.clone() else { return };
        // what is still on (a helper may have switched itself off already)
        let on = self.instance(&folder).map(|i| {
            (
                crate::launch::fast_start_enabled(&self.settings, i),
                lite::enabled(&self.settings, i),
                speedup::enabled(&self.settings, i),
            )
        });
        let Some(which) = on.and_then(|(fast, lite, speedup)| blame_text(fast, lite, speedup)) else {
            self.boost_blame = None;
            return;
        };
        let (mut off, mut keep) = (false, false);
        egui::Modal::new(egui::Id::unique("helpers-blame")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new("Minecraft closed early").size(FS_SECTION).semibold());
            muted(
                ui,
                format!(
                    "The game stopped soon after it started, and {which}. On some setups a helper can get in the \
                     way. Try the game again with the helpers turned off. The Logs tab shows what happened."
                ),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary(ui, "Turn off the helpers for this game").clicked() {
                    off = true;
                }
                if secondary(ui, "Keep them on").clicked() {
                    keep = true;
                }
            });
            // Esc keeps them as they are
            keep |= ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
        if off {
            self.apply_helper_change(&folder, HelperChange::FastStart(Some(false)));
            self.apply_helper_change(&folder, HelperChange::Lite(Some(false)));
            self.apply_helper_change(&folder, HelperChange::Speedup(Some(false)));
            self.toast(
                "Fast start, the smoothness helper and Speed-up are off for this instance. Its menu turns them on \
                 again.",
            );
            self.boost_blame = None;
        } else if keep {
            self.boost_blame = None;
        }
    }
}

/// How a collector choice is said.
fn gc_label(choice: GcChoice) -> &'static str {
    match choice {
        GcChoice::Auto => "Automatic",
        GcChoice::On => "On",
        GcChoice::Off => "Off",
    }
}

/// How wide the lines of the Performance menu are (a menu is as wide as what is put in it).
const MENU_W: f32 = 270.0;

/// A hairline between the groups of the Performance menu.
fn menu_rule(ui: &mut Ui) {
    ui.add_space(4.0);
    let (line, _) = ui.allocate_exact_size(egui::vec2(MENU_W, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, c_border());
    ui.add_space(4.0);
}

/// What is said about Speed-up in the Performance menu, a line that wraps in the menu's width.
fn menu_note(ui: &mut Ui, e: &Exception) {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
        ui.set_width(MENU_W - 20.0);
        exception_line(ui, e);
    });
}

/// A switch with its label in a menu: the whole line takes the click, with a fill under the pointer, and the switch
/// sits at its right end. The hint shows on hover.
fn menu_switch(ui: &mut Ui, on: &mut bool, label: &str, hint: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(MENU_W, 36.0), egui::Sense::click());
    let changed = resp.clicked();
    if changed {
        *on = !*on;
    }
    let t = motion::hover(ui, resp.id, resp.hovered());
    if t > 0.0 {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(R_CONTROL - 2), c_hover().gamma_multiply(t));
    }
    let galley = ui.painter().layout_no_wrap(label.to_string(), super::fonts::medium(FS_BODY), c_text());
    ui.painter().galley(egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, c_text());
    // the switch, drawn as the launcher's other switches are: the brand gradient sliding in as it turns on
    let s = motion::toward(ui.ctx(), resp.id.with("on"), *on, motion::MEDIUM);
    let track = egui::Rect::from_center_size(egui::pos2(rect.right() - 28.0, rect.center().y), egui::vec2(36.0, 20.0));
    ui.painter().rect_filled(track, egui::CornerRadius::same(10), c_active());
    ui.painter().rect_stroke(
        track,
        egui::CornerRadius::same(10),
        egui::Stroke::new(1.0, c_control().gamma_multiply(1.0 - s)),
        egui::StrokeKind::Inside,
    );
    if s > 0.0 {
        motion::gradient_rect(ui.painter(), track, 10, c_accent().gamma_multiply(s), c_accent2().gamma_multiply(s));
    }
    let knob = egui::pos2(track.left() + 10.0 + s * (track.width() - 20.0), track.center().y);
    ui.painter().circle_filled(knob + egui::vec2(0.0, 1.0), 7.5, egui::Color32::from_black_alpha(40));
    ui.painter().circle_filled(knob, 7.0, motion::mix(c_muted(), egui::Color32::WHITE, s));
    focus_ring(ui, &resp, rect, R_CONTROL - 2);
    let state = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, true, state, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hint);
    changed
}

/// A small grey title over a group of menu items.
fn menu_heading(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(RichText::new(text).size(FS_META).color(c_muted()));
    });
}

/// One of several choices in a menu: a line with a fill under the pointer and a round mark, filled with the brand
/// gradient when it is the chosen one.
fn menu_choice(ui: &mut Ui, selected: bool, label: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(MENU_W, 32.0), egui::Sense::click());
    let t = motion::hover(ui, resp.id, resp.hovered());
    if t > 0.0 {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(R_CONTROL - 2), c_hover().gamma_multiply(t));
    }
    let c = egui::pos2(rect.left() + 18.0, rect.center().y);
    let s = motion::toward(ui.ctx(), resp.id.with("on"), selected, motion::FAST);
    ui.painter().circle_stroke(c, 7.5, egui::Stroke::new(1.5, c_control().gamma_multiply(1.0 - s)));
    if s > 0.0 {
        let dot = egui::Rect::from_center_size(c, egui::vec2(16.0, 16.0));
        motion::gradient_rect(ui.painter(), dot, 8, c_accent().gamma_multiply(s), c_accent2().gamma_multiply(s));
        ui.painter().circle_filled(c, 3.0 * s, c_on_accent());
    }
    let fg = if selected { c_text() } else { motion::mix(c_muted(), c_text(), t) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), super::fonts::medium(FS_BODY), fg);
    ui.painter().galley(egui::pos2(rect.left() + 34.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    focus_ring(ui, &resp, rect, R_CONTROL - 2);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "Launcher (on)" / "On" / "Off" for a switch an instance can follow or override. Returns the new choice.
fn tri_choice(ui: &mut Ui, id: &str, current: Option<bool>, launcher: bool) -> Option<Option<bool>> {
    let follow = format!("Launcher ({})", if launcher { "on" } else { "off" });
    let values = [None, Some(true), Some(false)];
    let chosen = values.iter().position(|v| *v == current).unwrap_or(0);
    let picked = ui.push_id(id, |ui| segmented(ui, &[follow.as_str(), "On", "Off"], chosen)).inner?;
    (values[picked] != current).then_some(values[picked])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_ages_read_plainly() {
        assert_eq!(ago(SystemTime::now()), "just now");
        assert_eq!(ago(SystemTime::now() - Duration::from_secs(600)), "10 min ago");
        assert_eq!(ago(SystemTime::now() - Duration::from_secs(3 * 86400)), "3 days ago");
    }

    #[test]
    fn the_overview_line_says_what_each_helper_is_doing() {
        assert_eq!(
            speed_summary_text(true, true, true, None, Mode::Off),
            "Speed-up on \u{00B7} Fast start on \u{00B7} Smoothness helper on \u{00B7} Low-end mode like the launcher (off)"
        );
        assert_eq!(
            speed_summary_text(false, false, true, Some(Mode::Auto), Mode::Off),
            "Speed-up off \u{00B7} Fast start off \u{00B7} Smoothness helper on \u{00B7} Low-end mode automatic"
        );
        assert!(speed_summary_text(true, true, false, None, Mode::On).ends_with("like the launcher (on)"));
    }

    #[test]
    fn a_switch_turned_back_to_the_launchers_setting_follows_it_again() {
        assert_eq!(pin(true, true), None);
        assert_eq!(pin(false, false), None);
        assert_eq!(pin(false, true), Some(false));
        assert_eq!(pin(true, false), Some(true));
    }

    #[test]
    fn the_switch_beside_play_says_when_a_change_waits_for_the_next_start() {
        assert_eq!(speedup_tip(None, true), SPEEDUP_TIP);
        assert_eq!(
            speedup_tip(Some(true), true),
            SPEEDUP_TIP,
            "a game that started the same way has nothing to wait for"
        );
        assert!(SPEEDUP_TIP.ends_with("Applies the next time the game starts."));
        let waits = speedup_tip(Some(true), false);
        assert!(waits.contains("started with Speed-up on") && waits.contains("applies the next time it starts"));
        assert!(speedup_tip(Some(false), true).contains("started with Speed-up off"));
    }

    #[test]
    fn what_is_said_about_speedup_goes_by_what_matters_most() {
        use Exception::*;
        // the player's own arguments are said whatever the switch is
        assert_eq!(exception(true, false, Some(21), false), Some(OwnArguments));
        assert_eq!(exception(true, true, Some(8), true), Some(OwnArguments));
        // switched off: nothing else to say
        assert_eq!(exception(false, false, Some(8), true), None);
        assert_eq!(exception(false, true, Some(21), true), Some(OldJava(21)));
        assert_eq!(exception(false, true, Some(25), true), Some(CacheBeingMade));
        // nothing when all is normal, and a version that names no Java isn't called too old
        assert_eq!(exception(false, true, Some(25), false), None);
        assert_eq!(exception(false, true, None, false), None);
    }

    #[test]
    fn the_status_lines_say_exactly_this() {
        assert_eq!(Exception::OwnArguments.text(), "Your Java arguments are in charge, so these do nothing.");
        assert_eq!(
            Exception::OldJava(17).text(),
            "Not used here: this game runs Java 17. This needs Java 25 or newer."
        );
        assert_eq!(
            Exception::CacheBeingMade.text(),
            "Getting this ready after this game closes. The start after that is the fast one."
        );
    }

    #[test]
    fn the_simulation_distance_is_only_worth_a_word_above_the_limit() {
        assert_eq!(simulation_advice(None), None);
        assert_eq!(simulation_advice(Some(speedup::HIGH_SIMULATION_DISTANCE)), None);
        assert_eq!(
            simulation_advice(Some(32)).as_deref(),
            Some("Simulation distance is 32. This is often the biggest load on the game.")
        );
    }

    #[test]
    fn the_distance_is_read_again_when_the_menu_is_opened_again() {
        assert!(menu_still_open(40, 40));
        assert!(menu_still_open(40, 41));
        assert!(!menu_still_open(40, 43), "frames went by without the menu");
        assert!(menu_still_open(41, 40), "a pass counted backwards is no reason to read the file");
    }

    #[test]
    fn low_end_mode_is_said_in_sentences_of_its_own() {
        let weak = Some("4 processor threads".to_string());
        assert_eq!(
            low_end_state(Mode::Auto, Some(&weak), "x"),
            "Right now it is on, because this computer looks slow (4 processor threads)."
        );
        assert_eq!(
            low_end_state(Mode::Auto, Some(&None), "x"),
            "Right now it is off, because this computer doesn't look slow."
        );
        assert!(low_end_state(Mode::Auto, None, "x").starts_with("Right now: checking"));
        assert_eq!(low_end_state(Mode::On, None, "this game"), "It is on for this game.");
        // every state ends its sentence, so what follows it can start a new one
        for state in [Mode::Auto, Mode::On, Mode::Off] {
            assert!(low_end_state(state, Some(&None), "x").ends_with('.'));
        }
    }

    #[test]
    fn the_helpers_that_were_on_are_named_for_the_question_after_an_early_close() {
        assert_eq!(blame_text(false, false, false), None);
        assert_eq!(blame_text(true, false, false).as_deref(), Some("Fast start was on"));
        assert_eq!(blame_text(false, false, true).as_deref(), Some("Speed-up was on"));
        assert_eq!(blame_text(true, true, false).as_deref(), Some("Fast start and the smoothness helper were on"));
        assert_eq!(
            blame_text(true, true, true).as_deref(),
            Some("Fast start, the smoothness helper and Speed-up were on")
        );
    }

    #[test]
    fn the_hints_leave_out_jargon_and_the_old_remarks() {
        for hint in [SPEEDUP_HINT, QUICKER_START_HINT, THREADS_HINT, MEMORY_HINT, GC_HINT, UNSIGNED_JAR_HINT] {
            for word in ["AOT", "ZGC", "CDS", "compact object", "G1"] {
                assert!(!hint.contains(word), "{word} in {hint}");
            }
        }
        assert!(!LITE_HINT.contains("each instance has its own switch"));
        assert!(!FAST_START_HINT.contains("each instance has its own switch"));
    }
}
