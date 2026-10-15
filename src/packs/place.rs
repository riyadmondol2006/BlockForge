//! Putting pack files where they belong inside an instance: each downloaded mod goes to a folder chosen by its
//! `type`, exactly as the Java edition does it (`Mod.install`), plus the follow-up steps that work on those files
//! (actions, case conversion), what is kept and deleted when a pack is reinstalled, and the backup of files a
//! pack update must not lose.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::fsx;
use super::json::{
    ActionAfter, ActionFolder, ActionKind, CaseKind, DecompKind, ExtractTo, ModKind, PackAction, PackJson, PackMod,
    PathRules,
};
use crate::modrecord::ModRecord;

/// Where an install happens.
#[derive(Clone, Debug)]
pub struct Site {
    /// The instance (or server) folder.
    pub root: PathBuf,
    /// The scratch folder for this install.
    pub temp: PathBuf,
    /// `<data>/downloads`
    pub downloads: PathBuf,
    /// The Minecraft version id (the `dependency` type puts files in `mods/<id>`).
    pub mc: String,
    pub is_server: bool,
}

/// The file a mod was downloaded to, under the name used on this kind of install.
pub fn downloaded_name(m: &PackMod, is_server: bool) -> String {
    if m.uses_server_file(is_server) { m.server_file.clone().unwrap_or_else(|| m.file_name()) } else { m.file_name() }
}

fn type_for(m: &PackMod, is_server: bool) -> Option<ModKind> {
    if m.uses_server_file(is_server) { m.server_type.or(m.kind) } else { m.kind }
}

/// Copy a downloaded mod to its place in the instance. Returns the folder it was put in.
pub fn install_mod(site: &Site, m: &PackMod) -> Result<()> {
    let name = downloaded_name(m, site.is_server);
    let file = site.downloads.join(&name);
    let root = &site.root;
    let kind = type_for(m, site.is_server);
    let Some(kind) = kind else { bail!("{} has no type, so there is no way to install it", m.name) };
    let into = |dir: PathBuf| -> Result<()> {
        fsx::copy_file(&file, &dir, false)?;
        Ok(())
    };
    match kind {
        ModKind::Jar | ModKind::Forge => {
            if site.is_server && kind == ModKind::Forge {
                into(root.clone())?;
            } else if site.is_server && kind == ModKind::Jar {
                fsx::unzip(&file, &site.temp.join("jar"))?;
            } else {
                into(root.join("jarmods"))?;
            }
        }
        ModKind::Mcpc => {
            if site.is_server {
                into(root.clone())?;
            }
        }
        ModKind::Texturepack => into(root.join("texturepacks"))?,
        ModKind::Resourcepack => into(root.join("resourcepacks"))?,
        ModKind::Datapack => into(root.join("datapacks"))?,
        ModKind::Texturepackextract => fsx::unzip(&file, &root.join("texturepacks").join("extracted"))?,
        ModKind::Resourcepackextract => fsx::unzip(&file, &root.join("resourcepacks").join("extracted"))?,
        ModKind::Millenaire => {
            let tmp = site.temp.join("millenaire").join(m.safe_name());
            fsx::delete_path(&tmp);
            fsx::unzip(&file, &tmp)?;
            for folder in std::fs::read_dir(&tmp)?.flatten() {
                if !folder.path().is_dir() {
                    continue;
                }
                for dir in std::fs::read_dir(folder.path())?.flatten() {
                    if dir.path().is_dir() {
                        fsx::copy_dir(&dir.path(), &root.join("mods"), true)?;
                    }
                }
            }
            fsx::delete_path(&tmp);
        }
        ModKind::Mods => match m.path.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => into(root.join(p))?,
            None => into(root.join("mods"))?,
        },
        ModKind::Ic2lib => into(root.join("mods").join("ic2"))?,
        ModKind::Flan => into(root.join("Flan"))?,
        ModKind::Denlib => into(root.join("mods").join("denlib"))?,
        ModKind::Dependency | ModKind::Depandency => into(root.join("mods").join(&site.mc))?,
        ModKind::Plugins => into(root.join("plugins"))?,
        ModKind::Coremods => into(root.join("coremods"))?,
        ModKind::Shaderpack => into(root.join("shaderpacks"))?,
        ModKind::Extract => {
            let tmp = site.temp.join("extract").join(m.safe_name());
            fsx::delete_path(&tmp);
            fsx::unzip(&file, &tmp)?;
            let folder = tmp.join(m.extract_folder.as_deref().unwrap_or(""));
            match m.extract_to {
                Some(ExtractTo::Coremods) => fsx::copy_dir(&folder, &root.join("coremods"), false)?,
                Some(ExtractTo::Mods) => fsx::copy_dir(&folder, &root.join("mods"), false)?,
                Some(ExtractTo::Root) => fsx::copy_dir(&folder, root, false)?,
                None => bail!("No known way to extract mod {}", m.name),
            }
            fsx::delete_path(&tmp);
        }
        ModKind::Decomp => {
            let tmp = site.temp.join("decomp").join(m.safe_name());
            fsx::delete_path(&tmp);
            fsx::unzip(&file, &tmp)?;
            let inner = tmp.join(m.decomp_file.as_deref().unwrap_or(""));
            if inner.exists() {
                let copy_into = |dir: PathBuf| -> Result<()> {
                    if inner.is_file() {
                        fsx::copy_file(&inner, &dir, false)?;
                    } else {
                        fsx::copy_dir(&inner, &dir, false)?;
                    }
                    Ok(())
                };
                match m.decomp_type {
                    Some(DecompKind::Coremods) => copy_into(root.join("coremods"))?,
                    Some(DecompKind::Jar) => {
                        if inner.is_file() {
                            fsx::copy_file(&inner, &root.join("jarmods"), false)?;
                        } else {
                            fsx::zip_dir(&inner, &root.join("jarmods").join(format!("{}.zip", m.safe_name())))?;
                        }
                    }
                    Some(DecompKind::Mods) => copy_into(root.join("mods"))?,
                    Some(DecompKind::Root) => copy_into(root.clone())?,
                    None => bail!("No known way to decomp mod {}", m.name),
                }
            } else {
                bail!("Couldn't find decomp file {} for mod {}", m.decomp_file.as_deref().unwrap_or(""), m.name);
            }
            fsx::delete_path(&tmp);
        }
    }
    Ok(())
}

