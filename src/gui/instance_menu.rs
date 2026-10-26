//! The instance menu and the small dialogs behind it (description, play offline as another name, mod loader),
//! shared by the cards, the home page and the instance page.

use std::path::PathBuf;

use egui::{Align, Color32, CornerRadius, Layout, Response, RichText, Sense, Stroke, Ui, UiBuilder, vec2};

use super::theme::*;
use super::widgets::*;
use super::{App, InstTab, Msg, Page};
use crate::instances::{self, Instance};
use crate::loaders::{self, Loader};

/// What the player asked an instance to do.
pub(super) enum Act {
    Open(String),
    Play(String),
    PlayOffline(String),
    /// Play this instance as the account with this id (instance folder, account id).
    PlayAs(String, String),
    AddAccount,
    /// Switch Fast start on or off for an instance (folder, new state).
    FastStart(String, bool),
    /// Change another of an instance's helper switches (see `performance`).
    Helper(String, super::performance::HelperChange),
    Stop(String),
    /// Stop one running session (its key in `games`).
    StopSession(String),
    Folder(String),
    Mods(String),
    AddMods(String),
    Settings(String),
    Rename(String),
    ChangeImage(String),
    ChangeDescription(String),
    Duplicate(String),
    Export(String),
    ChangeLoader(String),
    Repair(String),
    Delete(String),
}

/// What a file chooser was opened for.
#[derive(Clone, Debug)]
pub enum PickPurpose {
    InstanceImage(String),
    JavaFolder,
}

pub struct LoaderDialog {
    folder: String,
    loader: Loader,
    versions: Vec<String>,
    /// Empty means the newest.
    version: String,
    loading: bool,
    requested: Option<(Loader, String)>,
    error: Option<String>,
}

// ---- quiet controls shared by the instance page (header, Overview, Mods, Backups)

/// A text button with no frame until it is pointed at: for the small actions of a row. `color` is the resting colour
/// (muted when `None`) and `hot` the colour while pointed at (the text colour when `None`). With `visible` false
/// nothing is drawn (a row that shows its actions on hover) unless the keyboard is on it, but it can still be
/// reached.
pub(super) fn ghost_button(
    ui: &mut Ui,
    text: &str,
    color: Option<Color32>,
    hot: Option<Color32>,
    visible: bool,
) -> Response {
    let rest = color.unwrap_or_else(c_muted);
    let w = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_BODY), rest).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 20.0, 28.0), Sense::click());
    let pointed = resp.hovered() || resp.is_pointer_button_down_on();
    if ui.is_rect_visible(rect) && (visible || resp.has_focus() || pointed) {
        if pointed {
            ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), c_hover());
        }
        let fg = if pointed { hot.unwrap_or_else(c_text) } else { rest };
        let galley = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(FS_BODY), fg);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The small chevron of a button that opens a menu.
fn chevron_down(ui: &Ui, c: egui::Pos2, color: Color32) {
    let s = Stroke::new(1.5, color);
    ui.painter().line_segment([c + vec2(-3.5, -1.5), c + vec2(0.0, 2.0)], s);
    ui.painter().line_segment([c + vec2(0.0, 2.0), c + vec2(3.5, -1.5)], s);
}

/// The frame of a menu that floats: the raised surface, one line, padding 8.
fn menu_frame(ctx: &egui::Context) -> egui::Frame {
    let style = ctx.global_style();
    egui::Frame::new()
        .fill(c_raised())
        .stroke(Stroke::new(1.0, c_control()))
        .corner_radius(CornerRadius::same(R_CARD))
        .shadow(style.visuals.popup_shadow)
        .inner_margin(egui::Margin::same(8))
}

/// Items of a menu 32 high with a little air between them.
pub(super) fn menu_items(ui: &mut Ui) {
    let s = ui.spacing_mut();
    s.item_spacing.y = 2.0;
    s.button_padding = vec2(10.0, 6.0);
    s.interact_size.y = 32.0;
}

