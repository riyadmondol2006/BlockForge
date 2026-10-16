//! Settings: the same sections, options, wording and defaults as the Java edition's Settings page. Every change is
//! saved as it is made.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::App;
use super::theme::{self, *};
use super::widgets::*;

/// A titled card of rows, with the small coloured heading the Java edition uses.
fn section(ui: &mut Ui, heading: &str, rows: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    card(ui, |ui| {
        ui.label(RichText::new(heading.to_uppercase()).size(11.0).strong().color(c_accent()));
        rows(ui);
    });
}

/// A setting row for a control wider than [`setting_row`] leaves room for (a drop-down, a text field, a row of
/// chips): the explanation keeps clear of the control, and on a narrow page the control goes under the text.
pub(super) fn wide_row(ui: &mut Ui, title: &str, hint: &str, control_width: f32, control: impl FnOnce(&mut Ui)) {
    let room = ui.available_width();
    if room < control_width + 240.0 {
        ui.label(RichText::new(title).strong());
        if !hint.is_empty() {
            small_muted(ui, hint);
        }
        ui.horizontal_wrapped(control);
        ui.add_space(4.0);
        return;
    }
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(room - control_width - 16.0);
            ui.label(RichText::new(title).strong());
            if !hint.is_empty() {
                small_muted(ui, hint);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

fn memory_text(mb: u32) -> String {
    if mb % 1024 == 0 { format!("{} GB", mb / 1024) } else { format!("{:.1} GB", mb as f32 / 1024.0) }
}

/// The computer's memory (total, free) in MB, read again at most every few seconds rather than on every frame.
pub(super) fn memory_now() -> (u64, u64) {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
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
    } else {
        if r.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, draft));
        } else {
            ui.data_mut(|d| d.remove::<String>(id));
        }
        None
    }
}

