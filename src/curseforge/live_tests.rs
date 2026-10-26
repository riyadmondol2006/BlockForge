//! Checks against the real CurseForge service. They are skipped by `cargo test` (they need the network and download
//! a few files); run them with `cargo test --lib curseforge::live_tests -- --ignored --test-threads=1`.
//! Set `BF_TEST_DIR` to a folder for the downloaded files (default: the system temp folder).

use std::path::PathBuf;

use super::api::{self, SearchQuery, Section, Sort};
use super::filter::{self, LoaderKind, Target};
use super::{CfSettings, fingerprint, install, pack};
use crate::install::Progress;
use crate::instances::{Instance, InstanceFile, LauncherBlock, LoaderVersion};
use crate::paths::Dirs;

fn work_dir(tag: &str) -> PathBuf {
    let base = std::env::var_os("BF_TEST_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("bf-cf-live-{tag}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An instance record that was never installed (no game files), enough for choosing and adding mods.
fn fake_instance(dirs: &Dirs, mc: &str, loader: &str) -> Instance {
    let root = dirs.instance_dir("Test");
    std::fs::create_dir_all(root.join("mods")).unwrap();
    let mut version = crate::model::VersionJson::default();
    version.id = mc.to_string();
    let file = InstanceFile {
        uuid: "test".into(),
        launcher: LauncherBlock {
            name: "Test".into(),
            pack: "Minecraft".into(),
            version: mc.into(),
            enable_curse_forge_integration: true,
            enable_editing_mods: true,
            loader_version: Some(LoaderVersion {
                version: "0.16.0".into(),
                raw_version: "0.16.0".into(),
                recommended: false,
                kind: loader.into(),
                downloadables: Default::default(),
            }),
            vanilla_instance: true,
            ..Default::default()
        },
        version,
    };
    let inst = Instance { folder: "Test".into(), mod_count: 0, root, file };
    inst.save().unwrap();
    inst
}

#[tokio::test]
#[ignore]
async fn search_every_section() {
    for s in Section::ALL {
        let mut q = SearchQuery::new(s);
        q.text = String::new();
        let page = api::search(&q).await.unwrap_or_else(|e| panic!("{}: {e:#}", s.label()));
        println!(
            "{:<15} {} results on the first page, {} in total, first: {}",
            s.label(),
            page.projects.len(),
            page.total,
            page.projects.first().map(|p| p.name.as_str()).unwrap_or("-")
        );
        assert_eq!(page.projects.len(), 20, "{}", s.label());
        assert!(page.has_more);
    }
    let mut q = SearchQuery::new(Section::Mods);
    q.text = "sodium".into();
    q.game_version = Some("1.21.1".into());
    q.mod_loaders = vec![super::MODLOADER_FABRIC];
    q.sort = Sort::TotalDownloads;
    let page = api::search(&q).await.unwrap();
    assert!(page.projects.iter().any(|p| p.id == 394468), "Sodium should be found");
    // second page differs from the first
    q.page = 1;
    let page2 = api::search(&q).await.unwrap();
    assert!(page2.projects.first().map(|p| p.id) != page.projects.first().map(|p| p.id));
    // other sort orders work
    for sort in Sort::ALL {
        let mut q = SearchQuery::new(Section::Mods);
        q.sort = sort;
        assert_eq!(api::search(&q).await.unwrap().projects.len(), 20);
    }
}

#[tokio::test]
#[ignore]
async fn project_files_description_changelog_categories() {
    let sodium = api::project(394468).await.unwrap();
    assert_eq!(sodium.slug, "sodium");
    assert_eq!(sodium.root_category_id(), super::CLASS_MODS);
    assert_eq!(sodium.instance_folder(), "mods");
    let files = api::files(sodium.id).await.unwrap();
    println!("sodium has {} files", files.len());
    assert!(files.len() > 50);
    assert!(files.windows(2).all(|w| w[0].id > w[1].id));
    let target = Target::new("1.21.1", LoaderKind::Fabric);
    let cf = CfSettings::default();
    let fitting = filter::files_for_target(&sodium, &files, &target, &cf);
    assert!(!fitting.is_empty());
    assert!(fitting.iter().all(|f| f.has_game_version("1.21.1")));
    let best = filter::pick_latest(&fitting, false).unwrap();
    println!("best for 1.21.1 Fabric: {} ({})", best.display_name, best.file_name);
    assert!(best.download_url.is_some());
    assert!(best.package_fingerprint > 0);
    let one = api::file(sodium.id, best.id).await.unwrap();
    assert_eq!(one.id, best.id);
    let many = api::files_by_ids(&[best.id, fitting.last().unwrap().id]).await.unwrap();
    assert_eq!(many.len(), 2);
    let desc = api::description(sodium.id).await.unwrap();
    assert!(desc.len() > 100);
    let log = api::changelog(sodium.id, best.id).await.unwrap();
    println!("changelog: {} bytes", log.len());
    let cats = api::categories_for(Section::Mods).await.unwrap();
    assert!(cats.len() > 10, "mod categories");
    for s in [Section::ResourcePacks, Section::Worlds, Section::ShaderPacks, Section::DataPacks, Section::Modpacks] {
        assert!(!api::categories_for(s).await.unwrap().is_empty(), "{}", s.label());
    }
    let by_slug = api::project_by_slug("sodium", Section::Mods).await.unwrap().unwrap();
    assert_eq!(by_slug.id, 394468);
    let many_projects = api::projects(&[394468, 306612]).await.unwrap();
    assert_eq!(many_projects.len(), 2);
    // a mod file that depends on Fabric API says so
    let xaero = api::project(263420).await.unwrap();
    let xfiles = api::files(xaero.id).await.unwrap();
    let with_dep =
        xfiles.iter().find(|f| f.dependencies.iter().any(|d| d.is_required() && d.mod_id == super::FABRIC_API_ID));
    assert!(with_dep.is_some(), "Xaero's Minimap needs Fabric API");
}

#[tokio::test]
#[ignore]
async fn fingerprint_lookup_of_a_known_jar() {
    let home = std::env::var("HOME").unwrap();
    let jar = PathBuf::from(home)
        .join(".local/share/atlauncher/instances/Minecraft263withFabric/mods/xaerominimap-fabric-26.3-26.5.1.jar");
    assert!(jar.is_file(), "sample jar missing");
    assert_eq!(fingerprint::fingerprint_file(&jar).unwrap(), 3_100_092_788);
    let found = install::identify_files(vec![jar.clone()]).await.unwrap();
    assert_eq!(found.len(), 1, "CurseForge should know this jar");
    let m = &found[0];
    println!("identified as {} / {}", m.project.name, m.file.display_name);
    assert_eq!(m.project.id, 263420);
    assert_eq!(m.file.id, 8885210);
    // the record that would be stored matches what Java stored for the same file
    let mut rec = crate::modrecord::ModRecord::new("x", "x.jar", "mods");
    install::apply_match(&mut rec, m);
    let v = serde_json::to_value(&rec).unwrap();
    let java: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(&std::env::var("HOME").unwrap())
                .join(".local/share/atlauncher/instances/Minecraft263withFabric/instance.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let java_rec =
        java["launcher"]["mods"].as_array().unwrap().iter().find(|m| m["curseForgeProjectId"] == 263420).unwrap();
    assert_eq!(v["curseForgeFile"]["fileFingerprint"], java_rec["curseForgeFile"]["fileFingerprint"]);
    let keys = |x: &serde_json::Value| -> Vec<String> { x.as_object().unwrap().keys().cloned().collect() };
    let mut ours = keys(&v["curseForgeFile"]);
    let mut theirs = keys(&java_rec["curseForgeFile"]);
    ours.sort();
    theirs.sort();
    println!("file keys ours {ours:?}\nfile keys java {theirs:?}");
    assert_eq!(ours, theirs, "the stored file object has the same fields as Java's");
    let mut ours = keys(&v["curseForgeProject"]);
    let mut theirs = keys(&java_rec["curseForgeProject"]);
    ours.sort();
    theirs.sort();
    println!("project keys ours {ours:?}\nproject keys java {theirs:?}");
    assert_eq!(ours, theirs, "the stored project object has the same fields as Java's");
    assert_eq!(v["curseForgeFile"]["dependencies"], java_rec["curseForgeFile"]["dependencies"]);
}

#[tokio::test]
#[ignore]
async fn install_a_mod_with_its_dependencies_and_check_updates() {
    let dir = work_dir("mod");
    let dirs = Dirs::at(&dir);
    dirs.ensure().unwrap();
    let inst = fake_instance(&dirs, "1.21.1", "Fabric");
    let cf = CfSettings::default();
    let progress = Progress::new();
    // Xaero's Minimap needs Fabric API
    let project = api::project(263420).await.unwrap();
    let report = install::install_latest(&dirs, &cf, &inst, &project, &progress, None).await.unwrap();
    println!("{}", report.summary());
    let reloaded = crate::instances::load_one(&inst.root).unwrap();
    let records = reloaded.mod_records();
    assert!(records.iter().any(|r| r.curse_forge_project_id == Some(263420)));
    assert!(
        records.iter().any(|r| r.curse_forge_project_id == Some(super::FABRIC_API_ID)),
        "the required Fabric API came along"
    );
    for r in &records {
        assert!(inst.root.join("mods").join(&r.file).is_file(), "{} is on disk", r.file);
        assert!(r.curse_forge_project.is_some() && r.curse_forge_file.is_some());
        assert!(r.user_added && r.was_selected);
    }
    // the instance.json holds the Java-style keys
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(inst.root.join("instance.json")).unwrap()).unwrap();
    let first = &raw["launcher"]["mods"][0];
    assert!(first["curseForgeProjectId"].is_number() && first["curseForgeFileId"].is_number());
    assert!(first["curseForgeFile"].get("downloadUrl").is_none(), "Java does not save downloadUrl");

    // nothing is missing any more
    let deps = install::dependencies_of(
        &dirs,
        &cf,
        &reloaded,
        &records
            .iter()
            .find(|r| r.curse_forge_project_id == Some(263420))
            .and_then(|r| serde_json::from_value::<super::File>(r.curse_forge_file.clone().unwrap()).ok())
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(deps.iter().all(|d| !d.is_required()), "required dependencies are installed: {deps:?}");

    // reinstall of an older file is detected as having an update
    let files = api::files(263420).await.unwrap();
    let target = Target::from_instance(&reloaded, &dirs, &cf);
    let fitting = filter::files_for_target(&project, &files, &target, &cf);
    assert!(fitting.len() >= 2, "need two files to test an update");
    let old = fitting.last().unwrap().clone();
    let mut rec_old = install::record_for(&project, &old);
    rec_old.file = "xaero-old.jar".into();
    std::fs::write(inst.root.join("mods").join("xaero-old.jar"), b"old").unwrap();
    install::update_instance(&inst.root, |i| i.upsert_mod_record(rec_old)).unwrap();
    let again = crate::instances::load_one(&inst.root).unwrap();
    let updates = install::check_updates(&dirs, &cf, &again).await.unwrap();
    println!(
        "updates: {:?}",
        updates.iter().map(|u| (&u.installed_name, u.installed_file_id, u.latest.id)).collect::<Vec<_>>()
    );
    let u = updates.iter().find(|u| u.project.id == 263420).expect("an update for the old Xaero file");
    assert!(u.latest.id > old.id);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore]
async fn modpack_manifest_download_and_populate() {
    let dir = work_dir("pack");
    let dirs = Dirs::at(&dir);
    dirs.ensure().unwrap();
    let cf = CfSettings::default();
    let progress = Progress::new();
    let project = api::project(396246).await.unwrap(); // Fabulously Optimized
    assert_eq!(project.class_id, super::CLASS_MODPACKS);
    let file = pack::choose_pack_file(&cf, &project).await.unwrap();
    println!("pack file {} ({} bytes), modpack: {}", file.display(), file.file_length, file.is_modpack());
    let zip = super::install::fetch_file(&dirs, &cf, &project, &file, &progress, None, false).await.unwrap().unwrap();
    let manifest = pack::read_manifest(&zip).unwrap();
    println!(
        "manifest: {} {} for Minecraft {} with {:?}, {} files",
        manifest.name,
        manifest.version.clone().unwrap_or_default(),
        manifest.minecraft.version,
        manifest.minecraft.mod_loaders,
        manifest.files.len()
    );
    assert!(!manifest.files.is_empty());
    let (loader, version) = pack::resolve_loader(&dirs, &manifest).await.unwrap();
    println!("loader {loader:?} {version:?}");
    assert_eq!(loader, crate::loaders::Loader::Fabric);
    let root = dirs.instance_dir("Pack");
    std::fs::create_dir_all(&root).unwrap();
    let populated =
        pack::populate(&dirs, &cf, &root, &manifest.minecraft.version, &manifest, &zip, &progress, None).await.unwrap();
    println!("{} records, {} override files", populated.records.len(), populated.override_paths.len());
    assert!(populated.records.len() >= manifest.files.len());
    let missing: Vec<_> = populated
        .records
        .iter()
        .filter(|r| r.curse_forge_project_id.is_some() && !r.skipped)
        .filter(|r| {
            let folder = match r.kind.as_str() {
                "resourcepack" => "resourcepacks",
                "shaderpack" => "shaderpacks",
                "datapack" => "datapacks",
                _ => "mods",
            };
            !root.join(folder).join(&r.file).is_file()
        })
        .map(|r| r.file.clone())
        .collect();
    assert!(missing.is_empty(), "files missing: {missing:?}");

    // the launcher block fields, then the update check on that instance
    let mut inst = fake_instance(&dirs, &manifest.minecraft.version, "Fabric");
    inst.file.launcher.extras.insert("curseForgeProject".into(), serde_json::to_value(&project).unwrap());
    let mut older = api::files(project.id).await.unwrap();
    older.sort_by_key(|f| f.id);
    let first = older.first().unwrap().clone();
    inst.file.launcher.extras.insert("curseForgeFile".into(), serde_json::to_value(&first).unwrap());
    inst.save().unwrap();
    let inst = crate::instances::load_one(&inst.root).unwrap();
    assert!(pack::is_curseforge_pack(&inst));
    let upd = pack::check_pack_update(&dirs, &cf, &inst).await.unwrap().expect("the oldest file has an update");
    println!("update from {} to {} ({})", first.id, upd.latest.id, upd.latest.display());
    assert!(upd.latest.id > first.id);
    assert!(!upd.ignored);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore]
async fn restricted_pack_file_has_no_download_url() {
    // Fabulously Refined's file is one the author does not let launchers fetch
    let project = api::project(1679520).await.unwrap();
    let files = api::files(project.id).await.unwrap();
    let f = files.first().unwrap();
    println!("{} download_url={:?} browser page={}", f.display_name, f.download_url, project.browser_download_url(f));
    assert!(f.download_url.is_none());
    assert!(project.browser_download_url(f).contains("/download/"));
}

/// Installs a whole modpack as a new instance and then updates it. Needs `BF_FULL_DIR`: a data folder that already
/// holds the libraries, assets and runtimes of the pack's Minecraft version, so nothing big is downloaded.
#[tokio::test]
#[ignore]
async fn full_pack_install_and_update() {
    let Some(base) = std::env::var_os("BF_FULL_DIR").map(PathBuf::from) else {
        eprintln!("BF_FULL_DIR is not set, skipping");
        return;
    };
    let dirs = Dirs::at(&base);
    dirs.ensure().unwrap();
    let cf = CfSettings::default();
    let progress = Progress::new();
    let project = api::project(396246).await.unwrap(); // Fabulously Optimized
    let mut files = api::files(project.id).await.unwrap();
    files.retain(|f| f.game_versions.iter().any(|g| g == "26.3") && f.is_modpack());
    files.sort_by_key(|f| f.id);
    let file = files.first().expect("a pack file for 26.3").clone();
    println!("installing {}", file.display());
    let inst = pack::install_pack(
        &dirs,
        &cf,
        pack::PackSource::Project { project: project.clone(), file: Some(file.clone()) },
        Some("FO Test".into()),
        &progress,
        None,
    )
    .await
    .unwrap();
    println!("instance {} mc {} loader {:?}", inst.name(), inst.mc_version(), inst.loader());
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(inst.root.join("instance.json")).unwrap()).unwrap();
    let l = &raw["launcher"];
    assert_eq!(l["pack"], "Fabulously Optimized");
    assert_eq!(l["externalPackId"], 396246);
    assert_eq!(l["curseForgeFile"]["id"], file.id);
    assert!(l["curseForgeProject"]["slug"] == "fabulously-optimized");
    assert!(l["curseForgeProjectDescription"].as_str().is_some_and(|d| d.len() > 100));
    assert!(l["overridePaths"].as_array().is_some_and(|a| !a.is_empty()));
    assert_eq!(l["isPlayable"], true);
    assert!(l["mods"].as_array().unwrap().len() >= 40);
    assert!(inst.root.join("instance.png").is_file());
    assert!(pack::is_curseforge_pack(&inst));
    let mods_on_disk = std::fs::read_dir(inst.root.join("mods")).unwrap().count();
    println!("{} mod files on disk", mods_on_disk);
    assert!(mods_on_disk >= 30);

    // a mod the player added by hand survives the update
    std::fs::write(inst.root.join("mods").join("my-own-mod.jar"), b"mine").unwrap();
    std::fs::write(inst.root.join("options.txt"), b"keep=me").unwrap();
    install::update_instance(&inst.root, |i| {
        let mut r = crate::modrecord::ModRecord::new("Mine", "my-own-mod.jar", "mods");
        r.user_added = true;
        i.upsert_mod_record(r);
    })
    .unwrap();
    // pretend the instance came from the oldest file of the pack, so a newer one exists
    let oldest =
        api::files(project.id).await.unwrap().into_iter().filter(|f| f.is_modpack()).min_by_key(|f| f.id).unwrap();
    install::update_instance(&inst.root, |i| {
        i.file.launcher.extras.insert("curseForgeFile".into(), serde_json::to_value(&oldest).unwrap());
    })
    .unwrap();
    let inst = crate::instances::load_one(&inst.root).unwrap();
    let update = pack::check_pack_update(&dirs, &cf, &inst).await.unwrap().expect("a newer file exists");
    println!("updating to {}", update.latest.display());
    let updated = pack::update_pack(&dirs, &cf, &inst, &update.latest, &progress, None).await.unwrap();
    assert_eq!(pack::pack_file(&updated).unwrap().id, update.latest.id);
    assert!(updated.root.join("mods").join("my-own-mod.jar").is_file());
    assert_eq!(std::fs::read_to_string(updated.root.join("options.txt")).unwrap(), "keep=me");
    assert_eq!(updated.file.uuid, inst.file.uuid);
    assert!(updated.mod_records().iter().any(|r| r.file == "my-own-mod.jar" && r.user_added));
    println!(
        "updated: mc {} loader {:?}, {} records",
        updated.mc_version(),
        updated.loader(),
        updated.mod_records().len()
    );
}

/// The whole hand-download path on a real restricted file: the API gives no address, so the launcher waits; the
/// "browser" (here: a download from CurseForge's file server, saved under a browser-style name into a pretend
/// Downloads folder) delivers the file and it is recognised by its hash.
#[tokio::test]
#[ignore]
async fn restricted_file_is_picked_up_from_the_downloads_folder() {
    let dir = work_dir("hand");
    let dirs = Dirs::at(dir.join("data"));
    dirs.ensure().unwrap();
    let user = dir.join("Downloads");
    std::fs::create_dir_all(&user).unwrap();
    let cf = CfSettings::default();
    let project = api::project(1679520).await.unwrap(); // Fabulously Refined
    let files = api::files(project.id).await.unwrap();
    let file = files.first().unwrap().clone();
    assert!(file.download_url.is_none());
    let hub = super::browser::BrowserHub::new(&dirs, Some(&user));
    let progress = Progress::new();

    let (u, f2) = (user.clone(), file.clone());
    let browser = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        // a half-written download first, as browsers do
        std::fs::write(u.join(format!("{}.crdownload", f2.file_name)), b"partial").unwrap();
        let url = format!("https://edge.forgecdn.net/files/{}/{}/{}", f2.id / 1000, f2.id % 1000, f2.file_name);
        let bytes = crate::http::get_bytes(&url).await.unwrap();
        // the browser saved it with a "(1)" added
        let renamed = f2.file_name.replace(".zip", " (1).zip");
        std::fs::write(u.join(&renamed), bytes).unwrap();
    });
    let got = super::install::fetch_file(&dirs, &cf, &project, &file, &progress, Some(&hub), false)
        .await
        .unwrap()
        .expect("found");
    browser.await.unwrap();
    println!("picked up as {}", got.display());
    assert_eq!(got, dirs.base.join("downloads").join(&file.file_name));
    assert!(super::browser::contents_match(&got, file.md5(), file.sha1()));
    assert!(!user.join(file.file_name.replace(".zip", " (1).zip")).exists(), "the file was moved, not copied");
    let manifest = pack::read_manifest(&got).unwrap();
    println!(
        "hand-downloaded pack: {} {} for {}",
        manifest.name,
        manifest.version.unwrap_or_default(),
        manifest.minecraft.version
    );
    // and skipping: nothing arrives, the player skips
    let hub2 = super::browser::BrowserHub::new(&dirs, Some(&user));
    let h2 = hub2.clone();
    let skipper = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        h2.skip_remaining();
    });
    let mut other = file.clone();
    other.file_name = "never-downloaded.zip".into();
    other.hashes.clear();
    other.file_length = 0;
    let skipped =
        super::install::fetch_file(&dirs, &cf, &project, &other, &progress, Some(&hub2), false).await.unwrap();
    skipper.await.unwrap();
    assert!(skipped.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A big pack (RLCraft, about 190 files): every file a launcher may fetch arrives and passes its checksum, and how
/// fast. Files the authors only allow in a browser are left: the run stops once only those are waiting.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn a_big_modpack_downloads_completely() {
    use std::sync::atomic::Ordering;
    let dir = work_dir("rlcraft");
    let dirs = Dirs::at(&dir);
    dirs.ensure().unwrap();
    let cf = CfSettings::default();
    let project = api::project(285109).await.unwrap(); // RLCraft
    let file = pack::choose_pack_file(&cf, &project).await.unwrap();
    let zip =
        super::install::fetch_file(&dirs, &cf, &project, &file, &Progress::new(), None, false).await.unwrap().unwrap();
    let manifest = pack::read_manifest(&zip).unwrap();
    println!("{}: {} files for Minecraft {}", project.name, manifest.files.len(), manifest.minecraft.version);
    let root = dirs.instance_dir("Pack");
    std::fs::create_dir_all(&root).unwrap();
    let progress = Progress::new();
    let started = std::time::Instant::now();
    let (d, c, r, m, z, p) = (dirs.clone(), cf.clone(), root.clone(), manifest.clone(), zip.clone(), progress.clone());
    let job = tokio::spawn(async move { pack::populate(&d, &c, &r, &m.minecraft.version, &m, &z, &p, None).await });
    let outcome = loop {
        if job.is_finished() {
            break match job.await.unwrap() {
                Ok(p) => format!("done, {} records", p.records.len()),
                Err(e) => format!("failed: {e:#}"),
            };
        }
        if progress.stage_text().contains("browser") {
            job.abort();
            break "every launcher download done; the rest waits for a browser".to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let secs = started.elapsed().as_secs_f64();
    let bytes = progress.bytes_done.load(Ordering::Relaxed);
    let (done, total) = (progress.files_done.load(Ordering::Relaxed), progress.files_total.load(Ordering::Relaxed));
    println!(
        "{outcome}: {done} of {total} files, {:.1} MB in {secs:.1} s = {:.1} MB/s",
        bytes as f64 / 1e6,
        bytes as f64 / 1e6 / secs
    );
    assert!(!outcome.starts_with("failed"), "{outcome}");
    let _ = std::fs::remove_dir_all(&dir);
}
