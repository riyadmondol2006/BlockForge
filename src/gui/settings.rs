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

fn memory_text(mb: u32) -> String {
    if mb % 1024 == 0 { format!("{} GB", mb / 1024) } else { format!("{:.1} GB", mb as f32 / 1024.0) }
}

/// A text setting that is saved when the field is left or Enter is pressed. Returns true when it was committed.
fn text_row(ui: &mut Ui, title: &str, hint: &str, placeholder: &str, value: &mut String) -> bool {
    let mut committed = false;
    setting_row(ui, title, hint, |ui| {
        let r = ui.add_sized(
            [300.0, 40.0],
            egui::TextEdit::singleline(value)
                .hint_text(placeholder)
                .margin(egui::Margin::symmetric(12, 10))
                .background_color(c_field()),
        );
        committed = r.lost_focus();
    });
    committed
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
    setting_row(ui, "Graphics card", &hint, |ui| {
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

impl App {
    /// The language names offered in the Language drop-down.
    pub fn language_names(&self) -> Vec<String> {
        vec!["English".to_string()]
    }

    pub(super) fn settings_ui(&mut self, ui: &mut Ui) {
        heading(ui, "Settings");
        muted(ui, "Changes are saved as you make them.");
        let mut dirty = false;
        let mut theme_changed = false;
        let mut open_data = false;
        let (total_mb, free_mb) = crate::platform::memory_mb();
        let max_mb = ((if total_mb > 0 { total_mb / 2 / 256 * 256 } else { 8192 }) as u32).max(2048);
        let example = self.automatic_memory_example(total_mb, free_mb);
        let languages = self.language_names();
        let self_gpus = self.gpus.clone();
        let perf = super::performance::SettingsView::new(self, ui.ctx());
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(780.0);
            let s = &mut self.settings;
            section(ui, "Appearance", |ui| {
                setting_row(ui, "Theme", "Light and dark looks for the whole launcher", |ui| {
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
                setting_row(ui, "Language", "", |ui| {
                    egui::ComboBox::from_id_salt("language").selected_text(s.language.clone()).width(300.0).show_ui(
                        ui,
                        |ui| {
                            for l in &languages {
                                if ui.selectable_label(&s.language == l, l).clicked() && &s.language != l {
                                    s.language = l.clone();
                                    dirty = true;
                                }
                            }
                        },
                    );
                });
            });
            section(ui, "Game", |ui| {
                // memory
                let mut auto = s.automatic_memory != Some(false);
                let hint = if auto {
                    example.clone()
                } else {
                    "Set by hand, for every game without its own setting".to_string()
                };
                setting_row(ui, "Memory", &hint, |ui| {
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
                setting_row(
                    ui,
                    "Java compiler",
                    "Automatic uses the light one on computers with few cores, for much quicker starts. Full performance can give higher frame rates in long sessions.",
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
                // an empty path means "use the Java that comes with Minecraft"
                let mut path = if s.using_custom_java_path { s.java_path.clone() } else { String::new() };
                if text_row(
                    ui,
                    "Java path",
                    "Leave empty to use the Java that comes with Minecraft",
                    "Automatic (recommended)",
                    &mut path,
                ) {
                    let path = path.trim().to_string();
                    s.using_custom_java_path = !path.is_empty();
                    if !path.is_empty() {
                        s.java_path = path;
                    }
                    dirty = true;
                }
                let mut args = s.java_parameters.clone();
                if text_row(ui, "Java arguments", "Extra options passed to the Java virtual machine", "", &mut args)
                    && args != s.java_parameters
                {
                    s.java_parameters = args.trim().to_string();
                    dirty = true;
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
                dirty |= perf.rows(ui, s);
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
                    "Counts for public modpacks, so authors can see how they are used. Off unless you turn it on",
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
            let _ = open::that(&self.dirs.base);
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
