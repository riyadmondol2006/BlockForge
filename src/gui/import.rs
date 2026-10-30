//! The import window: a file, folder or link field (with Browse and drag and drop), what BlockForge made of
//! it, and the instances found in other launchers on this computer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{CornerRadius, RichText, ScrollArea, Stroke, Ui};

use super::App;
use super::icons::Icon;
use super::theme::*;
use super::widgets::*;
use crate::imports::detect::{self, Source, UrlSource};
use crate::imports::{self, ImportContext, ImportedInstance, Preview};

/// What the text in the field turned out to be.
#[derive(Clone, Debug, PartialEq)]
pub enum Detected {
    Empty,
    Missing,
    Url(UrlSource),
    Path(Source),
}

impl Detected {
    pub fn of(input: &str) -> Detected {
        let text = input.trim().trim_matches('"');
        if text.is_empty() {
            return Detected::Empty;
        }
        if let Some(url) = detect::detect_url(text) {
            return Detected::Url(url);
        }
        let path = Path::new(text.strip_prefix("file://").unwrap_or(text));
        if !path.exists() {
            return Detected::Missing;
        }
        Detected::Path(detect::detect_path(path))
    }

    /// What to tell the player, and whether it can be imported.
    pub fn describe(&self) -> (String, bool) {
        match self {
            Detected::Empty => ("Choose a modpack file or an instance folder, paste a link, or drop a file on this window.".into(), false),
            Detected::Missing => ("That file or folder doesn't exist.".into(), false),
            Detected::Url(UrlSource::CurseForge { slug, .. }) => (format!("A CurseForge modpack page ({slug})."), true),
            Detected::Url(UrlSource::Modrinth { slug }) => (format!("A Modrinth modpack page ({slug})."), true),
            Detected::Url(UrlSource::Ftb { pack_id }) => (format!("A Feed The Beast pack page (pack {pack_id}); the newest release is installed."), true),
            Detected::Url(UrlSource::File { .. }) => ("A file on the web. It is downloaded, then imported by what it contains.".into(), true),
            Detected::Path(Source::Modrinth(_)) => ("A Modrinth modpack (.mrpack).".into(), true),
            Detected::Path(Source::CurseForge(_)) => ("A CurseForge modpack.".into(), true),
            Detected::Path(Source::MultiMcArchive(_)) => ("An instance exported from Prism Launcher or MultiMC.".into(), true),
            Detected::Path(Source::LauncherFolder(p)) => match imports::read(p) {
                Ok(i) => (format!("{} instance, Minecraft {}, {}", i.source, i.minecraft_version, i.loader_label()), true),
                Err(e) => (format!("An instance folder that can't be read: {e:#}"), false),
            },
            Detected::Path(Source::World(_)) => ("A Minecraft world. Worlds go into an instance's saves folder: drop it on the instance instead.".into(), false),
            Detected::Path(Source::MinecraftFolder(_)) => ("A plain Minecraft folder. Choose the instance folder from Prism, MultiMC, CurseForge, GDLauncher or ATLauncher.".into(), false),
            Detected::Path(Source::Unknown) => ("BlockForge can't tell what this is. It reads Modrinth (.mrpack), CurseForge and MultiMC / Prism zips, and launcher instance folders.".into(), false),
        }
    }
}

enum Event {
    Scanned(Vec<ImportedInstance>),
    Preview(PathBuf, Option<Preview>),
    Picked(Option<PathBuf>),
}

