//! The Servers page: pack servers installed on this computer. Start one in a terminal window, open its folder,
//! back it up or delete it.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use super::App;
use super::services::confirm_modal;
use super::theme::*;
use super::widgets::*;
use crate::servers::{self, Server};
use crate::svcsettings::ServiceSettings;

enum Act {
    Launch(String),
    Folder(String),
    Backup(String),
    Remove(String),
    Collapse(String, bool),
    Reload,
}

/// What a server's card shows that needs the disk: worked out when the list is read.
pub struct ServerRow {
    /// One of the usual start scripts is in the folder.
    has_script: bool,
    /// The line under the name: pack, number of mods, description.
    details: String,
    folder: String,
}

impl ServerRow {
    fn of(s: &Server) -> ServerRow {
        let mut details = Vec::new();
        if !s.pack.is_empty() && s.pack != "Minecraft" {
            details.push(format!("Pack {} {}", s.pack, s.version));
        }
        let mods = s.mod_names().len();
        if mods > 0 {
            details.push(format!("{mods} mods"));
        }
        if let Some(d) = s.description.as_deref().filter(|d| !d.is_empty()) {
            details.push(d.to_string());
        }
        ServerRow {
            has_script: s.script().is_some(),
            details: details.join("  \u{00B7}  "),
            folder: s.root.display().to_string(),
        }
    }
}

impl App {
    /// Read the servers folder again.
    pub(super) fn reload_servers(&mut self) {
        self.svc.servers = servers::load_all(&self.dirs);
        self.svc.server_rows = self.svc.servers.iter().map(ServerRow::of).collect();
        self.svc.servers_loaded = true;
    }

    /// Whether a server is being deleted or backed up right now (its card waits for that).
    fn server_busy(&self, name: &str) -> bool {
        let (deleting, backing_up) = (format!("Deleting {name}"), format!("Backing up {name}"));
        self.tasks.iter().any(|t| t.label == deleting || t.label == backing_up)
    }

