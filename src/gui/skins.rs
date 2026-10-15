//! Players' faces for the account list and sidebar, and the "Change skin" window with its preview.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{ColorImage, Context, CornerRadius, RichText, Sense, TextureHandle, TextureOptions, Ui, vec2};
use image::RgbaImage;

use super::services::SvcMsg;
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page};
use crate::skins;

/// A face drawn from a saved skin, remembered until the skin file changes.
pub struct HeadTexture {
    stamp: Option<SystemTime>,
    tex: TextureHandle,
}

/// The "Change skin" window.
pub struct SkinDialog {
    uuid: String,
    name: String,
    offline: bool,
    path_text: String,
    chosen: Option<PathBuf>,
    chosen_image: Option<RgbaImage>,
    current: RgbaImage,
    slim: bool,
    message: Option<(String, bool)>,
    uploading: bool,
    /// Skin-sized pictures found in the downloads folder, newest first, to pick from.
    downloads: Vec<PathBuf>,
    preview: Option<((Option<PathBuf>, bool), TextureHandle)>,
}

impl SkinDialog {
    pub fn finished_ok(&mut self) {
        self.uploading = false;
    }

    pub fn failed(&mut self, message: String) {
        self.uploading = false;
        self.message = Some((message, true));
    }

    fn use_file(&mut self, file: &Path) {
        match skins::check_skin_file(file) {
            Ok(img) => {
                self.chosen = Some(file.to_path_buf());
                self.chosen_image = Some(img);
                self.path_text = file.to_string_lossy().to_string();
                self.message = None;
            }
            Err(e) => self.message = Some((format!("{e}"), true)),
        }
    }
}

/// PNG files in `dir` that are the size of a skin, newest first (at most `limit`).
fn skin_candidates(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter(|e| image::image_dimensions(e.path()).is_ok_and(|(w, h)| skins::is_valid_skin(w, h)))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().take(limit).map(|(_, p)| p).collect()
}