pub struct ImportState {
    pub open: bool,
    pub input: String,
    pub name: String,
    detected: Detected,
    detected_for: String,
    /// What `detected` says about itself; for an instance folder that means reading its files, so it is worked
    /// out once when the field changes, not on every frame.
    described: (String, bool),
    scan: Option<Vec<ImportedInstance>>,
    scanning: bool,
    previews: HashMap<PathBuf, Option<Preview>>,
    /// Modpacks dropped while the window was busy, waiting their turn.
    queue: Vec<PathBuf>,
    picking: bool,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Default for ImportState {
    fn default() -> Self {
        let (tx, rx) = channel();
        ImportState {
            open: false,
            input: String::new(),
            name: String::new(),
            detected: Detected::Empty,
            detected_for: String::new(),
            described: Detected::Empty.describe(),
            scan: None,
            scanning: false,
            previews: HashMap::new(),
            queue: Vec::new(),
            picking: false,
            tx,
            rx,
        }
    }
}

/// Ask the system's own file chooser for a modpack file or a folder. Blocks until it closes, so call it from a
/// background thread. `None` when cancelled or when the system has no chooser we can start.
pub fn pick_path_blocking(folder: bool) -> Option<PathBuf> {
    pick_blocking(folder, "Modpacks", &["mrpack", "zip"])
}

/// Like [`pick_path_blocking`], for a file of one of the `extensions` (without dots), shown as `kind` in the chooser.
pub fn pick_file_blocking(kind: &'static str, extensions: &'static [&'static str]) -> Option<PathBuf> {
    pick_blocking(false, kind, extensions)
}

fn pick_blocking(folder: bool, kind: &'static str, extensions: &'static [&'static str]) -> Option<PathBuf> {
    use std::process::Command;
    let patterns: Vec<String> = extensions.iter().map(|e| format!("*.{e}")).collect();
    // the chooser through the desktop portal, for a system where the usual program isn't there
    let portal = || {
        let dialog = rfd::FileDialog::new();
        if folder { dialog.pick_folder() } else { dialog.add_filter(kind, extensions).pick_file() }
    };
    let run = |mut c: Command| -> Option<PathBuf> {
        let Ok(out) = c.output() else { return portal() };
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(PathBuf::from(text)) }
    };
    if cfg!(target_os = "windows") {
        let script = if folder {
            "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.FolderBrowserDialog; if ($f.ShowDialog() -eq 'OK') { $f.SelectedPath }"
        } else {
            let p = patterns.join(";");
            &format!(
                "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.OpenFileDialog; $f.Filter = '{kind} ({p})|{p}|All files (*.*)|*.*'; if ($f.ShowDialog() -eq 'OK') {{ $f.FileName }}"
            )
        };
        let mut c = crate::platform::command("powershell");
        c.args(["-NoProfile", "-Command", script]);
        return run(c);
    }
    if cfg!(target_os = "macos") {
        let script = if folder { "POSIX path of (choose folder)" } else { "POSIX path of (choose file)" };
        let mut c = crate::platform::command("osascript");
        c.args(["-e", script]);
        return run(c);
    }
    let mut zenity = crate::platform::command("zenity");
    zenity.arg("--file-selection");
    if folder {
        zenity.arg("--directory");
    } else {
        zenity.arg(format!("--file-filter={kind} | {}", patterns.join(" ")));
        zenity.arg("--file-filter=All files | *");
    }
    // zenity answers with a non-zero code when the player cancels; only a missing program moves on to kdialog
    if let Ok(out) = zenity.output() {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return if out.status.success() && !text.is_empty() { Some(PathBuf::from(text)) } else { None };
    }
    let mut k = crate::platform::command("kdialog");
    if folder {
        k.args(["--getexistingdirectory", "."]);
    } else {
        k.args(["--getopenfilename".to_string(), ".".to_string(), format!("{}|{kind}", patterns.join(" "))]);
    }
    run(k)
}

/// What the field turned out to be, in one small line: a check when it can be imported, a warning when not.
fn status_line(ui: &mut Ui, ok: bool, text: &str) {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 18.0;
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
            if ok {
                super::discover::paint_check(ui.painter(), rect.center(), 13.0, c_text());
            } else {
                super::icons::paint(ui.painter(), super::icons::Icon::Info, rect.center(), 14.0, c_warn());
            }
            let color = if ok { c_muted() } else { c_warn() };
            ui.add(egui::Label::new(RichText::new(text).size(FS_META).color(color)).wrap());
        });
    });
}

