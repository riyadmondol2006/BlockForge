//! Keeping the instance's list of mods (`launcher.mods` in `instance.json`) in step with its folders, the way the
//! Java edition does: files the player put in by hand are noticed and recorded, mods can be switched off by moving
//! them to `disabledmods/`, removed (to the recycle bin if the player wants that), and records whose file has gone
//! are dropped.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::ModSettings;
use super::metadata::{self, ContentKind};
use super::trash;
use super::versions::{self, MODRINTH_SINYTRA_CONNECTOR_MOD_ID};
use crate::instances::Instance;
use crate::modrecord::ModRecord;

/// Files the launcher treats as mods and packs.
pub fn is_accepted_mod_file(name: &str) -> bool {
    name.ends_with(".jar") || name.ends_with(".zip") || name.ends_with(".litemod")
}

/// Which record is meant: a file name inside a kind of folder.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModKey {
    pub file: String,
    pub kind: String,
}

impl ModKey {
    pub fn of(r: &ModRecord) -> ModKey {
        ModKey { file: r.file.clone(), kind: r.kind.clone() }
    }
}

/// The name to show for a mod: Modrinth's title when it came from there, otherwise the name read from the file.
pub fn name_of(r: &ModRecord) -> String {
    if let Some(title) = r.modrinth_project.as_ref().and_then(|p| p.get("title")).and_then(|t| t.as_str()) {
        if r.modrinth_project_id().is_some() && r.modrinth_version.is_some() {
            return title.to_string();
        }
    }
    if r.name.is_empty() { r.file.clone() } else { r.name.clone() }
}

pub fn is_from_modrinth(r: &ModRecord) -> bool {
    r.modrinth_project.is_some() && r.modrinth_version.is_some()
}

pub fn is_from_curseforge(r: &ModRecord) -> bool {
    r.curse_forge_project_id.is_some() && r.curse_forge_file_id.is_some()
}

pub fn is_updatable(r: &ModRecord) -> bool {
    is_from_modrinth(r) || is_from_curseforge(r)
}

/// A file name that stays inside its folder: no folders, no `..`.
fn safe_file_name(file: &str) -> bool {
    !file.is_empty()
        && Path::new(file).components().all(|c| matches!(c, Component::Normal(_)))
        && !file.contains(['/', '\\'])
}

