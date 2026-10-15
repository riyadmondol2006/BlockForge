//! Mod management: everything the launcher does to the mods, resource packs and shader packs of an instance.
//!
//! * [`metadata`] reads what is inside a jar (name, version, picture, loader, Minecraft versions).
//! * [`files`] keeps the instance's `launcher.mods` list in step with its folders: new files, switching on and off,
//!   removing, and cleaning up records whose file is gone.
//! * [`api`] and [`identify`] talk to Modrinth: finding out what an unknown jar is from its hash.
//! * [`updates`] looks for newer versions, updates, rolls back and keeps the ignored-update lists.
//! * [`install`] puts a Modrinth version into an instance and records it the way the Java edition does.
//! * [`scan`] is the safety check run before the game starts.
//! * [`versions`] holds the pure rules: loader names, version ranges, which version to pick.
//!
//! The list of mods lives in `instance.json` in the Java edition's format, so both editions see the same data.

pub mod api;
pub mod files;
pub mod identify;
pub mod install;
pub mod metadata;
pub mod performance;
pub mod scan;
pub mod trash;
pub mod updates;
pub mod versions;

use serde_json::{Map, Value};

use crate::config::Settings;
use crate::instances::Instance;

pub use metadata::{ContentKind, FileKind};

/// How strictly mods are matched to the Minecraft version (`addModRestriction` in the Java edition).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AddModRestriction {
    /// Only versions made for exactly this Minecraft version.
    #[default]
    Strict,
    /// Versions made for the same Minecraft family (1.20.x).
    Lax,
    /// Any version.
    None,
}

impl AddModRestriction {
    pub fn parse(s: &str) -> Option<AddModRestriction> {
        match s.to_ascii_uppercase().as_str() {
            "STRICT" => Some(AddModRestriction::Strict),
            "LAX" => Some(AddModRestriction::Lax),
            "NONE" => Some(AddModRestriction::None),
            _ => None,
        }
    }

    /// The name the Java edition saves.
    pub fn as_str(self) -> &'static str {
        match self {
            AddModRestriction::Strict => "STRICT",
            AddModRestriction::Lax => "LAX",
            AddModRestriction::None => "NONE",
        }
    }
}

/// The launcher settings that decide how mods are handled. They are read from the settings file's extra keys
/// (the Java edition's field names) with the Java defaults, so a Java-written file means the same thing here.
#[derive(Clone, Debug)]
pub struct ModSettings {
    pub add_mod_restriction: AddModRestriction,
    pub enable_added_mods_by_default: bool,
    pub show_fabric_mods_when_sinytra_installed: bool,
    pub dont_check_mods_on_modrinth: bool,
    pub dont_check_mods_on_curseforge: bool,
    pub scan_mods_on_launch: bool,
    pub use_recycle_bin: bool,
    /// `MODRINTH` or `CURSEFORGE`: which one wins when a mod is known to both.
    pub default_mod_platform: String,
    pub modrinth_api_key: Option<String>,
}

impl Default for ModSettings {
    fn default() -> Self {
        ModSettings {
            add_mod_restriction: AddModRestriction::Strict,
            enable_added_mods_by_default: true,
            show_fabric_mods_when_sinytra_installed: true,
            dont_check_mods_on_modrinth: false,
            dont_check_mods_on_curseforge: false,
            scan_mods_on_launch: true,
            use_recycle_bin: true,
            default_mod_platform: "MODRINTH".into(),
            modrinth_api_key: None,
        }
    }
}

impl ModSettings {
    pub fn from_settings(settings: &Settings) -> ModSettings {
        Self::from_extras(&settings.extras, &settings.default_mod_platform)
    }