/// The file a mod ends up as inside the instance (used by actions). `None` when the type has no single file.
pub fn installed_file(site: &Site, m: &PackMod) -> Option<PathBuf> {
    let name = downloaded_name(m, site.is_server);
    let kind = type_for(m, site.is_server)?;
    let root = &site.root;
    let base = match kind {
        ModKind::Jar | ModKind::Forge => {
            if site.is_server && kind == ModKind::Forge {
                root.clone()
            } else {
                root.join("jarmods")
            }
        }
        ModKind::Mcpc => {
            if site.is_server {
                root.clone()
            } else {
                return None;
            }
        }
        ModKind::Texturepack => root.join("texturepacks"),
        ModKind::Resourcepack => root.join("resourcepacks"),
        ModKind::Datapack => root.join("datapacks"),
        ModKind::Mods => root.join("mods"),
        ModKind::Ic2lib => root.join("mods").join("ic2"),
        ModKind::Denlib => root.join("mods").join("denlib"),
        ModKind::Plugins => root.join("plugins"),
        ModKind::Coremods => root.join("coremods"),
        ModKind::Shaderpack => root.join("shaderpacks"),
        _ => return None,
    };
    let base = match m.path.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => root.join(p),
        None => base,
    };
    Some(base.join(name))
}

/// Run the pack's actions: join mods into one zip, rename a file, and delete the originals afterwards.
pub fn run_actions(site: &Site, json: &PackJson, installed: &[PackMod]) -> Result<()> {
    for action in &json.actions {
        run_action(site, action, installed)?;
    }
    Ok(())
}

