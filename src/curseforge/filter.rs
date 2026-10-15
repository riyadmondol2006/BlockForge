//! Which CurseForge files and mod loaders fit an instance. This is the filtering the Java edition does in its
//! file selector (`CurseForgeProjectFileSelectorDialog`), its search (`AddModsDialog`) and its update check
//! (`DisableableMod.checkForUpdate`), kept as plain functions so the window and the command line share them.

use super::model::*;
use super::{CLASS_MODS, CLASS_PLUGINS, CLASS_RESOURCE_PACKS, CfSettings, Restriction};
use crate::instances::Instance;
use crate::modrecord::ModRecord;
use crate::paths::Dirs;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoaderKind {
    None,
    Fabric,
    LegacyFabric,
    Quilt,
    Forge,
    NeoForge,
}

impl LoaderKind {
    pub fn parse(kind: &str) -> LoaderKind {
        match kind.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "fabric" => LoaderKind::Fabric,
            "legacyfabric" => LoaderKind::LegacyFabric,
            "quilt" => LoaderKind::Quilt,
            "forge" => LoaderKind::Forge,
            "neoforge" => LoaderKind::NeoForge,
            _ => LoaderKind::None,
        }
    }

    pub fn is_fabric_like(self) -> bool {
        matches!(self, LoaderKind::Fabric | LoaderKind::LegacyFabric | LoaderKind::Quilt)
    }
}

/// What an instance (or a server) is, as far as choosing CurseForge files goes.
#[derive(Clone, Debug)]
pub struct Target {
    pub mc_version: String,
    pub loader: LoaderKind,
    /// Forge or NeoForge with Sinytra Connector installed, which lets Fabric mods run.
    pub sinytra: bool,
    /// NeoForge on a Minecraft version where it also runs Forge mods.
    pub neoforge_runs_forge: bool,
}

impl Target {
    pub fn new(mc_version: &str, loader: LoaderKind) -> Target {
        Target { mc_version: mc_version.to_string(), loader, sinytra: false, neoforge_runs_forge: false }
    }

    pub fn from_instance(instance: &Instance, dirs: &Dirs, cf: &CfSettings) -> Target {
        let loader = instance.loader().map(|(k, _)| LoaderKind::parse(k)).unwrap_or(LoaderKind::None);
        let mc = instance.mc_version().to_string();
        let mods = instance.mod_records();
        let sinytra = matches!(loader, LoaderKind::Forge | LoaderKind::NeoForge)
            && cf.show_fabric_mods_with_sinytra
            && mods.iter().any(|m| {
                m.curse_forge_project_id == Some(super::SINYTRA_CONNECTOR_ID)
                    || m.modrinth_project_id()
                        .is_some_and(|id| id.eq_ignore_ascii_case(super::MODRINTH_SINYTRA_CONNECTOR_ID))
            });
        let neoforge_runs_forge = super::neoforge_forge_compatible(dirs).contains(&mc);
        Target { mc_version: mc, loader, sinytra, neoforge_runs_forge }
    }

    /// The mod loader numbers to give a search for mods (`modLoaderTypes`).
    pub fn search_loaders(&self) -> Vec<i64> {
        use super::{MODLOADER_FABRIC, MODLOADER_FORGE, MODLOADER_NEOFORGE, MODLOADER_QUILT};
        match self.loader {
            LoaderKind::Fabric | LoaderKind::LegacyFabric => vec![MODLOADER_FABRIC],
            LoaderKind::Quilt => vec![MODLOADER_FABRIC, MODLOADER_QUILT],
            LoaderKind::Forge if self.sinytra => vec![MODLOADER_FORGE, MODLOADER_FABRIC],
            LoaderKind::NeoForge if self.sinytra => {
                let mut v = vec![MODLOADER_NEOFORGE, MODLOADER_FABRIC];
                if self.neoforge_runs_forge {
                    v.push(MODLOADER_FORGE);
                }
                v
            }
            LoaderKind::Forge => vec![MODLOADER_FORGE],
            LoaderKind::NeoForge => {
                let mut v = vec![MODLOADER_NEOFORGE];
                if self.neoforge_runs_forge {
                    v.push(MODLOADER_FORGE);
                }
                v
            }
            LoaderKind::None => Vec::new(),
        }
    }

    /// The version to give a search: only in strict mode is the search narrowed to the instance's version.
    pub fn search_game_version(&self, cf: &CfSettings) -> Option<String> {
        (cf.restriction == Restriction::Strict && !self.mc_version.is_empty()).then(|| self.mc_version.clone())
    }