    pub fn from_extras(extras: &Map<String, Value>, default_platform: &str) -> ModSettings {
        let d = ModSettings::default();
        let flag = |key: &str, default: bool| extras.get(key).and_then(Value::as_bool).unwrap_or(default);
        // old files only have the on/off switch that came before the three-way setting
        let restriction =
            extras.get("addModRestriction").and_then(Value::as_str).and_then(AddModRestriction::parse).unwrap_or(
                if flag("disableAddModRestrictions", false) {
                    AddModRestriction::None
                } else {
                    AddModRestriction::Strict
                },
            );
        ModSettings {
            add_mod_restriction: restriction,
            enable_added_mods_by_default: flag("enableAddedModsByDefault", d.enable_added_mods_by_default),
            show_fabric_mods_when_sinytra_installed: flag(
                "showFabricModsWhenSinytraInstalled",
                d.show_fabric_mods_when_sinytra_installed,
            ),
            dont_check_mods_on_modrinth: flag("dontCheckModsOnModrinth", d.dont_check_mods_on_modrinth),
            dont_check_mods_on_curseforge: flag("dontCheckModsOnCurseForge", d.dont_check_mods_on_curseforge),
            scan_mods_on_launch: flag("scanModsOnLaunch", d.scan_mods_on_launch),
            use_recycle_bin: flag("useRecycleBin", d.use_recycle_bin),
            default_mod_platform: match default_platform.to_ascii_uppercase().as_str() {
                "CURSEFORGE" => "CURSEFORGE".into(),
                _ => "MODRINTH".into(),
            },
            modrinth_api_key: extras
                .get("modrinthApiKey")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|k| !k.is_empty()),
        }
    }
}

/// What an instance's mods are matched against: its Minecraft version and loader.
#[derive(Clone, Debug)]
pub struct InstanceTarget {
    pub minecraft_version: String,
    /// Modrinth's name for the loader (`fabric`, `quilt`, `forge`, `neoforge`, `legacy-fabric`), if any.
    pub loader_key: Option<&'static str>,
    /// The loader's type as the instance saves it (`Fabric`, `Paper` ...), if any.
    pub loader_type: Option<String>,
    /// Minecraft versions where NeoForge still runs Forge mods.
    pub neoforge_forge_versions: Vec<String>,
}

impl InstanceTarget {
    pub fn of(instance: &Instance, remote_config: &RemoteConfig) -> InstanceTarget {
        let loader_type = instance.loader().map(|(k, _)| k.to_string());
        InstanceTarget {
            minecraft_version: instance.mc_version().to_string(),
            loader_key: loader_type.as_deref().and_then(versions::loader_key),
            loader_type,
            neoforge_forge_versions: remote_config.neoforge_forge_versions.clone(),
        }
    }

    /// The loaders whose mods this instance can run.
    pub fn accepted_loaders(&self) -> Vec<String> {
        versions::accepted_loaders(self.loader_key, &self.minecraft_version, &self.neoforge_forge_versions)
    }
}

/// Everything the network side of mod management needs to know about one instance: where to ask, how strictly to
/// match, and which loader and Minecraft version the mods are for.
#[derive(Clone, Debug)]
pub struct ModContext {
    pub api: api::Modrinth,
    pub settings: ModSettings,
    pub target: InstanceTarget,
    /// Minecraft versions near enough to the instance's when nothing is made for the exact one (`LAX` only).
    pub nearby_versions: Vec<String>,
}

impl ModContext {
    /// Builds the context for an instance. With the `LAX` restriction the list of Minecraft versions is fetched
    /// (it is cached on disk) to know which are in the same family.
    pub async fn build(dirs: &crate::paths::Dirs, instance: &Instance, settings: &Settings) -> ModContext {
        let mod_settings = ModSettings::from_settings(settings);
        let target = InstanceTarget::of(instance, &RemoteConfig::load(dirs));
        let mut nearby_versions = Vec::new();
        if mod_settings.add_mod_restriction == AddModRestriction::Lax {
            if let Ok(manifest) = crate::install::manifest(dirs).await {
                let list: Vec<(String, bool)> =
                    manifest.versions.iter().map(|v| (v.id.clone(), v.kind == "release")).collect();
                nearby_versions = same_family(&target.minecraft_version, &list);
            }
        }
        ModContext {
            api: api::Modrinth::new(mod_settings.modrinth_api_key.clone()),
            settings: mod_settings,
            target,
            nearby_versions,
        }
    }

