//! The Performance controls: Fast start, the smoothness helper and Low-end mode, for all games (Settings > Game) and
//! for one instance (its card, its page and its settings); the hint that suggests Low-end mode on a slow computer; and
//! what happens when a game closes early while a helper was on.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use egui::{RichText, Ui};

use super::App;
use super::instance_menu::Act;
use super::settings::{Rows, wide_row};
use super::theme::*;
use super::widgets::*;
use crate::config::Settings;
use crate::instances::Instance;
use crate::launch::lite::{self, Patch, PatchState};
use crate::launch::lowend::{self, Mode};
use crate::launch::{Level, LogLine};
use crate::paths::Dirs;

/// A change to one instance's helper switches. `None` for Low-end mode means "follow the launcher".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelperChange {
    FastStart(Option<bool>),
    Lite(Option<bool>),
    LowEnd(Option<Mode>),
}

const LOW_END_HINT: &str = "Lowers the view distance, clouds, shadows, particles and smooth lighting so the game runs \
    smoother. It only ever lowers, and switching it off puts your own settings back.";

const LITE_HINT: &str = "Small fixes inside the game that make it run smoother. Works with every Minecraft version and \
    mod loader (Fabric, Quilt, Forge, NeoForge). Switch it off if a game misbehaves; each instance has its own switch too";

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

/// The hairline between two rows of a card, with the same air on both sides as the rules of the other cards.
fn row_gap(ui: &mut Ui) {
    let pad = (10.0 - ui.spacing().item_spacing.y).max(0.0);
    ui.add_space(pad);
    rule(ui);
    ui.add_space(pad);
}

/// The words of [`App::speed_summary`]: `own` is the instance's own Low-end mode (`None`: it follows the launcher's).
fn speed_summary_text(fast: bool, smooth: bool, own: Option<Mode>, launcher: Mode) -> String {
    let state = |on: bool| if on { "on" } else { "off" };
    let low_end = match own {
        Some(mode) => format!("Low-end mode {}", mode.label().to_lowercase()),
        None => format!("Low-end mode like the launcher ({})", launcher.label().to_lowercase()),
    };
    format!("Fast start {} \u{00B7} Smoothness helper {} \u{00B7} {low_end}", state(fast), state(smooth))
}

fn state_color(state: PatchState) -> egui::Color32 {
    match state {
        PatchState::On => c_text(),
        PatchState::Off | PatchState::Other => c_muted(),
        PatchState::Problem => c_danger(),
    }
}

/// What Settings needs to draw the Performance rows, gathered before the settings are borrowed for editing.
pub(super) struct SettingsView {
    slow: Option<Option<String>>,
    report: Report,
    names: Vec<(String, String)>,
}

impl SettingsView {
    pub(super) fn new(app: &App, ctx: &egui::Context) -> SettingsView {
        SettingsView {
            slow: slow_reason(&app.dirs, &app.settings, ctx),
            report: last_report(&app.dirs),
            names: app.instances.iter().map(|i| (lite::instance_key(i), i.name().to_string())).collect(),
        }
    }

    fn low_end_status(&self, mode: Mode) -> String {
        match (mode, &self.slow) {
            (Mode::Auto, None) => "Automatic: checking this computer\u{2026}".to_string(),
            (Mode::Auto, Some(Some(why))) => format!("Automatic: on, this computer looks slow ({why})"),
            (Mode::Auto, Some(None)) => "Automatic: off, this computer doesn't look slow".to_string(),
            (Mode::On, _) => "On for every game without its own setting".to_string(),
            (Mode::Off, _) => "Off: games keep their own graphics settings".to_string(),
        }
    }

