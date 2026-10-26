//! Settings: the same sections, options, wording and defaults as the Java edition's Settings page. Every change is
//! saved as it is made, and a small "Saved" says so for a moment.

use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::App;
use super::theme::{self, *};
use super::widgets::*;

/// The widest the page's content gets; a wider window leaves the rest empty rather than stretching the rows.
const PAGE_WIDTH: f32 = 720.0;

/// How long "Saved" stays on the title row after a setting was saved, and how much of that it takes to fade out.
const SAVED_FOR: Duration = Duration::from_millis(2000);
const SAVED_FADE: Duration = Duration::from_millis(400);
/// The room "Saved" always takes, shown or not, so nothing on the title row moves.
const SAVED_WIDTH: f32 = 56.0;

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

/// "Saved" in a fixed slot at the right of the title row, there for two seconds after a save and then gone.
fn saved_stamp(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SAVED_WIDTH, 20.0), egui::Sense::hover());
    let Some(at) = ui.data(|d| d.get_temp::<Instant>(saved_at_id())) else { return };
    let Some(alpha) = saved_alpha(at.elapsed()) else { return };
    ui.painter().text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        "Saved",
        egui::FontId::proportional(FS_META),
        c_faint().gamma_multiply(alpha),
    );
    ui.ctx().request_repaint_after(Duration::from_millis(50));
}

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

/// The rules between the rows of one card: call [`Rows::gap`] before each row.
pub(super) struct Rows {
    first: bool,
}

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
        let pad = (10.0 - ui.spacing().item_spacing.y).max(0.0);
        ui.add_space(pad);
        rule(ui);
        ui.add_space(pad);
    }
}

/// A titled block: the heading with its hint, then one card holding the rows, divided by rules.
fn section(ui: &mut Ui, title: &str, hint: &str, rows: impl FnOnce(&mut Ui, &mut Rows)) {
    section_head(ui, title, hint);
    card(ui, |ui| {
        // the title of a row sits close to its explanation
        ui.spacing_mut().item_spacing.y = 2.0;
        rows(ui, &mut Rows::new());
    });
}