impl App {
    /// Open the import window (empty, or with something already in the field).
    pub fn open_import(&mut self, input: Option<String>) {
        self.import.open = true;
        if let Some(i) = input {
            self.import.input = i;
        }
        if self.import.scan.is_none() && !self.import.scanning {
            self.import.scanning = true;
            let (tx, ctx) = (self.import.tx.clone(), self.ctx.clone());
            self.rt.spawn_blocking(move || {
                let found = imports::scan(&imports::default_roots());
                for i in &found {
                    let _ = tx.send(Event::Preview(i.root.clone(), imports::preview(i).ok()));
                }
                let _ = tx.send(Event::Scanned(found));
                ctx.request_repaint();
            });
        }
    }

    /// Files dropped on the window: modpacks open the import window; other files aren't imported.
    pub fn handle_dropped_files(&mut self, paths: Vec<PathBuf>) {
        let mut modpacks = Vec::new();
        let mut other = 0;
        for p in paths {
            match detect::detect_path(&p) {
                Source::Unknown | Source::World(_) | Source::MinecraftFolder(_) => {
                    // a mod or a pack of resources is not an instance; only say so when it isn't a known file type either
                    let kind = detect::detect_file(&p).kind;
                    if kind.is_modpack() {
                        modpacks.push(p);
                    } else {
                        other += 1;
                    }
                }
                _ => modpacks.push(p),
            }
        }
        if other > 0 {
            self.toast_err("Only modpacks (.mrpack, CurseForge and Prism / MultiMC zips) and launcher instance folders can be dropped here");
        }
        if let Some(first) = modpacks.first().cloned() {
            self.open_import(Some(first.to_string_lossy().to_string()));
            self.import.queue.extend(modpacks.into_iter().skip(1));
        }
    }

    fn import_context(&self) -> ImportContext {
        ImportContext {
            ftb_options: crate::ftb::InstallOptions::from_settings(&self.settings, None),
            ..Default::default()
        }
    }

