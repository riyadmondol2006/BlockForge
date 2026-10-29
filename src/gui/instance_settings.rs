//! The settings of one instance: three tabs (General, Java and Minecraft, Commands) with the same options, wording,
//! checks and "null means follow the launcher" rules as the Java edition's instance settings dialog.

use egui::{RichText, ScrollArea, Ui};
use serde_json::{Value, json};

use super::instance_menu::{SplitRow, ghost_button};
use super::library::FORM_COLUMN;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::instances::{Instance, LauncherBlock};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MemChoice {
    Follow,
    Automatic,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QuickKind {
    Disabled,
    Single,
    Multi,
    Realm,
}

/// What the dialog is editing: a copy of the instance's settings that is written back on Save.
#[derive(Clone)]
pub struct InstDraft {
    pub folder: String,
    pub tab: u8,
    account: Option<String>,
    quick: QuickKind,
    server: String,
    world: String,
    realm: String,
    mem: MemChoice,
    mem_custom: u32,
    perm_gen: u32,
    use_mc_java: Option<bool>,
    java_path: String,
    java_args: String,
    runtime: Option<String>,
    disable_legacy: Option<bool>,
    system_glfw: Option<bool>,
    system_openal: Option<bool>,
    enable_commands: Option<bool>,
    pre: String,
    post: String,
    wrapper: String,
    pub error: Option<String>,
    /// When Save last worked: "Saved" shows for a moment after it (not for ever, while more is changed).
    saved: Option<std::time::Instant>,
    /// The single player worlds of the instance, read once when the settings are opened (not on every frame).
    worlds: Vec<String>,
    /// The choices as they were saved, to tell whether anything was changed since (see [`InstDraft::dirty`]).
    base: Option<Box<InstDraft>>,
}

fn ex_str(l: &LauncherBlock, key: &str) -> Option<String> {
    l.extras.get(key).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
}

fn ex_bool(l: &LauncherBlock, key: &str) -> Option<bool> {
    l.extras.get(key).and_then(Value::as_bool)
}

fn set_ex(l: &mut LauncherBlock, key: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            l.extras.insert(key.to_string(), v);
        }
        None => {
            l.extras.remove(key);
        }
    }
}

impl InstDraft {
    pub fn from(inst: &Instance, settings: &crate::config::Settings) -> InstDraft {
        let l = &inst.file.launcher;
        let quick = l.extras.get("quickPlay").cloned().unwrap_or(Value::Null);
        let q = |k: &str| quick.get(k).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
        let (kind, server, world, realm) = if let Some(s) = q("serverAddress") {
            (QuickKind::Multi, s, String::new(), String::new())
        } else if let Some(w) = q("worldName") {
            (QuickKind::Single, String::new(), w, String::new())
        } else if let Some(r) = q("realmId") {
            (QuickKind::Realm, String::new(), String::new(), r)
        } else {
            (QuickKind::Disabled, String::new(), String::new(), String::new())
        };
        let mem = if l.automatic_memory == Some(true) {
            MemChoice::Automatic
        } else if l.maximum_memory.is_some() {
            MemChoice::Custom
        } else {
            MemChoice::Follow
        };
        InstDraft {
            folder: inst.folder.clone(),
            tab: 0,
            account: ex_str(l, "account"),
            quick: kind,
            server,
            world,
            realm,
            mem,
            mem_custom: l.maximum_memory.unwrap_or(settings.maximum_memory),
            perm_gen: l.perm_gen.unwrap_or(settings.metaspace),
            use_mc_java: l.use_java_provided_by_minecraft,
            java_path: l.java_path.clone().unwrap_or_else(|| settings.java_path.clone()),
            java_args: l.java_arguments.clone().unwrap_or_else(|| settings.java_parameters.clone()),
            runtime: ex_str(l, "javaRuntimeOverride"),
            disable_legacy: ex_bool(l, "disableLegacyLaunching"),
            system_glfw: ex_bool(l, "useSystemGlfw"),
            system_openal: ex_bool(l, "useSystemOpenAl"),
            enable_commands: ex_bool(l, "enableCommands"),
            pre: ex_str(l, "preLaunchCommand").unwrap_or_default(),
            post: ex_str(l, "postExitCommand").unwrap_or_default(),
            wrapper: l.wrapper_command.clone().unwrap_or_default(),
            error: None,
            saved: None,
            worlds: world_names(&inst.root.join("saves")),
            base: None,
        }
        .with_base()
    }

    /// Remembers the choices as they are now as the saved ones.
    fn with_base(mut self) -> InstDraft {
        let mut base = self.clone();
        base.base = None;
        self.base = Some(Box::new(base));
        self
    }