    pub(super) fn servers_ui(&mut self, ui: &mut Ui) {
        if !self.svc.servers_loaded || self.svc.server_rows.len() != self.svc.servers.len() {
            self.reload_servers();
        }
        let mut act: Option<Act> = None;
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(760.0));
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    // leave room for the two buttons on the right, so the line wraps instead of running under them
                    ui.set_max_width((ui.available_width() - 300.0).max(160.0));
                    heading(ui, "Servers");
                    muted(ui, "Servers installed from modpacks. They run in their own window.");
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if secondary_icon(ui, super::icons::Icon::Folder, "Open servers folder").clicked() {
                        act = Some(Act::Folder(String::new()));
                    }
                    if secondary(ui, "Refresh").clicked() {
                        act = Some(Act::Reload);
                    }
                });
            });
        });
        ui.add_space(12.0);
        let account = self.current_account().cloned();
        let busy: Vec<bool> = self.svc.servers.iter().map(|s| self.server_busy(&s.name)).collect();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // at most 760 wide, but never wider than the window (the scroll bar floats over the right edge)
            ui.set_max_width((ui.available_width() - 14.0).min(760.0));
            if self.svc.servers.is_empty() {
                card(ui, |ui| {
                    ui.label(RichText::new("No servers yet").size(15.0).semibold());
                    muted(ui, "Servers are installed from a modpack's server files. Ones the Java edition installed show up here too.");
                });
            }
            for ((s, row), busy) in self.svc.servers.iter().zip(&self.svc.server_rows).zip(busy) {
                let collapsed = account.as_ref().is_some_and(|a| servers::is_collapsed(a, s));
                card(ui, |ui| {
                    // the buttons keep their room on the right (as wide as on the last frame)
                    let id = ui.scope_id().with(("server-buttons", &s.name));
                    let buttons_w = remembered_width(ui, id, 160.0);
                    let mut used = buttons_w;
                    ui.horizontal(|ui| {
                        avatar(ui, &s.name, 40.0);
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - buttons_w - 12.0).max(140.0));
                            server_title(ui, s, row);
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if collapsed {
                                // folding a card away is remembered per account, so it needs one
                                if secondary(ui, "Show").clicked() {
                                    act = Some(Act::Collapse(s.name.clone(), false));
                                }
                            } else if busy {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    muted(ui, "Backing up or deleting\u{2026}");
                                });
                            } else {
                                kebab_menu(ui, |ui| {
                                    if ui.button("Open folder").clicked() {
                                        act = Some(Act::Folder(s.name.clone()));
                                        ui.close();
                                    }
                                    if ui.button("Back up").clicked() {
                                        act = Some(Act::Backup(s.name.clone()));
                                        ui.close();
                                    }
                                    if account.is_some() && ui.button("Hide").clicked() {
                                        act = Some(Act::Collapse(s.name.clone(), true));
                                        ui.close();
                                    }
                                    if ui.button(RichText::new("Delete").color(c_danger())).clicked() {
                                        act = Some(Act::Remove(s.name.clone()));
                                        ui.close();
                                    }
                                });
                                let launch = if row.has_script {
                                    secondary_icon(ui, super::icons::Icon::Play, "Launch")
                                } else {
                                    danger_enabled(ui, "Launch", false)
                                };
                                if launch.clicked() {
                                    act = Some(Act::Launch(s.name.clone()));
                                }
                            }
                            used = ui.min_rect().width();
                        });
                    });
                    remember_width(ui, id, used);
                    if !collapsed {
                        ui.add_space(4.0);
                        if !row.details.is_empty() {
                            small_muted(ui, &row.details);
                        }
                        ui.label(RichText::new(&row.folder).size(12.0).color(c_faint()));
                    }
                });
                ui.add_space(8.0);
            }
            ui.add_space(8.0);
        });
        if let Some(a) = act {
            self.server_action(a);
        }
    }

    fn server(&self, name: &str) -> Option<Server> {
        self.svc.servers.iter().find(|s| s.name == name).cloned()
    }

    fn server_action(&mut self, act: Act) {
        match act {
            Act::Reload => self.reload_servers(),
            // an empty name is the servers folder itself
            Act::Folder(name) if name.is_empty() => self.open_folder(&servers::servers_dir(&self.dirs)),
            Act::Folder(name) => {
                if let Some(s) = self.server(&name) {
                    self.open_folder(&s.root);
                }
            }
            Act::Launch(name) => {
                if let Some(s) = self.server(&name) {
                    // a server that isn't patched against Log4Shell is dangerous: ask first
                    if s.is_patched_for_log4_shell {
                        self.launch_server(&s);
                    } else {
                        self.svc.server_confirm_launch = Some(name);
                    }
                }
            }
            Act::Backup(name) => {
                if let Some(s) = self.server(&name).filter(|_| !self.server_busy(&name)) {
                    let (dirs, svc) = (self.dirs.clone(), ServiceSettings::from_settings(&self.settings));
                    self.start_task(format!("Backing up {}", s.name), move |_p, _n| async move {
                        let made = tokio::task::spawn_blocking(move || {
                            crate::backups::create_server(&dirs, &svc, &s.root, &s.name)
                        })
                        .await??;
                        Ok(format!(
                            "Backup is complete ({})",
                            made.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                        ))
                    });
                }
            }
            Act::Remove(name) => self.svc.server_confirm_remove = Some(name),
            Act::Collapse(name, collapsed) => {
                if let (Some(s), Some(uuid)) = (self.server(&name), self.current_account().map(|a| a.uuid.clone())) {
                    if let Some(a) = self.accounts.iter_mut().find(|a| a.uuid == uuid) {
                        servers::set_collapsed(a, &s, collapsed);
                        self.save_accounts();
                    }
                }
            }
        }
    }

    pub(super) fn launch_server(&mut self, s: &Server) {
        let use_mojang_java =
            self.settings.extras.get("useJavaProvidedByMinecraft").and_then(|v| v.as_bool()).unwrap_or(true);
        match servers::launch(&self.dirs, s, "", use_mojang_java) {
            Ok(_) => self.toast(format!("{} started in a window of its own", s.name)),
            Err(e) => self.toast_err(format!("Couldn't start {}: {e:#}", s.name)),
        }
    }

    pub(super) fn servers_overlays(&mut self, ctx: &egui::Context) {
        if let Some(name) = self.svc.server_confirm_launch.clone() {
            let body = "This server is open to the Log4Shell attack. For your safety, and for the players on your server, delete it and install it again. Start it anyway (not recommended)?";
            match confirm_modal(ctx, "server-launch", "This server isn't safe", body, "Launch anyway", true) {
                Some(true) => {
                    self.svc.server_confirm_launch = None;
                    if let Some(s) = self.server(&name) {
                        self.launch_server(&s);
                    }
                }
                Some(false) => self.svc.server_confirm_launch = None,
                None => {}
            }
        }
        if let Some(name) = self.svc.server_confirm_remove.clone() {
            let body =
                "The server's world, mods and settings will be removed from this computer. This can't be undone.";
            match confirm_modal(ctx, "server-remove", &format!("Delete \"{name}\"?"), body, "Delete", true) {
                Some(true) => {
                    self.svc.server_confirm_remove = None;
                    if let Some(s) = self.server(&name).filter(|_| !self.server_busy(&name)) {
                        // a world can be gigabytes: delete it off the window's thread, then read the list again
                        self.start_task(format!("Deleting {name}"), move |_p, n| async move {
                            let result = tokio::task::spawn_blocking(move || servers::remove(&s)).await?;
                            n.send(super::Msg::Svc(super::services::SvcMsg::ServersChanged));
                            result?;
                            Ok(format!("Deleted {name}"))
                        });
                    }
                }
                Some(false) => self.svc.server_confirm_remove = None,
                None => {}
            }
        }
    }
}

/// A server's name and its tags.
fn server_title(ui: &mut Ui, s: &Server, row: &ServerRow) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.add(egui::Label::new(RichText::new(&s.name).semibold().size(15.0)).wrap());
    ui.horizontal_wrapped(|ui| {
        match s.loader_label() {
            Some(l) => tag(ui, &l, widgets_loader(&l)),
            None => tag(ui, "Vanilla", c_muted()),
        }
        tag(ui, &format!("Minecraft {}", s.minecraft_version), c_muted());
        if !s.is_patched_for_log4_shell {
            tag(ui, "Not patched for Log4Shell", c_danger());
        }
        if !row.has_script {
            tag(ui, "No start script", c_warn());
        }
    });
}

/// The colour of a loader's tag, taken from its name (`Fabric 0.18.4` -> Fabric).
fn widgets_loader(label: &str) -> egui::Color32 {
    loader_color(label.split_whitespace().next().unwrap_or(""))
}