fn run_action(site: &Site, action: &PackAction, installed: &[PackMod]) -> Result<()> {
    if (site.is_server && !action.server) || (!site.is_server && !action.client) {
        return Ok(());
    }
    let mods: Vec<&PackMod> =
        action.mod_names.iter().filter_map(|n| installed.iter().find(|m| m.name.eq_ignore_ascii_case(n))).collect();
    let work = site.temp.join("actions");
    fsx::delete_contents(&work);
    match action.action {
        Some(ActionKind::CreateZip) => {
            if action.mod_names.len() >= 2 {
                for m in &mods {
                    if let Some(f) = installed_file(site, m) {
                        fsx::unzip(&f, &work)?;
                    }
                }
                let save_as = action.save_as.clone().unwrap_or_default();
                let target = match action.kind {
                    Some(ActionFolder::Mods) => Some(site.root.join("mods")),
                    Some(ActionFolder::Coremods) => Some(site.root.join("coremods")),
                    Some(ActionFolder::Jar) => Some(site.root.join("jarmods")),
                    None => None,
                };
                if let Some(dir) = target.filter(|_| !save_as.is_empty()) {
                    std::fs::create_dir_all(&work)?;
                    fsx::zip_dir(&work, &dir.join(save_as))?;
                }
            }
        }
        Some(ActionKind::Rename) => {
            if mods.len() == 1 {
                if let (Some(from), Some(save_as)) = (installed_file(site, mods[0]), action.save_as.as_deref()) {
                    if let Some(parent) = from.parent() {
                        if from.is_file() {
                            fsx::copy_file(&from, &parent.join(save_as), true)?;
                            fsx::delete_path(&from);
                        }
                    }
                }
            }
        }
        None => {}
    }
    if action.after == Some(ActionAfter::Delete) {
        for m in &mods {
            if let Some(f) = installed_file(site, m) {
                fsx::delete_path(&f);
            }
        }
    }
    Ok(())
}

/// The file name a mod gets under the pack's `caseAllFiles` setting. Only mods of type `mods` change; the
/// extension keeps its case.
pub fn cased_file_name(json: &PackJson, m: &PackMod) -> String {
    let file = m.file_name();
    match (m.kind, json.case_all_files) {
        (Some(ModKind::Mods), Some(CaseKind::Upper)) => fsx::recase_name(&file, true),
        (Some(ModKind::Mods), Some(CaseKind::Lower)) => fsx::recase_name(&file, false),
        _ => file,
    }
}

