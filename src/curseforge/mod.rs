//! CurseForge, at the same level as the Java edition: searching, installing mods with the things they need,
//! the browser-download flow for files authors do not let launchers fetch, modpacks (install and import),
//! and update checks for single mods and for modpack instances.
//!
//! * [`model`]       the JSON objects (and what is saved into `instance.json`)
//! * [`api`]         the web calls
//! * [`fingerprint`] CurseForge's murmur2 file fingerprint
//! * [`filter`]      which files / loaders fit an instance
//! * [`install`]     adding a file to an instance, dependencies, mod update checks
//! * [`browser`]     files that must be downloaded by hand
//! * [`pack`]        modpacks
//!
//! Settings are read from `Settings.extras` under the Java edition's field names (see [`CfSettings`]).

pub mod api;
pub mod browser;
pub mod filter;
pub mod fingerprint;
pub mod install;
pub mod model;
pub mod pack;

#[cfg(test)]
mod live_tests;

use std::path::PathBuf;
use std::sync::RwLock;

use crate::config::Settings;
use crate::paths::Dirs;

pub use model::{File, Project};

/// The key the Java edition ships with (`Constants.CURSEFORGE_CORE_API_KEY`).
pub const DEFAULT_API_KEY: &str = "$2a$10$.7CSxLm/lnj5lCBSM5jGQ.3SICSX4j9r661AgoB1Rc4Nw8jCMKcv2";
pub const API_URL: &str = "https://api.curseforge.com/v1";
pub const API_KEY_HEADER: &str = "x-api-key";
/// Minecraft's id on CurseForge.
pub const GAME_ID: i64 = 432;
/// How many results one page of a search has.
pub const PAGE_SIZE: u32 = 20;

pub const CLASS_PLUGINS: i64 = 5;
pub const CLASS_MODS: i64 = 6;
pub const CLASS_MODPACKS: i64 = 4471;
pub const CLASS_RESOURCE_PACKS: i64 = 12;
pub const CLASS_WORLDS: i64 = 17;
pub const CLASS_SHADER_PACKS: i64 = 6552;
pub const CLASS_DATA_PACKS: i64 = 6945;

pub const MODLOADER_FORGE: i64 = 1;
pub const MODLOADER_FABRIC: i64 = 4;
pub const MODLOADER_QUILT: i64 = 5;
pub const MODLOADER_NEOFORGE: i64 = 6;

pub const FABRIC_API_ID: i64 = 306612;
pub const LEGACY_FABRIC_API_ID: i64 = 400281;
pub const JUMPLOADER_ID: i64 = 361988;
pub const SINYTRA_CONNECTOR_ID: i64 = 890127;
pub const FORGIFIED_FABRIC_API_ID: i64 = 889079;

/// Modrinth ids the Java edition treats as "the same thing" as the CurseForge projects above.
pub const MODRINTH_FABRIC_API_ID: &str = "P7dR8mSH";
pub const MODRINTH_LEGACY_FABRIC_API_ID: &str = "9CJED7xi";
pub const MODRINTH_FORGIFIED_FABRIC_API_ID: &str = "Aqlf1Shp";
pub const MODRINTH_QSL_ID: &str = "qvIfYCYJ";
pub const MODRINTH_SINYTRA_CONNECTOR_ID: &str = "u58R1TMW";

/// A key set from the program (the settings) after the environment is looked at.
static KEY_OVERRIDE: RwLock<Option<String>> = RwLock::new(None);

/// The API key in use: `BLOCKFORGE_CF_KEY`, else `curseForgeApiKey` from the settings, else the built-in key.
pub fn api_key() -> String {
    if let Some(k) = std::env::var("BLOCKFORGE_CF_KEY").ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        return k;
    }
    if let Ok(g) = KEY_OVERRIDE.read() {
        if let Some(k) = g.as_ref().filter(|k| !k.is_empty()) {
            return k.clone();
        }
    }
    DEFAULT_API_KEY.to_string()
}