fn color_image(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

impl App {
    /// A player's face as a rounded square, in place of `avatar()`'s coloured letter. `uuid` is the account's id;
    /// the skin is read from the saved copy, and fetched once if there is none yet.
    pub(super) fn account_avatar(&mut self, ui: &mut Ui, uuid: &str, name: &str, offline: bool, size: f32) {
        let path = skins::skin_path(&self.dirs, uuid);
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if self.svc.heads.get(uuid).is_none_or(|h| h.stamp != stamp) {
            let skin = std::fs::read(&path)
                .ok()
                .and_then(|b| skins::decode_png(&b).ok())
                .or_else(|| skins::decode_png(&skins::default_skin_png(uuid)).ok());
            match skin.as_ref().and_then(skins::head_pixels) {
                Some(head) => {
                    let tex =
                        self.ctx.load_texture(format!("head-{uuid}"), color_image(&head), TextureOptions::NEAREST);
                    self.svc.heads.insert(uuid.to_string(), HeadTexture { stamp, tex });
                }
                None => {
                    avatar(ui, name, size);
                    return;
                }
            }
            // no saved skin yet: fetch it once, then the face is drawn again
            if stamp.is_none() && !offline && self.svc.skin_requested.insert(uuid.to_string()) {
                if let Some(account) = self.accounts.iter().find(|a| a.uuid == uuid).cloned() {
                    let (dirs, n, id) = (self.dirs.clone(), self.notifier(), uuid.to_string());
                    self.rt.spawn(async move {
                        let _ = skins::update_skin(&dirs, &account).await;
                        n.send(Msg::Svc(SvcMsg::SkinFetched(id)));
                    });
                }
            }
        }
        let Some(head) = self.svc.heads.get(uuid) else { return };
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        let radius = CornerRadius::same((size * 0.24).round() as u8);
        ui.painter().rect_filled(rect, radius, c_field());
        egui::Image::from_texture(egui::load::SizedTexture::new(head.tex.id(), vec2(size, size)))
            .corner_radius(radius)
            .paint_at(ui, rect);
        if resp.hovered() {
            resp.on_hover_text(name);
        }
    }

    /// Open the "Change skin" window for an account.
    pub(super) fn open_skin_dialog(&mut self, uuid: &str) {
        let Some(account) = self.accounts.iter().find(|a| a.uuid == uuid) else { return };
        let (name, offline) = (account.name().to_string(), account.offline);
        let current = skins::load_skin(&self.dirs, account);
        let settings = crate::svcsettings::ServiceSettings::from_settings(&self.settings);
        let downloads =
            if offline { Vec::new() } else { skin_candidates(&crate::tools::user_downloads_dir(&settings), 6) };
        self.svc.skin_dialog = Some(SkinDialog {
            uuid: uuid.to_string(),
            name,
            offline,
            path_text: String::new(),
            chosen: None,
            chosen_image: None,
            current,
            slim: false,
            message: None,
            uploading: false,
            downloads,
            preview: None,
        });
    }

    /// A file was dropped on the window while the skin window is open: it becomes the chosen skin. Returns
    /// whether the file was taken.
    pub(super) fn svc_handle_drop(&mut self, path: &Path) -> bool {
        match self.svc.skin_dialog.as_mut() {
            Some(d) if !d.offline => {
                d.use_file(path);
                true
            }
            _ => false,
        }
    }

    pub(super) fn skin_dialog_ui(&mut self, ctx: &Context) {
        let Some(mut d) = self.svc.skin_dialog.take() else { return };
        let mut close = false;
        let mut upload = false;
        let mut sign_in = false;
        let mut open_site = false;
        let mut pick: Option<PathBuf> = None;
        let mut typed = false;

        // the picture: the chosen skin if there is one, otherwise the current skin
        let key = (d.chosen.clone(), d.slim);
        if !d.offline && d.preview.as_ref().is_none_or(|(k, _)| *k != key) {
            let source = d.chosen_image.as_ref().unwrap_or(&d.current);
            let fig = skins::figure_pixels(source, d.slim);
            let tex = ctx.load_texture("skin-preview", color_image(&fig), TextureOptions::NEAREST);
            d.preview = Some((key, tex));
        }

        egui::Modal::new(egui::Id::unique("skin-dialog")).show(ctx, |ui| {
            ui.set_width(if d.offline { 460.0 } else { 600.0 });
            ui.label(RichText::new("Change skin").size(22.0).strong());
            if d.offline {
                muted(ui, format!("{} is an offline player", d.name));
                ui.add_space(8.0);
                ui.label("Minecraft keeps skins on Microsoft's servers, so offline players always look like one of the default characters in game.");
                ui.add_space(4.0);
                ui.label("To use your own skin, sign in with the Microsoft account that owns Minecraft, then choose Change skin again.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Sign in with Microsoft").clicked() {
                        sign_in = true;
                    }
                    if secondary(ui, "Close").clicked() {
                        close = true;
                    }
                });
                return;
            }
            muted(ui, format!("For {}. Your skin shows in every game and on every server.", d.name));
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                // the figure, 8 times its size
                let size = vec2(16.0 * 8.0, 32.0 * 8.0);
                let (rect, _) = ui.allocate_exact_size(size + vec2(24.0, 24.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(14), c_field());
                if let Some((_, tex)) = &d.preview {
                    egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size)).paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
                }
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.set_width(330.0);
                    ui.label(RichText::new("1. Choose a skin").strong());
                    small_muted(ui, "Pick a 64x64 PNG skin file, or drag one onto this window. Need one? Browse a skin site and download a PNG.");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let r = ui.add_sized([250.0, 32.0], egui::TextEdit::singleline(&mut d.path_text).hint_text("Path to a skin file").margin(egui::Margin::symmetric(10, 6)).background_color(c_field()));
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if secondary(ui, "Use").clicked() || enter {
                            typed = true;
                        }
                    });
                    if secondary_icon(ui, super::icons::Icon::Search, "Find skins online").clicked() {
                        open_site = true;
                    }
                    if !d.downloads.is_empty() {
                        small_muted(ui, "In your downloads:");
                        ui.horizontal_wrapped(|ui| {
                            for p in &d.downloads {
                                let label = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                let label = if label.chars().count() > 26 { format!("{}\u{2026}", label.chars().take(25).collect::<String>()) } else { label };
                                if chip(ui, &label, d.chosen.as_ref() == Some(p)).clicked() {
                                    pick = Some(p.clone());
                                }
                            }
                        });
                    }
                    if let Some(c) = &d.chosen {
                        small_muted(ui, c.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                    } else {
                        small_muted(ui, "Showing your current skin");
                    }
                    ui.add_space(10.0);
                    ui.label(RichText::new("2. Arm width").strong());
                    small_muted(ui, "Skins are made for one of two arm widths. Pick the one that looks right in the preview.");
                    ui.horizontal(|ui| {
                        if chip(ui, "Classic arms", !d.slim).clicked() {
                            d.slim = false;
                        }
                        if chip(ui, "Slim arms", d.slim).clicked() {
                            d.slim = true;
                        }
                    });
                    ui.add_space(8.0);
                    if d.uploading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            muted(ui, "Uploading your skin...");
                        });
                    } else if let Some((text, error)) = &d.message {
                        ui.label(RichText::new(text).color(if *error { c_danger() } else { c_muted() }).size(13.0));
                    }
                });
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if primary_enabled(ui, "Use this skin", d.chosen.is_some() && !d.uploading).clicked() {
                    upload = true;
                }
                if secondary(ui, "Cancel").clicked() {
                    close = true;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !d.uploading {
            close = true;
        }

        if open_site {
            let _ = open::that(skins::SKIN_SITE);
        }
        if let Some(p) = pick {
            d.use_file(&p);
        }
        if typed {
            let p = PathBuf::from(d.path_text.trim().trim_matches('"'));
            d.use_file(&p);
        }
        if sign_in {
            self.page = Page::Accounts;
            close = true;
        }
        if upload {
            if let (Some(file), Some(account)) =
                (d.chosen.clone(), self.accounts.iter().find(|a| a.uuid == d.uuid).cloned())
            {
                d.uploading = true;
                d.message = None;
                let (dirs, n, slim) = (self.dirs.clone(), self.notifier(), d.slim);
                self.rt.spawn(async move {
                    let r = skins::change_skin(&dirs, &account, &file, slim).await.map_err(|e| format!("{e:#}"));
                    n.send(Msg::Svc(SvcMsg::SkinChanged(r)));
                });
            }
        }
        if !close {
            self.svc.skin_dialog = Some(d);
        }
    }
}