    /// The API mod the loader needs next to it, if the instance does not have it yet.
    pub fn missing_loader_api(&self, mods: &[ModRecord]) -> Option<(i64, &'static str)> {
        let has_cf = |id: i64| mods.iter().any(|m| m.curse_forge_project_id == Some(id));
        let has_mr =
            |id: &str| mods.iter().any(|m| m.modrinth_project_id().is_some_and(|p| p.eq_ignore_ascii_case(id)));
        match self.loader {
            LoaderKind::Fabric if !has_cf(super::FABRIC_API_ID) && !has_mr(super::MODRINTH_FABRIC_API_ID) => {
                Some((super::FABRIC_API_ID, "Fabric API"))
            }
            LoaderKind::LegacyFabric
                if !has_cf(super::LEGACY_FABRIC_API_ID) && !has_mr(super::MODRINTH_LEGACY_FABRIC_API_ID) =>
            {
                Some((super::LEGACY_FABRIC_API_ID, "Legacy Fabric API"))
            }
            _ if self.sinytra
                && !has_cf(super::FORGIFIED_FABRIC_API_ID)
                && !has_mr(super::MODRINTH_FORGIFIED_FABRIC_API_ID) =>
            {
                Some((super::FORGIFIED_FABRIC_API_ID, "Forgified Fabric API"))
            }
            _ => None,
        }
    }
}

/// `1.20.4` -> versions `1.20` and `1.20.x` count as the same family (`LAX`); `1.21` -> `1.21` and `1.21.x`.
/// Snapshots only match themselves.
pub fn lax_matches(mc: &str, file_version: &str) -> bool {
    if !is_release_version(mc) {
        return mc == file_version;
    }
    if !is_release_version(file_version) {
        return false;
    }
    let parts: Vec<&str> = mc.split('.').collect();
    let prefix = if parts.len() == 2 { mc.to_string() } else { mc[..mc.rfind('.').unwrap_or(mc.len())].to_string() };
    file_version == prefix || file_version.starts_with(&format!("{prefix}."))
}

/// Whether a mod file can run on the target's loader, going by the loader names in its game versions.
fn loader_fits(f: &File, t: &Target, has_forge_version: bool, has_neoforge_version: bool) -> bool {
    let tag = |n: &str| f.game_versions.iter().any(|g| g == n);
    if tag("Fabric")
        && t.loader != LoaderKind::None
        && (t.loader.is_fabric_like()
            || (t.sinytra && t.loader == LoaderKind::Forge && !has_forge_version)
            || (t.sinytra && t.loader == LoaderKind::NeoForge && !has_neoforge_version))
    {
        return true;
    }
    if tag("NeoForge") && t.loader == LoaderKind::NeoForge {
        return true;
    }
    if tag("Forge") && (t.loader == LoaderKind::Forge || (t.loader == LoaderKind::NeoForge && t.neoforge_runs_forge)) {
        return true;
    }
    if tag("Quilt") && t.loader == LoaderKind::Quilt {
        return true;
    }
    // no loader named at all: assume the file is untagged and let the player see it
    !tag("Fabric") && !tag("NeoForge") && !tag("Forge") && !tag("Quilt")
}

/// The files of `project` that fit `target`, newest first, as the Java edition's file selector lists them.
pub fn files_for_target(project: &Project, all: &[File], target: &Target, cf: &CfSettings) -> Vec<File> {
    let mut files: Vec<File> = all.to_vec();
    files.sort_by(|a, b| b.id.cmp(&a.id));
    let root = project.root_category_id();

    match cf.restriction {
        Restriction::Strict => files.retain(|f| {
            root == CLASS_RESOURCE_PACKS || root == CLASS_PLUGINS || f.has_game_version(&target.mc_version)
        }),
        Restriction::Lax => files.retain(|f| f.game_versions.iter().any(|g| lax_matches(&target.mc_version, g))),
        Restriction::None => {}
    }

    if root == CLASS_MODS {
        let has_neoforge = all
            .iter()
            .any(|f| f.has_game_version("NeoForge") || (target.neoforge_runs_forge && f.has_game_version("Forge")));
        let has_forge = all.iter().any(|f| f.has_game_version("Forge"));
        files.retain(|f| loader_fits(f, target, has_forge, has_neoforge));
    }

    if cf.restriction != Restriction::None {
        files.retain(|f| {
            if f.has_game_version("Forge") || f.has_game_version("Fabric") {
                return true;
            }
            let file_name = f.file_name.to_lowercase();
            let display = f.display_name.to_lowercase();
            match target.loader {
                LoaderKind::None => true,
                LoaderKind::Fabric => {
                    !display.contains("-forge-")
                        && !display.contains("(forge)")
                        && !display.contains("[forge")
                        && !file_name.contains("forgemod")
                }
                _ => {
                    !display.contains("-fabric-")
                        && !display.contains("(fabric)")
                        && !display.contains("[fabric")
                        && !file_name.contains("fabricmod")
                }
            }
        });
    }
    files
}

/// The file to install when the player does not choose one: the newest release, or the newest of any kind when
/// alpha and beta files are allowed or there is no release.
pub fn pick_latest(files: &[File], allow_alpha_beta: bool) -> Option<&File> {
    if allow_alpha_beta {
        return files.first();
    }
    files.iter().find(|f| f.is_release()).or_else(|| files.iter().find(|f| f.is_beta())).or_else(|| files.first())
}

/// Whether `candidate` is allowed as an update for `installed` when alpha and beta files are not wanted: a
/// release only moves to releases, a beta to releases or betas, an alpha to anything.
pub fn release_type_ok(installed: Option<i64>, candidate: &File, allow_alpha_beta: bool) -> bool {
    if allow_alpha_beta {
        return true;
    }
    match installed {
        Some(RELEASE) => candidate.is_release(),
        Some(BETA) => candidate.is_release() || candidate.is_beta(),
        _ => true,
    }
}