/// Opens `add` as a menu under `resp` when it is clicked, in the look of the app.
/// Closing as `close` says: on any click, or (a menu of switches, which stays open while they are flipped) only on a
/// click outside it.
fn open_menu_closing<R>(resp: &Response, close: egui::PopupCloseBehavior, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    egui::Popup::menu(resp)
        .close_behavior(close)
        .gap(4.0)
        .frame(menu_frame(&resp.ctx))
        .show(|ui| {
            menu_items(ui);
            add(ui)
        })
        .map(|r| r.inner)
}

/// A neutral button with a chevron that opens a menu (the "Play as" of the header). Returns what the menu returned
/// while it is open.
pub(super) fn secondary_menu<R>(ui: &mut Ui, text: &str, add: impl FnOnce(&mut Ui) -> R) -> (Response, Option<R>) {
    secondary_menu_closing(ui, text, egui::PopupCloseBehavior::CloseOnClick, add)
}

/// [`secondary_menu`] for a menu of switches: it stays open while they are flipped and closes on a click outside it.
pub(super) fn secondary_switch_menu<R>(
    ui: &mut Ui,
    text: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<R>) {
    secondary_menu_closing(ui, text, egui::PopupCloseBehavior::CloseOnClickOutside, add)
}

fn secondary_menu_closing<R>(
    ui: &mut Ui,
    text: &str,
    close: egui::PopupCloseBehavior,
    add: impl FnOnce(&mut Ui) -> R,
) -> (Response, Option<R>) {
    let font = super::fonts::medium(FS_BODY);
    let w = ui.painter().layout_no_wrap(text.to_string(), font.clone(), c_text()).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 28.0 + 18.0, 32.0), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if ui.is_rect_visible(rect) {
        let bg = if resp.hovered() || open { c_hover() } else { c_field() };
        ui.painter().rect(
            rect,
            CornerRadius::same(R_CONTROL),
            bg,
            Stroke::new(1.0, c_control()),
            egui::StrokeKind::Inside,
        );
        let galley = ui.painter().layout_no_wrap(text.to_string(), font, c_text());
        let left = rect.left() + 14.0;
        ui.painter().galley(egui::pos2(left, rect.center().y - galley.size().y / 2.0), galley, c_text());
        chevron_down(ui, egui::pos2(rect.right() - 16.0, rect.center().y), c_muted());
    }
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let out = open_menu_closing(&resp, close, add);
    (resp, out)
}

/// One line across the page between two blocks, with the same air on both sides.
pub(super) fn section_break(ui: &mut Ui) {
    ui.add_space(12.0);
    rule(ui);
    ui.add_space(12.0);
}

/// A row with something at each end. The part on the right is laid out left to right in the width it had on the last
/// frame (see [`remember_width`]), so the keyboard (Tab) visits it in the order it is read; the part on the left gets
/// what is left. Call `left`, then `right`, then `finish`.
pub(super) struct SplitRow {
    id: Option<egui::Id>,
    row: egui::Rect,
    right_w: f32,
    used: f32,
}

impl SplitRow {
    /// The right part is as wide as it was on the last frame.
    pub(super) fn new(ui: &mut Ui, id: egui::Id, height: f32) -> SplitRow {
        let right_w = remembered_width(ui, id, 240.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        SplitRow { id: Some(id), row, right_w, used: right_w }
    }

    /// The right part has a width that is known (a column of the same buttons in every row).
    pub(super) fn fixed(ui: &mut Ui, height: f32, right_w: f32) -> SplitRow {
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        SplitRow { id: None, row, right_w, used: right_w }
    }

    /// How wide the right part turned out this frame.
    pub(super) fn used(&self) -> f32 {
        self.used
    }

    /// How wide the left part is.
    pub(super) fn left_width(&self) -> f32 {
        (self.row.width() - self.right_w - 16.0).max(120.0)
    }

    pub(super) fn left<R>(&self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        let rect = egui::Rect::from_min_size(self.row.min, vec2(self.left_width(), self.row.height()));
        let layout = Layout::left_to_right(Align::Center);
        ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout), add).inner
    }

    pub(super) fn right<R>(&mut self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        let rect = egui::Rect::from_min_max(egui::pos2(self.row.right() - self.right_w, self.row.top()), self.row.max);
        let layout = Layout::left_to_right(Align::Center);
        let (out, used) = ui
            .scope_builder(UiBuilder::new().max_rect(rect).layout(layout), |ui| {
                let out = add(ui);
                (out, ui.min_rect().width())
            })
            .inner;
        self.used = used;
        out
    }

    pub(super) fn finish(self, ui: &Ui) {
        if let Some(id) = self.id {
            remember_width(ui, id, self.used);
        }
    }
}

