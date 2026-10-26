//! The description of one version of an ATLauncher pack (`Configs.json` on the pack servers). Technic packs are
//! turned into the same shape so one installer handles both. Field names and defaults follow the Java edition's
//! `data/json/*` classes, so a value means the same thing in both editions.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How a mod file reaches the computer.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadKind {
    /// From the pack servers: the `url` is a path under the download server.
    Server,
    /// The player has to fetch it in a web browser (the site does not allow direct downloads).
    Browser,
    /// A plain link to the file.
    Direct,
}

/// Where a mod goes and how it is treated (`type` in the JSON).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModKind {
    Jar,
    Dependency,
    /// A misspelling that exists in some pack files; it means the same as `dependency`.
    Depandency,
    Forge,
    Mcpc,
    Mods,
    Plugins,
    Ic2lib,
    Denlib,
    Flan,
    Coremods,
    Extract,
    Decomp,
    Millenaire,
    Texturepack,
    Resourcepack,
    Texturepackextract,
    Resourcepackextract,
    Shaderpack,
    Datapack,
}

impl ModKind {
    /// The name the Java edition saves in `launcher.mods[].type` (`depandency` has no entry there, it is
    /// `dependency`).
    pub fn record_name(self) -> &'static str {
        match self {
            ModKind::Jar => "jar",
            ModKind::Dependency | ModKind::Depandency => "dependency",
            ModKind::Forge => "forge",
            ModKind::Mcpc => "mcpc",
            ModKind::Mods => "mods",
            ModKind::Plugins => "plugins",
            ModKind::Ic2lib => "ic2lib",
            ModKind::Denlib => "denlib",
            ModKind::Flan => "flan",
            ModKind::Coremods => "coremods",
            ModKind::Extract => "extract",
            ModKind::Decomp => "decomp",
            ModKind::Millenaire => "millenaire",
            ModKind::Texturepack => "texturepack",
            ModKind::Resourcepack => "resourcepack",
            ModKind::Texturepackextract => "texturepackextract",
            ModKind::Resourcepackextract => "resourcepackextract",
            ModKind::Shaderpack => "shaderpack",
            ModKind::Datapack => "datapack",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExtractTo {
    Root,
    Mods,
    Coremods,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DecompKind {
    Jar,
    Mods,
    Coremods,
    Root,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaseKind {
    Upper,
    Lower,
}

fn yes() -> bool {
    true
}

/// An enum field that pack files sometimes spell in another case (`createzip`) or with a value that does not exist.
/// The Java edition's JSON reader turns those into nothing; so does this.
pub fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    let Value::String(text) = v else { return Ok(None) };
    Ok(serde_json::from_value(Value::String(text.clone()))
        .ok()
        .or_else(|| serde_json::from_value(Value::String(text.to_lowercase())).ok()))
}

/// A text field that some pack files write as `null`: read as empty.
pub fn nullable_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// One mod (or resource pack, config bundle, library mod ...) of a pack version.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct PackMod {
    #[serde(deserialize_with = "nullable_string")]
    pub name: String,
    #[serde(deserialize_with = "nullable_string")]
    pub version: String,
    #[serde(deserialize_with = "nullable_string")]
    pub url: String,
    #[serde(deserialize_with = "nullable_string")]
    pub file: String,
    pub path: Option<String>,
    pub md5: Option<String>,
    pub filesize: i64,
    pub fingerprint: Option<i64>,
    #[serde(deserialize_with = "lenient")]
    pub download: Option<DownloadKind>,
    pub website: Option<String>,
    pub donation: Option<String>,
    pub authors: Vec<String>,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub colour: Option<String>,
    pub warning: Option<String>,
    pub force: bool,
    #[serde(rename = "type")]
    #[serde(deserialize_with = "lenient")]
    pub kind: Option<ModKind>,
    #[serde(deserialize_with = "lenient")]
    pub extract_to: Option<ExtractTo>,
    pub extract_folder: Option<String>,
    pub decomp_file: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub decomp_type: Option<DecompKind>,
    pub file_pattern: bool,
    pub file_preference: Option<String>,
    pub file_check: Option<String>,
    #[serde(default = "yes")]
    pub client: bool,
    #[serde(default = "yes")]
    pub server: bool,
    pub server_separate: bool,
    pub server_url: Option<String>,
    pub server_file: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub server_type: Option<ModKind>,
    #[serde(deserialize_with = "lenient")]
    pub server_download: Option<DownloadKind>,
    #[serde(rename = "serverMD5")]
    pub server_md5: Option<String>,
    pub server_optional: Option<bool>,
    pub optional: bool,
    pub selected: bool,
    #[serde(default = "yes")]
    pub recommended: bool,
    pub hidden: bool,
    pub library: bool,
    pub group: Option<String>,
    pub linked: Option<String>,
    pub depends: Option<Vec<String>>,
    pub file_prefix: Option<String>,
    pub description: Option<String>,
    pub ignore_failures: bool,
    #[serde(rename = "curseforge_project_id", alias = "curse_id")]
    pub curse_forge_project_id: Option<i64>,
    #[serde(rename = "curseforge_file_id", alias = "curse_file_id")]
    pub curse_forge_file_id: Option<i64>,
    /// Modrinth objects the Java edition attaches to mods it found there; kept untouched.
    pub modrinth_project: Option<Value>,
    pub modrinth_version: Option<Value>,
    pub curse_forge_project: Option<Value>,
    pub curse_forge_file: Option<Value>,
}

impl Default for PackMod {
    fn default() -> Self {
        PackMod {
            name: String::new(),
            version: String::new(),
            url: String::new(),
            file: String::new(),
            path: None,
            md5: None,
            filesize: 0,
            fingerprint: None,
            download: None,
            website: None,
            donation: None,
            authors: Vec::new(),
            sha1: None,
            sha512: None,
            colour: None,
            warning: None,
            force: false,
            kind: None,
            extract_to: None,
            extract_folder: None,
            decomp_file: None,
            decomp_type: None,
            file_pattern: false,
            file_preference: None,
            file_check: None,
            client: true,
            server: true,
            server_separate: false,
            server_url: None,
            server_file: None,
            server_type: None,
            server_download: None,
            server_md5: None,
            server_optional: None,
            optional: false,
            selected: false,
            recommended: true,
            hidden: false,
            library: false,
            group: None,
            linked: None,
            depends: None,
            file_prefix: None,
            description: None,
            ignore_failures: false,
            curse_forge_project_id: None,
            curse_forge_file_id: None,
            modrinth_project: None,
            modrinth_version: None,
            curse_forge_file: None,
            curse_forge_project: None,
        }
    }
}

impl PackMod {
    /// Name with everything but letters and digits removed (used for temporary folders).
    pub fn safe_name(&self) -> String {
        self.name.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
    }

    /// The link with `&amp;` turned into `&` and spaces escaped, the way the Java edition cleans it.
    pub fn clean_url(&self) -> String {
        self.url.replace("&amp;", "&").replace(' ', "%20")
    }

    /// The address to download from: server mods live under the pack download server.
    pub fn download_url(&self) -> String {
        if self.download == Some(DownloadKind::Server) {
            return format!("{}/{}", super::DOWNLOAD_SERVER, self.clean_url());
        }
        self.clean_url()
    }

    /// File name including the optional prefix.
    pub fn file_name(&self) -> String {
        match &self.file_prefix {
            Some(p) => format!("{p}{}", self.file),
            None => self.file.clone(),
        }
    }

    pub fn is_optional(&self, server: bool) -> bool {
        if server { self.server_optional.unwrap_or(self.optional) } else { self.optional }
    }

    pub fn installs_on(&self, server: bool) -> bool {
        if server { self.server } else { self.client }
    }

    pub fn has_group(&self) -> bool {
        self.group.as_deref().is_some_and(|g| !g.is_empty())
    }

    pub fn has_linked(&self) -> bool {
        self.linked.as_deref().is_some_and(|g| !g.is_empty())
    }

    pub fn depends_list(&self) -> &[String] {
        self.depends.as_deref().unwrap_or(&[])
    }

    pub fn has_md5(&self) -> bool {
        self.md5.as_deref().is_some_and(|m| !m.is_empty())
    }

    /// Whether a separate file is used on servers.
    pub fn uses_server_file(&self, is_server: bool) -> bool {
        is_server && self.server_url.is_some()
    }
}

/// A library the pack brings (mostly for old Minecraft versions).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PackLibrary {
    #[serde(deserialize_with = "nullable_string")]
    pub url: String,
    pub path: Option<String>,
    #[serde(deserialize_with = "nullable_string")]
    pub file: String,
    pub server: Option<String>,
    pub md5: Option<String>,
    pub force: bool,
    #[serde(deserialize_with = "lenient")]
    pub download: Option<DownloadKind>,
    pub filesize: i64,
    pub depends: Option<String>,
    pub depends_group: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MainClass {
    pub main_class: Option<String>,
    pub depends: Option<String>,
    pub depends_group: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtraArguments {
    pub arguments: Option<String>,
    pub depends: Option<String>,
    pub depends_group: Option<String>,
}

/// The Java versions a pack version works with (`java` in the JSON). Zero means no limit.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct JavaRequirement {
    pub min: i64,
    pub max: i64,
}

impl JavaRequirement {
    pub fn conforms(&self, java_major: i64) -> bool {
        let mut ok = self.min == 0 || java_major >= self.min;
        if self.max != 0 && java_major > self.max {
            ok = false;
        }
        ok
    }

    /// Text for the "wrong Java" warning, e.g. `Java 21 minimum`.
    pub fn describe(&self) -> String {
        if self.min != 0 && self.max != 0 && self.min == self.max {
            return format!("Java {}", self.min);
        }
        if self.min != 0 && self.max != 0 && self.min + 1 == self.max {
            return format!("Java {} or {}", self.min, self.max);
        }
        let mut s = String::new();
        if self.min != 0 {
            s += &format!("Java {}{}", self.min, if self.max == 0 { " minimum" } else { "" });
        }
        if self.max != 0 {
            s += &format!(
                "{}Java {}",
                if !s.is_empty() { " up to, and including, " } else { "Nothing newer than " },
                self.max
            );
        }
        s
    }
}

/// The loader a pack version wants (`loader` in the JSON).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PackLoader {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub choose: bool,
    pub metadata: Map<String, Value>,
    pub class_name: Option<String>,
    pub choose_class_name: Option<String>,
    pub choose_method: Option<String>,
}

/// One entry of `deletes` or `keeps`: `base` is `root` or `config`, `target` the path below it
/// (`%s%` stands for the path separator).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PathRule {
    pub base: String,
    pub target: String,
}

