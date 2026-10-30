//! `blockforge packs ...`: FTB packs, importing from other launchers and exporting, from the command line.
//! It is also how these features are checked without opening the window.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Subcommand;

use crate::cf_seam::MinimalCurseForge;
use crate::config::Settings;
use crate::export::{self, ExportFormat};
use crate::ftb;
use crate::horizon::policy::{self, Action};
use crate::imports::{self, ImportContext, detect};
use crate::install::Progress;
use crate::instances;
use crate::paths::Dirs;

#[derive(Subcommand)]
pub enum PacksCmd {
    /// List FTB modpacks (most installed first), or search them
    FtbList {
        /// Only packs whose name matches
        #[arg(long)]
        search: Option<String>,
        /// installs, plays, updated, featured or all
        #[arg(long, default_value = "installs")]
        sort: String,
        #[arg(long, default_value_t = 1)]
        page: usize,
    },
    /// Show an FTB pack and its versions
    FtbShow {
        pack: i64,
        /// How many versions to list
        #[arg(long, default_value_t = 12)]
        versions: usize,
    },
    /// Install an FTB pack as an instance
    FtbInstall {
        pack: i64,
        /// A version id (see ftb-show); the newest release when left out
        #[arg(long)]
        version: Option<i64>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Check the FTB instances for a newer pack version
    FtbCheck,
    /// Update an FTB instance to its pack's newest version
    FtbUpdate { instance: String },
    /// Say what a file, folder or link is and how it would be imported
    Detect { input: String },
    /// Import a file, folder or link (Modrinth, MultiMC / Prism, launcher folders, FTB pages, direct downloads)
    ImportAny {
        input: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Find instances from other launchers on this computer
    Scan,
    /// Export an instance
    Export {
        instance: String,
        /// modrinth, curseforge, both or multimc
        #[arg(long, default_value = "modrinth")]
        format: String,
        /// The folder to write the archive to
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        author: Option<String>,
        /// Folders and files to include (default: config, mods, resourcepacks, shaderpacks, datapacks ...)
        #[arg(long, value_delimiter = ',')]
        include: Vec<String>,
    },
}

fn show_progress(progress: Arc<Progress>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last = String::new();
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let stage = progress.stage_text();
            let (d, t) = (progress.files_done.load(Ordering::Relaxed), progress.files_total.load(Ordering::Relaxed));
            let line = if t > 0 {
                format!("{stage}  {d}/{t} files  {:.0} MB", progress.bytes_done.load(Ordering::Relaxed) as f64 / 1e6)
            } else {
                stage
            };
            if line != last {
                eprintln!("  {line}");
                last = line;
            }
        }
    })
}

fn find_instance(dirs: &Dirs, key: &str) -> Result<instances::Instance> {
    let lower = key.to_lowercase();
    instances::load_all(dirs)
        .into_iter()
        .find(|i| i.folder.to_lowercase() == lower || i.name().to_lowercase() == lower)
        .with_context(|| format!("no instance called \"{key}\" (try `blockforge list`)"))
}

fn sort_of(name: &str) -> ftb::Sort {
    match name {
        "plays" => ftb::Sort::Plays,
        "updated" => ftb::Sort::Updated,
        "featured" => ftb::Sort::Featured,
        "all" => ftb::Sort::All,
        _ => ftb::Sort::Installs,
    }
}