    /// What the player changed in this draft, carried over to `fresh` (made from the instance file and the launcher
    /// settings as they are now): everything else is as `fresh` read it. A draft kept a while (or saved after a job
    /// changed the file) so never writes back old values the player didn't touch, such as yesterday's launcher-wide
    /// Java arguments.
    fn rebase(&self, mut fresh: InstDraft) -> InstDraft {
        let Some(base) = self.base.as_deref() else { return fresh };
        macro_rules! carry {
            ($($field:ident),*) => {
                $( if self.$field != base.$field { fresh.$field = self.$field.clone(); } )*
            };
        }
        carry!(
            account,
            quick,
            server,
            world,
            realm,
            mem,
            mem_custom,
            perm_gen,
            use_mc_java,
            java_path,
            java_args,
            runtime,
            disable_legacy,
            system_glfw,
            system_openal,
            enable_commands,
            pre,
            post,
            wrapper
        );
        fresh.tab = self.tab;
        fresh.saved = self.saved;
        fresh
    }

    /// Something was changed and not saved yet.
    pub fn dirty(&self) -> bool {
        self.base.as_deref().is_some_and(|b| !self.same_choices(b))
    }

    /// The same settings chosen (what Save writes), whatever tab shows or message is up.
    fn same_choices(&self, o: &InstDraft) -> bool {
        self.account == o.account
            && self.quick == o.quick
            && self.server == o.server
            && self.world == o.world
            && self.realm == o.realm
            && self.mem == o.mem
            && self.mem_custom == o.mem_custom
            && self.perm_gen == o.perm_gen
            && self.use_mc_java == o.use_mc_java
            && self.java_path == o.java_path
            && self.java_args == o.java_args
            && self.runtime == o.runtime
            && self.disable_legacy == o.disable_legacy
            && self.system_glfw == o.system_glfw
            && self.system_openal == o.system_openal
            && self.enable_commands == o.enable_commands
            && self.pre == o.pre
            && self.post == o.post
            && self.wrapper == o.wrapper
    }

    /// The checks Java makes before saving; returns the message to show when something is wrong. `global_args` are
    /// the launcher-wide Java arguments: the draft shows them when the instance has none of its own, and they are
    /// not this page's to judge (with `-Xmx` there, saving anything at all here used to fail).
    fn validate(&self, uses_mc_java: bool, global_args: &str) -> Option<String> {
        if !uses_mc_java && !std::path::Path::new(&self.java_path).join("bin").exists() {
            return Some("The Java Path you set is incorrect. Please verify it points to the folder where the bin folder is and try again.".into());
        }
        let own_args = self.java_args.trim() != global_args.trim();
        if own_args
            && (self.java_args.contains("-Xmx")
                || self.java_args.contains("-XX:PermSize")
                || self.java_args.contains("-XX:MetaspaceSize"))
        {
            return Some(
                "The entered Java Parameters were incorrect. Please remove any references to Xmx or XX:PermSize."
                    .into(),
            );
        }
        if self.quick == QuickKind::Multi {
            let a = self.server.trim();
            if a.is_empty() {
                return Some("The server address is empty.".into());
            }
            let ok = match a.rsplit_once(':') {
                Some((h, p)) => !h.is_empty() && p.parse::<u16>().is_ok(),
                None => !a.contains(' '),
            };
            if !ok {
                return Some("The entered server address is invalid.".into());
            }
        }
        if self.quick == QuickKind::Realm && self.realm.trim().is_empty() {
            return Some("The realm id is empty.".into());
        }
        if self.quick == QuickKind::Single && self.world.trim().is_empty() {
            // saved as it is, it would quietly mean the title screen
            return Some("Pick a world to start in, or play once to make one first.".into());
        }
        None
    }

    fn write_to(
        &self,
        l: &mut LauncherBlock,
        settings: &crate::config::Settings,
        uses_mc_java: bool,
        has_java_version: bool,
    ) {
        set_ex(l, "account", self.account.clone().map(Value::String));
        let quick = match self.quick {
            QuickKind::Multi => json!({"serverAddress": self.server.trim()}),
            QuickKind::Single => json!({"worldName": self.world}),
            QuickKind::Realm => json!({"realmId": self.realm.trim()}),
            QuickKind::Disabled => json!({}),
        };
        set_ex(l, "quickPlay", Some(quick));
        match self.mem {
            MemChoice::Custom => {
                l.automatic_memory = None;
                l.maximum_memory = Some(self.mem_custom);
            }
            MemChoice::Automatic => {
                l.automatic_memory = Some(true);
                l.maximum_memory = None;
            }
            MemChoice::Follow => {
                l.automatic_memory = None;
                l.maximum_memory = None;
            }
        }
        l.perm_gen = if self.perm_gen == settings.metaspace { None } else { Some(self.perm_gen) };
        if !uses_mc_java || !has_java_version {
            l.java_path = if self.java_path == settings.java_path { None } else { Some(self.java_path.clone()) };
        }
        l.java_arguments = if self.java_args == settings.java_parameters { None } else { Some(self.java_args.clone()) };
        l.use_java_provided_by_minecraft = self.use_mc_java;
        set_ex(l, "disableLegacyLaunching", self.disable_legacy.map(Value::Bool));
        set_ex(l, "javaRuntimeOverride", self.runtime.clone().map(Value::String));
        set_ex(l, "useSystemGlfw", self.system_glfw.map(Value::Bool));
        set_ex(l, "useSystemOpenAl", self.system_openal.map(Value::Bool));
        set_ex(l, "enableCommands", self.enable_commands.map(Value::Bool));
        let some = |s: &str| Some(s.to_string()).filter(|s| !s.trim().is_empty());
        set_ex(l, "preLaunchCommand", some(&self.pre).map(Value::String));
        set_ex(l, "postExitCommand", some(&self.post).map(Value::String));
        l.wrapper_command = some(&self.wrapper);
    }
}

