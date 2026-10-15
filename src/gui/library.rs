//! Home, Library and the page for a single instance.

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, vec2};

use super::icons::Icon;
use super::instance_menu::Act;
use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Page, Phase};
use crate::instances::Instance;
use crate::launch::{LaunchOptions, Level};

fn meta_line(i: &Instance) -> String {
    format!("Minecraft {}", i.mc_version())
}

impl App {
    /// One instance as a card. Returns what the player asked for, if anything.
    fn instance_card(&self, ui: &mut Ui, inst: &Instance, width: f32) -> Option<Act> {
        let mut act = None;
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 150.0), Sense::click());
        let hovered = resp.hovered();
        ui.painter().rect(
            rect,
            CornerRadius::same(16),
            if hovered { c_card_hover() } else { c_card() },
            Stroke::new(1.0, if hovered { c_accent().gamma_multiply(0.7) } else { c_border() }),
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink(14.0)).layout(Layout::top_down(Align::Min)));
        child.spacing_mut().item_spacing = vec2(8.0, 6.0);
        child.horizontal(|ui| {
            self.instance_icon(ui, inst, 46.0);
            ui.vertical(|ui| {
                ui.add(egui::Label::new(RichText::new(inst.name()).strong().size(15.5)).truncate());
                small_muted(ui, meta_line(inst));
            });
        });
        child.horizontal(|ui| {
            match inst.loader() {
                Some((kind, ver)) => {
                    tag(ui, &format!("{kind} {ver}"), loader_color(kind));
                }
                None => {
                    tag(ui, "Vanilla", c_muted());
                }
            }
            if inst.mod_count > 0 {
                tag(ui, &format!("{} mods", inst.mod_count), c_muted());
            }
        });
        child.add_space(2.0);
        child.horizontal(|ui| {
            let folder = inst.folder.clone();
            if self.is_playing(&folder) {
                if danger_icon(ui, Icon::Stop, "Stop").clicked() {
                    act = Some(Act::Stop(folder.clone()));
                }
            } else if primary_icon(ui, Icon::Play, "Play").clicked() {
                act = Some(Act::Play(folder.clone()));
            }
            ui.menu_button("\u{00B7}\u{00B7}\u{00B7}", |ui| {
                if let Some(a) = self.instance_menu(ui, inst) {
                    act = Some(a);
                }
            });
        });
        if act.is_none() && resp.clicked() {
            act = Some(Act::Open(inst.folder.clone()));
        }
        act
    }

    pub(super) fn home_ui(&mut self, ui: &mut Ui) {
        let name = self.current_account().map(|a| a.name().to_string());
        let hero = self.instances.iter().find(|i| i.last_played() > 0).or(self.instances.first()).cloned();
        let mut action = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            match &name {
                Some(n) => heading(ui, &format!("Welcome back, {n}")),
                None => heading(ui, "Welcome to BlockForge"),
            }
            muted(ui, "Pick up where you left off, or start something new.");
            ui.add_space(14.0);

            match &hero {
                Some(h) => {
                    egui::Frame::new()
                        .fill(c_accent_soft())
                        .stroke(Stroke::new(1.0, c_accent().gamma_multiply(0.35)))
                        .corner_radius(CornerRadius::same(18))
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                self.instance_icon(ui, h, 76.0);
                                ui.vertical(|ui| {
                                    ui.add_space(4.0);
                                    small_muted(
                                        ui,
                                        if h.last_played() > 0 { "CONTINUE PLAYING" } else { "READY TO PLAY" },
                                    );
                                    ui.label(RichText::new(h.name()).size(24.0).strong());
                                    let loader =
                                        h.loader().map(|(k, v)| format!("{k} {v}")).unwrap_or_else(|| "Vanilla".into());
                                    muted(
                                        ui,
                                        format!(
                                            "Minecraft {}  \u{00B7}  {}  \u{00B7}  {}",
                                            h.mc_version(),
                                            loader,
                                            time_ago(h.last_played())
                                        ),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if self.is_playing(&h.folder) {
                                        if danger_big(ui, Icon::Stop, "Stop").clicked() {
                                            action = Some(Act::Stop(h.folder.clone()));
                                        }
                                    } else if primary_big(ui, Icon::Play, "Play").clicked() {
                                        action = Some(Act::Play(h.folder.clone()));
                                    }
                                    if secondary(ui, "Details").clicked() {
                                        action = Some(Act::Open(h.folder.clone()));
                                    }
                                });
                            });
                        });
                }
                None => {
                    card(ui, |ui| {
                        ui.label(RichText::new("No instances yet").size(18.0).strong());
                        muted(ui, "An instance is one copy of Minecraft with its own worlds and mods.");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if primary(ui, "Create your first instance").clicked() {
                                self.page = Page::Create;
                            }
                            if secondary(ui, "Browse modpacks").clicked() {
                                self.page = Page::Discover;
                                self.discover.kind = crate::modrinth::Kind::Modpack;
                                self.discover_search(true);
                            }
                        });
                    });
                }
            }

            if self.instances.len() > 1 {
                ui.add_space(12.0);
                section(ui, "Your instances");
                let width = 262.0;
                let per_row = ((ui.available_width() + 12.0) / (width + 12.0)).floor().max(1.0) as usize;
                let list: Vec<Instance> = self.instances.iter().take(per_row * 2).cloned().collect();
                for row in list.chunks(per_row) {
                    ui.horizontal(|ui| {
                        for inst in row {
                            if let Some(a) = self.instance_card(ui, inst, width) {
                                action = Some(a);
                            }
                        }
                    });
                }
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if secondary_icon(ui, Icon::Plus, "New instance").clicked() {
                    self.page = Page::Create;
                }
                if secondary_icon(ui, Icon::Search, "Find mods").clicked() {
                    self.page = Page::Discover;
                }
            });
            self.news_section(ui);
        });
        if let Some(a) = action {
            self.apply(a);
        }
    }

    pub(super) fn library_ui(&mut self, ui: &mut Ui) {
        let mut action = None;
        ui.horizontal(|ui| {
            heading(ui, "Library");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_icon(ui, Icon::Plus, "New instance").clicked() {
                    self.page = Page::Create;
                }
                if secondary(ui, "Import").clicked() {
                    self.open_import(None);
                }
                let sorting = self.settings.default_instance_sorting.clone();
                let label = match sorting.as_str() {
                    "BY_LAST_PLAYED" => "By Last Played",
                    "BY_NUMBER_OF_PLAYS" => "By Number of Plays",
                    _ => "By Name",
                };
                egui::ComboBox::from_id_salt("library-sort").selected_text(label).width(170.0).show_ui(ui, |ui| {
                    for (id, text) in [
                        ("BY_NAME", "By Name"),
                        ("BY_LAST_PLAYED", "By Last Played"),
                        ("BY_NUMBER_OF_PLAYS", "By Number of Plays"),
                    ] {
                        if ui.selectable_label(sorting == id, text).clicked() && sorting != id {
                            self.settings.default_instance_sorting = id.to_string();
                            self.save_settings();
                        }
                    }
                });
                let mut s = std::mem::take(&mut self.library_search);
                search_field(ui, &mut s, "Search instances", 240.0);
                self.library_search = s;
            });
        });
        ui.add_space(8.0);
        let q = self.library_search.to_lowercase();
        let mut list: Vec<Instance> =
            self.instances.iter().filter(|i| q.is_empty() || i.name().to_lowercase().contains(&q)).cloned().collect();
        match self.settings.default_instance_sorting.as_str() {
            "BY_LAST_PLAYED" => list.sort_by(|a, b| b.last_played().cmp(&a.last_played())),
            "BY_NUMBER_OF_PLAYS" => {
                list.sort_by(|a, b| b.file.launcher.num_plays.unwrap_or(0).cmp(&a.file.launcher.num_plays.unwrap_or(0)))
            }
            _ => list.sort_by_key(|i| i.name().to_lowercase()),
        }
        if list.is_empty() {
            card(ui, |ui| {
                if self.instances.is_empty() {
                    ui.label(RichText::new("Nothing here yet").size(18.0).strong());
                    muted(ui, "Create an instance, or drop a Modrinth modpack (.mrpack) onto this window.");
                } else {
                    muted(ui, "No instance matches your search.");
                }
            });
            return;
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let width = 262.0;
            let per_row = ((ui.available_width() + 12.0) / (width + 12.0)).floor().max(1.0) as usize;
            for row in list.chunks(per_row) {
                ui.horizontal(|ui| {
                    for inst in row {
                        if let Some(a) = self.instance_card(ui, inst, width) {
                            action = Some(a);
                        }
                    }
                });
            }
        });
        if let Some(a) = action {
            self.apply(a);
        }
    }

    pub(super) fn instance_ui(&mut self, ui: &mut Ui) {
        let Some(folder) = self.selected.clone() else {
            self.page = Page::Library;
            return;
        };
        let Some(inst) = self.instance(&folder).cloned() else {
            self.page = Page::Library;
            return;
        };
        let mut action = None;
        ui.horizontal(|ui| {
            if secondary_icon(ui, Icon::Back, "Library").clicked() {
                self.page = Page::Library;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            self.instance_icon(ui, &inst, 68.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(RichText::new(inst.name()).size(26.0).strong());
                ui.horizontal(|ui| {
                    match inst.loader() {
                        Some((kind, ver)) => tag(ui, &format!("{kind} {ver}"), loader_color(kind)),
                        None => tag(ui, "Vanilla", c_muted()),
                    }
                    tag(ui, &format!("Minecraft {}", inst.mc_version()), c_muted());
                    if inst.mod_count > 0 {
                        tag(ui, &format!("{} mods", inst.mod_count), c_muted());
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.is_playing(&folder) {
                    if danger_big(ui, Icon::Stop, "Stop").clicked() {
                        action = Some(Act::Stop(folder.clone()));
                    }
                } else if primary_big(ui, Icon::Play, "Play").clicked() {
                    action = Some(Act::Play(folder.clone()));
                }
                ui.menu_button("\u{00B7}\u{00B7}\u{00B7}", |ui| {
                    if let Some(a) = self.instance_menu(ui, &inst) {
                        action = Some(a);
                    }
                });
                match self.games.get(&folder).map(|g| g.phase) {
                    Some(Phase::Preparing) => {
                        ui.spinner();
                        muted(ui, "Getting ready");
                    }
                    Some(Phase::Running) => {
                        small_muted(ui, "Running");
                    }
                    _ => {}
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (t, label) in [
                (InstTab::Overview, "Overview"),
                (InstTab::Mods, "Mods"),
                (InstTab::Logs, "Logs"),
                (InstTab::Settings, "Settings"),
            ] {
                if chip(ui, label, self.tab == t).clicked() {
                    self.tab = t;
                    if t == InstTab::Mods {
                        self.mods_cache = None;
                    }
                }
            }
        });
        ui.add_space(6.0);
        match self.tab {
            InstTab::Overview => self.tab_overview(ui, &inst),
            InstTab::Mods => self.mods_tab(ui, &inst),
            InstTab::Logs => self.tab_logs(ui, &inst),
            InstTab::Settings => self.tab_settings(ui, &inst),
        }
        if let Some(a) = action {
            self.apply(a);
        }
    }

    fn tab_overview(&mut self, ui: &mut Ui, inst: &Instance) {
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            card(ui, |ui| {
                let java = inst
                    .file
                    .version
                    .java_version
                    .as_ref()
                    .map(|j| format!("Java {}", j.major_version))
                    .unwrap_or_else(|| "Automatic".into());
                egui::Grid::new("overview").num_columns(2).spacing([28.0, 10.0]).show(ui, |ui| {
                    let row = |ui: &mut Ui, k: &str, v: String| {
                        muted(ui, k);
                        ui.label(v);
                        ui.end_row();
                    };
                    row(ui, "Minecraft", inst.mc_version().to_string());
                    row(
                        ui,
                        "Loader",
                        inst.loader().map(|(k, v)| format!("{k} {v}")).unwrap_or_else(|| "None (vanilla)".into()),
                    );
                    row(ui, "Java", java);
                    row(ui, "Last played", time_ago(inst.last_played()));
                    row(ui, "Times played", inst.file.launcher.num_plays.unwrap_or(0).to_string());
                });
                ui.add_space(2.0);
                muted(ui, "Folder");
                ui.add(egui::Label::new(inst.root.display().to_string()).truncate());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if secondary_icon(ui, Icon::Folder, "Open folder").clicked() {
                        let _ = open::that(&inst.root);
                    }
                    if secondary(ui, "Rename").clicked() {
                        self.rename_target = Some((inst.folder.clone(), inst.name().to_string()));
                    }
                    if secondary(ui, "Open saves").clicked() {
                        let _ = open::that(inst.root.join("saves"));
                    }
                    if secondary(ui, "Export").clicked() {
                        self.open_export(&inst.folder);
                    }
                    if let Some(v) = self.ftb.updates.get(&inst.folder).cloned() {
                        if primary(ui, &format!("Update to {}", v.label())).clicked() {
                            self.ftb_update_instance(&inst.folder);
                        }
                    }
                });
            });
            ui.add_space(6.0);
            card(ui, |ui| {
                ui.label(RichText::new("Join a server on start").strong());
                small_muted(ui, "Enter an address like play.example.com or play.example.com:25566");
                ui.horizontal(|ui| {
                    let mut s = std::mem::take(&mut self.quick_server);
                    search_field(ui, &mut s, "Server address", 300.0);
                    self.quick_server = s;
                    if primary_enabled_icon(
                        ui,
                        Icon::Play,
                        "Play and join",
                        !self.quick_server.trim().is_empty() && !self.is_playing(&inst.folder),
                    )
                    .clicked()
                    {
                        let server = self.quick_server.trim().to_string();
                        self.play(&inst.folder, LaunchOptions { server: Some(server), world: None });
                    }
                });
            });
            ui.add_space(6.0);
            self.backups_card(ui, inst);
        });
    }

    fn tab_logs(&mut self, ui: &mut Ui, inst: &Instance) {
        let folder = inst.folder.clone();
        let Some(game) = self.games.get(&folder) else {
            card(ui, |ui| {
                muted(ui, "The game's output shows up here while it runs. Press Play to start it.");
            });
            return;
        };
        let lines: Vec<(Level, String)> = game.lines.iter().map(|l| (l.level, l.text.clone())).collect();
        let phase = game.phase;
        ui.horizontal(|ui| {
            let status = match phase {
                Phase::Preparing => "Getting ready".to_string(),
                Phase::Running => "Running".to_string(),
                Phase::Exited(Some(0)) => "Closed normally".to_string(),
                Phase::Exited(Some(c)) => format!("Closed with code {c}"),
                Phase::Exited(None) => "Closed".to_string(),
                Phase::Failed => "Couldn't start".to_string(),
            };
            muted(ui, status);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if secondary(ui, "Copy all").clicked() {
                    let text: Vec<&str> = lines.iter().map(|(_, t)| t.as_str()).collect();
                    ui.ctx().copy_text(text.join("\n"));
                    self.toast("Log copied");
                }
                if secondary(ui, "Clear").clicked() {
                    if let Some(g) = self.games.get_mut(&folder) {
                        g.lines.clear();
                    }
                }
                let mut f = std::mem::take(&mut self.log_filter);
                search_field(ui, &mut f, "Filter", 200.0);
                self.log_filter = f;
            });
        });
        let filter = self.log_filter.to_lowercase();
        let shown: Vec<&(Level, String)> =
            lines.iter().filter(|(_, t)| filter.is_empty() || t.to_lowercase().contains(&filter)).collect();
        egui::Frame::new()
            .fill(c_field())
            .stroke(Stroke::new(1.0, c_border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let height = ui.available_height().max(120.0);
                let row_h = 17.0;
                ScrollArea::both().auto_shrink(false).max_height(height).stick_to_bottom(true).show_rows(
                    ui,
                    row_h,
                    shown.len(),
                    |ui, range| {
                        for (level, text) in &shown[range] {
                            let color = match level {
                                Level::Error => c_danger(),
                                Level::Warn => c_warn(),
                                Level::Debug => c_muted(),
                                Level::Info => c_text(),
                            };
                            ui.add(egui::Label::new(RichText::new(text.as_str()).monospace().color(color)).extend());
                        }
                    },
                );
            });
    }

    fn tab_settings(&mut self, ui: &mut Ui, inst: &Instance) {
        self.instance_settings_ui(ui, inst);
    }
}