/// The newest file newer than `installed_id` that fits the target, if any (a mod update).
pub fn newer_file(
    files: &[File],
    installed_id: i64,
    installed_type: Option<i64>,
    allow_alpha_beta: bool,
) -> Option<&File> {
    files
        .iter()
        .filter(|f| f.id > installed_id && release_type_ok(installed_type, f, allow_alpha_beta))
        .max_by_key(|f| f.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: i64, name: &str, versions: &[&str], release: i64) -> File {
        File {
            id,
            display_name: name.into(),
            file_name: format!("{name}.jar"),
            release_type: Some(release),
            game_versions: versions.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn mod_project() -> Project {
        Project { class_id: CLASS_MODS, primary_category_id: 0, ..Default::default() }
    }

    #[test]
    fn lax_matches_the_same_minecraft_family() {
        assert!(lax_matches("1.20.4", "1.20.1"));
        assert!(lax_matches("1.20.4", "1.20"));
        assert!(!lax_matches("1.20.4", "1.21"));
        assert!(lax_matches("1.21", "1.21.1"));
        assert!(!lax_matches("1.21", "1.2"));
        assert!(!lax_matches("24w14a", "1.20.4"));
        assert!(lax_matches("24w14a", "24w14a"));
    }

    #[test]
    fn strict_keeps_exact_version_and_loader() {
        let files = vec![
            file(5, "mod-fabric-1.21.1", &["1.21.1", "Fabric"], RELEASE),
            file(4, "mod-forge-1.21.1", &["1.21.1", "Forge"], RELEASE),
            file(3, "mod-fabric-1.20.1", &["1.20.1", "Fabric"], RELEASE),
            file(2, "mod-untagged", &["1.21.1"], RELEASE),
        ];
        let cf = CfSettings::default();
        let t = Target::new("1.21.1", LoaderKind::Fabric);
        let got: Vec<i64> = files_for_target(&mod_project(), &files, &t, &cf).iter().map(|f| f.id).collect();
        assert_eq!(got, vec![5, 2]);
        let t = Target::new("1.21.1", LoaderKind::Forge);
        let got: Vec<i64> = files_for_target(&mod_project(), &files, &t, &cf).iter().map(|f| f.id).collect();
        assert_eq!(got, vec![4, 2]);
    }

    #[test]
    fn untagged_files_are_judged_by_name() {
        let files =
            vec![file(2, "Thing-forge-1.0", &["1.20.1"], RELEASE), file(1, "Thing-fabric-1.0", &["1.20.1"], RELEASE)];
        let cf = CfSettings::default();
        let t = Target::new("1.20.1", LoaderKind::Fabric);
        // "-forge-" in the name is dropped for Fabric instances
        let got: Vec<i64> = files_for_target(&mod_project(), &files, &t, &cf).iter().map(|f| f.id).collect();
        assert_eq!(got, vec![1]);
    }

    #[test]
    fn sinytra_lets_fabric_mods_onto_forge() {
        let files = vec![file(2, "fabric-only", &["1.20.1", "Fabric"], RELEASE)];
        let cf = CfSettings::default();
        let mut t = Target::new("1.20.1", LoaderKind::Forge);
        assert!(files_for_target(&mod_project(), &files, &t, &cf).is_empty());
        t.sinytra = true;
        assert_eq!(files_for_target(&mod_project(), &files, &t, &cf).len(), 1);
        assert_eq!(t.search_loaders(), vec![1, 4]);
    }

    #[test]
    fn search_loaders_follow_the_java_edition() {
        let t = |l, nf| Target { mc_version: "1.20.1".into(), loader: l, sinytra: false, neoforge_runs_forge: nf };
        assert_eq!(t(LoaderKind::Quilt, false).search_loaders(), vec![4, 5]);
        assert_eq!(t(LoaderKind::NeoForge, true).search_loaders(), vec![6, 1]);
        assert_eq!(t(LoaderKind::NeoForge, false).search_loaders(), vec![6]);
        assert!(t(LoaderKind::None, false).search_loaders().is_empty());
    }

    #[test]
    fn release_picking_and_update_rules() {
        let files = vec![file(9, "c-beta", &[], BETA), file(8, "b", &[], RELEASE), file(7, "a-alpha", &[], ALPHA)];
        assert_eq!(pick_latest(&files, false).unwrap().id, 8);
        assert_eq!(pick_latest(&files, true).unwrap().id, 9);
        // an installed release does not move to a newer beta unless betas are allowed
        assert_eq!(newer_file(&files, 5, Some(RELEASE), false).unwrap().id, 8);
        assert_eq!(newer_file(&files, 5, Some(RELEASE), true).unwrap().id, 9);
        assert_eq!(newer_file(&files, 8, Some(RELEASE), false).map(|f| f.id), None);
        assert_eq!(newer_file(&files, 5, Some(BETA), false).unwrap().id, 9);
    }
}