/// A yes / no / "use the launcher default" choice (Java keeps these as a nullable Boolean).
fn tri(ui: &mut Ui, id: &str, value: &mut Option<bool>) {
    let values = [None, Some(true), Some(false)];
    let chosen = values.iter().position(|v| v == value).unwrap_or(0);
    if let Some(i) = ui.push_id(id, |ui| segmented(ui, &["Launcher default", "Yes", "No"], chosen)).inner {
        *value = values[i];
    }
}

/// The rows of one card of a settings tab: call [`Rows::gap`] before each row.
pub(super) struct Rows {
    first: bool,
}

impl Rows {
    fn new() -> Rows {
        Rows { first: true }
    }

    /// Nothing before the first row, a hairline with the same air above and below it before every other.
    pub(super) fn gap(&mut self, ui: &mut Ui) {
        if std::mem::take(&mut self.first) {
            return;
        }
        let pad = (ROW_AIR - ui.spacing().item_spacing.y).max(0.0);
        ui.add_space(pad);
        rule(ui);
        ui.add_space(pad);
    }
}

/// The room above and below the hairline between two rows.
const ROW_AIR: f32 = 13.0;

/// A row with the title above a text field as wide as the form (long paths and commands).
fn stacked(ui: &mut Ui, rows: &mut Rows, title: &str, hint: &str, value: &mut String, placeholder: &str) {
    rows.gap(ui);
    ui.label(RichText::new(title).semibold());
    if !hint.is_empty() {
        small_muted(ui, hint);
    }
    ui.add_space(6.0);
    let width = ui.available_width().min(FORM_COLUMN);
    search_field(ui, value, placeholder, width);
}

/// One setting: its title and hint, and the control at the right.
fn row(ui: &mut Ui, rows: &mut Rows, title: &str, hint: &str, control: impl FnOnce(&mut Ui)) {
    rows.gap(ui);
    setting_row(ui, title, hint, control);
}

/// A titled group of a settings tab: one glass card with its name and one line about it at the top, a hairline
/// across the card under them, then its rows divided by hairlines.
fn group(ui: &mut Ui, title: &str, hint: &str, add: impl FnOnce(&mut Ui, &mut Rows)) {
    ui.add_space(14.0);
    card(ui, |ui| {
        // the title of a row sits close to its explanation
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(title).size(FS_SECTION).semibold());
        if !hint.is_empty() {
            ui.label(RichText::new(hint).size(FS_META).color(c_muted()));
        }
        ui.add_space(ROW_AIR);
        // from edge to edge of the card, through its padding: the head of the card is set apart from its rows
        let (line, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        let bleed = card_frame().inner_margin.leftf();
        ui.painter().hline(
            line.left() - bleed..=line.right() + bleed,
            line.center().y,
            egui::Stroke::new(1.0, c_rule()),
        );
        ui.add_space(ROW_AIR);
        add(ui, &mut Rows::new());
        ui.add_space(2.0);
    });
}

/// A name to type, such as a variable of the commands: monospace on a small tinted plate.
fn code(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(c_field())
        .stroke(egui::Stroke::new(1.0, c_border()))
        .corner_radius(egui::CornerRadius::same(R_TAG))
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| ui.label(RichText::new(text).monospace().color(c_text())));
}

/// A warning under the row it is about: one line of small text on a tint of the warning colour.
fn warning(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(c_warn_bg())
        .stroke(egui::Stroke::new(1.0, c_warn().gamma_multiply(0.3)))
        .corner_radius(egui::CornerRadius::same(R_CONTROL - 2))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(FS_META).color(c_warn())).wrap());
        });
}