    /// The Performance rows of Settings > Game. Returns true when a setting changed.
    pub(super) fn rows(&self, ui: &mut Ui, s: &mut Settings) -> bool {
        let mut dirty = false;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            row_gap(ui);
            ui.label(RichText::new("Performance").size(FS_SECTION).semibold());
            ui.add_space(6.0);
            setting_row(
                ui,
                "Fast start",
                "A tiny helper that loads Minecraft's classes ahead of time so games open sooner. Works with every version and mod loader; each instance has its own switch too",
                |ui| {
                    dirty |= toggle(ui, &mut s.fast_start).changed();
                },
            );
            row_gap(ui);
            setting_row(ui, "Smoothness helper", LITE_HINT, |ui| {
                dirty |= toggle(ui, &mut s.lite).changed();
            });
            row_gap(ui);
            let mode = Mode::parse(&s.low_end_mode);
            wide_row(ui, "Low-end mode", &format!("{LOW_END_HINT} {}.", self.low_end_status(mode)), 200.0, |ui| {
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
            row_gap(ui);
            egui::CollapsingHeader::new(RichText::new("Details").color(c_muted()))
                .id_salt("perf-details")
                .default_open(false)
                .show(ui, |ui| self.details(ui));
        });
        dirty
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
        egui::Grid::new("perf-patches").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
            for p in patches {
                ui.label(RichText::new(&p.id).monospace());
                ui.label(RichText::new(p.state.label()).color(state_color(p.state)).semibold());
                ui.vertical(|ui| {
                    // what it is, in plain words, then what happened in that game
                    if !p.description.is_empty() {
                        ui.label(RichText::new(&p.description).size(FS_META));
                    }
                    if !p.detail.is_empty() && p.detail != p.description {
                        small_muted(ui, &p.detail);
                    }
                });
                ui.end_row();
            }
        });
        small_muted(
            ui,
            "To leave out one of these, put its name in \"liteOff\" in BlockForge.json (several: separated by ;).",
        );
    }
}

impl App {
    /// What an instance's three helpers are doing, in one line, for its Overview (they are changed in its settings):
    /// "Fast start on · Smoothness helper on · Low-end mode like the launcher (off)".
    pub(super) fn speed_summary(&self, inst: &Instance) -> String {
        speed_summary_text(
            crate::launch::fast_start_enabled(&self.settings, inst),
            lite::enabled(&self.settings, inst),
            lowend::instance_override(inst),
            Mode::parse(&self.settings.low_end_mode),
        )
    }