impl PathRule {
    /// The target as a path that stays below its base: `%s%` turned into separators first, then anything empty,
    /// absolute, with a drive letter or with `..` refused. (Rust's `join` replaces the whole path with an absolute
    /// one, where Java's `new File(parent, child)` didn't, so the Java edition's text checks are not enough here.)
    fn relative(&self) -> Option<std::path::PathBuf> {
        let target = self.target.replace("%s%", "/");
        if target.contains('~') {
            return None;
        }
        crate::packutil::safe_relative(&target)
    }

    /// Deleting: nothing that could reach worlds, `instance.json` or outside the instance.
    pub fn delete_allowed(&self) -> bool {
        self.relative().is_some_and(|rel| !self.reaches_player_data(&rel))
    }

    /// Keeping a file through a reinstall has the same limits.
    pub fn keep_allowed(&self) -> bool {
        self.delete_allowed()
    }

    /// Whether a rule below the instance folder points at worlds or the instance's own file, whatever the case
    /// (`Saves` is the same folder as `saves` on Windows and macOS).
    fn reaches_player_data(&self, rel: &std::path::Path) -> bool {
        if !self.base.eq_ignore_ascii_case("root") && !self.base.is_empty() {
            return false;
        }
        crate::packs::is_player_data(rel)
    }