/// A folder path from a record that stays inside the instance.
fn safe_relative(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// The folder (relative to the instance) a record's file lives in while it is switched on.
/// `None` when the kind has no folder (or a dependency without a Minecraft version).
pub fn folder_of(rec: &ModRecord, minecraft_version: Option<&str>) -> Option<String> {
    if let Some(p) = rec.path.as_deref().filter(|p| !p.is_empty()) {
        return safe_relative(p).then(|| p.trim_matches(['/', '\\']).to_string());
    }
    Some(match rec.kind.as_str() {
        "jar" | "forge" | "mcpc" => "jarmods".into(),
        "texturepack" => "texturepacks".into(),
        "resourcepack" => "resourcepacks".into(),
        "mods" => "mods".into(),
        "plugins" => "plugins".into(),
        "ic2lib" => "mods/ic2".into(),
        "denlib" => "mods/denlib".into(),
        "coremods" => "coremods".into(),
        "shaderpack" => "shaderpacks".into(),
        "datapack" => "datapacks".into(),
        "dependency" => format!("mods/{}", minecraft_version?),
        _ => return None,
    })
}

/// Where the file of a record is while it is switched on.
pub fn enabled_path(instance: &Instance, rec: &ModRecord) -> Option<PathBuf> {
    if !safe_file_name(&rec.file) {
        return None;
    }
    Some(instance.root.join(folder_of(rec, Some(instance.mc_version()))?).join(&rec.file))
}

/// Where the file of a record is while it is switched off (every kind goes in the same folder).
pub fn disabled_path(instance: &Instance, rec: &ModRecord) -> Option<PathBuf> {
    safe_file_name(&rec.file).then(|| instance.disabled_mods_dir().join(&rec.file))
}

/// Where the file of a record is right now, according to its `disabled` flag.
pub fn current_path(instance: &Instance, rec: &ModRecord) -> Option<PathBuf> {
    if rec.disabled { disabled_path(instance, rec) } else { enabled_path(instance, rec) }
}

/// A mod as the manager shows it: the record, and what is on disk for it.
#[derive(Clone, Debug)]
pub struct ModEntry {
    pub record: ModRecord,
    pub path: Option<PathBuf>,
    pub exists: bool,
    pub size: u64,
}

impl ModEntry {
    pub fn key(&self) -> ModKey {
        ModKey::of(&self.record)
    }
    pub fn name(&self) -> String {
        name_of(&self.record)
    }
}

/// The instance's content of one kind, sorted by name. Only what the player chose to install is listed
/// (`wasSelected`), never entries that were skipped.
pub fn entries(instance: &Instance, kind: ContentKind) -> Vec<ModEntry> {
    let mut list: Vec<ModEntry> = instance
        .mod_records()
        .into_iter()
        .filter(|m| m.kind == kind.record_type() && m.was_selected && !m.skipped)
        .map(|record| {
            let path = current_path(instance, &record);
            let meta = path.as_ref().and_then(|p| std::fs::metadata(p).ok());
            ModEntry { exists: meta.is_some(), size: meta.map(|m| m.len()).unwrap_or(0), path, record }
        })
        .collect();
    list.sort_by_key(|e| e.name().to_lowercase());
    list
}

/// Every mod the Edit Mods window of the Java edition lists: all kinds except worlds.
pub fn all_entries(instance: &Instance) -> Vec<ModEntry> {
    let mut list: Vec<ModEntry> = instance
        .mod_records()
        .into_iter()
        .filter(|m| m.was_selected && !m.skipped && m.kind != "worlds")
        .map(|record| {
            let path = current_path(instance, &record);
            let meta = path.as_ref().and_then(|p| std::fs::metadata(p).ok());
            ModEntry { exists: meta.is_some(), size: meta.map(|m| m.len()).unwrap_or(0), path, record }
        })
        .collect();
    list.sort_by_key(|e| e.name().to_lowercase());
    list
}

/// Modrinth projects that are in the instance, counting stand-ins (Quilt's QSL does the job of Fabric API).
pub fn installed_project_ids(instance: &Instance) -> HashSet<String> {
    let mut ids = HashSet::new();
    for m in instance.mod_records() {
        if is_from_modrinth(&m) {
            if let Some(id) = m.modrinth_project_id() {
                ids.insert(id.to_string());
                if id == versions::MODRINTH_QSL_MOD_ID {
                    ids.insert(versions::MODRINTH_FABRIC_MOD_ID.to_string());
                }
            }
        }
        if is_from_curseforge(&m) && m.curse_forge_project_id == Some(versions::CURSEFORGE_FABRIC_MOD_ID) {
            ids.insert(versions::MODRINTH_FABRIC_MOD_ID.to_string());
        }
    }
    ids
}

/// Whether the instance runs Forge or NeoForge with Sinytra Connector, so Fabric mods can be offered too.
pub fn forge_like_with_sinytra(instance: &Instance, settings: &ModSettings) -> bool {
    let forge_like = instance
        .loader()
        .and_then(|(k, _)| versions::loader_key(k))
        .is_some_and(|k| k == versions::FORGE || k == versions::NEOFORGE);
    if !forge_like || !settings.show_fabric_mods_when_sinytra_installed {
        return false;
    }
    instance.mod_records().iter().any(|m| {
        (is_from_curseforge(m) && m.curse_forge_project_id == Some(versions::CURSEFORGE_SINYTRA_CONNECTOR_MOD_ID))
            || (is_from_modrinth(m)
                && m.modrinth_project_id().is_some_and(|id| id.eq_ignore_ascii_case(MODRINTH_SINYTRA_CONNECTOR_MOD_ID)))
    })
}

// ---------------------------------------------------------------------------------------------------------
// Switching on and off, removing

/// Changes the record with this key, saving the instance if the closure says something changed.
fn with_record<T>(instance: &mut Instance, key: &ModKey, f: impl FnOnce(&mut ModRecord) -> Result<T>) -> Result<T> {
    let mut list = instance.mod_records();
    let rec = list
        .iter_mut()
        .find(|m| m.file == key.file && m.kind == key.kind)
        .with_context(|| format!("{} isn't in this instance's mod list", key.file))?;
    let out = f(rec)?;
    instance.set_mod_records(&list);
    instance.save()?;
    Ok(out)
}

/// Switches a mod on or off by moving its file between its folder and `disabledmods/`.
pub fn set_enabled(instance: &mut Instance, key: &ModKey, enabled: bool) -> Result<()> {
    let snapshot = instance.clone();
    with_record(instance, key, |rec| {
        if rec.disabled == !enabled {
            return Ok(());
        }
        let on = enabled_path(&snapshot, rec).with_context(|| format!("{} has no folder to go back to", rec.file))?;
        let off = disabled_path(&snapshot, rec).context("that file name isn't allowed")?;
        let (from, to) = if enabled { (off, on) } else { (on, off) };
        if !from.exists() {
            bail!("{} is not at {} any more", rec.file, from.display());
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if to.exists() {
            std::fs::remove_file(&to).with_context(|| format!("replacing {}", to.display()))?;
        }
        move_file(&from, &to)?;
        rec.disabled = !enabled;
        Ok(())
    })
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
    std::fs::remove_file(from)?;
    Ok(())
}

/// Removes a mod: its record and its file (to the recycle bin when `useRecycleBin` is on).
pub fn remove(instance: &mut Instance, key: &ModKey, settings: &ModSettings) -> Result<()> {
    let list = instance.mod_records();
    let Some(rec) = list.iter().find(|m| m.file == key.file && m.kind == key.kind).cloned() else {
        bail!("{} isn't in this instance's mod list", key.file)
    };
    let path = current_path(instance, &rec);
    // the record goes first: a file that can't be deleted shows up again at the next scan, which beats a record
    // for a file that is gone
    instance.remove_mod_record_exact(&key.file, &key.kind);
    instance.save()?;
    if let Some(p) = path.filter(|p| p.exists()) {
        trash::delete(&p, settings.use_recycle_bin)?;
    }
    Ok(())
}

impl Instance {
    /// Drops the record with this file name and kind (`remove_mod_record_file` drops by name alone).
    pub fn remove_mod_record_exact(&mut self, file: &str, kind: &str) {
        let mut mods = self.mod_records();
        mods.retain(|m| !(m.file == file && m.kind == kind));
        self.set_mod_records(&mods);
    }
}

// ---------------------------------------------------------------------------------------------------------
// Adding files

/// A record for a file the player added: its name, version and description are read from inside the file.
pub fn generate_mod(file: &Path, kind: &str, enabled: bool) -> ModRecord {
    let file_name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let info = metadata::read_info(file);
    let mut rec = ModRecord::new(&file_name, &file_name, kind);
    rec.disabled = !enabled;
    rec.user_added = true;
    rec.was_selected = true;
    rec.optional = true;
    rec.version = "Unknown".into();
    if let Some(n) = info.name.filter(|n| !n.trim().is_empty()) {
        rec.name = n;
    }
    if let Some(v) = info.version.filter(|v| !v.trim().is_empty()) {
        rec.version = v;
    }
    if let Some(d) = info.description {
        rec.description = d;
    }
    rec
}

/// What adding a file did.
#[derive(Debug, PartialEq, Eq)]
pub enum AddOutcome {
    Added(ModKey),
    /// A file with that name was already there; nothing was copied.
    AlreadyThere,
}

/// Copies a file into the instance and records it (the Edit Mods window's "Add Mod"). The file goes to its folder,
/// or to `disabledmods/` when `enableAddedModsByDefault` is off.
pub fn add_file(instance: &mut Instance, source: &Path, kind: &str, settings: &ModSettings) -> Result<AddOutcome> {
    let mut rec = generate_mod(source, kind, settings.enable_added_mods_by_default);
    if !safe_file_name(&rec.file) {
        bail!("{} can't be added: the file name isn't allowed", rec.file);
    }
    let target = if rec.disabled { disabled_path(instance, &rec) } else { enabled_path(instance, &rec) }
        .with_context(|| format!("there is no folder for {kind} files"))?;
    if target.exists() {
        return Ok(AddOutcome::AlreadyThere);
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(source, &target).with_context(|| format!("copying {} into the instance", source.display()))?;
    rec.version = if rec.version.is_empty() { "Unknown".into() } else { rec.version };
    let key = ModKey::of(&rec);
    let mut list = instance.mod_records();
    list.push(rec);
    instance.set_mod_records(&list);
    instance.save()?;
    Ok(AddOutcome::Added(key))
}

/// Like [`add_file`], and then asks Modrinth what the new file is (unless `dontCheckModsOnModrinth`).
pub async fn add_file_identified(
    instance: &mut Instance,
    source: &Path,
    kind: &str,
    settings: &ModSettings,
) -> Result<AddOutcome> {
    let outcome = add_file(instance, source, kind, settings)?;
    if let AddOutcome::Added(key) = &outcome {
        let mut list = instance.mod_records();
        if let Some(i) = list.iter().position(|m| ModKey::of(m) == *key) {
            let mut one = vec![list[i].clone()];
            if super::identify::identify_records(instance, &mut one, settings).await.unwrap_or(0) > 0 {
                list[i] = one.remove(0);
                instance.set_mod_records(&list);
                instance.save()?;
            }
        }
    }
    Ok(outcome)
}

// ---------------------------------------------------------------------------------------------------------
// Finding files nobody recorded

/// Which folders are looked through for files nobody recorded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanScope {
    /// The three folders the manager shows: mods, resource packs and shader packs.
    Content,
    /// What the Java edition's Edit Mods window looks at: also data packs, jar mods and `disabledmods/`.
    Full,
}

fn scan_folders(instance: &Instance, scope: ScanScope) -> Vec<(PathBuf, &'static str, bool)> {
    // folder, record type, enabled
    let r = &instance.root;
    match scope {
        ScanScope::Content => ContentKind::ALL.iter().map(|k| (r.join(k.folder()), k.record_type(), true)).collect(),
        ScanScope::Full => vec![
            (r.join("mods"), "mods", true),
            (r.join("disabledmods"), "mods", false),
            (r.join("datapacks"), "datapack", true),
            (r.join("resourcepacks"), "resourcepack", true),
            (r.join("shaderpacks"), "shaderpack", true),
            (r.join("jarmods"), "jar", true),
        ],
    }
}

/// Records for files in the instance's folders that the mod list doesn't know yet.
pub fn find_untracked(instance: &Instance, scope: ScanScope) -> Vec<ModRecord> {
    let known = instance.mod_records();
    let mut found = Vec::new();
    for (dir, kind, enabled) in scan_folders(instance, scope) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut names: Vec<(String, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
            .filter(|(n, _)| is_accepted_mod_file(n))
            .collect();
        names.sort();
        for (name, path) in names {
            let is_known = known.iter().any(|m| {
                m.file == name
                    && match scope {
                        // a folder file belongs to a record of that kind, or to one of a kind the manager doesn't list
                        ScanScope::Content => m.kind == kind || ContentKind::of_record_type(&m.kind).is_none(),
                        // files in disabledmods/ belong to any switched-off record with that name
                        ScanScope::Full if !enabled => m.disabled || m.kind == kind,
                        ScanScope::Full => m.kind == kind,
                    }
            });
            if !is_known && !found.iter().any(|f: &ModRecord| f.file == name && f.kind == kind) {
                found.push(generate_mod(&path, kind, enabled));
            }
        }
    }
    found
}

/// What cleaning up the list found.
#[derive(Clone, Debug, Default)]
pub struct Stale {
    /// Records whose file is no longer in the instance.
    pub missing: Vec<ModKey>,
    /// Records that point at a file another record already covers.
    pub duplicates: Vec<ModKey>,
}

/// Records of mods whose file has gone, and records that repeat another one.
pub fn find_stale(instance: &Instance) -> Stale {
    let mut stale = Stale::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for m in instance.mod_records() {
        if !m.was_selected || m.skipped || m.kind != "mods" {
            continue;
        }
        let Some(path) = current_path(instance, &m) else { continue };
        if !path.exists() {
            stale.missing.push(ModKey::of(&m));
        } else if !seen.insert(path) {
            stale.duplicates.push(ModKey::of(&m));
        }
    }
    stale
}

/// Drops the records `find_stale` found.
pub fn drop_stale(instance: &mut Instance, stale: &Stale) -> Result<()> {
    if stale.missing.is_empty() && stale.duplicates.is_empty() {
        return Ok(());
    }
    let mut list = instance.mod_records();
    let mut dup_seen: HashSet<ModKey> = HashSet::new();
    list.retain(|m| {
        let key = ModKey::of(m);
        if stale.missing.contains(&key) {
            return false;
        }
        // the first of the duplicates stays, later ones go
        if stale.duplicates.contains(&key) && !dup_seen.insert(key) {
            return false;
        }
        true
    });
    instance.set_mod_records(&list);
    instance.save()
}

/// What a scan did.
#[derive(Clone, Debug, Default)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub duplicates: Vec<String>,
    /// How many of the new files Modrinth recognised.
    pub identified: usize,
}

impl SyncReport {
    pub fn changed(&self) -> bool {
        !(self.added.is_empty() && self.removed.is_empty() && self.duplicates.is_empty())
    }
}

/// Brings the mod list in step with the folders: records files nobody knew (asking Modrinth what they are, unless
/// `dontCheckModsOnModrinth`), and drops records whose file has gone. The CurseForge fingerprint lookup of the Java
/// edition belongs to the CurseForge module and plugs in through [`super::updates::ExternalSource`].
pub async fn sync(instance: &mut Instance, settings: &ModSettings, scope: ScanScope) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    let mut fresh = find_untracked(instance, scope);
    if !fresh.is_empty() {
        report.identified = super::identify::identify_records(instance, &mut fresh, settings).await.unwrap_or(0);
        report.added = fresh.iter().map(|m| m.file.clone()).collect();
        let mut list = instance.mod_records();
        list.extend(fresh);
        instance.set_mod_records(&list);
        instance.save()?;
    }
    if scope == ScanScope::Full {
        let stale = find_stale(instance);
        report.removed = stale.missing.iter().map(|k| k.file.clone()).collect();
        report.duplicates = stale.duplicates.iter().map(|k| k.file.clone()).collect();
        drop_stale(instance, &stale)?;
    }
    Ok(report)
}