pub async fn run(cmd: PacksCmd, dirs: Dirs) -> Result<()> {
    let settings = Settings::load(&dirs);
    let cf = MinimalCurseForge;
    match cmd {
        PacksCmd::FtbList { search, sort, page } => {
            let ids = match search.as_deref().filter(|s| !s.is_empty()) {
                Some(q) => ftb::search_ids(&dirs, q).await?,
                None => ftb::list_ids(&dirs, sort_of(&sort)).await?,
            };
            let packs = ftb::page(&dirs, &ids, page).await;
            for p in &packs {
                println!("{:>4}  {:<44} {:>9} installs  {} versions", p.id, p.name, p.installs, p.versions.len());
            }
            eprintln!("{} packs in total, page {page} shows {}", ids.len(), packs.len());
        }
        PacksCmd::FtbShow { pack, versions } => {
            let p = ftb::pack(&dirs, pack).await?.with_context(|| format!("FTB has no pack {pack}"))?;
            println!("{} (id {})  {}", p.name, p.id, p.website_url());
            println!("{}", p.synopsis.clone().unwrap_or_default());
            println!("tags: {}", p.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "));
            for v in p.versions_newest_first().into_iter().take(versions) {
                let m = ftb::version_manifest(&dirs, p.id, v.id).await.map_err(|e| eprintln!("  ({e:#})")).ok();
                let target = |k| {
                    m.as_ref()
                        .and_then(|m| m.target(k))
                        .map(|t| format!("{} {}", t.name, t.version))
                        .unwrap_or_default()
                };
                println!(
                    "  {:>7}  {:<14} {:<9} {} / {}",
                    v.id,
                    v.name,
                    v.kind.map(|k| format!("{k:?}")).unwrap_or_default(),
                    target(ftb::TargetType::Game),
                    target(ftb::TargetType::Modloader)
                );
            }
        }
        PacksCmd::FtbInstall { pack, version, name } => {
            let p = ftb::pack(&dirs, pack).await?.with_context(|| format!("FTB has no pack {pack}"))?;
            let version = match version {
                Some(v) => v,
                None => {
                    p.newest_release()
                        .or_else(|| p.versions_newest_first().into_iter().next())
                        .context("this pack has no versions")?
                        .id
                }
            };
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let opts = ftb::InstallOptions::from_settings(&settings, name);
            let result = ftb::install(&dirs, pack, version, &opts, &cf, &progress).await;
            printer.abort();
            match result {
                Ok(inst) => println!("installed \"{}\" at {}", inst.name(), inst.root.display()),
                Err(e) => {
                    if let Some(m) = e.downcast_ref::<ftb::ManualDownloadsNeeded>() {
                        eprintln!(
                            "These mods must be downloaded in a browser and saved to your Downloads folder, then run this again:"
                        );
                        for d in &m.0 {
                            eprintln!("  {}  ({})  {}", d.file_name, d.mod_name, d.url);
                        }
                    }
                    return Err(e);
                }
            }
        }
        PacksCmd::FtbCheck => {
            let all = instances::load_all(&dirs);
            let updates = ftb::check_updates(&dirs, &all).await;
            let ftb_count = all.iter().filter(|i| ftb::is_ftb_instance(i)).count();
            for i in all.iter().filter(|i| updates.contains_key(&i.folder)) {
                let v = &updates[&i.folder];
                println!("{}: update available, {} (version id {})", i.name(), v.label(), v.id);
            }
            eprintln!("{ftb_count} FTB instances, {} with an update", updates.len());
        }
        PacksCmd::FtbUpdate { instance } => {
            let inst = find_instance(&dirs, &instance)?;
            policy::allows(&inst, Action::UpdateModpack).map_err(anyhow::Error::msg)?;
            let latest = ftb::check_update(&dirs, &inst).await?;
            let Some(latest) = latest else {
                println!("{} is up to date", inst.name());
                return Ok(());
            };
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let opts = ftb::InstallOptions::from_settings(&settings, None);
            let result = ftb::update(&dirs, &inst, latest.id, &opts, &cf, &progress).await;
            printer.abort();
            println!("updated \"{}\" to {}", result?.name(), latest.label());
        }
        PacksCmd::Detect { input } => {
            if let Some(url) = detect::detect_url(&input) {
                println!("{url:?}");
            } else {
                let path = PathBuf::from(&input);
                println!("{:?}", detect::detect_path(&path));
                if path.is_file() {
                    let f = detect::detect_file(&path);
                    println!("{:?} {} {:?}", f.kind, f.display_name(), f.loader);
                }
            }
        }
        PacksCmd::ImportAny { input, name } => {
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let ctx = ImportContext {
                ftb_options: ftb::InstallOptions::from_settings(&settings, None),
                ..Default::default()
            };
            let result = imports::import_input(&dirs, &ctx, &input, name.as_deref(), &progress).await;
            printer.abort();
            let inst = result?;
            println!("created \"{}\" at {}", inst.name(), inst.root.display());
        }
        PacksCmd::Scan => {
            let found = imports::scan(&imports::default_roots());
            for i in &found {
                let p = imports::preview(i).ok();
                println!(
                    "{:<30} {:<16} Minecraft {:<8} {:<18} {}",
                    i.name,
                    i.source,
                    i.minecraft_version,
                    i.loader_label(),
                    p.map(|p| format!(
                        "{} mods, {} worlds, {}",
                        p.mods,
                        p.worlds,
                        crate::packutil::dir_size(&i.game_dir) / 1_000_000
                    ))
                    .unwrap_or_default()
                );
            }
            eprintln!("{} instances found", found.len());
        }
        PacksCmd::Export { instance, format, out, name, version, author, include } => {
            let mut inst = find_instance(&dirs, &instance)?;
            policy::allows(&inst, Action::ExportModpack).map_err(anyhow::Error::msg)?;
            let accounts = crate::accounts::load(&dirs);
            let account = settings
                .last_account
                .as_ref()
                .and_then(|id| accounts.iter().find(|a| &a.uuid == id))
                .or(accounts.first())
                .map(|a| a.name().to_string());
            let mut opts = export::default_options(&inst, account.as_deref(), &settings);
            opts.format = match format.as_str() {
                "curseforge" => ExportFormat::CurseForge,
                "both" => ExportFormat::CurseForgeAndModrinth,
                "multimc" => ExportFormat::MultiMc,
                _ => ExportFormat::Modrinth,
            };
            opts.save_to = out;
            if let Some(n) = name {
                opts.name = n;
            }
            if let Some(v) = version {
                opts.version = v;
            }
            if let Some(a) = author {
                opts.author = a;
            }
            if !include.is_empty() {
                opts.overrides = include;
            }
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let scanned = export::scan_missing_mods(&mut inst, Some(&cf), true).await;
            let result = export::export(&dirs, &settings, &mut inst, &opts, &cf, &progress).await;
            printer.abort();
            if let Ok(n) = scanned {
                if n > 0 {
                    eprintln!("  {n} files that weren't in the mod list were added to it");
                }
            }
            let outcome = result?;
            for p in &outcome.paths {
                println!("wrote {}", p.display());
            }
            if !outcome.overrides_note.is_empty() {
                println!(
                    "These files were put in the archive itself (Modrinth asks for permission to distribute them):\n{}",
                    outcome.overrides_note
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn horizon_export_and_pack_update_refuse_before_any_work() {
        let root = crate::testutil::scratch("horizon-packs-cli");
        let dirs = Dirs::at(&root);
        dirs.ensure().unwrap();
        let mut inst = instances::Instance {
            folder: "Horizon".into(),
            root: dirs.instances.join("Horizon"),
            mod_count: 0,
            file: Default::default(),
        };
        crate::horizon::mark(&mut inst);
        std::fs::create_dir_all(&inst.root).unwrap();
        inst.save().unwrap();
        let before = std::fs::read(inst.root.join("instance.json")).unwrap();
        for format in ["modrinth", "curseforge", "both", "multimc", "other"] {
            let cmd = PacksCmd::Export {
                instance: inst.folder.clone(),
                format: format.into(),
                out: root.join("exports"),
                name: None,
                version: None,
                author: None,
                include: Vec::new(),
            };
            let err = run(cmd, dirs.clone()).await.unwrap_err();
            assert!(err.to_string().contains("exporting it as a Java modpack"));
            assert!(!root.join("exports").exists());
        }
        let err = run(PacksCmd::FtbUpdate { instance: inst.folder.clone() }, dirs).await.unwrap_err();
        assert!(err.to_string().contains("Java modpacks"));
        assert_eq!(std::fs::read(inst.root.join("instance.json")).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }
}