    /// The path below `root` (or below `<root>/config` for `base: config`); `None` when the target would lead
    /// anywhere else.
    pub fn resolve(&self, root: &std::path::Path) -> Option<std::path::PathBuf> {
        let base = if self.base.eq_ignore_ascii_case("config") { root.join("config") } else { root.to_path_buf() };
        self.relative().map(|rel| base.join(rel))
    }

    /// The same target, but below a temporary folder (no `config` prefix: kept files are stored by target).
    pub fn resolve_flat(&self, temp: &std::path::Path) -> Option<std::path::PathBuf> {
        self.relative().map(|rel| temp.join(rel))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PathRules {
    pub files: Option<Vec<PathRule>>,
    pub folders: Option<Vec<PathRule>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Messages {
    pub install: Option<String>,
    pub update: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    #[serde(alias = "createzip")]
    CreateZip,
    Rename,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActionAfter {
    Delete,
    Nothing,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActionFolder {
    Jar,
    Mods,
    Coremods,
}

/// Something done after the mods are in place: join several into one zip, or rename a file.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PackAction {
    /// Mod names the action works on (`mod` in the file).
    #[serde(rename = "mod")]
    pub mod_names: Vec<String>,
    #[serde(deserialize_with = "lenient")]
    pub action: Option<ActionKind>,
    #[serde(rename = "type")]
    #[serde(deserialize_with = "lenient")]
    pub kind: Option<ActionFolder>,
    #[serde(deserialize_with = "lenient")]
    pub after: Option<ActionAfter>,
    pub save_as: Option<String>,
    pub client: bool,
    pub server: bool,
}

/// Size and checksum of `Configs.zip`.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ConfigsInfo {
    pub filesize: i64,
    pub sha1: Option<String>,
}

impl Default for ConfigsInfo {
    fn default() -> Self {
        ConfigsInfo { filesize: -1, sha1: None }
    }
}

/// One whole pack version.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct PackJson {
    #[serde(deserialize_with = "nullable_string")]
    pub version: String,
    #[serde(deserialize_with = "nullable_string")]
    pub minecraft: String,
    pub memory: i64,
    #[serde(rename = "permGen", alias = "permgen")]
    pub perm_gen: i64,
    pub no_configs: bool,
    #[serde(rename = "enableCurseForgeIntegration", alias = "enableCurseIntegration")]
    pub enable_curse_forge_integration: bool,
    pub enable_editing_mods: bool,
    #[serde(deserialize_with = "lenient")]
    pub case_all_files: Option<CaseKind>,
    pub main_class: Option<MainClass>,
    pub extra_arguments: Option<ExtraArguments>,
    pub server_arguments: String,
    pub java: Option<JavaRequirement>,
    pub loader: Option<PackLoader>,
    pub deletes: Option<PathRules>,
    pub keeps: Option<PathRules>,
    pub messages: Option<Messages>,
    pub warnings: HashMap<String, String>,
    pub libraries: Vec<PackLibrary>,
    pub colours: HashMap<String, String>,
    pub mods: Vec<PackMod>,
    pub actions: Vec<PackAction>,
    pub configs: Option<ConfigsInfo>,
}

impl Default for PackJson {
    fn default() -> Self {
        PackJson {
            version: String::new(),
            minecraft: String::new(),
            memory: 0,
            perm_gen: 0,
            no_configs: false,
            enable_curse_forge_integration: false,
            enable_editing_mods: true,
            case_all_files: None,
            main_class: None,
            extra_arguments: None,
            server_arguments: String::new(),
            java: None,
            loader: None,
            deletes: None,
            keeps: None,
            messages: None,
            warnings: HashMap::new(),
            libraries: Vec::new(),
            colours: HashMap::new(),
            mods: Vec::new(),
            actions: Vec::new(),
            configs: None,
        }
    }
}

impl PackJson {
    pub fn parse(bytes: &[u8]) -> anyhow::Result<PackJson> {
        let json: PackJson = serde_json::from_slice(bytes)?;
        json.check_paths()?;
        Ok(json)
    }

    /// Refuse a pack whose file names or folders would put files outside the place they belong (an absolute
    /// path, a drive letter, `..`). Every later step joins these to a folder as they are, so this is the one check.
    pub fn check_paths(&self) -> anyhow::Result<()> {
        use crate::packutil::{safe_file_name, safe_relative};
        // empty or "this folder" (`./`, which Java read as the folder itself) is the default folder
        let here = |p: &str| matches!(p, "" | "." | "./" | ".\\");
        let folder_ok = |p: &Option<String>| p.as_deref().is_none_or(|p| here(p) || safe_relative(p).is_some());
        let name_ok = |n: &str| n.is_empty() || safe_file_name(n);
        for m in &self.mods {
            // with `filePattern` the file is a pattern to look for, not a name to write
            let file_ok = m.file_pattern || name_ok(&m.file_name());
            let server_ok = m.server_file.as_deref().is_none_or(name_ok);
            if !(file_ok
                && server_ok
                && folder_ok(&m.path)
                && folder_ok(&m.extract_folder)
                && folder_ok(&m.decomp_file))
            {
                anyhow::bail!(
                    "the pack's mod \"{}\" has a file name or folder that points outside the instance",
                    m.name
                );
            }
        }
        for a in &self.actions {
            if !a.save_as.as_deref().is_none_or(name_ok) {
                anyhow::bail!("one of the pack's actions saves to a name that isn't allowed");
            }
        }
        for l in &self.libraries {
            if !(name_ok(&l.file) && folder_ok(&l.path) && folder_ok(&l.server)) {
                anyhow::bail!("the pack's library {} has a path that points outside the libraries folder", l.file);
            }
        }
        Ok(())
    }

    /// The colour a mod asked for (a key of `colours`), as red, green, blue.
    pub fn colour_rgb(&self, key: &str) -> Option<(u8, u8, u8)> {
        let hex = self.colours.get(key)?.trim_start_matches('#').to_string();
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let n = u32::from_str_radix(&hex, 16).ok()?;
        Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
 "enableCurseIntegration": true,
 "enableEditingMods": true,
 "version": "1.21.11-1",
 "minecraft": "1.21.11",
 "memory": 8192,
 "permgen": 512,
 "java": {"min": 25},
 "loader": {"type": "fabric", "choose": false, "metadata": {"minecraft": "1.21.11", "loader": "0.18.4"},
   "className": "com.atlauncher.data.loaders.fabric.FabricLoader"},
 "deletes": {"folders": [{"base": "root", "target": "scripts%s%"}]},
 "colours": {"Green": "cef4cb"},
 "extraArguments": {"arguments": "--tweakClass=cpw.mods.fml.common.launcher.FMLTweaker"},
 "actions": [{"mod": ["A","B"], "action": "createZip", "type": "mods", "after": "delete", "saveAs": "ab.zip", "client": true, "server": false}],
 "libraries": [{"url":"packs/X/files/libraries/asm-all-4.1.jar","file":"asm-all-4.1.jar","server":"org/ow2/asm/asm-all/4.1/asm-all-4.1.jar","md5":"d21c2a06a4e6b175aa01e328f38a1182","download":"server","filesize":214592,"force":false}],
 "mods": [
  {"name":"Cardinal Components API","version":"7.3.0","url":"packs/Lithium/files/a b.jar","file":"cardinal.jar","download":"server",
   "md5":"b64bc620d91ab9f208a9c18a35f97586","filesize":234840,"curse_id":318449,"curse_file_id":7315135,"force":false,"type":"mods",
   "client":true,"optional":false,"server":true,"selected":false,"recommended":true,"hidden":false,"library":false,
   "description":"Attaching more data","website":"https://modrinth.com/mod/x","authors":["UpcraftLP"],"depends":["Fabric API"]},
  {"name":"Old","version":"1","url":"x","file":"x.jar","download":"browser","type":"depandency","optional":true}
 ],
 "configs": {"filesize": 4155, "sha1": "abc"}
}"##;

    #[test]
    fn parses_a_real_style_pack_version() {
        let j = PackJson::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(j.version, "1.21.11-1");
        assert!(j.enable_curse_forge_integration);
        assert!(j.enable_editing_mods);
        assert_eq!(j.memory, 8192);
        assert_eq!(j.perm_gen, 512);
        assert_eq!(j.java, Some(JavaRequirement { min: 25, max: 0 }));
        let loader = j.loader.as_ref().unwrap();
        assert_eq!(loader.metadata["loader"], "0.18.4");
        assert!(loader.class_name.as_deref().unwrap().ends_with("FabricLoader"));
        assert_eq!(j.deletes.as_ref().unwrap().folders.as_ref().unwrap()[0].target, "scripts%s%");
        assert_eq!(j.actions[0].mod_names, vec!["A", "B"]);
        assert_eq!(j.actions[0].action, Some(ActionKind::CreateZip));
        assert_eq!(j.libraries[0].server.as_deref(), Some("org/ow2/asm/asm-all/4.1/asm-all-4.1.jar"));
        assert_eq!(j.configs.as_ref().unwrap().filesize, 4155);
        let m = &j.mods[0];
        assert_eq!(m.curse_forge_project_id, Some(318449));
        assert_eq!(m.curse_forge_file_id, Some(7315135));
        assert_eq!(m.kind, Some(ModKind::Mods));
        assert!(m.client && m.server && m.recommended);
        assert_eq!(m.download_url(), "https://download.nodecdn.net/containers/atl/packs/Lithium/files/a%20b.jar");
        assert_eq!(j.mods[1].kind, Some(ModKind::Depandency));
        assert_eq!(j.mods[1].kind.unwrap().record_name(), "dependency");
        assert_eq!(j.mods[1].download, Some(DownloadKind::Browser));
        assert_eq!(j.colour_rgb("Green"), Some((0xce, 0xf4, 0xcb)));
    }

    #[test]
    fn java_requirement_text_matches_the_java_edition() {
        assert_eq!(JavaRequirement { min: 8, max: 8 }.describe(), "Java 8");
        assert_eq!(JavaRequirement { min: 8, max: 9 }.describe(), "Java 8 or 9");
        assert_eq!(JavaRequirement { min: 17, max: 0 }.describe(), "Java 17 minimum");
        assert_eq!(JavaRequirement { min: 0, max: 8 }.describe(), "Nothing newer than Java 8");
        assert_eq!(JavaRequirement { min: 8, max: 17 }.describe(), "Java 8 up to, and including, Java 17");
        assert!(JavaRequirement { min: 8, max: 17 }.conforms(11));
        assert!(!JavaRequirement { min: 8, max: 17 }.conforms(21));
        assert!(!JavaRequirement { min: 25, max: 0 }.conforms(21));
    }

    #[test]
    fn delete_rules_keep_worlds_safe() {
        let r = |base: &str, target: &str| PathRule { base: base.into(), target: target.into() };
        assert!(r("root", "scripts%s%").delete_allowed());
        assert!(!r("root", "world").delete_allowed());
        assert!(!r("root", "saves%s%x").delete_allowed());
        assert!(!r("root", "../x").delete_allowed());
        assert!(!r("root", "instance.json").delete_allowed());
        assert!(r("config", "x.cfg").delete_allowed());
        assert!(!r("config", "../x.cfg").keep_allowed());
        assert!(!r("root", "DIM1").keep_allowed());
        assert!(r("config", "NEI.cfg").keep_allowed());
        assert_eq!(
            r("config", "a%s%b.cfg").resolve(std::path::Path::new("/i")),
            Some(std::path::Path::new("/i/config").join("a").join("b.cfg"))
        );
    }

    /// Ways a bad or hostile pack definition got past the checks before: each would have deleted worlds or files
    /// outside the instance on a reinstall.
    #[test]
    fn delete_and_keep_rules_cannot_escape_or_reach_worlds() {
        let r = |base: &str, target: &str| PathRule { base: base.into(), target: target.into() };
        let root = std::path::Path::new("/i");
        for (base, target) in [
            ("config", "../saves"),
            ("config", "..%s%saves"),
            ("root", "..%s%..%s%Documents"),
            ("root", "/home/u"),
            ("root", "%s%home%s%u"),
            ("root", ""),
            ("root", "."),
            ("root", "Saves"),
            ("root", "SAVES%s%World"),
            ("root", "World_nether"),
            ("root", "dim-1"),
            ("root", "Instance.JSON"),
            ("root", "~/x"),
            ("root", "a%s%..%s%..%s%x"),
        ] {
            let rule = r(base, target);
            assert!(!rule.delete_allowed(), "delete {base}:{target}");
            assert!(!rule.keep_allowed(), "keep {base}:{target}");
            if let Some(p) = rule.resolve(root) {
                assert!(p.starts_with(root) && p != root, "{base}:{target} -> {}", p.display());
            }
        }
        // ordinary rules still work
        assert!(r("root", "scripts%s%old.zs").delete_allowed());
        assert!(r("config", "saves.cfg").delete_allowed(), "below config/ a name like saves is just a file");
    }

    #[test]
    fn packs_whose_files_would_land_outside_are_refused() {
        let bad = [
            r#"{"mods":[{"name":"m","file":"../../evil.jar"}]}"#,
            r#"{"mods":[{"name":"m","file":"a.jar","path":"/home/u/.config/autostart"}]}"#,
            r#"{"mods":[{"name":"m","file":"a.jar","path":"..\..\x"}]}"#,
            r#"{"mods":[{"name":"m","file":"a.jar","filePrefix":"../"}]}"#,
            r#"{"mods":[{"name":"m","file":"a.jar","serverFile":"C:x.jar"}]}"#,
            r#"{"mods":[{"name":"m","file":"a.zip","extractFolder":"/etc"}]}"#,
            r#"{"mods":[{"name":"m","file":"a.zip","decompFile":"../../../etc/passwd"}]}"#,
            r#"{"actions":[{"action":"createZip","mod":["a","b"],"saveAs":"../../x.jar"}]}"#,
            r#"{"libraries":[{"file":"x.jar","path":"/usr/lib/x.jar"}]}"#,
            r#"{"libraries":[{"file":"x.jar","server":"../../x.jar"}]}"#,
        ];
        for json in bad {
            assert!(PackJson::parse(json.as_bytes()).is_err(), "{json}");
        }
        // a file pattern is a pattern, not a name, and ordinary packs still read
        assert!(PackJson::parse(br#"{"mods":[{"name":"m","file":"jei-.*\\.jar","filePattern":true}]}"#).is_ok());
        assert!(PackJson::parse(br#"{"mods":[{"name":"m","file":"a b.jar","path":"mods/sub/"}]}"#).is_ok());
        assert!(PackJson::parse(br#"{"mods":[{"name":"m","file":"a.jar","path":""}]}"#).is_ok());
        assert!(PackJson::parse(br#"{"mods":[{"name":"m","file":"a.zip","extractFolder":"./","path":"."}]}"#).is_ok());
        assert!(PackJson::parse(br#"{"mods":[{"name":"m","file":"a.zip","extractFolder":"/"}]}"#).is_err());
    }
}