/// Helpers shared by the tests of this module group.
#[cfg(test)]
pub(crate) mod tests_support {
    use crate::instances::{Instance, InstanceFile};
    use std::io::Write;
    use std::path::Path;

    pub(crate) fn temp_instance(name: &str) -> Instance {
        let root = std::env::temp_dir().join(format!("blockforge-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("mods")).unwrap();
        let mut file = InstanceFile::default();
        file.launcher.name = name.to_string();
        file.version.id = "1.20.1".into();
        Instance { folder: name.to_string(), mod_count: 0, root, file }
    }

    pub(crate) fn jar(path: &Path, fabric_json: &str) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        z.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(fabric_json.as_bytes()).unwrap();
        z.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{jar, temp_instance};
    use super::*;

    #[test]
    fn paths_follow_the_record_kind() {
        let inst = temp_instance("paths");
        let mut r = ModRecord::new("x", "x.jar", "mods");
        assert_eq!(enabled_path(&inst, &r).unwrap(), inst.root.join("mods/x.jar"));
        assert_eq!(disabled_path(&inst, &r).unwrap(), inst.root.join("disabledmods/x.jar"));
        r.kind = "shaderpack".into();
        assert_eq!(enabled_path(&inst, &r).unwrap(), inst.root.join("shaderpacks/x.jar"));
        r.kind = "jar".into();
        assert_eq!(folder_of(&r, None).unwrap(), "jarmods");
        r.kind = "dependency".into();
        assert_eq!(folder_of(&r, Some("1.7.10")).unwrap(), "mods/1.7.10");
        assert!(folder_of(&r, None).is_none());
        r.kind = "mystery".into();
        assert!(enabled_path(&inst, &r).is_none());
        r.kind = "mods".into();
        r.path = Some("custom/dir".into());
        assert_eq!(enabled_path(&inst, &r).unwrap(), inst.root.join("custom/dir/x.jar"));
        r.path = Some("../outside".into());
        assert!(enabled_path(&inst, &r).is_none());
        r.path = None;
        r.file = "../../etc/passwd".into();
        assert!(enabled_path(&inst, &r).is_none() && disabled_path(&inst, &r).is_none());
    }

    #[test]
    fn switching_off_and_on_moves_the_file_and_the_flag() {
        let mut inst = temp_instance("toggle");
        jar(&inst.root.join("mods/a.jar"), r#"{"name":"Alpha","version":"1.0","description":"first"}"#);
        let existing = inst.root.join("mods/a.jar");
        let added = add_file(&mut inst, &existing, "mods", &ModSettings::default());
        // adding a file that is already in the folder is a no-op
        assert_eq!(added.unwrap(), AddOutcome::AlreadyThere);

        let rec = generate_mod(&inst.root.join("mods/a.jar"), "mods", true);
        assert_eq!((rec.name.as_str(), rec.version.as_str(), rec.description.as_str()), ("Alpha", "1.0", "first"));
        inst.set_mod_records(&[rec]);
        let key = ModKey { file: "a.jar".into(), kind: "mods".into() };

        set_enabled(&mut inst, &key, false).unwrap();
        assert!(!inst.root.join("mods/a.jar").exists());
        assert!(inst.root.join("disabledmods/a.jar").exists());
        assert!(inst.mod_records()[0].disabled);
        // saved to instance.json
        assert!(std::fs::read_to_string(inst.root.join("instance.json")).unwrap().contains("\"disabled\": true"));

        set_enabled(&mut inst, &key, true).unwrap();
        assert!(inst.root.join("mods/a.jar").exists() && !inst.root.join("disabledmods/a.jar").exists());
        assert!(!inst.mod_records()[0].disabled);
        let _ = std::fs::remove_dir_all(&inst.root);
    }

    #[test]
    fn adding_a_file_honours_enable_added_mods_by_default() {
        let mut inst = temp_instance("add");
        let src = std::env::temp_dir().join(format!("blockforge-add-src-{}.jar", std::process::id()));
        jar(&src, r#"{"name":"Beta","version":"2"}"#);
        let mut s = ModSettings::default();
        s.enable_added_mods_by_default = false;
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(
            add_file(&mut inst, &src, "mods", &s).unwrap(),
            AddOutcome::Added(ModKey { file: name.clone(), kind: "mods".into() })
        );
        assert!(inst.root.join("disabledmods").join(&name).exists());
        assert!(!inst.root.join("mods").join(&name).exists());
        let r = &inst.mod_records()[0];
        assert!(r.disabled && r.user_added && r.was_selected);
        assert_eq!(r.name, "Beta");
        // a second try finds it there
        assert_eq!(add_file(&mut inst, &src, "mods", &s).unwrap(), AddOutcome::AlreadyThere);
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_dir_all(&inst.root);
    }

    #[test]
    fn untracked_files_are_found_and_stale_records_dropped() {
        let mut inst = temp_instance("scan");
        jar(&inst.root.join("mods/new.jar"), r#"{"name":"New One","version":"3"}"#);
        std::fs::create_dir_all(inst.root.join("disabledmods")).unwrap();
        jar(&inst.root.join("disabledmods/off.jar"), r#"{"name":"Off"}"#);
        std::fs::create_dir_all(inst.root.join("resourcepacks")).unwrap();
        std::fs::write(inst.root.join("resourcepacks/pack.zip"), "not a zip").unwrap();
        std::fs::write(inst.root.join("mods/readme.txt"), "x").unwrap();
        // a record for a file that is gone, and one that repeats another
        let gone = ModRecord::new("Gone", "gone.jar", "mods");
        let mut known = ModRecord::new("Known", "new.jar", "mods");
        known.was_selected = true;
        inst.set_mod_records(&[gone, known.clone(), known]);

        let fresh = find_untracked(&inst, ScanScope::Full);
        let names: Vec<(&str, bool)> = fresh.iter().map(|m| (m.file.as_str(), m.disabled)).collect();
        assert!(names.contains(&("off.jar", true)), "{names:?}");
        assert!(names.contains(&("pack.zip", false)));
        assert!(!names.iter().any(|(n, _)| *n == "new.jar" || *n == "readme.txt"));
        // the content scope only looks at the three shown folders
        assert!(!find_untracked(&inst, ScanScope::Content).iter().any(|m| m.file == "off.jar"));

        let stale = find_stale(&inst);
        assert_eq!(stale.missing.len(), 1);
        assert_eq!(stale.duplicates.len(), 1);
        drop_stale(&mut inst, &stale).unwrap();
        let left = inst.mod_records();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].file, "new.jar");
        let _ = std::fs::remove_dir_all(&inst.root);
    }

    #[test]
    fn removing_deletes_the_file_and_the_record() {
        let mut inst = temp_instance("remove");
        jar(&inst.root.join("mods/a.jar"), r#"{"name":"A"}"#);
        inst.set_mod_records(&[ModRecord::new("A", "a.jar", "mods")]);
        let mut s = ModSettings::default();
        s.use_recycle_bin = false;
        remove(&mut inst, &ModKey { file: "a.jar".into(), kind: "mods".into() }, &s).unwrap();
        assert!(!inst.root.join("mods/a.jar").exists());
        assert!(inst.mod_records().is_empty());
        assert!(remove(&mut inst, &ModKey { file: "a.jar".into(), kind: "mods".into() }, &s).is_err());
        let _ = std::fs::remove_dir_all(&inst.root);
    }

    /// A mod list the Java edition wrote must come back out exactly as it went in, or the two editions would
    /// slowly damage each other's data.
    #[test]
    fn a_java_written_mod_list_round_trips() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let dir = std::path::PathBuf::from(home).join(".local/share/blockforge/instances");
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Some(inst) = crate::instances::load_one(&entry.path()) else { continue };
            let original = inst.file.launcher.extras.get("mods").cloned();
            let mut copy = inst.clone();
            let records = inst.mod_records();
            copy.set_mod_records(&records);
            assert_eq!(copy.file.launcher.extras.get("mods"), original.as_ref(), "{}", entry.path().display());
        }
    }

    #[test]
    fn names_prefer_modrinth_titles() {
        let mut r = ModRecord::new("from file", "a.jar", "mods");
        assert_eq!(name_of(&r), "from file");
        r.modrinth_project = Some(serde_json::json!({"id": "p", "title": "Pretty Title"}));
        assert_eq!(name_of(&r), "from file"); // needs the version too, as in the Java edition
        r.modrinth_version = Some(serde_json::json!({"id": "v"}));
        assert_eq!(name_of(&r), "Pretty Title");
        r.name = String::new();
        r.modrinth_project = None;
        assert_eq!(name_of(&r), "a.jar");
    }
}
