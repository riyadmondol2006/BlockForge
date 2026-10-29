//! Players' faces for the account list and sidebar, and the "Change skin" window with its preview.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use egui::{ColorImage, Context, CornerRadius, RichText, Sense, TextureHandle, TextureOptions, Ui, vec2};
use image::RgbaImage;

use super::services::{SvcMsg, modal_scroll_height, modal_width};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::skins;

/// A face drawn from a saved skin, remembered until the skin file changes.
pub struct HeadTexture {
    stamp: Option<SystemTime>,
    /// When the skin file's date was last looked at; it is looked at again only now and then, not on every frame.
    checked: Instant,
    tex: TextureHandle,
}

/// How often a drawn face checks whether its skin file changed (a fetched or uploaded skin replaces the face at once).
const HEAD_RECHECK: Duration = Duration::from_secs(5);

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
    /// The system's file chooser is open.
    picking: bool,
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

    /// The file chooser closed, with a file or cancelled.
    pub fn picked(&mut self, file: Option<&Path>) {
        self.picking = false;
        if let Some(file) = file {
            self.use_file(file);
        }
    }

    fn use_file(&mut self, file: &Path) {
        match skins::check_skin_file(file) {
            Ok(img) => {
                self.chosen = Some(file.to_path_buf());
                self.chosen_image = Some(img);
                self.path_text = file.to_string_lossy().to_string();
                self.message = None;
                // the same file name may hold a different picture now (downloaded again), so draw it afresh
                self.preview = None;
            }
            Err(e) => self.message = Some((format!("{e}"), true)),
        }
    }
}

/// How many of the newest pictures in the downloads folder are opened to see if they are skins. A downloads folder
/// can hold thousands of pictures, and this runs while the skin window opens.
const MAX_LOOKED_AT: usize = 60;