    /// The Low-end choices of an instance, as menu items.
    fn low_end_choices(&self, ui: &mut Ui, inst: &Instance) -> Option<Act> {
        let mut act = None;
        ui.set_min_width(220.0);
        small_muted(ui, "Low-end mode for this instance");
        let own = lowend::instance_override(inst);
        let launcher = Mode::parse(&self.settings.low_end_mode);
        let choices = [
            (None, format!("Like the launcher ({})", launcher.label().to_lowercase())),
            (Some(Mode::Auto), "Automatic".to_string()),
            (Some(Mode::On), "On".to_string()),
            (Some(Mode::Off), "Off".to_string()),
        ];
        for (value, label) in choices {
            if ui.radio(own == value, label).clicked() {
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
            ui.set_min_width(220.0);
            let mut fast = crate::launch::fast_start_enabled(&self.settings, inst);
            if menu_switch(ui, &mut fast, "Fast start", "Opens the game sooner") {
                act = Some(Act::FastStart(inst.folder.clone(), fast));
            }
            let mut smooth = lite::enabled(&self.settings, inst);
            if menu_switch(ui, &mut smooth, "Smoothness helper", LITE_HINT) {
                act = Some(Act::Helper(inst.folder.clone(), HelperChange::Lite(Some(smooth))));
            }
            ui.separator();
            if let Some(a) = self.low_end_choices(ui, inst) {
                act = Some(a);
            }
        });
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
        let fast_own = crate::launch::boost::instance_override(inst);
        rows.gap(ui);
        wide_row(ui, "Fast start", "Loads Minecraft's classes ahead of time so the game opens sooner", 230.0, |ui| {
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
        let status = if inst.root.join(lowend::BACKUP_FILE).exists() {
            let n = lowend::Backup::load(&inst.root).changed.len();
            format!(" Now: {n} settings of this game are lowered.")
        } else {
            String::new()
        };
        rows.gap(ui);
        wide_row(ui, "Low-end mode", &format!("{LOW_END_HINT}{status}"), 310.0, |ui| {
            let follow = format!("Launcher ({})", launcher.label().to_lowercase());
            let choices = [
                (follow.as_str(), None),
                ("Automatic", Some(Mode::Auto)),
                ("On", Some(Mode::On)),
                ("Off", Some(Mode::Off)),
            ];
            let labels: Vec<&str> = choices.iter().map(|c| c.0).collect();
            let chosen = choices.iter().position(|c| c.1 == own).unwrap_or(0);
            if let Some(i) = ui.push_id("low-end", |ui| segmented(ui, &labels, chosen)).inner {
                if choices[i].1 != own {
                    change = Some(HelperChange::LowEnd(choices[i].1));
                }
            }
        });
        change
    }

    /// Applies a change to an instance's helper switches (kept in the instance's own file).
    pub(super) fn apply_helper_change(&mut self, folder: &str, change: HelperChange) {
        let Some(inst) = self.instances.iter_mut().find(|i| i.folder == folder) else { return };
        if matches!(change, HelperChange::Lite(Some(true)) | HelperChange::Lite(None)) {
            // switched on again by the player: the count of early closes starts over
            lite::record_start(&self.dirs, &lite::instance_key(inst), false);
        }
        let extras = &mut inst.file.launcher.extras;
        let (key, value) = match change {
            HelperChange::FastStart(v) => ("fastStart", v.map(serde_json::Value::Bool)),
            HelperChange::Lite(v) => ("lite", v.map(serde_json::Value::Bool)),
            HelperChange::LowEnd(v) => ("lowEndMode", v.map(|m| serde_json::Value::String(m.as_setting().into()))),
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
                self.toast("Low-end mode is on (automatic). Settings > Game changes it.");
            }
            self.settings.low_end_hint_dismissed = true;
            self.save_settings();
        }
    }

    /// A game has ended: if a helper may have stopped it, offer to turn the helpers off; after two early failures in a
    /// row the smoothness helper switches itself off for that instance.
    pub(super) fn helpers_after_exit(&mut self, key: &str, code: Option<i32>) {
        let Some(g) = self.games.get(key) else { return };
        if !g.fast_start && !g.lite {
            return;
        }
        let log: Vec<String> = g.lines.iter().map(|l| l.text.clone()).collect();
        let early = lite::may_be_the_cause(g.fast_start, g.lite, code, g.started.elapsed(), &log);
        let (folder, lite_was_on, fast_was_on) = (g.folder.clone(), g.lite, g.fast_start);
        let Some(inst) = self.instance(&folder).cloned() else { return };
        let failures = lite::record_start(&self.dirs, &lite::instance_key(&inst), early);
        if !early {
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
        // what is still on (the smoothness helper may have switched itself off already)
        let on = self
            .instance(&folder)
            .map(|i| (crate::launch::fast_start_enabled(&self.settings, i), lite::enabled(&self.settings, i)));
        let which = match on {
            Some((true, true)) => {
                "BlockForge's helpers were on (Fast start, which opens the game sooner, and the \
                                   smoothness helper)"
            }
            Some((true, false)) => "Fast start was on (a helper that opens the game sooner)",
            Some((false, true)) => "the smoothness helper was on",
            _ => {
                self.boost_blame = None;
                return;
            }
        };
        let (mut off, mut keep) = (false, false);
        egui::Modal::new(egui::Id::unique("helpers-blame")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new("Minecraft closed early").size(FS_SECTION).semibold());
            muted(
                ui,
                format!(
                    "The game stopped soon after it started while {which}. On some setups a helper can get in the \
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
            self.toast("Fast start and the smoothness helper are off for this instance. Its menu turns them on again.");
            self.boost_blame = None;
        } else if keep {
            self.boost_blame = None;
        }
    }
}

/// A switch with its label in a menu.
fn menu_switch(ui: &mut Ui, on: &mut bool, label: &str, hint: &str) -> bool {
    ui.horizontal(|ui| {
        let mut changed = toggle(ui, on).on_hover_text(hint).changed();
        let r = ui.add(egui::Label::new(label).sense(egui::Sense::click())).on_hover_text(hint);
        if r.clicked() {
            *on = !*on;
            changed = true;
        }
        changed
    })
    .inner
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
            speed_summary_text(true, true, None, Mode::Off),
            "Fast start on \u{00B7} Smoothness helper on \u{00B7} Low-end mode like the launcher (off)"
        );
        assert_eq!(
            speed_summary_text(false, true, Some(Mode::Auto), Mode::Off),
            "Fast start off \u{00B7} Smoothness helper on \u{00B7} Low-end mode automatic"
        );
        assert!(speed_summary_text(true, false, None, Mode::On).ends_with("like the launcher (on)"));
    }
}