/// A text setting that is saved when the field is left or Enter is pressed. Returns the typed text then.
fn text_row(ui: &mut Ui, title: &str, hint: &str, placeholder: &str, value: &str) -> Option<String> {
    let mut committed = None;
    wide_row(ui, title, hint, 300.0, |ui| {
        let width = (ui.available_width() - 8.0).clamp(160.0, 300.0);
        committed = draft_text(ui, title, value, placeholder, [width, 40.0], egui::Margin::symmetric(12, 10));
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
        heading(ui, "Settings");
        muted(ui, "Changes are saved as you make them.");
        let mut dirty = false;
        let mut theme_changed = false;
        let mut open_data = false;
        let (total_mb, free_mb) = memory_now();
        let max_mb = memory_slider_max(total_mb);
        let example = self.automatic_memory_example(total_mb, free_mb);
        let self_gpus = self.gpus.clone();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // at most 780 wide, but never wider than the window (set_max_width alone would widen a narrow page)
            ui.set_max_width(ui.available_width().min(780.0));
            let s = &mut self.settings;
            section(ui, "Appearance", |ui| {
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
                // no Language row: the window is in English only for now, so a choice there would change nothing.
                // `language` stays in the settings file for the Java edition.
            });
            section(ui, "Game", |ui| {
                // memory
                let mut auto = s.automatic_memory != Some(false);
                let hint = if auto {
                    example.clone()
                } else {
                    "Set by hand, for every game without its own setting".to_string()
                };
                wide_row(ui, "Memory", &hint, if auto { 140.0 } else { 400.0 }, |ui| {
                    if toggle(ui, &mut auto).changed() {
                        s.automatic_memory = Some(auto);
                        dirty = true;
                    }
                    ui.label("Automatic");
                    if !auto {
                        ui.label(RichText::new(memory_text(s.maximum_memory)).strong().color(c_accent()));
                        ui.spacing_mut().slider_width = 200.0;
                        let mut mb = s.maximum_memory.clamp(1024, max_mb);
                        if ui.add(egui::Slider::new(&mut mb, 1024..=max_mb).step_by(256.0).show_value(false)).changed()
                        {
                            s.maximum_memory = mb;
                            dirty = true;
                        }
                    }
                });
                setting_row(
                    ui,
                    "Speed up game start (class cache)",
                    "Makes the next start of a game quicker (Minecraft 1.18 and newer)",
                    |ui| {
                        dirty |= toggle(ui, &mut s.class_data_cache).changed();
                    },
                );
                let mut warm = s.extras.get("warmUpGameFiles").and_then(|v| v.as_bool()).unwrap_or(true);
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
                setting_row(
                    ui,
                    "Fast start",
                    "A tiny helper that loads Minecraft's classes ahead of time so games open sooner. Works with every version and mod loader; each instance has its own switch too",
                    |ui| {
                        dirty |= toggle(ui, &mut s.fast_start).changed();
                    },
                );
                wide_row(
                    ui,
                    "Java compiler",
                    "Automatic uses the light one on computers with few cores, for much quicker starts. Full performance can give higher frame rates in long sessions.",
                    300.0,
                    |ui| {
                        for (mode, label) in
                            [("auto", "Automatic"), ("fast", "Light"), ("full", "Full performance")]
                        {
                            if chip(ui, label, s.startup_mode == mode).clicked() && s.startup_mode != mode {
                                s.startup_mode = mode.to_string();
                                dirty = true;
                            }
                        }
                    },
                );
                setting_row(
                    ui,
                    "Pick game settings for this PC on first launch",
                    "New games start with graphics that suit this computer",
                    |ui| {
                        dirty |= toggle(ui, &mut s.auto_game_options).changed();
                    },
                );
                if crate::gpu::supported() {
                    dirty |= graphics_card_row(ui, &mut s.gpu, self_gpus.as_deref());
                }
                // an empty path means automatic. The game only falls back to this Java when it doesn't use the one that
                // comes with Minecraft (an instance's own Java setting), as in the Java edition.
                let custom = s.using_custom_java_path.then(|| s.java_path.clone()).filter(|p| !p.is_empty());
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
                    ui.label(RichText::new(warning).color(c_warn()).size(12.0));
                }
                if let Some(args) =
                    text_row(ui, "Java arguments", "Extra options passed to the Java virtual machine", "", &s.java_parameters)
                {
                    let args = args.trim().to_string();
                    if args != s.java_parameters {
                        s.java_parameters = args;
                        dirty = true;
                    }
                }
                setting_row(
                    ui,
                    "Check for BlockForge updates",
                    "Look on GitHub for a newer version and offer to install it",
                    |ui| {
                        dirty |= toggle(ui, &mut s.check_for_launcher_updates).changed();
                    },
                );
                setting_row(ui, "Keep the launcher open", "Leave BlockForge open while Minecraft is running", |ui| {
                    dirty |= toggle(ui, &mut s.keep_launcher_open).changed();
                });
                setting_row(ui, "Show the console", "Open the log window next to the launcher", |ui| {
                    dirty |= toggle(ui, &mut s.enable_console).changed();
                });
            });
            section(ui, "Privacy", |ui| {
                setting_row(
                    ui,
                    "Share anonymous usage statistics",
                    "Which screens are used and how often. Off unless you turn it on",
                    |ui| {
                        dirty |= toggle(ui, &mut s.enable_analytics).changed();
                    },
                );
                setting_row(
                    ui,
                    "Report pack installs and play time",
                    "Counts for public modpacks, so authors can see how they are used. Off unless you turn it on. The network checker in Tools needs this on",
                    |ui| {
                        dirty |= toggle(ui, &mut s.enable_logs).changed();
                    },
                );
            });
            section(ui, "Launcher", |ui| {
                let low_end = super::low_end_pc();
                let hint = if low_end {
                    "Fewer animations and a smaller picture cache. On by default for this computer"
                } else {
                    "Fewer animations and a smaller picture cache, for computers with 4 GB of memory or less"
                };
                setting_row(ui, "Light mode for slower PCs", hint, |ui| {
                    let mut on = s.light_mode.unwrap_or(low_end);
                    if toggle(ui, &mut on).changed() {
                        s.light_mode = Some(on);
                        theme_changed = true;
                        dirty = true;
                    }
                });
                setting_row(ui, "Tray icon", "Show a tray menu while the launcher runs", |ui| {
                    dirty |= toggle(ui, &mut s.enable_tray_menu).changed();
                });
                setting_row(ui, "Data folder", "Where instances, mods and logs are stored", |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_icon(ui, super::icons::Icon::Folder, "Open").clicked() {
                            open_data = true;
                        }
                    });
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
            self.sync_tray();
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
    fn the_memory_slider_stops_at_half_the_memory() {
        assert_eq!(memory_slider_max(16_000), 7936);
        assert_eq!(memory_slider_max(0), 8192, "unknown memory");
        assert_eq!(memory_slider_max(2_000), 2048, "never less than 2 GB");
        assert_eq!(memory_slider_max(u64::MAX), u32::MAX, "no overflow");
    }
}