/// A setting row for a control wider than [`setting_row`] leaves room for (a drop-down, a text field, a segmented
/// control): the explanation keeps clear of the control, and on a narrow page the control goes under the text.
pub(super) fn wide_row(ui: &mut Ui, title: &str, hint: &str, control_width: f32, control: impl FnOnce(&mut Ui)) {
    let room = ui.available_width();
    if room < control_width + 240.0 {
        ui.label(RichText::new(title).semibold());
        if !hint.is_empty() {
            small_muted(ui, hint);
        }
        ui.horizontal_wrapped(control);
        ui.add_space(4.0);
        return;
    }
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.set_max_width(room - control_width - 16.0);
            ui.label(RichText::new(title).semibold());
            if !hint.is_empty() {
                small_muted(ui, hint);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
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

/// A text setting that is saved when the field is left or Enter is pressed. Returns the typed text then.
fn text_row(ui: &mut Ui, title: &str, hint: &str, placeholder: &str, value: &str) -> Option<String> {
    let mut committed = None;
    wide_row(ui, title, hint, 300.0, |ui| {
        let width = (ui.available_width() - 8.0).clamp(160.0, 300.0);
        committed = draft_text(ui, title, value, placeholder, [width, 32.0], egui::Margin::symmetric(10, 7));
    });
    committed
}

/// The warning under the Java path row when the folder typed there has no Java in it.
fn java_path_warning(custom: Option<&str>) -> Option<&'static str> {
    custom.filter(|p| !java_found(p)).map(|_| {
        "No Java was found in that folder, so the automatic one is used. Enter the Java folder, the one with bin in it."
    })
}

/// The "Graphics card" row for computers with two cards. `gpus` is `None` while they are being looked for. Returns
/// true when the choice changed.
fn graphics_card_row(ui: &mut Ui, setting: &mut String, gpus: Option<&[crate::gpu::Gpu]>) -> bool {
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
            None => "For laptops with two graphics cards. Applies the next time the game starts.".to_string(),
        },
    };
    let mut changed = false;
    wide_row(ui, "Graphics card", &hint, 310.0, |ui| {
        ui.add_enabled_ui(two, |ui| {
            egui::ComboBox::from_id_salt("gpu").selected_text(short(&choice)).width(300.0).show_ui(ui, |ui| {
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
    changed
}

/// The top of the memory slider: half of the computer's memory in steps of 256 MB, at least 2 GB.
fn memory_slider_max(total_mb: u64) -> u32 {
    let half = if total_mb > 0 { total_mb / 2 / 256 * 256 } else { 8192 };
    (half.min(u32::MAX as u64) as u32).max(2048)
}

impl App {
    pub(super) fn settings_ui(&mut self, ui: &mut Ui) {
        // the title, and at the right of the same row a "Saved" that shows for a moment after each save
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - 14.0).min(PAGE_WIDTH));
            // as tall as the title, like the title row of every other page
            ui.spacing_mut().interact_size.y = 0.0;
            ui.horizontal(|ui| {
                heading(ui, "Settings");
                ui.with_layout(Layout::right_to_left(Align::Center), saved_stamp);
            });
        });
        let mut dirty = false;
        let mut dragged = false;
        let mut theme_changed = false;
        let mut open_data = false;
        let (total_mb, free_mb) = memory_now();
        let max_mb = memory_slider_max(total_mb);
        let example = self.automatic_memory_example(total_mb, free_mb);
        let self_gpus = self.gpus.clone();
        let perf = super::performance::SettingsView::new(self, ui.ctx());
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // at most 760 wide, but never wider than the window (set_max_width alone would widen a narrow page)
            ui.set_max_width((ui.available_width() - 14.0).min(PAGE_WIDTH));
            let s = &mut self.settings;
            section(ui, "Appearance", "How the launcher looks", |ui, r| {
                r.gap(ui);
                wide_row(ui, "Theme", "Light and dark looks for the whole launcher", 310.0, |ui| {
                    egui::ComboBox::from_id_salt("theme")
                        .selected_text(theme::theme_label(&s.theme))
                        .width(300.0)
                        .show_ui(ui, |ui| {
                            for (id, label, _) in theme::THEMES {
                                if ui.selectable_label(s.theme == id, label).clicked() && s.theme != id {
                                    s.theme = id.to_string();
                                    theme_changed = true;
                                    dirty = true;
                                }
                            }
                        });
                });
                r.gap(ui);
                wide_row(
                    ui,
                    "Interface size",
                    "How big text and buttons are. Ctrl + and Ctrl \u{2212} change it too",
                    330.0,
                    |ui| {
                        let zoom = ui.ctx().zoom_factor();
                        let labels: Vec<&str> = ZOOMS.iter().map(|z| z.1).collect();
                        if let Some(i) = segmented(ui, &labels, nearest_zoom(zoom)) {
                            // the window applies it (and keeps it for the next start) on the next frame
                            ui.ctx().set_zoom_factor(ZOOMS[i].0);
                        }
                    },
                );
                // no Language row: the window is in English only for now, so a choice there would change nothing.
                // `language` stays in the settings file for the Java edition.
            });
            section(ui, "Game", "Memory, Java and how games start", |ui, r| {
                // memory
                let mut auto = s.automatic_memory != Some(false);
                let hint = if auto {
                    example.clone()
                } else {
                    "Set by hand, for every game without its own setting".to_string()
                };
                r.gap(ui);
                wide_row(ui, "Memory", &hint, if auto { 140.0 } else { 400.0 }, |ui| {
                    if toggle(ui, &mut auto).changed() {
                        s.automatic_memory = Some(auto);
                        dirty = true;
                    }
                    ui.label("Automatic");
                    if !auto {
                        // the row lays out from the right: the amount, then the slider to its left
                        ui.add_space(16.0);
                        let label = ui.label(
                            RichText::new(memory_text(s.maximum_memory)).font(super::fonts::medium(FS_BODY)),
                        );
                        if s.maximum_memory > max_mb {
                            // set elsewhere (an imported setting, the file): more than the slider goes to
                            label.on_hover_text(format!(
                                "More than the slider goes to ({}, half of this computer's memory). Drag it to pick less.",
                                memory_text(max_mb)
                            ));
                        }
                        ui.scope(|ui| {
                            // a rail that can be seen, filled in ink up to the handle (like a switch that is on)
                            ui.visuals_mut().widgets.inactive.bg_fill = c_active();
                            ui.visuals_mut().selection.bg_fill = c_ink();
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
                r.gap(ui);
                setting_row(
                    ui,
                    "Speed up game start (class cache)",
                    "Makes the next start of a game quicker (Minecraft 1.18 and newer)",
                    |ui| {
                        dirty |= toggle(ui, &mut s.class_data_cache).changed();
                    },
                );
                let mut warm = s.extras.get("warmUpGameFiles").and_then(|v| v.as_bool()).unwrap_or(true);
                r.gap(ui);
                setting_row(
                    ui,
                    "Read game files ahead of Play",
                    "Loads a game's files into memory while the launcher is idle so Play starts quicker",
                    |ui| {
                        if toggle(ui, &mut warm).changed() {
                            s.extras.insert("warmUpGameFiles".into(), warm.into());
                            dirty = true;
                        }
                    },
                );
                r.gap(ui);
                wide_row(
                    ui,
                    "Java compiler",
                    "Automatic uses the light one on computers with few cores, for much quicker starts. Full performance can give higher frame rates in long sessions.",
                    300.0,
                    |ui| {
                        const MODES: [(&str, &str); 3] =
                            [("auto", "Automatic"), ("fast", "Light"), ("full", "Full performance")];
                        let labels = MODES.map(|(_, label)| label);
                        let now = MODES.iter().position(|(mode, _)| *mode == s.startup_mode).unwrap_or(usize::MAX);
                        if let Some(i) = segmented(ui, &labels, now).filter(|&i| i != now) {
                            s.startup_mode = MODES[i].0.to_string();
                            dirty = true;
                        }
                    },
                );
                r.gap(ui);
                setting_row(
                    ui,
                    "Pick game settings for this PC on first launch",
                    "New games start with graphics that suit this computer",
                    |ui| {
                        dirty |= toggle(ui, &mut s.auto_game_options).changed();
                    },
                );
                if crate::gpu::supported() {
                    r.gap(ui);
                    dirty |= graphics_card_row(ui, &mut s.gpu, self_gpus.as_deref());
                }
                // an empty path means automatic. The game only falls back to this Java when it doesn't use the one that
                // comes with Minecraft (an instance's own Java setting), as in the Java edition.
                let custom = s.using_custom_java_path.then(|| s.java_path.clone()).filter(|p| !p.is_empty());
                r.gap(ui);
                if let Some(path) = text_row(
                    ui,
                    "Java path",
                    "For games set not to use the Java that comes with Minecraft. Leave empty for automatic",
                    "Automatic (recommended)",
                    custom.as_deref().unwrap_or(""),
                ) {
                    let path = path.trim().trim_matches('"').to_string();
                    if custom.as_deref().unwrap_or("") != path {
                        // a cleared path is cleared for real, or the old Java would still be used at launch
                        s.using_custom_java_path = !path.is_empty();
                        s.java_path = path;
                        dirty = true;
                    }
                }
                if let Some(warning) = java_path_warning(custom.as_deref()) {
                    ui.add(egui::Label::new(RichText::new(warning).color(c_warn()).size(FS_META)).wrap());
                }
                r.gap(ui);
                if let Some(args) =
                    text_row(ui, "Java arguments", "Extra options passed to the Java virtual machine", "", &s.java_parameters)
                {
                    let args = args.trim().to_string();
                    if args != s.java_parameters {
                        s.java_parameters = args;
                        dirty = true;
                    }
                }
                r.gap(ui);
                setting_row(
                    ui,
                    "Check for BlockForge updates",
                    "Look on GitHub for a newer version and offer to install it",
                    |ui| {
                        dirty |= toggle(ui, &mut s.check_for_launcher_updates).changed();
                    },
                );
                r.gap(ui);
                setting_row(ui, "Keep the launcher open", "Leave BlockForge open while Minecraft is running", |ui| {
                    dirty |= toggle(ui, &mut s.keep_launcher_open).changed();
                });
                r.gap(ui);
                setting_row(ui, "Show the console", "Open the log window next to the launcher", |ui| {
                    dirty |= toggle(ui, &mut s.enable_console).changed();
                });
                // its rows are laid out by their own code, with more room between them than the rest of the card
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    dirty |= perf.rows(ui, s);
                });
            });
            section(ui, "Privacy", "Off unless you turn it on", |ui, r| {
                r.gap(ui);
                setting_row(
                    ui,
                    "Share anonymous usage statistics",
                    "Which screens are used and how often. Off unless you turn it on",
                    |ui| {
                        dirty |= toggle(ui, &mut s.enable_analytics).changed();
                    },
                );
                r.gap(ui);
                setting_row(
                    ui,
                    "Report pack installs and play time",
                    "Counts for public modpacks, so authors can see how they are used. Off unless you turn it on. The network checker in Tools needs this on",
                    |ui| {
                        dirty |= toggle(ui, &mut s.enable_logs).changed();
                    },
                );
            });
            section(ui, "Launcher", "The window, the tray and where files are kept", |ui, r| {
                let low_end = super::low_end_pc();
                let hint = if low_end {
                    "Fewer animations and a smaller picture cache. On by default for this computer"
                } else {
                    "Fewer animations and a smaller picture cache, for computers with 4 GB of memory or less"
                };
                r.gap(ui);
                setting_row(ui, "Light mode for slower PCs", hint, |ui| {
                    let mut on = s.light_mode.unwrap_or(low_end);
                    if toggle(ui, &mut on).changed() {
                        s.light_mode = Some(on);
                        theme_changed = true;
                        dirty = true;
                    }
                });
                r.gap(ui);
                setting_row(ui, "Tray icon", "Show a tray menu while the launcher runs", |ui| {
                    dirty |= toggle(ui, &mut s.enable_tray_menu).changed();
                });
                r.gap(ui);
                setting_row(ui, "Data folder", "Where instances, mods and logs are stored", |ui| {
                    if secondary_icon(ui, super::icons::Icon::Folder, "Open").clicked() {
                        open_data = true;
                    }
                });
            });
            self.services_settings_ui(ui);
            ui.add_space(16.0);
        });
        if open_data {
            self.open_folder(&self.dirs.base.clone());
        }
        if theme_changed {
            theme::set_dark(theme::theme_is_dark(&self.settings.theme));
            theme::apply(ui.ctx(), self.settings.light_mode.unwrap_or_else(super::low_end_pc));
        }
        if dirty {
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
}