/// Remember the key from the settings. Call it after loading and after saving the settings.
pub fn configure(settings: &Settings) {
    let key = settings
        .extras
        .get("curseForgeApiKey")
        .and_then(|v| v.as_str())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let same = KEY_OVERRIDE.read().map(|g| *g == key).unwrap_or(false);
    if !same {
        if let Ok(mut g) = KEY_OVERRIDE.write() {
            *g = key;
        }
    }
}

/// How strictly the add-mods dialog limits files to the instance's Minecraft version (`addModRestriction`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Restriction {
    /// Only files that list the instance's exact version.
    Strict,
    /// Files for any release of the same Minecraft family (1.20.x).
    Lax,
    /// No limit.
    None,
}

/// The CurseForge-related Java settings, read from `Settings` (typed fields and `extras`).
#[derive(Clone, Debug)]
pub struct CfSettings {
    /// `allowCurseForgeAlphaBetaFiles`
    pub allow_alpha_beta: bool,
    /// `addModRestriction` (`disableAddModRestrictions` is the older form of it)
    pub restriction: Restriction,
    /// `dontCheckModsOnCurseForge`
    pub dont_check_mods: bool,
    /// `dontCheckModsOnModrinth`
    pub dont_check_modrinth: bool,
    /// `customDownloadsPath`
    pub custom_downloads_path: Option<PathBuf>,
    /// `showFabricModsWhenSinytraInstalled`
    pub show_fabric_mods_with_sinytra: bool,
    /// `seenCurseForgeProjectDistributionDialog`
    pub seen_distribution_notice: bool,
    /// `defaultModPlatform` is CurseForge
    pub prefer_curseforge: bool,
}

impl Default for CfSettings {
    fn default() -> Self {
        CfSettings {
            allow_alpha_beta: false,
            restriction: Restriction::Strict,
            dont_check_mods: false,
            dont_check_modrinth: false,
            custom_downloads_path: None,
            show_fabric_mods_with_sinytra: true,
            seen_distribution_notice: false,
            prefer_curseforge: false,
        }
    }
}

impl CfSettings {
    pub fn from_settings(s: &Settings) -> CfSettings {
        let flag = |k: &str, d: bool| s.extras.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
        let restriction =
            match s.extras.get("addModRestriction").and_then(|v| v.as_str()).map(|v| v.to_ascii_uppercase()) {
                Some(v) if v == "LAX" => Restriction::Lax,
                Some(v) if v == "NONE" => Restriction::None,
                Some(v) if v == "STRICT" => Restriction::Strict,
                _ => {
                    if flag("disableAddModRestrictions", false) {
                        Restriction::None
                    } else {
                        Restriction::Strict
                    }
                }
            };
        CfSettings {
            allow_alpha_beta: flag("allowCurseForgeAlphaBetaFiles", false),
            restriction,
            dont_check_mods: flag("dontCheckModsOnCurseForge", false),
            dont_check_modrinth: flag("dontCheckModsOnModrinth", false),
            custom_downloads_path: s
                .extras
                .get("customDownloadsPath")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.is_dir()),
            show_fabric_mods_with_sinytra: flag("showFabricModsWhenSinytraInstalled", true),
            seen_distribution_notice: flag("seenCurseForgeProjectDistributionDialog", false),
            prefer_curseforge: s.default_mod_platform.eq_ignore_ascii_case("CurseForge"),
        }
    }

    /// Remember that the player has been told about hand-downloaded files (the Java edition shows that
    /// notice once). Returns `true` when this was the first time, so the caller can show the notice.
    pub fn mark_notice_seen(settings: &mut Settings) -> bool {
        let first =
            !settings.extras.get("seenCurseForgeProjectDistributionDialog").and_then(|v| v.as_bool()).unwrap_or(false);
        settings.extras.insert("seenCurseForgeProjectDistributionDialog".into(), serde_json::Value::Bool(true));
        first
    }
}

/// Switches in the launcher's own `config.json` (`platforms.curseforge.modsEnabled` and `modpacksEnabled`).
/// Missing means on, as in the Java edition.
pub fn platform_enabled(dirs: &Dirs, what: &str) -> bool {
    launcher_config(dirs).and_then(|c| c.get("platforms")?.get("curseforge")?.get(what)?.as_bool()).unwrap_or(true)
}