/// A small status beside Save: a dot of its colour and a word.
fn save_state(ui: &mut Ui, text: &str, color: egui::Color32) {
    let font = super::fonts::medium(FS_META);
    let w = ui.painter().layout_no_wrap(text.to_string(), font.clone(), color).size().x;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w + 30.0, 26.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::same(13), color.gamma_multiply(0.12));
    ui.painter().circle_filled(egui::pos2(rect.left() + 12.0, rect.center().y), 3.5, color);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    ui.painter().galley(egui::pos2(rect.left() + 21.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// The folders in `saves`, sorted by name.
fn world_names(saves: &std::path::Path) -> Vec<String> {
    let mut worlds: Vec<String> = std::fs::read_dir(saves)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    worlds.sort_by_cached_key(|w| w.to_lowercase());
    worlds
}

/// This computer's memory in MB (0 when unknown). It doesn't change while the launcher runs, so it is asked for once.
fn total_memory_mb() -> u64 {
    static TOTAL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *TOTAL.get_or_init(|| crate::platform::memory_mb().0)
}

/// More than three quarters of the computer's memory for the game: the system and the launcher need the rest.
fn too_much_memory(mb: u32, total_mb: u64) -> bool {
    total_mb > 0 && u64::from(mb) * 4 > total_mb * 3
}

fn gigabytes(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

impl App {
    /// Write a settings draft into its instance. Returns whether it was saved; when not, `d.error` says why.
    pub(super) fn save_draft(&mut self, d: &mut InstDraft) -> bool {
        let Some(idx) = self.instances.iter().position(|i| i.folder == d.folder) else {
            d.error = Some("This instance isn't there any more.".into());
            return false;
        };
        let inst = &self.instances[idx];
        // the file as it is now, so what a background job wrote meanwhile (a mod it added) is kept, and only what the
        // player changed is written over it
        let mut fresh = crate::instances::load_one(&inst.root).unwrap_or_else(|| inst.clone());
        let settings = self.settings.clone();
        let mut chosen = d.rebase(InstDraft::from(&fresh, &settings));
        let has_java = fresh.file.version.java_version.is_some();
        let uses_mc_java = chosen.use_mc_java.unwrap_or(true) && has_java;
        if let Some(e) = chosen.validate(uses_mc_java, &settings.java_parameters) {
            d.error = Some(e);
            return false;
        }
        chosen.write_to(&mut fresh.file.launcher, &settings, uses_mc_java, has_java);
        match fresh.save() {
            Ok(()) => {
                self.instances[idx] = fresh;
                chosen.saved = Some(std::time::Instant::now());
                chosen.error = None;
                *d = chosen.with_base();
                true
            }
            Err(e) => {
                d.error = Some(format!("Couldn't save: {e:#}"));
                false
            }
        }
    }

    /// The settings draft that is open, put aside for its instance when it holds changes that weren't saved.
    pub(super) fn keep_unsaved_draft(&mut self) {
        if let Some(d) = self.inst_draft.take().filter(InstDraft::dirty) {
            self.kept_drafts.insert(d.folder.clone(), d);
        }
    }

    /// Changes to an instance's settings that weren't saved yet.
    pub(super) fn unsaved_draft(&self, folder: &str) -> bool {
        self.inst_draft.as_ref().is_some_and(|d| d.folder == folder && d.dirty())
            || self.kept_drafts.get(folder).is_some_and(InstDraft::dirty)
    }

    /// Used by the screenshot tour: open the instance settings on a given tab.
    pub(super) fn shot_instance_settings_tab(&mut self, folder: &str, tab: u8) {
        if let Some(inst) = self.instance(folder).cloned() {
            let mut d = InstDraft::from(&inst, &self.settings);
            d.tab = tab;
            self.inst_draft = Some(d);
        }
    }

    pub(super) fn instance_settings_ui(&mut self, ui: &mut Ui, inst: &Instance) {
        if self.inst_draft.as_ref().map(|d| d.folder != inst.folder).unwrap_or(true) {
            // changes not saved for another instance wait for it; changes left here earlier come back
            self.keep_unsaved_draft();
            let fresh = InstDraft::from(inst, &self.settings);
            let d = match self.kept_drafts.remove(&inst.folder) {
                Some(kept) => kept.rebase(fresh),
                None => fresh,
            };
            self.inst_draft = Some(d);
        }
        let mut d = self.inst_draft.take().unwrap();
        let total_mb = total_memory_mb();
        let launcher_auto = self.settings.automatic_memory != Some(false);
        let launcher_mem = self.settings.maximum_memory;
        let metaspace = self.settings.metaspace;
        let uses_mc_java = d.use_mc_java.unwrap_or(true) && inst.file.version.java_version.is_some();
        let accounts: Vec<(String, String)> =
            self.accounts.iter().map(|a| (a.username.clone(), a.name().to_string())).collect();
        let worlds = std::mem::take(&mut d.worlds);
        let runtimes = self.runtimes.clone();
        let default_args = self.settings.java_parameters.clone();
        let default_path = self.settings.java_path.clone();
        let mut save = false;
        let mut reset = false;
        let mut delete = false;
        let mut rename = false;
        let mut helper_change = None;

        // the tabs and Save sit over the form, so Save ends where the cards end; the scroll bar has a strip of its own
        // at the window's edge, kept whether or not it shows
        let form_w = (ui.available_width() - ui.spacing().scroll.allocated_width()).min(FORM_COLUMN);
        ui.scope(|ui| {
            ui.set_max_width(form_w);
            let mut bar = SplitRow::new(ui, egui::Id::unique("instance-settings-bar"), 32.0);
            bar.left(ui, |ui| {
                if let Some(i) = segmented(ui, &["General", "Java and Minecraft", "Commands"], d.tab as usize) {
                    d.tab = i as u8;
                }
            });
            bar.right(ui, |ui| {
                const SHOWN: std::time::Duration = std::time::Duration::from_secs(3);
                let dirty = d.dirty();
                if dirty {
                    save_state(ui, "Unsaved changes", c_warn());
                } else if let Some(left) = d.saved.and_then(|t| SHOWN.checked_sub(t.elapsed())) {
                    save_state(ui, "Saved", if is_dark() { c_accent() } else { c_accent_hover() });
                    ui.ctx().request_repaint_after(left);
                }
                // Discard puts back what is saved (the launcher defaults are reset row by row below); it is there only
                // when there is something to put back
                if dirty && ghost_button(ui, "Discard changes", None, None, true).clicked() {
                    reset = true;
                }
                if primary_enabled(ui, "Save", dirty).on_disabled_hover_text("Nothing was changed").clicked() {
                    save = true;
                }
            });
            bar.finish(ui);
        });
        ui.add_space(4.0);
        if let Some(e) = &d.error {
            ui.add_space(6.0);
            notice(ui, NoticeKind::Error, e, None);
        }
        ScrollArea::vertical().id_salt("instance-settings").auto_shrink(false).show(ui, |ui| {
            // as wide as the bar above, and never wider than the window
            ui.set_max_width(ui.available_width().min(form_w));
            ui.spacing_mut().item_spacing.y = 4.0;
            match d.tab {
                0 => {
                    group(ui, "Account", "Who plays this instance", |ui, r| {
                        row(ui, r, "Play as", "The account used to start this instance. The launcher default is whichever account is selected in the launcher", |ui| {
                            let shown = match &d.account {
                                None => "Use launcher default".to_string(),
                                Some(u) => accounts.iter().find(|a| a.0.eq_ignore_ascii_case(u)).map(|a| a.1.clone()).unwrap_or_else(|| u.clone()),
                            };
                            egui::ComboBox::from_id_salt("play-as").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut d.account, None, "Use launcher default");
                                for (user, name) in &accounts {
                                    ui.selectable_value(&mut d.account, Some(user.clone()), name);
                                }
                            });
                        });
                    });
                    group(ui, "Quick play", "Where the game starts", |ui, r| {
                        row(ui, r, "Start in", "Skip the title screen and go straight into a world, server or realm", |ui| {
                            let label = |k: QuickKind| match k {
                                QuickKind::Disabled => "The title screen",
                                QuickKind::Single => "A single player world",
                                QuickKind::Multi => "A multiplayer server",
                                QuickKind::Realm => "A realm",
                            };
                            egui::ComboBox::from_id_salt("quick-kind").selected_text(label(d.quick)).width(240.0).show_ui(ui, |ui| {
                                for k in [QuickKind::Disabled, QuickKind::Single, QuickKind::Multi, QuickKind::Realm] {
                                    ui.selectable_value(&mut d.quick, k, label(k));
                                }
                            });
                        });
                        match d.quick {
                            QuickKind::Multi => stacked(ui, r, "Server address", "The multiplayer server to join once the game has started", &mut d.server, "play.example.com"),
                            QuickKind::Realm => stacked(ui, r, "Realm ID", "The ID of the realm to join once the game has started", &mut d.realm, ""),
                            QuickKind::Single => row(ui, r, "World", "The single player world to load once the game has started", |ui| {
                                if worlds.is_empty() {
                                    small_muted(ui, "No single player worlds yet");
                                } else {
                                    if d.world.is_empty() {
                                        d.world = worlds[0].clone();
                                    }
                                    egui::ComboBox::from_id_salt("quick-world").selected_text(d.world.clone()).width(240.0).show_ui(ui, |ui| {
                                        for w in &worlds {
                                            ui.selectable_value(&mut d.world, w.clone(), w);
                                        }
                                    });
                                }
                            }),
                            QuickKind::Disabled => {}
                        }
                    });
                    group(ui, "Performance", "These apply right away, from the next start of the game", |ui, r| {
                        helper_change = self.instance_performance_rows(ui, inst, r);
                    });
                    group(ui, "Instance", "Change its name, or remove it", |ui, _| {
                        ui.horizontal(|ui| {
                            if secondary(ui, "Rename").clicked() {
                                rename = true;
                            }
                            if danger(ui, "Delete this instance").clicked() {
                                delete = true;
                            }
                        });
                    });
                }
                1 => {
                    group(ui, "Memory", "How much of this computer the game may use", |ui, r| {
                        let hint = if total_mb == 0 {
                            "The most memory Minecraft may use".to_string()
                        } else {
                            format!("The most memory Minecraft may use. This computer has {}", gigabytes(total_mb))
                        };
                        row(ui, r, "Maximum memory", &hint, |ui| {
                            let follow_label = if launcher_auto { "Automatic (recommended)".to_string() } else { format!("Launcher setting ({launcher_mem} MB)") };
                            let current = match d.mem {
                                MemChoice::Follow => follow_label.clone(),
                                MemChoice::Automatic => "Automatic (recommended)".to_string(),
                                MemChoice::Custom => "Custom".to_string(),
                            };
                            egui::ComboBox::from_id_salt("mem-choice").selected_text(current).width(240.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut d.mem, MemChoice::Follow, follow_label);
                                if !launcher_auto {
                                    ui.selectable_value(&mut d.mem, MemChoice::Automatic, "Automatic (recommended)");
                                }
                                ui.selectable_value(&mut d.mem, MemChoice::Custom, "Custom");
                            });
                        });
                        if d.mem == MemChoice::Custom {
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().slider_width = (ui.available_width() - 110.0).clamp(160.0, 420.0);
                                // the part of the rail up to the handle in the brand colour: how much is given
                                ui.visuals_mut().selection.bg_fill = c_accent();
                                let max = if total_mb == 0 { 16384 } else { total_mb as u32 }.max(1024);
                                // only moving it changes the value: an amount of another computer (or not on the 256
                                // steps) shown here isn't rounded into an unsaved change
                                ui.add(egui::Slider::new(&mut d.mem_custom, 512..=max).step_by(256.0).show_value(false).trailing_fill(true).clamping(egui::SliderClamping::Edits));
                                // in GB, as the rest of the launcher says it
                                ui.add_space(6.0);
                                ui.label(RichText::new(super::settings::memory_text(d.mem_custom)).font(super::fonts::medium(FS_FIGURE)));
                            });
                            if too_much_memory(d.mem_custom, total_mb) {
                                ui.add_space(8.0);
                                warning(ui, &format!("That leaves too little for the rest of this computer ({} in all). The game may fail to start, or freeze the computer.", gigabytes(total_mb)));
                            }
                        }
                        row(ui, r, "PermGen size", "Memory for loaded code, only used by Java 7 and by older modpacks that ask for it", |ui| {
                            ui.add(egui::DragValue::new(&mut d.perm_gen).range(32..=total_mb.max(256) as u32).speed(8.0).suffix(" MB"));
                        });
                        if d.perm_gen > 256 && d.perm_gen != metaspace {
                            ui.add_space(8.0);
                            warning(ui, "Setting PermGen size above 256MB is not recommended and can cause issues.");
                        }
                    });
                    group(ui, "Java", "Which Java starts the game, and how", |ui, r| {
                        if inst.file.version.java_version.is_some() {
                            row(ui, r, "Use Java provided by Minecraft", "Recommended. Turning this off may stop Minecraft from starting", |ui| tri(ui, "use-mc-java", &mut d.use_mc_java));
                        }
                        if uses_mc_java {
                            row(ui, r, "Java", "This Minecraft version comes with its own Java. To pick another, turn off \"Use Java provided by Minecraft\" (not recommended)", |ui| {
                                small_muted(ui, "Provided by Minecraft");
                            });
                            row(ui, r, "Java runtime", "Which of Mojang's Java runtimes starts this instance. Changing it is rarely needed", |ui| {
                                let shown = match &d.runtime {
                                    None => "Use default (recommended)".to_string(),
                                    Some(name) => runtimes.iter().find(|x| &x.0 == name).map(|x| format!("{} (Java {})", x.0, x.1)).unwrap_or_else(|| name.clone()),
                                };
                                egui::ComboBox::from_id_salt("runtime").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut d.runtime, None, "Use default (recommended)");
                                    for (c, v) in &runtimes {
                                        ui.selectable_value(&mut d.runtime, Some(c.clone()), format!("{c} (Java {v})"));
                                    }
                                });
                            });
                        } else {
                            stacked(ui, r, "Java path", "The folder that holds Java's bin folder. Best left as it is; Reset goes back to the default", &mut d.java_path, "");
                            let installed = self.java_list.clone();
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("installed-java").selected_text("Installed Javas").width(240.0).show_ui(ui, |ui| {
                                    if ui.selectable_label(false, "Use launcher default").clicked() {
                                        d.java_path = default_path.clone();
                                    }
                                    for j in &installed {
                                        let l = format!("Java {}  \u{2013}  {}", j.major.map(|m| m.to_string()).unwrap_or_else(|| "?".into()), j.home.display());
                                        if ui.selectable_label(false, l).clicked() {
                                            d.java_path = j.home.to_string_lossy().to_string();
                                        }
                                    }
                                });
                                if secondary(ui, "Reset").clicked() {
                                    d.java_path = default_path.clone();
                                }
                            });
                        }
                        stacked(ui, r, "Java arguments", "Extra options passed to Java. Memory is set above, so leave out -Xmx", &mut d.java_args, "");
                        ui.add_space(4.0);
                        if secondary(ui, "Reset arguments").clicked() {
                            d.java_args = default_args.clone();
                        }
                    });
                    group(ui, "Advanced", "Only change these if a version won't start", |ui, r| {
                        row(ui, r, "Turn off legacy launching", "For Minecraft older than 1.6. Only change this if an old version won't start", |ui| tri(ui, "legacy", &mut d.disable_legacy));
                        row(ui, r, "Use system GLFW", "Use the GLFW library installed on this computer instead of Minecraft's own", |ui| tri(ui, "glfw", &mut d.system_glfw));
                        row(ui, r, "Use system OpenAL", "Use the OpenAL sound library installed on this computer instead of Minecraft's own", |ui| tri(ui, "openal", &mut d.system_openal));
                    });
                }
                _ => {
                    group(ui, "Commands", "Run your own commands around the game", |ui, r| {
                        row(ui, r, "Run commands", "Turn the commands below on or off for this instance", |ui| tri(ui, "commands", &mut d.enable_commands));
                        stacked(ui, r, "Before launch", "Runs before the game starts. The game waits until it has finished", &mut d.pre, "");
                        stacked(ui, r, "After exit", "Runs after the game closes, even if it crashed or was stopped", &mut d.post, "");
                        stacked(ui, r, "Wrapper command", "Starts the game through another program, such as prime-run on Linux. Use %command% for the launch command, or %\"command\"% for it as one quoted string", &mut d.wrapper, "prime-run %command%");
                    });
                    group(ui, "Variables", "Available in every command", |ui, _| {
                        small_muted(ui, "Commands run in the folder of the instance that is launched or exited");
                        ui.add_space(10.0);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.spacing_mut().interact_size.y = 26.0;
                        for (k, v) in [
                            ("$INST_NAME", "The name of the instance"),
                            ("$INST_ID", "The name of the instance's root directory"),
                            ("$INST_DIR", "The absolute path to the instance directory"),
                            ("$INST_MC_DIR", "Alias for $INST_DIR"),
                            ("$INST_JAVA", "The absolute path to the java executable used for launch"),
                            ("$INST_JAVA_ARGS", "The JVM parameters used for launch"),
                        ] {
                            ui.horizontal(|ui| {
                                // the names in one column, so the explanations start at one edge
                                ui.allocate_ui_with_layout(egui::vec2(140.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.set_min_width(140.0);
                                    code(ui, k);
                                });
                                small_muted(ui, v);
                            });
                        }
                    });
                }
            }
            ui.add_space(12.0);
        });

        d.worlds = worlds;
        if reset {
            let tab = d.tab;
            d = InstDraft::from(inst, &self.settings);
            d.tab = tab;
        }
        if save {
            self.save_draft(&mut d);
        }
        self.inst_draft = Some(d);
        if let Some(change) = helper_change {
            self.apply_helper_change(&inst.folder, change);
        }
        if rename {
            self.rename_target = Some((inst.folder.clone(), inst.name().to_string()));
        }
        if delete {
            self.confirm_delete = Some(inst.folder.clone());
        }
    }

    /// The Rename dialog.
    pub(super) fn rename_ui(&mut self, ctx: &egui::Context) {
        let Some((folder, mut name)) = self.rename_target.clone() else { return };
        let mut close = false;
        let mut apply = false;
        let others: Vec<String> =
            self.instances.iter().filter(|i| i.folder != folder).map(|i| i.name().to_string()).collect();
        let modal = egui::Modal::new(egui::Id::unique("rename")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Rename instance").size(FS_SECTION).semibold());
            ui.add_space(6.0);
            let r = ui.add_sized(
                [ui.available_width(), 38.0],
                egui::TextEdit::singleline(&mut name)
                    .margin(egui::Margin::symmetric(12, 9))
                    .background_color(c_field()),
            );
            // typing can start at once
            focus_when_shown(ui, &r);
            ui.add_space(8.0);
            // two instances of one name would share their backups (they are found by name)
            let taken = others.iter().any(|o| o.trim().eq_ignore_ascii_case(name.trim()));
            if taken {
                ui.label(
                    RichText::new("Another instance is called that. Pick another name.")
                        .size(FS_META)
                        .color(c_danger()),
                );
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                let ok = !name.trim().is_empty() && !taken;
                if primary_enabled(ui, "Rename", ok).clicked()
                    || (ok && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    apply = true;
                }
                if secondary(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        self.rename_target = Some((folder.clone(), name.clone()));
        if modal.should_close() {
            close = true;
        }
        if apply {
            if let Some(idx) = self.instances.iter().position(|i| i.folder == folder) {
                let old = self.instances[idx].name().to_string();
                self.instances[idx].file.launcher.name = name.trim().to_string();
                match self.instances[idx].save() {
                    Ok(()) => {
                        // backups are found by the instance's name: they follow it, so its page still lists them
                        let svc = crate::svcsettings::ServiceSettings::from_settings(&self.settings);
                        let moved = crate::backups::rename_backups(&self.dirs, &svc, &old, name.trim(), &others);
                        if moved > 0 {
                            self.toast(format!("Renamed, and its {moved} backup(s) with it"));
                        }
                    }
                    Err(e) => self.toast_err(format!("{e:#}")),
                }
            }
            close = true;
        }
        if close {
            self.rename_target = None;
        }
    }

    pub(super) fn load_runtimes(&mut self) {
        let (dirs, n) = (self.dirs.clone(), self.notifier());
        self.rt.spawn(async move {
            if let Ok(list) = crate::java::available_runtimes(&dirs).await {
                n.send(Msg::Runtimes(list));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::world_names;

    #[test]
    fn worlds_are_the_folders_in_saves_sorted_by_name() {
        let saves = std::env::temp_dir().join(format!("bf-worlds-{}", std::process::id()));
        for w in ["beta", "Alpha", "gamma"] {
            std::fs::create_dir_all(saves.join(w)).unwrap();
        }
        std::fs::write(saves.join("notes.txt"), b"not a world").unwrap();
        assert_eq!(world_names(&saves), ["Alpha", "beta", "gamma"]);
        std::fs::remove_dir_all(&saves).unwrap();
        // no saves folder yet: no worlds, and no error
        assert!(world_names(&saves).is_empty());
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    use crate::instances::{Instance, InstanceFile};

    fn instance(java_args: Option<&str>) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher.java_arguments = java_args.map(str::to_string);
        Instance { folder: "x".into(), mod_count: 0, root: std::env::temp_dir(), file }
    }

    #[test]
    fn launcher_wide_java_arguments_never_block_saving_an_instance() {
        let mut settings = crate::config::Settings::default();
        // Aikar's flags, pasted into Settings > Java arguments, include a heap size
        settings.java_parameters = "-Xms4G -Xmx4G -XX:+UseG1GC".into();
        let d = InstDraft::from(&instance(None), &settings);
        assert_eq!(d.validate(true, &settings.java_parameters), None, "changing Play as must still save");
        // the instance's own arguments are still checked
        let d = InstDraft::from(&instance(Some("-Xmx2G")), &settings);
        assert!(d.validate(true, &settings.java_parameters).is_some());
        let d = InstDraft::from(&instance(Some("-XX:+UseZGC")), &settings);
        assert_eq!(d.validate(true, &settings.java_parameters), None);
    }

    #[test]
    fn a_draft_knows_when_it_holds_changes_that_were_not_saved() {
        let settings = crate::config::Settings::default();
        let mut d = InstDraft::from(&instance(None), &settings);
        assert!(!d.dirty(), "just opened");
        d.tab = 2;
        d.error = Some("a message".into());
        assert!(!d.dirty(), "looking at another tab changes nothing");
        d.wrapper = "prime-run %command%".into();
        assert!(d.dirty());
        d.wrapper.clear();
        assert!(!d.dirty(), "typed and taken back");
        d.mem = MemChoice::Custom;
        assert!(d.dirty());
        let d = d.with_base();
        assert!(!d.dirty(), "saved");
    }

    #[test]
    fn a_kept_draft_carries_only_what_was_changed() {
        let mut settings = crate::config::Settings::default();
        settings.java_parameters = "-XX:+UseG1GC".into();
        let mut kept = InstDraft::from(&instance(None), &settings);
        kept.wrapper = "prime-run %command%".into();
        kept.tab = 2;
        // the launcher-wide arguments change while the draft waits
        settings.java_parameters = "-XX:+UseZGC".into();
        let d = kept.rebase(InstDraft::from(&instance(None), &settings));
        assert_eq!(d.wrapper, "prime-run %command%", "the change is kept");
        assert_eq!(d.java_args, "-XX:+UseZGC", "an untouched field follows the launcher as it is now");
        assert_eq!(d.tab, 2);
        assert!(d.dirty());
    }

    #[test]
    fn quick_play_into_a_world_needs_a_world() {
        let settings = crate::config::Settings::default();
        let mut d = InstDraft::from(&instance(None), &settings);
        d.quick = QuickKind::Single;
        assert!(
            d.validate(true, &settings.java_parameters).is_some(),
            "no world picked: it would save as the title screen"
        );
        d.world = "New World".into();
        assert_eq!(d.validate(true, &settings.java_parameters), None);
    }

    #[test]
    fn more_than_three_quarters_of_the_memory_is_warned_about() {
        assert!(!too_much_memory(8192, 16384));
        assert!(!too_much_memory(12288, 16384), "exactly three quarters");
        assert!(too_much_memory(12544, 16384));
        assert!(!too_much_memory(65536, 0), "unknown memory: no warning");
    }
}
