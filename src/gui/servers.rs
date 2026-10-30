//! The Servers page: pack servers installed on this computer. Start one in a terminal window, open its folder,
//! back it up or delete it.

use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Sense, Ui, vec2};

use super::App;
use super::icons::Icon;
use super::services::confirm_modal;
use super::theme::*;
use super::widgets::*;
use crate::servers::{self, Server};
use crate::svcsettings::ServiceSettings;

/// The widest the list gets.
const COLUMN: f32 = 860.0;

enum Act {
    Launch(String),
    Folder(String),
    Backup(String),
    Remove(String),
    /// Open the window of what to do to install a broken server again.
    Repair(String),
    Collapse(String, bool),
    Reload,
}

/// What a server's card shows that needs the disk: worked out when the list is read.
pub struct ServerRow {
    /// One of the usual start scripts is in the folder.
    has_script: bool,
    /// The line under the name: loader, Minecraft version, pack, number of mods, description.
    details: String,
    folder: String,
}

impl ServerRow {
    fn of(s: &Server) -> ServerRow {
        let mut details = vec![s.loader_label().unwrap_or_else(|| "Vanilla".to_string())];
        details.push(format!("Minecraft {}", s.minecraft_version));
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
            details: details.join(" \u{00B7} "),
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
        // one width for the head and the list, so their right edges line up; at most as wide as the list gets, but
        // never wider than the window (the scroll bar floats over the edge)
        let width = (ui.available_width() - 14.0).min(COLUMN);
        ui.scope(|ui| {
            ui.set_max_width(width);
            let mut head =
                PageHeader::new("Servers").subtitle("Servers installed from modpacks. They run in their own window.");
            if !self.svc.servers.is_empty() {
                head = head.count(self.svc.servers.len());
            }
            head.show(ui, |ui| {
                if secondary_icon(ui, Icon::Folder, "Open servers folder").clicked() {
                    act = Some(Act::Folder(String::new()));
                }
                if ghost(ui, "Refresh").clicked() {
                    act = Some(Act::Reload);
                }
            });
        });
        let account = self.current_account().cloned();
        let busy: Vec<bool> = self.svc.servers.iter().map(|s| self.server_busy(&s.name)).collect();
        // the servers in three groups: the ones that can be started, the ones with something wrong, the hidden ones
        let mut groups: [Vec<usize>; 3] = Default::default();
        for (k, (s, row)) in self.svc.servers.iter().zip(&self.svc.server_rows).enumerate() {
            let hidden = account.as_ref().is_some_and(|a| servers::is_collapsed(a, s));
            groups[server_group(hidden, s.is_patched_for_log4_shell, row.has_script)].push(k);
        }
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(width);
            if self.svc.servers.is_empty() {
                card(ui, |ui| {
                    ui.add_space(18.0);
                    ui.vertical_centered(|ui| {
                        badge(ui, Icon::Server);
                        ui.add_space(8.0);
                        ui.label(RichText::new("No servers yet").size(FS_SECTION).semibold());
                        ui.scope(|ui| {
                            ui.set_max_width(420.0);
                            ui.add(egui::Label::new(RichText::new("Servers are installed from a modpack's server files. Ones the Java edition installed show up here too.").color(c_muted())).wrap());
                        });
                    });
                    ui.add_space(18.0);
                });
            }
            let titles = [
                ("Ready", "Launch one to start it in a terminal window of its own"),
                ("Need attention", "Fix these before anyone plays on them"),
                ("Hidden", "Put away for this account. Show one to use it again"),
            ];
            let mut first = true;
            for g in GROUP_ORDER {
                let members = &groups[g];
                if members.is_empty() {
                    continue;
                }
                if !std::mem::take(&mut first) {
                    ui.add_space(18.0);
                }
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new(titles[g].0).size(FS_SECTION).semibold());
                        ui.label(
                            RichText::new(members.len().to_string()).font(super::fonts::mono(FS_META)).color(c_faint()),
                        );
                    });
                    ui.label(RichText::new(titles[g].1).size(FS_META).color(c_muted()));
                });
                ui.add_space(8.0);
                card(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (n, &k) in members.iter().enumerate() {
                        if n > 0 {
                            ui.add_space(10.0);
                            rule(ui);
                            ui.add_space(10.0);
                        }
                        let s = &self.svc.servers[k];
                        let row = &self.svc.server_rows[k];
                        if let Some(a) = server_row(ui, s, row, g == 2, busy[k], account.is_some()) {
                            act = Some(a);
                        }
                    }
                });
            }
            ui.add_space(16.0);
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
            Act::Repair(name) => self.svc.server_repair = Some(name),
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
        self.server_repair_ui(ctx);
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