/// A [`SplitRow`] when both parts fit on one line, otherwise the left part on a line and the right part on the next.
/// What the closures share (`state`) is handed to whichever runs, so both can change it.
pub(super) fn split_or_stack<S>(
    ui: &mut Ui,
    id: egui::Id,
    height: f32,
    state: &mut S,
    left: impl FnOnce(&mut Ui, &mut S),
    right: impl FnOnce(&mut Ui, &mut S),
) {
    let left_id = id.with("left");
    let left_w = remembered_width(ui, left_id, 0.0);
    let right_w = remembered_width(ui, id, 240.0);
    if ui.available_width() >= left_w + right_w + 24.0 {
        let mut row = SplitRow::new(ui, id, height);
        let used = row.left(ui, |ui| {
            left(ui, state);
            ui.min_rect().width()
        });
        row.right(ui, |ui| right(ui, state));
        row.finish(ui);
        remember_width(ui, left_id, used);
    } else {
        let used = ui
            .horizontal(|ui| {
                left(ui, state);
                ui.min_rect().width()
            })
            .inner;
        let right_used = ui
            .horizontal(|ui| {
                right(ui, state);
                ui.min_rect().width()
            })
            .inner;
        remember_width(ui, left_id, used);
        remember_width(ui, id, right_used);
    }
}

impl App {
    pub(super) fn apply(&mut self, act: Act) {
        match act {
            Act::Open(f) => self.open_instance(&f),
            Act::Play(f) => self.play_checked(&f, super::PendingPlay::Play),
            Act::PlayOffline(f) => self.offline_play = Some((f, String::new(), false)),
            Act::PlayAs(f, id) => {
                let account = self.accounts.iter().find(|a| a.uuid == id).cloned();
                self.play_checked(&f, super::PendingPlay::As(account));
            }
            Act::AddAccount => self.start_sign_in(),
            Act::FastStart(f, on) => self.set_fast_start(&f, on),
            Act::Helper(f, change) => self.apply_helper_change(&f, change),
            Act::Stop(f) => self.ask_stop(&f, false),
            Act::StopSession(key) => self.ask_stop(&key, true),
            Act::Folder(f) => {
                if let Some(root) = self.instance(&f).map(|i| i.root.clone()) {
                    self.open_path(&root);
                }
            }
            Act::Mods(f) => {
                self.open_instance(&f);
                self.tab = InstTab::Mods;
                self.mods_cache = None;
            }
            Act::AddMods(f) => {
                self.discover.kind = crate::modrinth::Kind::Mod;
                self.discover.target = Some(f);
                self.page = Page::Discover;
                self.discover_search(true);
            }
            Act::Settings(f) => {
                self.open_instance(&f);
                self.tab = InstTab::Settings;
            }
            Act::Rename(f) => {
                if let Some(i) = self.instance(&f) {
                    self.rename_target = Some((f.clone(), i.name().to_string()));
                }
            }
            Act::ChangeImage(f) => self.pick_file(PickPurpose::InstanceImage(f)),
            Act::ChangeDescription(f) => {
                if let Some(i) = self.instance(&f) {
                    self.desc_edit = Some((f.clone(), i.file.launcher.description.clone()));
                }
            }
            Act::Repair(f) => {
                if self.is_playing(&f) {
                    self.toast_err("Close the game first: its files can't be checked while it runs.");
                } else if let Some(inst) = self.instance(&f).cloned() {
                    let dirs = self.dirs.clone();
                    self.start_task(format!("Repairing {}", inst.name()), move |p, _| async move {
                        instances::repair(&dirs, &inst, &p).await?;
                        Ok(format!("Checked {}: anything damaged was downloaded again", inst.name()))
                    });
                }
            }
            Act::Duplicate(f) => {
                if let Some(inst) = self.instance(&f).cloned() {
                    let name = super::create::unique_instance_name(&self.instances, &format!("{} copy", inst.name()));
                    let dirs = self.dirs.clone();
                    self.start_task(format!("Copying {}", inst.name()), move |_, _| async move {
                        tokio::task::spawn_blocking(move || instances::duplicate(&dirs, &inst, &name)).await??;
                        Ok("Instance copied".to_string())
                    });
                }
            }
            Act::ChangeLoader(f) => {
                if let Some(i) = self.instance(&f) {
                    let loader = i.loader().and_then(|(k, _)| Loader::parse(k)).unwrap_or(Loader::Vanilla);
                    self.loader_dialog = Some(LoaderDialog {
                        folder: f.clone(),
                        loader,
                        versions: Vec::new(),
                        version: String::new(),
                        loading: false,
                        requested: None,
                        error: None,
                    });
                    self.loader_dialog_request();
                }
            }
            Act::Export(f) => self.open_export(&f),
            Act::Delete(f) => self.confirm_delete = Some(f),
        }
    }