/// Rename the installed mod files to the case the pack asked for so the names match what the instance remembers.
pub fn convert_case(site: &Site, json: &PackJson, installed: &[PackMod]) {
    let Some(case) = json.case_all_files else { return };
    let Ok(rd) = std::fs::read_dir(site.root.join("mods")) else { return };
    let wanted: Vec<&PackMod> = installed.iter().filter(|m| m.kind == Some(ModKind::Mods)).collect();
    for e in rd.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(m) = wanted.iter().find(|m| m.file_name().eq_ignore_ascii_case(&name)) {
            let target = fsx::recase_name(&m.file_name(), case == CaseKind::Upper);
            if target != name {
                let _ = std::fs::rename(&path, path.with_file_name(target));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// reinstalling

/// Files and folders a reinstall must carry over even though the pack rewrites everything: some well-known
/// mod data, the player's options, and whatever the pack's `keeps` lists.
#[derive(Default, Debug)]
pub struct Saved {
    pub reis: bool,
    pub zans: bool,
    pub nei: bool,
    pub options: bool,
    pub servers: bool,
    pub portal_gun: bool,
}

pub fn backup_select_files(site: &Site, reinstall: bool, keeps: Option<&PathRules>) -> Saved {
    let mut saved = Saved::default();
    let (root, temp) = (&site.root, &site.temp);
    let _ = std::fs::create_dir_all(temp);
    let reis = root.join("mods").join("rei_minimap");
    if reis.is_dir() && fsx::copy_dir(&reis, temp, true).is_ok() {
        saved.reis = true;
    }
    let zans = root.join("mods").join("VoxelMods");
    if zans.is_dir() && fsx::copy_dir(&zans, temp, true).is_ok() {
        saved.zans = true;
    }
    let nei = root.join("config").join("NEI.cfg");
    if nei.is_file() && fsx::copy_file(&nei, temp, false).is_ok() {
        saved.nei = true;
    }
    let options = root.join("options.txt");
    if options.is_file() && fsx::copy_file(&options, temp, false).is_ok() {
        saved.options = true;
    }
    let servers = root.join("servers.dat");
    if servers.is_file() && fsx::copy_file(&servers, temp, false).is_ok() {
        saved.servers = true;
    }
    let pg = root.join("mods").join("PortalGunSounds.pak");
    if pg.is_file() && fsx::copy_file(&pg, temp, false).is_ok() {
        saved.portal_gun = true;
    }
    if reinstall {
        if let Some(keeps) = keeps {
            for k in keeps.files.iter().flatten().filter(|k| k.keep_allowed()) {
                let from = k.resolve(root);
                if from.is_file() {
                    let _ = fsx::copy_file(&from, &k.resolve_flat(&temp.join("keeps")), true);
                }
            }
            for k in keeps.folders.iter().flatten().filter(|k| k.keep_allowed()) {
                let from = k.resolve(root);
                if from.is_dir() {
                    let _ = fsx::copy_dir(&from, &k.resolve_flat(&temp.join("keeps")), false);
                }
            }
        }
    }
    saved
}

pub fn restore_select_files(site: &Site, saved: &Saved, reinstall: bool, keeps: Option<&PathRules>) {
    let (root, temp) = (&site.root, &site.temp);
    if saved.reis {
        let _ = fsx::copy_dir(&temp.join("rei_minimap"), &root.join("mods").join("rei_minimap"), false);
    }
    if saved.zans {
        let _ = fsx::copy_dir(&temp.join("VoxelMods"), &root.join("mods").join("VoxelMods"), false);
    }
    if saved.nei {
        let _ = fsx::copy_file(&temp.join("NEI.cfg"), &root.join("config").join("NEI.cfg"), true);
    }
    if saved.options {
        let _ = fsx::copy_file(&temp.join("options.txt"), &root.join("options.txt"), true);
    }
    if saved.servers {
        let _ = fsx::copy_file(&temp.join("servers.dat"), &root.join("servers.dat"), true);
    }
    if saved.portal_gun {
        let _ = fsx::copy_file(&temp.join("PortalGunSounds.pak"), &root.join("mods").join("PortalGunSounds.pak"), true);
    }
    if reinstall {
        if let Some(keeps) = keeps {
            for k in keeps.files.iter().flatten().filter(|k| k.keep_allowed()) {
                let from = k.resolve_flat(&temp.join("keeps"));
                if from.is_file() {
                    let _ = fsx::copy_file(&from, &k.resolve(root), true);
                }
            }
            for k in keeps.folders.iter().flatten().filter(|k| k.keep_allowed()) {
                let from = k.resolve_flat(&temp.join("keeps"));
                if from.is_dir() {
                    let _ = fsx::copy_dir(&from, &k.resolve(root), false);
                }
            }
        }
    }
}

/// The names of the files the pack installed earlier for one kind of mod (`getPackMods`).
pub fn pack_files(records: &[ModRecord], kinds: &[&str]) -> Vec<String> {
    records.iter().filter(|r| !r.user_added && kinds.contains(&r.kind.as_str())).map(|r| r.file.clone()).collect()
}

/// Everything the Java edition's `prepareFilesystem` does for a reinstall or a server: delete what the previous
/// install of the pack put there (and only that), then make the folders that are needed.
pub fn prepare_filesystem(
    site: &Site,
    reinstall: bool,
    records: &[ModRecord],
    override_paths: &[String],
    deletes: Option<&PathRules>,
) {
    let root = &site.root;
    if reinstall || site.is_server {
        fsx::delete_path(&root.join("bin"));
        // the config folder is only wiped when the previous install did not record which files it wrote
        if root.join("config").is_dir() && override_paths.is_empty() {
            fsx::delete_path(&root.join("config"));
        }
        if reinstall {
            if root.join("mods").is_dir() {
                fsx::delete_with_filter(&root.join("mods"), &pack_files(records, &["mods"]), true);
            }
            if root.join("coremods").is_dir() {
                fsx::delete_with_filter(&root.join("coremods"), &pack_files(records, &["coremods"]), true);
            }
            if root.join("jarmods").is_dir() {
                fsx::delete_with_filter(&root.join("jarmods"), &pack_files(records, &["jar"]), true);
                fsx::delete_with_filter(&root.join("jarmods"), &pack_files(records, &["forge"]), true);
            }
            for rel in override_paths {
                if !super::fsx::is_safe_relative(rel) {
                    continue;
                }
                let p = root.join(rel);
                if p.exists() {
                    fsx::delete_path(&p);
                    fsx::delete_empty_parents(&p, root);
                }
            }
        } else {
            fsx::delete_path(&root.join("mods"));
        }
        if reinstall {
            fsx::delete_path(&root.join("texturepacks").join("TexturePack.zip"));
            fsx::delete_path(&root.join("resourcepacks").join("ResourcePack.zip"));
        } else {
            fsx::delete_path(&root.join("libraries"));
        }
        if reinstall {
            if let Some(d) = deletes {
                for rule in d.files.iter().flatten().chain(d.folders.iter().flatten()) {
                    if rule.delete_allowed() {
                        let p = rule.resolve(root);
                        if p.exists() {
                            fsx::delete_path(&p);
                        }
                    }
                }
            }
        }
    }
    let mut dirs: Vec<PathBuf> = vec![root.clone(), root.join("mods"), site.temp.clone()];
    if site.is_server {
        dirs.push(root.join("libraries"));
    } else {
        dirs.extend([root.join("disabledmods"), site.temp.join("loader"), root.join("jarmods")]);
    }
    for d in dirs {
        let _ = std::fs::create_dir_all(d);
    }
}

/// Paths (relative, with `/`) of every file below `dir`.
pub fn relative_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::json::{PackAction, PathRule};

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bf-place-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn site(base: &Path) -> Site {
        let s = Site {
            root: base.join("inst"),
            temp: base.join("temp"),
            downloads: base.join("downloads"),
            mc: "1.7.10".into(),
            is_server: false,
        };
        for d in [&s.root, &s.temp, &s.downloads] {
            std::fs::create_dir_all(d).unwrap();
        }
        s
    }

    fn pm(name: &str, file: &str, kind: ModKind) -> PackMod {
        PackMod { name: name.into(), file: file.into(), kind: Some(kind), ..Default::default() }
    }

    fn put(s: &Site, file: &str, content: &str) {
        std::fs::write(s.downloads.join(file), content).unwrap();
    }

    #[test]
    fn each_type_goes_to_its_folder() {
        let base = tmp("types");
        let s = site(&base);
        let cases = [
            (ModKind::Mods, "mods/a.jar"),
            (ModKind::Jar, "jarmods/a.jar"),
            (ModKind::Forge, "jarmods/a.jar"),
            (ModKind::Resourcepack, "resourcepacks/a.jar"),
            (ModKind::Texturepack, "texturepacks/a.jar"),
            (ModKind::Datapack, "datapacks/a.jar"),
            (ModKind::Ic2lib, "mods/ic2/a.jar"),
            (ModKind::Denlib, "mods/denlib/a.jar"),
            (ModKind::Flan, "Flan/a.jar"),
            (ModKind::Dependency, "mods/1.7.10/a.jar"),
            (ModKind::Depandency, "mods/1.7.10/a.jar"),
            (ModKind::Plugins, "plugins/a.jar"),
            (ModKind::Coremods, "coremods/a.jar"),
            (ModKind::Shaderpack, "shaderpacks/a.jar"),
        ];
        put(&s, "a.jar", "x");
        for (kind, expect) in cases {
            install_mod(&s, &pm("A", "a.jar", kind)).unwrap();
            assert!(s.root.join(expect).is_file(), "{kind:?} should land in {expect}");
        }
        // a mod with its own path goes there
        let mut custom = pm("A", "a.jar", ModKind::Mods);
        custom.path = Some("mods/special".into());
        install_mod(&s, &custom).unwrap();
        assert!(s.root.join("mods/special/a.jar").is_file());
        // mcpc and unknown types do nothing on a client
        install_mod(&s, &pm("A", "a.jar", ModKind::Mcpc)).unwrap();
        assert!(!s.root.join("a.jar").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn extract_and_decomp_unpack_the_right_part() {
        let base = tmp("extract");
        let s = site(&base);
        let src = base.join("src");
        std::fs::create_dir_all(src.join("inner").join("data")).unwrap();
        std::fs::write(src.join("inner").join("data").join("file.txt"), "hi").unwrap();
        std::fs::write(src.join("inner").join("lib.jar"), "jar").unwrap();
        fsx::zip_dir(&src, &s.downloads.join("pack.zip")).unwrap();
        let mut ex = pm("Extractor", "pack.zip", ModKind::Extract);
        ex.extract_to = Some(ExtractTo::Root);
        ex.extract_folder = Some("inner".into());
        install_mod(&s, &ex).unwrap();
        assert!(s.root.join("data/file.txt").is_file());
        assert!(s.root.join("lib.jar").is_file());
        let mut de = pm("Decomp", "pack.zip", ModKind::Decomp);
        de.decomp_file = Some("inner/lib.jar".into());
        de.decomp_type = Some(DecompKind::Mods);
        install_mod(&s, &de).unwrap();
        assert!(s.root.join("mods/lib.jar").is_file());
        let mut missing = pm("Gone", "pack.zip", ModKind::Decomp);
        missing.decomp_file = Some("nope".into());
        missing.decomp_type = Some(DecompKind::Mods);
        assert!(install_mod(&s, &missing).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn actions_join_and_rename() {
        let base = tmp("actions");
        let s = site(&base);
        std::fs::create_dir_all(s.root.join("mods")).unwrap();
        for (n, content) in [("one.zip", "1"), ("two.zip", "2")] {
            let d = base.join(format!("{n}-dir"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("{n}.txt")), content).unwrap();
            fsx::zip_dir(&d, &s.root.join("mods").join(n)).unwrap();
        }
        let installed = vec![pm("One", "one.zip", ModKind::Mods), pm("Two", "two.zip", ModKind::Mods)];
        let json = PackJson {
            actions: vec![PackAction {
                mod_names: vec!["One".into(), "Two".into()],
                action: Some(ActionKind::CreateZip),
                kind: Some(ActionFolder::Mods),
                after: Some(ActionAfter::Delete),
                save_as: Some("both.zip".into()),
                client: true,
                server: false,
            }],
            ..Default::default()
        };
        run_actions(&s, &json, &installed).unwrap();
        assert!(s.root.join("mods/both.zip").is_file());
        assert!(!s.root.join("mods/one.zip").exists(), "the originals go after the action");
        assert!(fsx::zip_contains(&s.root.join("mods/both.zip"), "one.zip.txt"));
        assert!(fsx::zip_contains(&s.root.join("mods/both.zip"), "two.zip.txt"));
        // rename
        std::fs::write(s.root.join("mods/old.jar"), "x").unwrap();
        let json = PackJson {
            actions: vec![PackAction {
                mod_names: vec!["Old".into()],
                action: Some(ActionKind::Rename),
                kind: Some(ActionFolder::Mods),
                save_as: Some("new.jar".into()),
                client: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        run_actions(&s, &json, &[pm("Old", "old.jar", ModKind::Mods)]).unwrap();
        assert!(s.root.join("mods/new.jar").is_file());
        assert!(!s.root.join("mods/old.jar").exists());
        // a server-only action does nothing on a client
        let json = PackJson {
            actions: vec![PackAction {
                mod_names: vec!["New".into()],
                action: Some(ActionKind::Rename),
                save_as: Some("z.jar".into()),
                server: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        run_actions(&s, &json, &[pm("New", "new.jar", ModKind::Mods)]).unwrap();
        assert!(s.root.join("mods/new.jar").is_file());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_reinstall_removes_only_what_the_pack_installed() {
        let base = tmp("reinstall");
        let s = site(&base);
        for d in ["mods", "config", "saves/W", "jarmods"] {
            std::fs::create_dir_all(s.root.join(d)).unwrap();
        }
        std::fs::write(s.root.join("mods/pack.jar"), "p").unwrap();
        std::fs::write(s.root.join("mods/mine.jar"), "m").unwrap();
        std::fs::write(s.root.join("config/pack.cfg"), "c").unwrap();
        std::fs::write(s.root.join("config/mine.cfg"), "c").unwrap();
        std::fs::write(s.root.join("saves/W/level.dat"), "w").unwrap();
        std::fs::write(s.root.join("scripts_marker"), "x").unwrap();
        let mut rec = ModRecord::new("Pack", "pack.jar", "mods");
        rec.user_added = false;
        let mut mine = ModRecord::new("Mine", "mine.jar", "mods");
        mine.user_added = true;
        let deletes = PathRules {
            files: Some(vec![
                PathRule { base: "root".into(), target: "scripts_marker".into() },
                PathRule { base: "root".into(), target: "saves%s%W".into() },
            ]),
            folders: None,
        };
        // the previous install wrote config/pack.cfg, so the rest of config/ stays
        prepare_filesystem(&s, true, &[rec, mine], &["config/pack.cfg".to_string()], Some(&deletes));
        assert!(!s.root.join("mods/pack.jar").exists());
        assert!(s.root.join("mods/mine.jar").exists(), "a mod the player added stays");
        assert!(!s.root.join("config/pack.cfg").exists());
        assert!(s.root.join("config/mine.cfg").exists());
        assert!(!s.root.join("scripts_marker").exists());
        assert!(s.root.join("saves/W/level.dat").exists(), "a delete rule can never reach worlds");
        assert!(s.root.join("disabledmods").is_dir() && s.root.join("jarmods").is_dir());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn options_and_keeps_survive_a_reinstall() {
        let base = tmp("keeps");
        let s = site(&base);
        std::fs::create_dir_all(s.root.join("config")).unwrap();
        std::fs::write(s.root.join("options.txt"), "fov:90").unwrap();
        std::fs::write(s.root.join("config/NEI.cfg"), "nei").unwrap();
        std::fs::write(s.root.join("config/keepme.cfg"), "keep").unwrap();
        let keeps = PathRules {
            files: Some(vec![
                PathRule { base: "config".into(), target: "keepme.cfg".into() },
                PathRule { base: "root".into(), target: "world".into() },
            ]),
            folders: None,
        };
        let saved = backup_select_files(&s, true, Some(&keeps));
        assert!(saved.options && saved.nei);
        // the pack rewrites the instance
        fsx::delete_path(&s.root.join("options.txt"));
        fsx::delete_path(&s.root.join("config"));
        restore_select_files(&s, &saved, true, Some(&keeps));
        assert_eq!(std::fs::read_to_string(s.root.join("options.txt")).unwrap(), "fov:90");
        assert_eq!(std::fs::read_to_string(s.root.join("config/NEI.cfg")).unwrap(), "nei");
        assert_eq!(std::fs::read_to_string(s.root.join("config/keepme.cfg")).unwrap(), "keep");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn case_conversion_renames_installed_mods() {
        let base = tmp("case");
        let s = site(&base);
        std::fs::create_dir_all(s.root.join("mods")).unwrap();
        std::fs::write(s.root.join("mods/CoolMod-1.jar"), "x").unwrap();
        let json = PackJson { case_all_files: Some(CaseKind::Upper), ..Default::default() };
        let m = pm("Cool", "CoolMod-1.jar", ModKind::Mods);
        assert_eq!(cased_file_name(&json, &m), "COOLMOD-1.jar");
        convert_case(&s, &json, &[m]);
        assert!(s.root.join("mods/COOLMOD-1.jar").exists());
        let _ = std::fs::remove_dir_all(base);
    }
}