impl App {
    /// "Reinstall", for a server with something wrong with it: BlockForge doesn't install servers itself, so the
    /// window says what to do, in order, with a button for each step. Nothing is deleted from here without the
    /// usual question: Delete opens it.
    fn server_repair_ui(&mut self, ctx: &egui::Context) {
        let Some(name) = self.svc.server_repair.clone() else { return };
        let Some(s) = self.server(&name) else {
            self.svc.server_repair = None;
            return;
        };
        let busy = self.server_busy(&name);
        let mut act = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("server-repair")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(super::services::modal_width(ctx, 500.0));
            if super::discover::dialog_title(ui, &format!("Reinstall \u{201C}{name}\u{201D}")) {
                close = true;
            }
            let pack = if s.pack.is_empty() || s.pack == "Minecraft" {
                format!("Minecraft {}", s.minecraft_version)
            } else {
                format!("{} {}", s.pack, s.version)
            };
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "A fresh copy of {pack} replaces this one. BlockForge doesn't install servers, so it is done \
                         in three steps, and this copy stays until you delete it yourself."
                    ))
                    .color(c_muted()),
                )
                .wrap(),
            );
            ui.add_space(14.0);
            card(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let steps: [(&str, String); 3] = [
                    ("Back up the world", "Keeps the world, the mods and the settings in a backup".to_string()),
                    (
                        "Get the server files again",
                        format!(
                            "Download the server files of {pack} from the pack's page or the launcher that installed \
                             it, into a new folder of the servers folder"
                        ),
                    ),
                    ("Delete this copy", "Once the new one runs. You are asked before anything is removed".to_string()),
                ];
                for (n, (title, hint)) in steps.iter().enumerate() {
                    if n > 0 {
                        ui.add_space(10.0);
                        rule(ui);
                        ui.add_space(10.0);
                    }
                    super::settings::option_row(ui, &format!("{}. {title}", n + 1), hint, 180.0, None, |ui| {
                        match n {
                            0 if busy => {
                                spinner_sized(ui, 18.0);
                            }
                            0 => {
                                if secondary(ui, "Back up").clicked() {
                                    act = Some(Act::Backup(name.clone()));
                                }
                            }
                            1 => {
                                if secondary_icon(ui, Icon::Folder, "Open servers folder").clicked() {
                                    act = Some(Act::Folder(String::new()));
                                }
                            }
                            _ => {
                                if danger_enabled(ui, "Delete\u{2026}", !busy).clicked() {
                                    act = Some(Act::Remove(name.clone()));
                                }
                            }
                        }
                    });
                }
            });
            ui.add_space(16.0);
            super::discover::dialog_footer(
                ui,
                "server-repair",
                |_| {},
                |ui| {
                    if secondary(ui, "Close").clicked() {
                        close = true;
                    }
                },
            );
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(a) = act {
            // the delete question goes on top of this window's place
            if matches!(a, Act::Remove(_)) {
                close = true;
            }
            self.server_action(a);
        }
        if close {
            self.svc.server_repair = None;
        }
    }

    /// For the screenshot tour: the Servers page with the Reinstall window of the first server that needs attention.
    pub(super) fn shot_server_repair(&mut self) {
        self.page = super::Page::Servers;
        self.reload_servers();
        self.svc.server_repair = self
            .svc
            .servers
            .iter()
            .zip(&self.svc.server_rows)
            .find(|(s, row)| server_group(false, s.is_patched_for_log4_shell, row.has_script) == 1)
            .map(|(s, _)| s.name.clone());
    }
}

/// The order the groups are listed in: what needs attention first, then the servers that are ready, then the hidden
/// ones.
const GROUP_ORDER: [usize; 3] = [1, 0, 2];

/// Which group a server is listed in: 0 ready, 1 something wrong with it, 2 hidden by the player.
fn server_group(hidden: bool, patched: bool, has_script: bool) -> usize {
    if hidden {
        2
    } else if !patched || !has_script {
        1
    } else {
        0
    }
}