    /// The "Play as" button next to the big Play button, opening the menu for playing as somebody else. Under a
    /// right-to-left layout it is added after the button so it ends up on its left.
    pub(super) fn play_as_menu(&self, ui: &mut Ui, folder: &str) -> Option<Act> {
        let running =
            self.sessions_of(folder).iter().filter(|k| self.games.get(*k).is_some_and(|g| g.active())).count();
        let label = if running > 0 { format!("Play as ({running} running)") } else { "Play as".to_string() };
        let (resp, act) = secondary_menu(ui, &label, |ui| self.account_menu(ui, folder));
        resp.on_hover_text("Play as another account, or start another game at the same time");
        act.flatten()
    }

    /// The menu for playing as somebody else: every account, a new offline name, a new Microsoft account, and a way to
    /// stop each game of this instance that is running. Playing several at once is just playing again.
    pub(super) fn account_menu(&self, ui: &mut Ui, folder: &str) -> Option<Act> {
        let mut act = None;
        ui.set_min_width(240.0);
        let current = self.current_account().map(|a| a.uuid.clone());
        ui.add_space(2.0);
        if self.accounts.is_empty() {
            small_muted(ui, "No accounts yet");
        } else {
            small_muted(ui, "Play as");
        }
        for a in &self.accounts {
            let mark = if current.as_deref() == Some(a.uuid.as_str()) { "  \u{2713}" } else { "" };
            if ui.button(format!("{}  \u{00B7}  {}{mark}", a.name(), a.kind())).clicked() {
                act = Some(Act::PlayAs(folder.to_string(), a.uuid.clone()));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Play offline with a new name\u{2026}").clicked() {
            act = Some(Act::PlayOffline(folder.to_string()));
            ui.close();
        }
        if ui.button("Add a Microsoft account\u{2026}").clicked() {
            act = Some(Act::AddAccount);
            ui.close();
        }
        let running: Vec<(String, String)> = self
            .sessions_of(folder)
            .into_iter()
            .filter_map(|k| self.games.get(&k).filter(|g| g.active()).map(|g| (k, g.player.clone())))
            .collect();
        if !running.is_empty() {
            ui.separator();
            small_muted(ui, "Running now");
            for (key, player) in running {
                if ui.button(RichText::new(format!("Stop {player}")).color(c_danger())).clicked() {
                    act = Some(Act::StopSession(key));
                    ui.close();
                }
            }
        }
        act
    }

    /// The items of the "more" menu, grouped by what they are about. Returns what was clicked.
    pub(super) fn instance_menu(&self, ui: &mut Ui, inst: &Instance) -> Option<Act> {
        let f = inst.folder.clone();
        let mut act = None;
        ui.set_min_width(220.0);
        ui.menu_button("Play as\u{2026}", |ui| {
            if let Some(a) = self.account_menu(ui, &f) {
                act = Some(a);
            }
        });
        ui.separator();
        let mut item = |ui: &mut Ui, label: &str, make: fn(String) -> Act| {
            if ui.button(label).clicked() {
                act = Some(make(f.clone()));
                ui.close();
            }
        };
        item(ui, "Open folder", Act::Folder);
        item(ui, "Rename", Act::Rename);
        item(ui, "Change image", Act::ChangeImage);
        item(ui, "Change description", Act::ChangeDescription);
        item(ui, "Clone", Act::Duplicate);
        item(ui, "Export", Act::Export);
        ui.separator();
        item(ui, "Mod loader", Act::ChangeLoader);
        item(ui, "Add mods", Act::AddMods);
        item(ui, "Edit mods", Act::Mods);
        let helpers = self.helpers_menu(ui, inst);
        item(ui, "Settings", Act::Settings);
        item(ui, "Repair game files", Act::Repair);
        ui.separator();
        let delete = ui.button(RichText::new("Delete instance").color(c_danger())).clicked();
        if delete {
            ui.close();
        }
        helpers.or(act).or(delete.then(|| Act::Delete(inst.folder.clone())))
    }

    /// Open a file (or folder) chooser without freezing the window.
    pub fn pick_file(&mut self, purpose: PickPurpose) {
        let n = self.notifier();
        self.rt.spawn_blocking(move || {
            let dialog = rfd::FileDialog::new();
            let path = match &purpose {
                PickPurpose::InstanceImage(_) => {
                    dialog.add_filter("Images", &["png", "jpg", "jpeg", "webp"]).pick_file()
                }
                PickPurpose::JavaFolder => dialog.pick_folder(),
            };
            n.send(Msg::Picked(purpose, path));
        });
    }

    pub(super) fn picked(&mut self, purpose: PickPurpose, path: Option<PathBuf>) {
        let Some(path) = path else { return };
        match purpose {
            PickPurpose::InstanceImage(folder) => {
                let Some(inst) = self.instance(&folder) else { return };
                let target = inst.root.join("instance.png");
                // a photo of several megabytes takes a moment to read and shrink: not on the window's thread
                let n = self.notifier();
                self.rt.spawn_blocking(move || {
                    let result = image::open(&path)
                        .and_then(|img| {
                            img.resize(256, 256, image::imageops::FilterType::Lanczos3)
                                .save_with_format(&target, image::ImageFormat::Png)
                        })
                        .map_err(|e| e.to_string());
                    n.send(Msg::InstanceImage(folder, result));
                });
            }
            PickPurpose::JavaFolder => {
                self.settings.java_path = path.to_string_lossy().to_string();
                self.settings.using_custom_java_path = true;
                self.save_settings();
            }
        }
    }

    pub(super) fn instance_image_saved(&mut self, folder: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                // shown at once, without waiting for the next look at the file
                if let Some(root) = self.instance(folder).map(|i| i.root.clone()) {
                    self.local_pictures.forget(&root.join("instance.png"));
                }
                self.toast("Image changed");
            }
            Err(e) => self.toast_err(format!("Couldn't use that picture: {e}")),
        }
    }

    fn loader_dialog_request(&mut self) {
        let Some(d) = self.loader_dialog.as_mut() else { return };
        d.version.clear();
        if d.loader == Loader::Vanilla {
            d.versions.clear();
            d.requested = None;
            d.loading = false;
            return;
        }
        let Some(mc) = self.instances.iter().find(|i| i.folder == d.folder).map(|i| i.mc_version().to_string()) else {
            return;
        };
        let key = (d.loader, mc);
        if d.requested.as_ref() == Some(&key) {
            return;
        }
        d.requested = Some(key.clone());
        d.loading = true;
        d.versions.clear();
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            let r = loaders::versions(&dirs, key.0, &key.1).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::DialogLoaderVersions { loader: key.0, mc: key.1, result: r });
        });
    }

    pub(super) fn dialog_loader_versions(&mut self, loader: Loader, mc: &str, result: Result<Vec<String>, String>) {
        let Some(d) = self.loader_dialog.as_mut() else { return };
        if d.requested.as_ref() != Some(&(loader, mc.to_string())) {
            return;
        }
        d.loading = false;
        match result {
            Ok(v) => {
                d.versions = v;
                d.error = None;
            }
            Err(e) => d.error = Some(e),
        }
    }

    /// The three small dialogs of the menu.
    pub(super) fn instance_dialogs_ui(&mut self, ctx: &egui::Context) {
        // description
        if let Some((folder, mut text)) = self.desc_edit.clone() {
            let (mut save, mut close) = (false, false);
            let modal = egui::Modal::new(egui::Id::unique("desc")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("Change description").size(FS_SECTION).semibold());
                ui.add_space(6.0);
                let r = ui.add_sized(
                    [ui.available_width(), 90.0],
                    egui::TextEdit::multiline(&mut text)
                        .margin(egui::Margin::symmetric(12, 10))
                        .background_color(c_field()),
                );
                // typing can start at once
                focus_when_shown(ui, &r);
                small_muted(ui, "It shows on the instance's Overview.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Save").clicked() {
                        save = true;
                    }
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            self.desc_edit = Some((folder.clone(), text.clone()));
            close |= modal.should_close();
            if save {
                if let Some(idx) = self.instances.iter().position(|i| i.folder == folder) {
                    self.instances[idx].file.launcher.description = text;
                    if let Err(e) = self.instances[idx].save() {
                        self.toast_err(format!("{e:#}"));
                    }
                }
                close = true;
            }
            if close {
                self.desc_edit = None;
            }
        }

        // play offline with another name
        if let Some((folder, mut name, mut bad)) = self.offline_play.clone() {
            let (mut go, mut close) = (false, false);
            let modal = egui::Modal::new(egui::Id::unique("offline")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Play offline with another name").size(FS_SECTION).semibold());
                muted(ui, "3\u{2013}16 letters, numbers or underscores. No account needed.");
                ui.add_space(6.0);
                let r = ui.add_sized(
                    [ui.available_width(), 38.0],
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("Player name")
                        .margin(egui::Margin::symmetric(12, 9))
                        .background_color(c_field()),
                );
                focus_when_shown(ui, &r);
                if bad {
                    if let Some(problem) = super::whoplays::name_problem(name.trim()) {
                        ui.label(RichText::new(problem).color(c_danger()));
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if play_button(ui, false).clicked()
                        || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        go = true;
                    }
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            close |= modal.should_close();
            if go {
                if super::whoplays::valid_player_name(name.trim()) {
                    let acc = crate::accounts::Account::offline(name.trim());
                    // keep the name, so it is in the list of accounts next time
                    if !self.accounts.iter().any(|a| a.uuid == acc.uuid) {
                        self.accounts.push(acc.clone());
                        self.save_accounts();
                    }
                    self.play_checked(&folder, super::PendingPlay::As(Some(acc)));
                    close = true;
                } else {
                    bad = true;
                }
            }
            self.offline_play = if close { None } else { Some((folder, name, bad)) };
        }

        // the game closed soon after it started with a helper on
        self.helpers_blame_ui(ctx);

        // mod loader
        if self.loader_dialog.is_some() {
            let (mut apply, mut close, mut changed) = (false, false, false);
            let mut d = self.loader_dialog.take().unwrap();
            let current = self.instance(&d.folder).and_then(|i| i.loader().map(|(k, v)| format!("{k} {v}")));
            let mods_in = self.instance(&d.folder).map_or(0, |i| i.mod_count);
            let playing = self.is_playing(&d.folder);
            let modal = egui::Modal::new(egui::Id::unique("loader")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("Mod loader").size(FS_SECTION).semibold());
                muted(ui, format!("Now: {}", current.clone().unwrap_or_else(|| "None (vanilla)".into())));
                ui.add_space(6.0);
                let labels: Vec<&str> =
                    Loader::ALL.iter().map(|l| if *l == Loader::Vanilla { "None" } else { l.label() }).collect();
                let chosen = Loader::ALL.iter().position(|l| *l == d.loader).unwrap_or(0);
                if let Some(i) = segmented(ui, &labels, chosen) {
                    if Loader::ALL[i] != d.loader {
                        d.loader = Loader::ALL[i];
                        changed = true;
                    }
                }
                if d.loader != Loader::Vanilla && d.error.is_some() && !d.loading {
                    // under the choices, where a long message can wrap, and with a way to ask again
                    let e = d.error.clone().unwrap_or_default();
                    if notice(ui, NoticeKind::Error, &format!("Couldn't get the {} versions: {e}", d.loader.label()), Some("Try again"))
                        .is_some_and(|r| r.clicked())
                    {
                        d.requested = None;
                        d.error = None;
                        changed = true;
                    }
                } else if d.loader != Loader::Vanilla {
                    ui.horizontal(|ui| {
                        muted(ui, "Version");
                        if d.loading {
                            ui.add(egui::Spinner::new().size(14.0));
                        } else if d.versions.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "{} isn't available for this Minecraft version",
                                    d.loader.label()
                                ))
                                .color(c_warn()),
                            );
                        } else {
                            let newest = d.versions[0].clone();
                            let shown =
                                if d.version.is_empty() { format!("Latest ({newest})") } else { d.version.clone() };
                            egui::ComboBox::from_id_salt("dlg-loader-version")
                                .selected_text(shown)
                                .width(220.0)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(d.version.is_empty(), format!("Latest ({newest})")).clicked()
                                    {
                                        d.version.clear();
                                    }
                                    for v in d.versions.iter().take(60) {
                                        if ui.selectable_label(&d.version == v, v).clicked() {
                                            d.version = v.clone();
                                        }
                                    }
                                });
                        }
                    });
                }
                // mods made for one loader don't load in another
                let loader_now = current.as_deref().map(|c| c.split(' ').next().unwrap_or(c).to_lowercase());
                let switching = loader_now.as_deref() != Some(d.loader.label().to_lowercase().as_str())
                    && !(loader_now.is_none() && d.loader == Loader::Vanilla);
                if switching && mods_in > 0 {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "The {mods_in} mod(s) in this instance stay. Mods made for another loader won't load: switch them off or replace them in the Mods tab."
                        ))
                        .size(FS_META)
                        .color(c_warn()),
                    );
                }
                if playing {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Close Minecraft first, then change the mod loader.").size(FS_META).color(c_warn()));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ready = !playing && (d.loader == Loader::Vanilla || (!d.loading && !d.versions.is_empty()));
                    if primary_enabled(ui, "Apply", ready).clicked() {
                        apply = true;
                    }
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            close |= modal.should_close();
            if apply {
                if let Some(inst) = self.instance(&d.folder).cloned() {
                    let (dirs, loader) = (self.dirs.clone(), d.loader);
                    let version = Some(d.version.clone()).filter(|v| !v.is_empty());
                    // one change to the instance at a time: a mod installed meanwhile waits, and isn't written over
                    let lock = super::mods::instance_lock(&inst.root);
                    self.start_task(format!("Changing the mod loader of {}", inst.name()), move |p, _| async move {
                        let _turn = lock.lock().await;
                        let mut inst = inst;
                        instances::set_loader(&dirs, &mut inst, loader, version.as_deref(), &p).await?;
                        Ok(format!("{} now uses {}", inst.name(), loader.label()))
                    });
                }
                close = true;
            }
            self.loader_dialog = if close { None } else { Some(d) };
            if changed {
                self.loader_dialog_request();
            }
        }
    }
}
