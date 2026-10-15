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

impl App {
    /// Read the servers folder again.
    pub(super) fn reload_servers(&mut self) {
        self.svc.servers = servers::load_all(&self.dirs);
        self.svc.servers_loaded = true;
    }

    pub(super) fn servers_ui(&mut self, ui: &mut Ui) {
        if !self.svc.servers_loaded {
            self.reload_servers();
        }
        let mut act: Option<Act> = None;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                heading(ui, "Servers");
                muted(ui, "Servers installed from modpacks. They run in their own window.");
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if secondary(ui, "Refresh").clicked() {
                    act = Some(Act::Reload);
                }
                if secondary_icon(ui, super::icons::Icon::Folder, "Open servers folder").clicked() {
                    let _ = std::fs::create_dir_all(servers::servers_dir(&self.dirs));
                    let _ = open::that(servers::servers_dir(&self.dirs));
                }
            });
        });
        ui.add_space(8.0);
        let account = self.current_account().cloned();
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if self.svc.servers.is_empty() {
                card(ui, |ui| {
                    ui.label(RichText::new("No servers yet").size(18.0).strong());
                    muted(ui, "Servers are installed from a modpack's server files. Ones the Java edition installed show up here too.");
                });
            }
            for s in &self.svc.servers {
                let collapsed = account.as_ref().is_some_and(|a| servers::is_collapsed(a, s));
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        avatar(ui, &s.name, 44.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&s.name).strong().size(16.0));
                            ui.horizontal(|ui| {
                                match s.loader_label() {
                                    Some(l) => tag(ui, &l, widgets_loader(&l)),
                                    None => tag(ui, "Vanilla", c_muted()),
                                }
                                tag(ui, &format!("Minecraft {}", s.minecraft_version), c_muted());
                                if !s.is_patched_for_log4_shell {
                                    tag(ui, "Not patched for Log4Shell", c_danger());
                                }
                                if s.script().is_none() {
                                    tag(ui, "No start script", c_warn());
                                }
                            });
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if secondary(ui, if collapsed { "Show" } else { "Hide" }).clicked() {
                                act = Some(Act::Collapse(s.name.clone(), !collapsed));
                            }
                        });
                    });
                    if !collapsed {
                        ui.add_space(2.0);
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
                        if !details.is_empty() {
                            small_muted(ui, details.join("  \u{00B7}  "));
                        }
                        small_muted(ui, s.root.display().to_string());
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if primary_enabled(ui, "Launch", s.script().is_some()).clicked() {
                                act = Some(Act::Launch(s.name.clone()));
                            }
                            if secondary_icon(ui, super::icons::Icon::Folder, "Open folder").clicked() {
                                act = Some(Act::Folder(s.name.clone()));
                            }
                            if secondary(ui, "Back up").clicked() {
                                act = Some(Act::Backup(s.name.clone()));
                            }
                            if danger(ui, "Delete").clicked() {
                                act = Some(Act::Remove(s.name.clone()));
                            }
                        });
                    }
                });
                ui.add_space(4.0);
            }
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
            Act::Folder(name) => {
                if let Some(s) = self.server(&name) {
                    let _ = open::that(&s.root);
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
                if let Some(s) = self.server(&name) {
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
            Ok(_) => self.toast("The server has been launched."),
            Err(e) => self.toast_err(format!("{e:#}")),
        }
    }

    pub(super) fn servers_overlays(&mut self, ctx: &egui::Context) {
        if let Some(name) = self.svc.server_confirm_launch.clone() {
            let body = "This server is currently vulnerable to the Log4Shell exploit. For your safety, and of those on your server, please delete and recreate this server. Do you still want to launch the server (not recommended)?";
            match confirm_modal(ctx, "server-launch", "Server Is Vulnerable", body, "Launch anyway", true) {
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
                    if let Some(s) = self.server(&name) {
                        match servers::remove(&s) {
                            Ok(()) => self.toast(format!("Deleted {name}")),
                            Err(e) => self.toast_err(format!("{e:#}")),
                        }
                        self.reload_servers();
                    }
                }
                Some(false) => self.svc.server_confirm_remove = None,
                None => {}
            }
        }
    }
}

/// The colour of a loader's tag, taken from its name (`Fabric 0.18.4` -> Fabric).
fn widgets_loader(label: &str) -> egui::Color32 {
    loader_color(label.split_whitespace().next().unwrap_or(""))
}
