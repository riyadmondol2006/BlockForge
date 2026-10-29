//! "Add from GitHub": a mod that is on neither Modrinth nor CurseForge, added from the releases of its GitHub
//! project. The player pastes the link, picks the release and file (the one that fits the instance is chosen for
//! them), and it is added the way a file dropped on the Mods tab is.

use std::cell::Cell;

use egui::RichText;

use super::discover::{dialog_footer, dialog_title, instance_choice};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg};
use crate::github::{self, Asset, Release, RepoLink};
use crate::mods::metadata::{self, FileKind};

#[derive(Default)]
pub struct GhState {
    open: bool,
    pub(super) input: String,
    /// The project the releases shown are from.
    link: Option<RepoLink>,
    seq: u64,
    loading: bool,
    releases: Vec<Release>,
    error: Option<String>,
    release: usize,
    asset: usize,
    /// The instance the file goes into (folder name).
    target: Option<String>,
}

enum Action {
    Close,
    Find,
    Add(Asset),
}

impl App {
    pub(super) fn gh_open(&mut self) {
        let g = &mut self.gh;
        g.open = true;
        g.error = None;
        let target = self.discover.target.clone().or_else(|| self.selected.clone());
        let g = &mut self.gh;
        if g.target.is_none() || target.is_some() {
            g.target = target.or_else(|| self.instances.first().map(|i| i.folder.clone()));
        }
    }

    pub(super) fn gh_arrived(&mut self, seq: u64, result: Result<Vec<Release>, String>) {
        let mc = self.gh.target.as_ref().and_then(|f| self.instance(f)).map(|i| i.mc_version().to_string());
        let g = &mut self.gh;
        if seq != g.seq {
            return;
        }
        g.loading = false;
        match result {
            Ok(list) => {
                // the newest full release made for the instance's Minecraft, not a test version
                g.release = github::preferred_release(&list, mc.as_deref().unwrap_or_default());
                g.releases = list;
                g.asset = 0;
                g.error = None;
            }
            Err(e) => {
                g.releases.clear();
                g.error = Some(e);
            }
        }
    }

    pub(super) fn gh_find(&mut self) {
        let g = &mut self.gh;
        let Some(link) = github::parse_link(&g.input) else {
            g.error = Some("That isn't a link to a project on GitHub. It looks like github.com/owner/project.".into());
            return;
        };
        g.seq += 1;
        g.loading = true;
        g.error = None;
        g.releases.clear();
        g.link = Some(link.clone());
        let (seq, n) = (g.seq, self.notifier());
        self.rt.spawn(async move {
            let result = github::releases(&link).await.map_err(|e| format!("{e:#}"));
            n.send(Msg::GithubReleases { seq, result });
        });
    }