/// Minecraft versions where NeoForge can also run Forge mods (`loaders.neoforge.forgeCompatibleMinecraftVersions`).
pub fn neoforge_forge_compatible(dirs: &Dirs) -> Vec<String> {
    launcher_config(dirs)
        .and_then(|c| {
            c.get("loaders")?
                .get("neoforge")?
                .get("forgeCompatibleMinecraftVersions")?
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        })
        .unwrap_or_else(|| vec!["1.20.1".to_string()])
}

/// The Java edition keeps a copy of the launcher's remote config at `configs/json/config.json`.
fn launcher_config(dirs: &Dirs) -> Option<serde_json::Value> {
    let bytes = std::fs::read(dirs.configs.join("json").join("config.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The folder the player's browser saves to: `customDownloadsPath` if it exists, else the system Downloads folder.
pub fn user_downloads_dir(custom: Option<&std::path::Path>) -> PathBuf {
    if let Some(p) = custom.filter(|p| p.is_dir()) {
        return p.to_path_buf();
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "linux") {
        // honour a translated or moved Downloads folder
        if let Ok(text) = std::fs::read_to_string(home.join(".config").join("user-dirs.dirs")) {
            for line in text.lines() {
                if let Some(rest) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") {
                    let path = rest.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
                    if !path.is_empty() {
                        return PathBuf::from(path);
                    }
                }
            }
        }
    }
    home.join("Downloads")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key comes from the environment first, then the settings, then the built-in one. (One test, because
    /// it changes the process environment.)
    #[test]
    fn api_key_order() {
        // SAFETY: no other test in this program reads or writes BLOCKFORGE_CF_KEY
        unsafe { std::env::remove_var("BLOCKFORGE_CF_KEY") };
        let mut s = Settings::default();
        configure(&s);
        assert_eq!(api_key(), DEFAULT_API_KEY);
        s.extras.insert("curseForgeApiKey".into(), serde_json::json!("  from-settings  "));
        configure(&s);
        assert_eq!(api_key(), "from-settings");
        unsafe { std::env::set_var("BLOCKFORGE_CF_KEY", "from-env") };
        assert_eq!(api_key(), "from-env");
        unsafe { std::env::set_var("BLOCKFORGE_CF_KEY", "   ") };
        assert_eq!(api_key(), "from-settings", "a blank variable is ignored");
        unsafe { std::env::remove_var("BLOCKFORGE_CF_KEY") };
        configure(&Settings::default());
        assert_eq!(api_key(), DEFAULT_API_KEY);
    }

    #[test]
    fn java_settings_are_read_from_extras() {
        let s: Settings = serde_json::from_str(
            r#"{"defaultModPlatform":"CurseForge","allowCurseForgeAlphaBetaFiles":true,"addModRestriction":"LAX","dontCheckModsOnCurseForge":true,
                "dontCheckModsOnModrinth":true,"showFabricModsWhenSinytraInstalled":false,"seenCurseForgeProjectDistributionDialog":true}"#,
        )
        .unwrap();
        let c = CfSettings::from_settings(&s);
        assert!(
            c.prefer_curseforge
                && c.allow_alpha_beta
                && c.dont_check_mods
                && c.dont_check_modrinth
                && c.seen_distribution_notice
        );
        assert!(!c.show_fabric_mods_with_sinytra);
        assert_eq!(c.restriction, Restriction::Lax);
        // defaults when nothing is set, and the older "disableAddModRestrictions" switch
        let d = CfSettings::from_settings(&Settings::default());
        assert_eq!((d.restriction, d.allow_alpha_beta, d.prefer_curseforge), (Restriction::Strict, false, false));
        let old: Settings = serde_json::from_str(r#"{"disableAddModRestrictions":true}"#).unwrap();
        assert_eq!(CfSettings::from_settings(&old).restriction, Restriction::None);
        // the notice is remembered under the Java name
        let mut s2 = Settings::default();
        assert!(CfSettings::mark_notice_seen(&mut s2));
        assert!(!CfSettings::mark_notice_seen(&mut s2));
        assert_eq!(serde_json::to_value(&s2).unwrap()["seenCurseForgeProjectDistributionDialog"], true);
    }
}