    /// The Minecraft versions a mod may be made for, per the `addModRestriction` setting: `None` means any.
    pub fn allowed_game_versions(&self) -> Option<Vec<String>> {
        match self.settings.add_mod_restriction {
            AddModRestriction::None => None,
            AddModRestriction::Lax if !self.nearby_versions.is_empty() => Some(self.nearby_versions.clone()),
            _ => Some(vec![self.target.minecraft_version.clone()]),
        }
    }
}

/// The release versions of the same family as `version` (1.20.1 gives 1.20, 1.20.1, 1.20.2 ...), the Java edition's
/// `getMajorMinecraftVersions`. `known` lists every Minecraft version as (id, is_release). A version that is not a
/// release is only its own family.
pub fn same_family(version: &str, known: &[(String, bool)]) -> Vec<String> {
    match known.iter().find(|(id, _)| id == version) {
        Some((_, true)) => {}
        _ => return vec![version.to_string()],
    }
    let family = version.rsplit_once('.').map(|(f, _)| f).unwrap_or(version);
    let prefix = if version.split('.').count() == 2 { version } else { family };
    known
        .iter()
        .filter(|(id, release)| *release && (id == prefix || id.starts_with(&format!("{prefix}."))))
        .map(|(id, _)| id.clone())
        .collect()
}

/// The few values the Java edition reads from its downloaded `config.json`.
#[derive(Clone, Debug)]
pub struct RemoteConfig {
    pub neoforge_forge_versions: Vec<String>,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        RemoteConfig { neoforge_forge_versions: vec!["1.20.1".into()] }
    }
}

impl RemoteConfig {
    /// Reads `configs/json/config.json` (`loaders.neoforge.forgeCompatibleMinecraftVersions`); when the file is
    /// missing the one value known to be true is used.
    pub fn load(dirs: &crate::paths::Dirs) -> RemoteConfig {
        let list = std::fs::read(dirs.configs.join("json").join("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| {
                v.pointer("/loaders/neoforge/forgeCompatibleMinecraftVersions")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
            });
        match list {
            Some(neoforge_forge_versions) => RemoteConfig { neoforge_forge_versions },
            None => RemoteConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_of_minecraft_versions() {
        let known: Vec<(String, bool)> = ["1.20", "1.20.1", "1.20.2", "1.21", "1.21.1", "24w14a"]
            .iter()
            .map(|v| (v.to_string(), !v.contains('w')))
            .collect();
        assert_eq!(same_family("1.20.1", &known), vec!["1.20", "1.20.1", "1.20.2"]);
        assert_eq!(same_family("1.20", &known), vec!["1.20", "1.20.1", "1.20.2"]);
        assert_eq!(same_family("24w14a", &known), vec!["24w14a"]);
        assert_eq!(same_family("9.9", &known), vec!["9.9"]);
    }

    #[test]
    fn settings_read_java_names_and_defaults() {
        let empty = ModSettings::from_extras(&Map::new(), "Modrinth");
        assert_eq!(empty.add_mod_restriction, AddModRestriction::Strict);
        assert!(empty.enable_added_mods_by_default && empty.scan_mods_on_launch && empty.use_recycle_bin);
        assert!(!empty.dont_check_mods_on_modrinth);

        let v: Map<String, Value> = serde_json::from_str(
            r#"{"addModRestriction":"LAX","enableAddedModsByDefault":false,"dontCheckModsOnModrinth":true,"useRecycleBin":false,"modrinthApiKey":"k"}"#,
        )
        .unwrap();
        let s = ModSettings::from_extras(&v, "CurseForge");
        assert_eq!(s.add_mod_restriction, AddModRestriction::Lax);
        assert!(!s.enable_added_mods_by_default && s.dont_check_mods_on_modrinth && !s.use_recycle_bin);
        assert_eq!(s.default_mod_platform, "CURSEFORGE");
        assert_eq!(s.modrinth_api_key.as_deref(), Some("k"));

        let old: Map<String, Value> = serde_json::from_str(r#"{"disableAddModRestrictions":true}"#).unwrap();
        assert_eq!(ModSettings::from_extras(&old, "Modrinth").add_mod_restriction, AddModRestriction::None);
    }
}