/// One server in its group's card: its picture, name and facts, what is wrong with it and what to do, and at the
/// right Launch and the menu (Show for a hidden one). Returns what was asked for.
fn server_row(ui: &mut Ui, s: &Server, row: &ServerRow, hidden: bool, busy: bool, has_account: bool) -> Option<Act> {
    let mut act = None;
    let id = ui.scope_id().with(("server-buttons", &s.name));
    let buttons_w = remembered_width(ui, id, 160.0);
    let mut used = buttons_w;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        avatar(ui, &s.name, if hidden { 28.0 } else { 36.0 });
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - buttons_w - 16.0).max(120.0));
            ui.spacing_mut().item_spacing.y = 3.0;
            let name = RichText::new(&s.name).font(super::fonts::semibold(FS_BODY));
            if hidden {
                ui.add_space(5.0);
                ui.add(egui::Label::new(name.color(c_muted())).truncate());
                return;
            }
            ui.add(egui::Label::new(name).truncate());
            let tip = format!("{}\n{}", row.details, row.folder);
            ui.add(
                egui::Label::new(RichText::new(&row.details).font(super::fonts::mono(FS_META)).color(c_muted()))
                    .truncate(),
            )
            .on_hover_text(&tip);
            ui.add(
                egui::Label::new(RichText::new(&row.folder).font(super::fonts::mono(FS_META)).color(c_faint()))
                    .truncate(),
            )
            .on_hover_text(&tip);
            // red and amber are for what is actually wrong, each with what to do about it
            if !s.is_patched_for_log4_shell {
                ui.add_space(2.0);
                status_line(
                    ui,
                    Status::Error,
                    "Not patched for Log4Shell. Delete it and install it again before anyone joins.",
                );
            }
            if !row.has_script {
                ui.add_space(2.0);
                status_line(
                    ui,
                    Status::Warn,
                    "No start script, so there is nothing to launch. Open its folder to add one, or install it again.",
                );
            }
            // the ways out, right under what is wrong
            if (!s.is_patched_for_log4_shell || !row.has_script) && !busy {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                    if secondary_icon(ui, Icon::Folder, "Open folder").clicked() {
                        act = Some(Act::Folder(s.name.clone()));
                    }
                    if secondary(ui, "Reinstall\u{2026}")
                        .on_hover_text("What to do to install it again. Nothing is deleted without asking")
                        .clicked()
                    {
                        act = Some(Act::Repair(s.name.clone()));
                    }
                });
            }
        });
        used = ui
            .with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if hidden {
                    // folding a server away is remembered per account, so it needs one
                    if secondary(ui, "Show").clicked() {
                        act = Some(Act::Collapse(s.name.clone(), false));
                    }
                } else if busy {
                    ui.horizontal(|ui| {
                        spinner_sized(ui, 22.0);
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
                        if has_account && ui.button("Hide").clicked() {
                            act = Some(Act::Collapse(s.name.clone(), true));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(RichText::new("Delete").color(c_danger())).clicked() {
                            act = Some(Act::Remove(s.name.clone()));
                            ui.close();
                        }
                    });
                    // without a start script there is nothing to launch: its row has the ways to fix that instead
                    if row.has_script && secondary_icon(ui, Icon::Play, "Launch").clicked() {
                        act = Some(Act::Launch(s.name.clone()));
                    }
                }
                ui.min_rect().width()
            })
            .inner;
    });
    remember_width(ui, id, used);
    act
}

/// A picture in a soft brand-tinted square, over an empty state.
fn badge(ui: &mut Ui, icon: Icon) {
    let (rect, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(R_CARD), c_accent_soft());
    super::icons::paint(ui.painter(), icon, rect.center(), 22.0, c_accent());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_listed_where_it_belongs() {
        assert_eq!(server_group(false, true, true), 0, "nothing wrong: ready");
        assert_eq!(server_group(false, false, true), 1, "not patched");
        assert_eq!(server_group(false, true, false), 1, "no start script");
        assert_eq!(server_group(true, false, false), 2, "hidden wins: the player put it away");
    }

    #[test]
    fn the_servers_that_need_attention_are_listed_first() {
        assert_eq!(GROUP_ORDER[0], server_group(false, false, true));
        let mut order = GROUP_ORDER;
        order.sort_unstable();
        assert_eq!(order, [0, 1, 2], "every group is listed once");
    }
}