    /// Start importing `input` (a path or link) as a background task.
    fn start_import(&mut self, input: String, name: Option<String>) {
        let dirs = self.dirs.clone();
        let ctx = self.import_context();
        let label = format!(
            "Importing {}",
            Path::new(&input)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| input.chars().take(40).collect())
        );
        self.start_task(label, move |p, _| async move {
            let inst = imports::import_input(&dirs, &ctx, &input, name.as_deref(), &p).await?;
            Ok(format!("Imported \"{}\"", inst.name()))
        });
    }

    fn poll_import(&mut self) {
        while let Ok(ev) = self.import.rx.try_recv() {
            match ev {
                Event::Scanned(found) => {
                    self.import.scan = Some(found);
                    self.import.scanning = false;
                }
                Event::Preview(root, p) => {
                    self.import.previews.insert(root, p);
                }
                Event::Picked(p) => {
                    self.import.picking = false;
                    if let Some(p) = p {
                        self.import.input = p.to_string_lossy().to_string();
                    }
                }
            }
        }
    }

    fn pick(&mut self, folder: bool) {
        if self.import.picking {
            return;
        }
        self.import.picking = true;
        let (tx, ctx) = (self.import.tx.clone(), self.ctx.clone());
        self.rt.spawn_blocking(move || {
            let _ = tx.send(Event::Picked(pick_path_blocking(folder)));
            ctx.request_repaint();
        });
    }

    #[allow(deprecated)]
    /// The import window, drawn on top of whatever page is showing. Call it every frame.
    pub(super) fn import_modal(&mut self, ctx: &egui::Context) {
        self.poll_import();
        // dragging a file over the window
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            egui::Area::new(egui::Id::new("import-drop-hint"))
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    // a panel of glass in the brand tint, lit from inside, with the package it will become
                    let out = egui::Frame::new()
                        .fill(c_accent_soft())
                        .stroke(Stroke::new(2.0, c_accent()))
                        .corner_radius(CornerRadius::same(R_DIALOG))
                        .inner_margin(egui::Margin::symmetric(48, 30))
                        .shadow(ui.visuals().window_shadow)
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                                super::about::brand_glow(ui.painter(), rect.center(), 44.0, 0.5);
                                let ink = if is_dark() { c_accent() } else { c_accent_hover() };
                                super::icons::paint(
                                    ui.painter(),
                                    super::icons::Icon::Package,
                                    rect.center(),
                                    34.0,
                                    ink,
                                );
                                ui.add_space(8.0);
                                ui.label(RichText::new("Drop to import").size(FS_TITLE).semibold());
                            });
                        });
                    glass_edge(ui.painter(), out.response.rect, R_DIALOG);
                });
        }
        if !self.import.open {
            return;
        }
        if self.import.detected_for != self.import.input {
            self.import.detected = Detected::of(&self.import.input);
            self.import.described = self.import.detected.describe();
            self.import.detected_for = self.import.input.clone();
        }

        let mut close = false;
        let mut run: Option<(String, Option<String>)> = None;
        let mut enter = false;
        let mut pick_file = false;
        let mut pick_folder = false;
        let mut use_found: Option<ImportedInstance> = None;
        let (width, room) = super::curseforge::modal_size(ctx, 620.0, 380.0);
        let modal = egui::Modal::new(egui::Id::new("import-modal")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            use super::discover::*;
            ui.set_width(width);
            if dialog_title(ui, "Import an instance") {
                close = true;
            }
            muted(ui, "Your files are copied. Nothing is changed or removed in the other launcher.");
            ui.add_space(14.0);

            ui.label(RichText::new("File, folder or link").semibold());
            ui.add_space(2.0);
            // the field takes what is left of the row after the two choosers, measured on the last frame; when that
            // would leave the field too narrow, the choosers go on the line under it
            let buttons_id = ui.scope_id().with("import-buttons");
            let buttons_w = remembered_width(ui, buttons_id, 280.0);
            let gap = ui.spacing().item_spacing.x;
            let below = ui.available_width() - buttons_w - gap < 260.0;
            let mut choosers = |ui: &mut Ui| {
                let used = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if secondary_icon(ui, Icon::Package, "Choose file\u{2026}").clicked() {
                            pick_file = true;
                        }
                        if secondary_icon(ui, Icon::Folder, "Choose folder\u{2026}").clicked() {
                            pick_folder = true;
                        }
                    })
                    .response
                    .rect
                    .width();
                remember_width(ui, buttons_id, used);
            };
            ui.horizontal(|ui| {
                let field_w = if below { ui.available_width() } else { ui.available_width() - buttons_w - gap };
                let field = ui.add_sized(
                    [field_w.max(160.0), 32.0],
                    egui::TextEdit::singleline(&mut self.import.input)
                        .hint_text(
                            RichText::new("Paste a link, or choose a .mrpack / .zip / instance folder")
                                .color(c_faint()),
                        )
                        .margin(egui::Margin::symmetric(10, 7))
                        .background_color(c_field()),
                );
                // Enter in the field imports, like the button at the bottom
                enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if !below {
                    choosers(ui);
                }
            });
            if below {
                ui.add_space(4.0);
                choosers(ui);
            }
            if self.import.picking {
                ui.horizontal(|ui| {
                    spinner(ui);
                    small_muted(ui, "Waiting for the file chooser");
                });
            }
            let (text, ok) = self.import.described.clone();
            if self.import.detected == Detected::Empty {
                small_muted(ui, text);
            } else {
                status_line(ui, ok, &text);
            }
            if ok {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    filter_label(ui, "Name");
                    ui.add_sized(
                        [280.0, 32.0],
                        egui::TextEdit::singleline(&mut self.import.name)
                            .hint_text(RichText::new("Same as the original").color(c_faint()))
                            .margin(egui::Margin::symmetric(10, 7))
                            .background_color(c_field()),
                    );
                });
            }

            ui.add_space(20.0);
            ui.label(RichText::new("Found on this computer").semibold());
            ui.add_space(2.0);
            match &self.import.scan {
                None => {
                    ui.horizontal(|ui| {
                        spinner(ui);
                        muted(ui, "Looking for other launchers");
                    });
                }
                Some(found) if found.is_empty() => {
                    muted(ui, "No other launchers were found. Choose an instance folder yourself.");
                }
                Some(found) => {
                    // the instances on a well of their own, divided by hairlines
                    egui::Frame::new()
                        .fill(c_card())
                        .stroke(Stroke::new(1.0, c_border()))
                        .corner_radius(CornerRadius::same(R_CARD))
                        .inner_margin(egui::Margin::symmetric(14, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ScrollArea::vertical()
                                .id_salt("found-instances")
                                .max_height(room.min(260.0))
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    for (n, i) in found.iter().enumerate() {
                                        if n > 0 {
                                            rule(ui);
                                        }
                                        ui.add_space(6.0);
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 12.0;
                                            avatar(ui, &i.name, 36.0);
                                            ui.vertical(|ui| {
                                                ui.set_max_width((ui.available_width() - 110.0).max(200.0));
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                ui.add(egui::Label::new(RichText::new(&i.name).semibold()).truncate());
                                                small_muted(
                                                    ui,
                                                    format!(
                                                        "{}  \u{00B7}  Minecraft {}  \u{00B7}  {}",
                                                        i.source,
                                                        i.minecraft_version,
                                                        i.loader_label()
                                                    ),
                                                );
                                                match self.import.previews.get(&i.root) {
                                                    Some(Some(p)) => small_muted(
                                                        ui,
                                                        format!(
                                                            "{} mods  \u{00B7}  {} worlds  \u{00B7}  {}",
                                                            p.mods,
                                                            p.worlds,
                                                            human_size(p.bytes)
                                                        ),
                                                    ),
                                                    Some(None) => {
                                                        small_muted(ui, "Couldn't read this instance's files")
                                                    }
                                                    None => small_muted(ui, " "),
                                                }
                                            });
                                            let id = ui.scope_id().with(("found-row", &i.root));
                                            end_aligned(ui, id, |ui| {
                                                if secondary(ui, "Import").clicked() {
                                                    use_found = Some(i.clone());
                                                }
                                            });
                                        });
                                        ui.add_space(6.0);
                                    }
                                });
                        });
                }
            }
            ui.add_space(20.0);
            dialog_footer(
                ui,
                "import",
                |ui| {
                    if !self.import.queue.is_empty() {
                        small_muted(ui, format!("{} more dropped files are waiting", self.import.queue.len()));
                    }
                },
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                    if primary_enabled(ui, "Import", ok).clicked() || (ok && enter) {
                        run = Some((
                            self.import.input.clone(),
                            Some(self.import.name.trim().to_string()).filter(|n| !n.is_empty()),
                        ));
                    }
                },
            );
        });
        if modal.should_close() {
            close = true;
        }
        if pick_file {
            self.pick(false);
        }
        if pick_folder {
            self.pick(true);
        }
        if let Some(found) = use_found {
            run = Some((found.root.to_string_lossy().to_string(), None));
        }
        if let Some((input, name)) = run {
            self.start_import(input, name);
            self.import.input.clear();
            self.import.name.clear();
            // the next dropped modpack, if any, takes the window's place
            if let Some(next) = (!self.import.queue.is_empty()).then(|| self.import.queue.remove(0)) {
                self.import.input = next.to_string_lossy().to_string();
            } else {
                close = true;
            }
        }
        if close {
            self.import.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_is_understood() {
        assert_eq!(Detected::of("  "), Detected::Empty);
        assert_eq!(Detected::of("/definitely/not/here.mrpack"), Detected::Missing);
        assert!(matches!(
            Detected::of("https://modrinth.com/modpack/fabulously-optimized"),
            Detected::Url(UrlSource::Modrinth { .. })
        ));
        let (text, ok) = Detected::Url(UrlSource::Ftb { pack_id: 100 }).describe();
        assert!(ok && text.contains("Feed The Beast"));
        let (_, ok) = Detected::Path(Source::World(PathBuf::from("w.zip"))).describe();
        assert!(!ok, "a world is not an instance");
        let dir = std::env::temp_dir();
        assert!(matches!(Detected::of(&dir.to_string_lossy()), Detected::Path(_)));
    }
}