    /// Downloads a release file and adds it to the instance: it is looked at the way a dropped file is, so a mod
    /// goes into `mods`, a resource pack into `resourcepacks`, and so on.
    fn gh_add(&mut self, folder: &str, asset: Asset) {
        let Some(inst) = self.instance(folder) else {
            self.toast_err("That instance isn't there any more. Choose another one.");
            return;
        };
        if self.is_playing(folder) {
            self.toast_err("Close Minecraft first. Its files can't be changed while it runs.");
            return;
        }
        let (root, inst_name) = (inst.root.clone(), inst.name().to_string());
        let (dirs, settings, lock) = (self.dirs.clone(), self.settings.clone(), super::mods::instance_lock(&root));
        let name = asset.name.clone();
        self.start_task(format!("Adding {name}"), move |p, _| async move {
            let dir = dirs.temp.join(format!("github-{}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            let file = dir.join(&asset.name);
            let mut job = crate::install::Job::new(&asset.browser_download_url, &file);
            job.size = Some(asset.size).filter(|s| *s > 0);
            let result = async {
                crate::install::download_all(vec![job], &p, true)
                    .await
                    .map_err(|e| e.context(format!("Couldn't download {name} from GitHub")))?;
                let detected = metadata::detect(&file);
                let record_type = match detected.kind {
                    FileKind::DataPack => "datapack",
                    k if k.is_modpack() => {
                        anyhow::bail!("{name} is a modpack. Import it from the Library page to make an instance of it.")
                    }
                    k => match k.content_kind() {
                        Some(c) => c.record_type(),
                        None => anyhow::bail!("{name} isn't a mod, resource pack, shader pack or data pack"),
                    },
                };
                let _turn = lock.lock().await;
                let mut inst = crate::instances::load_one(&root)
                    .ok_or_else(|| anyhow::anyhow!("that instance can't be read any more"))?;
                let ctx = crate::mods::ModContext::build(&dirs, &inst, &settings).await;
                let problem = detected.problem_for(ctx.target.loader_key, &ctx.target.minecraft_version);
                let mod_settings = crate::mods::ModSettings::from_settings(&settings);
                let shown = detected.display_name().to_string();
                match crate::mods::files::add_file_identified(&mut inst, &file, record_type, &mod_settings).await? {
                    crate::mods::files::AddOutcome::Added(_) => match problem {
                        Some(why) => Err(anyhow::anyhow!("Added {shown} to {inst_name}, but it may not load: {why}")),
                        None => Ok(format!("Added {shown} to {inst_name}")),
                    },
                    crate::mods::files::AddOutcome::AlreadyThere => Ok(format!("{shown} is already in {inst_name}")),
                }
            }
            .await;
            let _ = std::fs::remove_dir_all(&dir);
            result
        });
        self.mods_cache = None;
    }

    /// The "Add from GitHub" window, drawn over whatever page is showing. Call it every frame.
    pub(super) fn gh_dialog(&mut self, ctx: &egui::Context) {
        if !self.gh.open {
            return;
        }
        let action: Cell<Option<Action>> = Cell::new(None);
        let (width, list_height) = super::curseforge::modal_size(ctx, 560.0, 380.0);
        let target_inst = self.gh.target.as_ref().and_then(|f| self.instance(f)).cloned();
        let loader = target_inst.as_ref().and_then(|i| i.loader()).map(|(k, _)| k.to_ascii_lowercase());
        let mc = target_inst.as_ref().map(|i| i.mc_version().to_string()).unwrap_or_default();
        let choices: Vec<(String, String)> =
            self.instances.iter().map(|i| (i.folder.clone(), instance_choice(i))).collect();
        let modal = egui::Modal::new(egui::Id::unique("github-add")).frame(super::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_width(width);
            let g = &mut self.gh;
            if dialog_title(ui, "Add from GitHub") {
                action.set(Some(Action::Close));
            }
            small_muted(ui, "For a mod that isn't on Modrinth or CurseForge. Paste the link to its GitHub page.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let field_w = (ui.available_width() - 90.0).max(160.0);
                let r = super::discover::search_input(ui, &mut g.input, "github.com/owner/project", field_w);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (secondary_enabled(ui, "Find", !g.loading && !g.input.trim().is_empty()).clicked() || enter)
                    && !g.input.trim().is_empty()
                {
                    action.set(Some(Action::Find));
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                super::discover::filter_label(ui, "Add to");
                let current = target_inst.as_ref().map(|i| i.name().to_string()).unwrap_or_else(|| "Choose".into());
                egui::ComboBox::from_id_salt("github-target").selected_text(current).width(260.0).show_ui(ui, |ui| {
                    for (folder, name) in &choices {
                        if ui.selectable_label(g.target.as_ref() == Some(folder), name).clicked() {
                            g.target = Some(folder.clone());
                            g.asset = 0;
                        }
                    }
                });
            });
            ui.add_space(10.0);
            if let Some(e) = &g.error {
                notice(ui, NoticeKind::Error, e, None);
                ui.add_space(8.0);
            }
            if g.loading {
                ui.horizontal(|ui| {
                    spinner(ui);
                    small_muted(ui, "Reading the releases");
                });
                ui.add_space(8.0);
            }
            let mut chosen: Option<Asset> = None;
            if !g.releases.is_empty() {
                let names: Vec<String> = g
                    .releases
                    .iter()
                    .map(|r| if r.prerelease { format!("{} (test version)", r.title()) } else { r.title().to_string() })
                    .collect();
                ui.horizontal(|ui| {
                    super::discover::filter_label(ui, "Release");
                    let shown = names.get(g.release).cloned().unwrap_or_default();
                    egui::ComboBox::from_id_salt("github-release").selected_text(shown).width(260.0).show_ui(
                        ui,
                        |ui| {
                            for (i, n) in names.iter().enumerate() {
                                if ui.selectable_label(g.release == i, n).clicked() {
                                    g.release = i;
                                    g.asset = 0;
                                }
                            }
                        },
                    );
                });
                ui.add_space(6.0);
                let assets = g
                    .releases
                    .get(g.release)
                    .map(|r| github::ranked_assets(r, loader.as_deref(), &mc))
                    .unwrap_or_default();
                g.asset = g.asset.min(assets.len().saturating_sub(1));
                egui::ScrollArea::vertical().id_salt("github-assets").max_height(list_height.min(220.0)).show(
                    ui,
                    |ui| {
                        for (i, a) in assets.iter().enumerate() {
                            let label = format!("{}   {}", a.name, human_size(a.size));
                            if ui.radio(g.asset == i, label).clicked() {
                                g.asset = i;
                            }
                        }
                    },
                );
                chosen = assets.get(g.asset).cloned();
                ui.add_space(8.0);
                small_muted(
                    ui,
                    "Files from GitHub aren't checked by Modrinth or CurseForge. Add mods only from people you trust.",
                );
                ui.add_space(8.0);
            }
            let page = g.link.as_ref().map(RepoLink::page_url);
            let can_add = chosen.is_some() && target_inst.is_some() && !g.loading;
            dialog_footer(
                ui,
                "github-add",
                |ui| {
                    if let Some(url) = &page {
                        if ghost(ui, "Open on GitHub").clicked() {
                            let _ = open::that(url);
                        }
                    }
                },
                |ui| {
                    if secondary(ui, "Cancel").clicked() {
                        action.set(Some(Action::Close));
                    }
                    if primary_enabled(ui, "Add", can_add).clicked() {
                        if let Some(a) = chosen.clone() {
                            action.set(Some(Action::Add(a)));
                        }
                    }
                },
            );
            if self.instances.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Create an instance first.").color(c_muted()));
            }
        });
        let action = action.into_inner().or_else(|| modal.should_close().then_some(Action::Close));
        match action {
            Some(Action::Close) => self.gh.open = false,
            Some(Action::Find) => self.gh_find(),
            Some(Action::Add(asset)) => {
                if let Some(folder) = self.gh.target.clone() {
                    self.gh.open = false;
                    self.gh_add(&folder, asset);
                }
            }
            None => {}
        }
    }
}