/// PNG files in `dir` that are the size of a skin, newest first (at most `limit`). Only the newest pictures are
/// opened to read their size.
fn skin_candidates(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut pngs: Vec<(SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    pngs.sort_by(|a, b| b.0.cmp(&a.0));
    pngs.into_iter()
        .take(MAX_LOOKED_AT)
        .map(|(_, p)| p)
        .filter(|p| image::image_dimensions(p).is_ok_and(|(w, h)| skins::is_valid_skin(w, h)))
        .take(limit)
        .collect()
}

/// A file of the downloads folder as a row of the list: the fill fades in under the pointer, and the chosen one is in
/// the brand tint.
fn file_row(ui: &mut Ui, name: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let t = super::motion::hover(ui, resp.id, resp.hovered());
    let s = super::motion::toward(ui.ctx(), resp.id.with("on"), selected, super::motion::FAST);
    let bg = super::motion::mix(c_hover().gamma_multiply(t), c_accent_soft(), s);
    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), bg);
    let ink = if is_dark() { c_accent() } else { c_accent_hover() };
    let fg = super::motion::mix(super::motion::mix(c_muted(), c_text(), t), ink, s);
    let mut job = egui::text::LayoutJob::single_section(
        name.to_string(),
        egui::TextFormat::simple(egui::FontId::proportional(FS_BODY), fg),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: rect.width() - 20.0,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    focus_ring(ui, &resp, rect, R_CONTROL);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn color_image(img: &RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

impl App {
    /// A player's face as a rounded square, in place of `avatar()`'s coloured letter. `uuid` is the account's id;
    /// the skin is read from the saved copy, and fetched once if there is none yet. Returns where the face is.
    pub(super) fn account_avatar(
        &mut self,
        ui: &mut Ui,
        uuid: &str,
        name: &str,
        offline: bool,
        size: f32,
    ) -> egui::Rect {
        let path = skins::skin_path(&self.dirs, uuid);
        let up_to_date = match self.svc.heads.get_mut(uuid) {
            Some(h) if h.checked.elapsed() < HEAD_RECHECK => true,
            Some(h) => {
                h.checked = Instant::now();
                h.stamp == std::fs::metadata(&path).and_then(|m| m.modified()).ok()
            }
            None => false,
        };
        if !up_to_date {
            let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let skin = std::fs::read(&path)
                .ok()
                .and_then(|b| skins::decode_png(&b).ok())
                .or_else(|| skins::decode_png(&skins::default_skin_png(uuid)).ok());
            match skin.as_ref().and_then(skins::head_pixels) {
                Some(head) => {
                    let tex =
                        self.ctx.load_texture(format!("head-{uuid}"), color_image(&head), TextureOptions::NEAREST);
                    self.svc.heads.insert(uuid.to_string(), HeadTexture { stamp, checked: Instant::now(), tex });
                }
                None => return avatar(ui, name, size).rect,
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
        let Some(head) = self.svc.heads.get(uuid) else { return egui::Rect::NOTHING };
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        let radius = CornerRadius::same((size * 0.24).round() as u8);
        ui.painter().rect_filled(rect, radius, c_field());
        egui::Image::from_texture(egui::load::SizedTexture::new(head.tex.id(), vec2(size, size)))
            .corner_radius(radius)
            .paint_at(ui, rect);
        if resp.hovered() {
            resp.on_hover_text(name);
        }
        rect
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
            picking: false,
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
        let mut browse = false;

        // the picture: the chosen skin if there is one, otherwise the current skin
        let key = (d.chosen.clone(), d.slim);
        if !d.offline && d.preview.as_ref().is_none_or(|(k, _)| *k != key) {
            let source = d.chosen_image.as_ref().unwrap_or(&d.current);
            let fig = skins::figure_pixels(source, d.slim);
            let tex = ctx.load_texture("skin-preview", color_image(&fig), TextureOptions::NEAREST);
            d.preview = Some((key, tex));
        }

        egui::Modal::new(egui::Id::unique("skin-dialog")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(modal_width(ctx, if d.offline { 460.0 } else { 600.0 }));
            ui.label(RichText::new("Change skin").size(FS_FIGURE).semibold());
            if d.offline {
                muted(ui, format!("{} is an offline player", d.name));
                ui.add_space(8.0);
                ui.label("Minecraft keeps skins on Microsoft's servers, so offline players always look like one of the default characters in game.");
                ui.add_space(4.0);
                ui.label("To use your own skin, sign in with the Microsoft account that owns Minecraft, then choose Change skin again.");
                ui.add_space(20.0);
                super::discover::dialog_footer(ui, "skin-offline", |_| {}, |ui| {
                    if secondary(ui, "Close").clicked() {
                        close = true;
                    }
                    if primary(ui, "Sign in with Microsoft").clicked() {
                        sign_in = true;
                    }
                });
                return;
            }
            muted(ui, format!("For {}. Your skin shows in every game and on every server.", d.name));
            ui.add_space(14.0);
            // a short window (or a big zoom) scrolls the middle part instead of pushing the buttons off the screen
            egui::ScrollArea::vertical().max_height(modal_scroll_height(ctx, 560.0, 170.0)).auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    // the figure, 8 times its size, or 5 times in a narrow window, on a panel of its own
                    let wide = ui.available_width() >= 480.0;
                    let scale = if wide { 8.0 } else { 5.0 };
                    let size = vec2(16.0 * scale, 32.0 * scale);
                    let panel = vec2(if wide { 200.0 } else { size.x + 24.0 }, size.y + 24.0);
                    let (rect, _) = ui.allocate_exact_size(panel, Sense::hover());
                    // a small stage: a well of glass, lit softly in the brand colours, with a shadow under the feet
                    ui.painter().rect(rect, CornerRadius::same(R_CARD), c_card(), egui::Stroke::new(1.0, c_border()), egui::StrokeKind::Inside);
                    let p = ui.painter().with_clip_rect(rect.shrink(1.0).intersect(ui.clip_rect()));
                    let r = rect.width().min(rect.height()) * 0.55;
                    super::about::brand_glow(&p, rect.center() - vec2(0.0, size.y * 0.04), r, 0.4);
                    let feet = egui::pos2(rect.center().x, rect.center().y + size.y / 2.0 - 2.0);
                    p.add(egui::Shape::ellipse_filled(feet, vec2(size.x * 0.42, 5.0), egui::Color32::from_black_alpha(if is_dark() { 90 } else { 28 })));
                    if let Some((_, tex)) = &d.preview {
                        egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size)).paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
                    }
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width().clamp(160.0, 330.0));
                        ui.label(RichText::new("Skin file").semibold());
                        small_muted(ui, "Pick a 64x64 PNG skin file, or drag one onto this window.");
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            let field = (ui.available_width() - 64.0).clamp(100.0, 250.0);
                            let r = ui.add_sized([field, 32.0], egui::TextEdit::singleline(&mut d.path_text).hint_text("Path to a skin file").margin(egui::Margin::symmetric(10, 7)).background_color(c_field()));
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if secondary(ui, "Use").clicked() || enter {
                                typed = true;
                            }
                        });
                        // the system's chooser: dropping a file doesn't work everywhere (Wayland), typing a path is a chore
                        if ui.add_enabled_ui(!d.picking, |ui| secondary(ui, "Browse\u{2026}")).inner.clicked() {
                            browse = true;
                        }
                        if !d.downloads.is_empty() {
                            small_muted(ui, "In your downloads");
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for p in &d.downloads {
                                let label = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                if file_row(ui, &label, d.chosen.as_ref() == Some(p)).clicked() {
                                    pick = Some(p.clone());
                                }
                            }
                            ui.spacing_mut().item_spacing.y = 8.0;
                        }
                        if secondary_icon(ui, super::icons::Icon::Search, "Find skins online").clicked() {
                            open_site = true;
                        }
                        if let Some(c) = &d.chosen {
                            small_muted(ui, c.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                        } else {
                            small_muted(ui, "Showing your current skin");
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new("Arms").semibold());
                        small_muted(ui, "Skins are made for one of two arm widths. Pick the one that looks right in the preview.");
                        if let Some(i) = segmented(ui, &["Classic", "Slim"], usize::from(d.slim)) {
                            d.slim = i == 1;
                        }
                        if d.uploading {
                            ui.horizontal(|ui| {
                                spinner(ui);
                                muted(ui, "Uploading your skin\u{2026}");
                            });
                        } else if let Some((text, error)) = &d.message {
                            ui.add(egui::Label::new(RichText::new(text).color(if *error { c_danger() } else { c_muted() }).size(FS_META)).wrap());
                        }
                    });
                });
            });
            ui.add_space(16.0);
            // why the main button is greyed out at the left, the buttons that finish the window at the right
            let ready = d.chosen.is_some() && !d.uploading;
            let waiting = d.chosen.is_none();
            super::discover::dialog_footer(
                ui,
                "skin-dialog",
                |ui| {
                    if waiting {
                        ui.label(RichText::new("Choose a skin file first").size(FS_META).color(c_faint()));
                    }
                },
                |ui| {
                    if secondary(ui, "Cancel").clicked() {
                        close = true;
                    }
                    if primary_enabled(ui, "Use this skin", ready).clicked() {
                        upload = true;
                    }
                },
            );
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !d.uploading {
            close = true;
        }

        if open_site {
            self.open_link(skins::SKIN_SITE);
        }
        if let Some(p) = pick {
            d.use_file(&p);
        }
        if typed {
            let text = d.path_text.trim().trim_matches('"').to_string();
            if text.is_empty() {
                d.message = Some(("Enter where the skin file is first, or pick one below".to_string(), true));
            } else {
                d.use_file(&PathBuf::from(text));
            }
        }
        if browse {
            d.picking = true;
            let n = self.notifier();
            std::thread::spawn(move || {
                let path = super::import::pick_file_blocking("Skins", &["png"]);
                n.send(Msg::Svc(SvcMsg::SkinPicked(path)));
            });
        }
        if sign_in {
            // straight to the Microsoft sign-in, not just the page it is on
            close = true;
            self.start_sign_in();
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
