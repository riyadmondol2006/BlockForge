//! CurseForge's JSON, as the Java edition reads and writes it.
//!
//! These structs are what ends up inside an instance's `launcher.mods` list (`curseForgeProject` /
//! `curseForgeFile`) and in the launcher block of a modpack instance, so their field names, their order and
//! the fields Java leaves out when saving (`downloadUrl`, `downloadCount`, `latestFiles`, `latestFilesIndexes`)
//! all follow `data/curseforge/*.java`. A file saved here opens in the Java edition and the other way round.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{CLASS_DATA_PACKS, CLASS_MODS, CLASS_PLUGINS, CLASS_RESOURCE_PACKS, CLASS_SHADER_PACKS, CLASS_WORLDS};

/// CurseForge sometimes sends `null` where a list or a text is expected; treat that like "nothing".
fn nn<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// The project's links, leaving out the ones CurseForge sends as `null` (Java does not save those either).
fn links_without_nulls<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, String>, D::Error> {
    let raw = Option::<BTreeMap<String, Option<String>>>::deserialize(d)?.unwrap_or_default();
    Ok(raw.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect())
}

/// A number that Java saves as a text (`"2"`), read from either a number or a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NumberText(pub i64);

impl Serialize for NumberText {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for NumberText {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Num(i64),
            Text(String),
        }
        Ok(match Either::deserialize(d)? {
            Either::Num(n) => NumberText(n),
            Either::Text(t) => NumberText(t.trim().parse().unwrap_or(0)),
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Author {
    pub id: i64,
    #[serde(deserialize_with = "nn")]
    pub name: String,
    #[serde(deserialize_with = "nn")]
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Attachment {
    pub id: i64,
    #[serde(deserialize_with = "nn")]
    pub description: String,
    #[serde(deserialize_with = "nn")]
    pub thumbnail_url: String,
    #[serde(deserialize_with = "nn")]
    pub title: String,
    #[serde(deserialize_with = "nn")]
    pub url: String,
    #[serde(alias = "projectId")]
    pub mod_id: i64,
}

/// A category as it is saved inside a project.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Category {
    #[serde(deserialize_with = "nn")]
    pub name: String,
    #[serde(deserialize_with = "nn")]
    pub slug: String,
    #[serde(deserialize_with = "nn")]
    pub url: String,
    #[serde(deserialize_with = "nn")]
    pub date_modified: String,
    pub game_id: i64,
    pub is_class: bool,
    #[serde(alias = "categoryId")]
    pub id: i64,
    #[serde(rename = "iconUrl", alias = "avatarUrl", deserialize_with = "nn")]
    pub avatar_url: String,
    #[serde(alias = "parentId")]
    pub parent_category_id: i64,
    #[serde(alias = "rootId")]
    pub class_id: i64,
}

/// A category from the game-wide list (`/categories`), used to fill the category drop-down.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CategoryForGame {
    pub id: i64,
    #[serde(deserialize_with = "nn")]
    pub name: String,
    #[serde(deserialize_with = "nn")]
    pub slug: String,
    #[serde(deserialize_with = "nn")]
    pub icon_url: String,
    #[serde(deserialize_with = "nn")]
    pub date_modified: String,
    pub parent_category_id: Option<i64>,
    pub class_id: Option<i64>,
    pub game_id: i64,
    #[serde(deserialize_with = "nn")]
    pub url: String,
    pub is_class: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SocialLink {
    #[serde(rename = "type")]
    pub kind: NumberText,
    #[serde(default, deserialize_with = "nn")]
    pub url: String,
}

/// The social link type numbers CurseForge uses.
pub const SOCIAL_DISCORD: i64 = 2;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FileDependency {
    pub file_id: i64,
    #[serde(alias = "addonId")]
    pub mod_id: i64,
    #[serde(alias = "type")]
    pub relation_type: i64,
}

/// What a file asks for: a required library, an optional extra, something it already contains, ...
pub const REL_EMBEDDED_LIBRARY: i64 = 1;
pub const REL_OPTIONAL: i64 = 2;
pub const REL_REQUIRED: i64 = 3;
pub const REL_TOOL: i64 = 4;
pub const REL_INCOMPATIBLE: i64 = 5;
pub const REL_INCLUDE: i64 = 6;

impl FileDependency {
    pub fn is_required(&self) -> bool {
        self.relation_type == REL_REQUIRED
    }
    pub fn is_optional(&self) -> bool {
        self.relation_type == REL_OPTIONAL
    }
    pub fn is_embedded(&self) -> bool {
        self.relation_type == REL_EMBEDDED_LIBRARY || self.relation_type == REL_INCLUDE
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FileHash {
    #[serde(deserialize_with = "nn")]
    pub value: String,
    pub algo: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FileModule {
    pub fingerprint: i64,
    #[serde(alias = "foldername", deserialize_with = "nn")]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SortableGameVersion {
    #[serde(deserialize_with = "nn")]
    pub game_version_padded: String,
    #[serde(deserialize_with = "nn")]
    pub game_version: String,
    #[serde(deserialize_with = "nn")]
    pub game_version_release_date: String,
    #[serde(deserialize_with = "nn")]
    pub game_version_name: String,
}

/// 1 = release, 2 = beta, 3 = alpha.
pub const RELEASE: i64 = 1;
pub const BETA: i64 = 2;
pub const ALPHA: i64 = 3;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct File {
    pub id: i64,
    pub game_id: i64,
    pub is_available: bool,
    #[serde(deserialize_with = "nn")]
    pub display_name: String,
    #[serde(deserialize_with = "nn")]
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_type: Option<i64>,
    pub file_status: i64,
    #[serde(deserialize_with = "nn")]
    pub file_date: String,
    pub file_length: i64,
    /// `null` when the author does not allow other programs to download the file (Java does not save it).
    #[serde(skip_serializing)]
    pub download_url: Option<String>,
    #[serde(deserialize_with = "nn")]
    pub dependencies: Vec<FileDependency>,
    pub alternate_file_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<Vec<FileModule>>,
    pub is_server_pack: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_pack_file_id: Option<i64>,
    #[serde(deserialize_with = "nn")]
    pub hashes: Vec<FileHash>,
    #[serde(skip_serializing)]
    pub download_count: f64,
    #[serde(deserialize_with = "nn")]
    pub sortable_game_versions: Vec<SortableGameVersion>,
    #[serde(alias = "gameVersion", deserialize_with = "nn")]
    pub game_versions: Vec<String>,
    #[serde(rename = "fileFingerprint", alias = "packageFingerprint")]
    pub package_fingerprint: i64,
    #[serde(alias = "projectId")]
    pub mod_id: i64,
}

impl File {
    pub fn is_release(&self) -> bool {
        self.release_type == Some(RELEASE)
    }
    pub fn is_beta(&self) -> bool {
        self.release_type == Some(BETA)
    }
    pub fn is_alpha(&self) -> bool {
        self.release_type == Some(ALPHA)
    }

    /// The sha1 CurseForge lists for the file, if any.
    pub fn sha1(&self) -> Option<&str> {
        self.hashes.iter().find(|h| h.algo == 1).map(|h| h.value.as_str())
    }

    /// The md5 CurseForge lists for the file, if any.
    pub fn md5(&self) -> Option<&str> {
        self.hashes.iter().find(|h| h.algo == 2).map(|h| h.value.as_str())
    }

    /// Whether the file is a modpack (a zip with a `manifest.json`).
    pub fn is_modpack(&self) -> bool {
        self.modules.as_ref().is_some_and(|m| m.iter().any(|m| m.name == "manifest.json"))
    }

    /// The name shown to the player; a modpack's `.zip` ending is dropped.
    pub fn display(&self) -> String {
        let n = &self.display_name;
        if self.is_modpack() && n.len() >= 4 && n[n.len() - 4..].eq_ignore_ascii_case(".zip") {
            return n[..n.len() - 4].to_string();
        }
        n.clone()
    }

    /// The name with the release type added, like the Java drop-down: `Name (Beta)`.
    pub fn label(&self) -> String {
        match self.release_type {
            None => self.display_name.clone(),
            Some(RELEASE) => self.display_name.clone(),
            Some(BETA) => format!("{} (Beta)", self.display_name),
            Some(_) => format!("{} (Alpha)", self.display_name),
        }
    }

    pub fn has_game_version(&self, v: &str) -> bool {
        self.game_versions.iter().any(|g| g == v)
    }

    /// The Minecraft version the file is for. When a file lists several, the newest one wins; versions are
    /// compared by their numbers, which for releases is also their release order.
    pub fn game_version(&self) -> Option<String> {
        let mc: Vec<&String> = self.game_versions.iter().filter(|v| looks_like_minecraft_version(v)).collect();
        // snapshots like `24w14a` have no place in a release order, so releases are compared first
        let releases: Vec<&String> = mc.iter().copied().filter(|v| is_release_version(v)).collect();
        let pool = if releases.is_empty() { mc } else { releases };
        match pool.len() {
            0 => self.game_versions.first().cloned(),
            _ => pool.into_iter().max_by(|a, b| version_key(a).cmp(&version_key(b))).cloned(),
        }
    }
}

/// `1.20.1`, `1.21`, `26.3`, `24w14a` and `1.20-pre1`, but not `Fabric` or `Client`.
pub fn looks_like_minecraft_version(v: &str) -> bool {
    let b = v.as_bytes();
    !b.is_empty() && b[0].is_ascii_digit()
}

/// A plain release number such as `1.20.1` or `26.3` (no snapshot or pre-release letters).
pub fn is_release_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Numbers inside a version, for sorting `1.9` before `1.10`.
pub fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect()
}

/// An entry of a project's list of newest files per game version.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GameVersionLatestFiles {
    #[serde(deserialize_with = "nn")]
    pub game_version: String,
    pub game_version_type_id: i64,
    pub mod_loader: i64,
    #[serde(alias = "projectFileId")]
    pub file_id: i64,
    #[serde(alias = "projectFileName", deserialize_with = "nn")]
    pub filename: String,
    #[serde(alias = "fileType")]
    pub release_type: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Project {
    pub id: i64,
    #[serde(deserialize_with = "nn")]
    pub name: String,
    #[serde(deserialize_with = "nn")]
    pub authors: Vec<Author>,
    pub game_id: i64,
    #[serde(deserialize_with = "nn")]
    pub summary: String,
    #[serde(skip_serializing)]
    pub download_count: f64,
    #[serde(skip_serializing)]
    pub latest_files: Vec<File>,
    #[serde(deserialize_with = "nn")]
    pub categories: Vec<Category>,
    pub status: i64,
    pub primary_category_id: i64,
    pub class_id: i64,
    #[serde(deserialize_with = "nn")]
    pub slug: String,
    pub is_featured: bool,
    #[serde(deserialize_with = "nn")]
    pub date_modified: String,
    #[serde(deserialize_with = "nn")]
    pub date_created: String,
    #[serde(deserialize_with = "nn")]
    pub date_released: String,
    #[serde(deserialize_with = "links_without_nulls")]
    pub links: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub social_links: Option<Vec<SocialLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<Attachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_mod_distribution: Option<bool>,
    #[serde(alias = "attachments", deserialize_with = "nn")]
    pub screenshots: Vec<Attachment>,
    #[serde(alias = "gameVersionLatestFiles", skip_serializing, deserialize_with = "nn")]
    pub latest_files_indexes: Vec<GameVersionLatestFiles>,
    #[serde(alias = "defaultFileId")]
    pub main_file_id: i64,
}

impl Project {
    /// The class (mods, resource packs, ...) of the project's main category, falling back to the project's own
    /// class, the way `getRootCategoryId` does.
    pub fn root_category_id(&self) -> i64 {
        if let Some(c) = self.categories.iter().find(|c| c.id == self.primary_category_id) {
            return c.class_id;
        }
        match self.class_id {
            CLASS_RESOURCE_PACKS | CLASS_WORLDS | CLASS_SHADER_PACKS | CLASS_PLUGINS | CLASS_DATA_PACKS => {
                self.class_id
            }
            _ => CLASS_MODS,
        }
    }

    fn is_data_pack(&self) -> bool {
        self.root_category_id() == CLASS_DATA_PACKS || self.class_id == CLASS_DATA_PACKS
    }

    fn is_shader_pack(&self) -> bool {
        self.root_category_id() == CLASS_SHADER_PACKS || self.class_id == CLASS_SHADER_PACKS
    }

    /// The `type` a modpack records for the project (`getModType`).
    pub fn mod_type(&self) -> &'static str {
        if self.is_data_pack() {
            "datapack"
        } else if self.root_category_id() == CLASS_RESOURCE_PACKS {
            "resourcepack"
        } else if self.is_shader_pack() {
            "shaderpack"
        } else {
            "mods"
        }
    }

    /// The `type` recorded when the player adds the project by hand (`getTypeFromCurseForgeMod`); worlds are
    /// extracted, not recorded.
    pub fn install_type(&self) -> &'static str {
        match self.root_category_id() {
            CLASS_DATA_PACKS => "datapack",
            CLASS_RESOURCE_PACKS => "resourcepack",
            CLASS_WORLDS => "worlds",
            CLASS_SHADER_PACKS => "shaderpack",
            _ => "mods",
        }
    }

    /// The folder (relative to the instance) a file of this project belongs in (`getInstanceDirectoryPath`).
    pub fn instance_folder(&self) -> &'static str {
        if self.is_data_pack() {
            "datapacks"
        } else if self.root_category_id() == CLASS_RESOURCE_PACKS {
            "resourcepacks"
        } else if self.root_category_id() == CLASS_WORLDS {
            "saves"
        } else if self.is_shader_pack() {
            "shaderpacks"
        } else if self.root_category_id() == CLASS_PLUGINS {
            "plugins"
        } else {
            "mods"
        }
    }

    pub fn is_world(&self) -> bool {
        self.root_category_id() == CLASS_WORLDS
    }

    pub fn link(&self, key: &str) -> Option<&str> {
        self.links.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    pub fn website_url(&self) -> Option<&str> {
        self.link("websiteUrl")
    }

    pub fn issues_url(&self) -> Option<&str> {
        self.link("issuesUrl")
    }

    pub fn wiki_url(&self) -> Option<&str> {
        self.link("wikiUrl")
    }

    pub fn social_link(&self, kind: i64) -> Option<&str> {
        self.social_links.as_ref()?.iter().find(|l| l.kind.0 == kind && !l.url.is_empty()).map(|l| l.url.as_str())
    }

    pub fn class_url_slug(&self) -> &'static str {
        match self.class_id {
            CLASS_RESOURCE_PACKS => "texture-packs",
            super::CLASS_MODPACKS => "modpacks",
            CLASS_DATA_PACKS => "data-packs",
            _ => "mc-mods",
        }
    }

    /// The CurseForge page whose button downloads `file`, for files the author does not let other programs fetch.
    pub fn browser_download_url(&self, file: &File) -> String {
        match self.website_url() {
            Some(site) => format!("{site}/download/{}", file.id),
            None => format!(
                "https://www.curseforge.com/minecraft/{}/{}/download/{}",
                self.class_url_slug(),
                self.slug,
                file.id
            ),
        }
    }

    /// The CurseForge page that lists `file`.
    pub fn file_page_url(&self, file: &File) -> String {
        match self.website_url() {
            Some(site) => format!("{site}/files/{}", file.id),
            None => format!(
                "https://www.curseforge.com/minecraft/{}/{}/files/{}",
                self.class_url_slug(),
                self.slug,
                file.id
            ),
        }
    }

    pub fn logo_url(&self) -> Option<&str> {
        self.logo
            .as_ref()
            .map(|l| if l.thumbnail_url.is_empty() { l.url.as_str() } else { l.thumbnail_url.as_str() })
            .filter(|u| !u.is_empty())
    }

    pub fn author_names(&self) -> String {
        self.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
    }
}

impl PartialEq for Project {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Pagination {
    pub index: i64,
    pub page_size: i64,
    pub result_count: i64,
    pub total_count: i64,
}

/// The answer of every CurseForge call: `{ "data": ..., "pagination": ... }`.
#[derive(Deserialize, Debug)]
pub struct Response<T> {
    pub data: T,
    #[serde(default)]
    pub pagination: Option<Pagination>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FingerprintedMod {
    pub id: i64,
    pub file: File,
    pub latest_files: Vec<File>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Fingerprint {
    pub is_cache_built: bool,
    #[serde(deserialize_with = "nn")]
    pub exact_matches: Vec<FingerprintedMod>,
    #[serde(deserialize_with = "nn")]
    pub exact_fingerprints: Vec<i64>,
}

// ---------------------------------------------------------------------------------------------------------
// modpack manifest.json

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ManifestFile {
    #[serde(rename = "projectID")]
    pub project_id: i64,
    #[serde(rename = "fileID")]
    pub file_id: i64,
    pub required: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ManifestModLoader {
    #[serde(deserialize_with = "nn")]
    pub id: String,
    pub primary: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ManifestMinecraft {
    #[serde(deserialize_with = "nn")]
    pub version: String,
    #[serde(deserialize_with = "nn")]
    pub mod_loaders: Vec<ManifestModLoader>,
    /// What the pack's author recommends, in MB (newer exports only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_ram: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Manifest {
    pub minecraft: ManifestMinecraft,
    #[serde(deserialize_with = "nn")]
    pub manifest_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    pub manifest_version: i64,
    #[serde(deserialize_with = "nn")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(deserialize_with = "nn")]
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curse: Option<String>,
    #[serde(rename = "projectID", skip_serializing_if = "Option::is_none")]
    pub project_id: Option<i64>,
    #[serde(rename = "fileID", skip_serializing_if = "Option::is_none")]
    pub file_id: Option<i64>,
    #[serde(deserialize_with = "nn")]
    pub files: Vec<ManifestFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<String>,
}

impl Manifest {
    /// The folder inside the zip whose contents are copied over the instance.
    pub fn overrides_dir(&self) -> &str {
        self.overrides.as_deref().filter(|o| !o.is_empty()).unwrap_or("overrides")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The `curseForgeProject` and `curseForgeFile` objects the Java edition saved for Xaero's Minimap.
    fn java_objects() -> (Value, Value) {
        let v: Value = serde_json::from_str(include_str!("testdata/java-curseforge-objects.json")).unwrap();
        (v["curseForgeProject"].clone(), v["curseForgeFile"].clone())
    }

    #[test]
    fn file_written_back_equals_what_java_wrote() {
        let (_, java_file) = java_objects();
        let file: File = serde_json::from_value(java_file.clone()).unwrap();
        assert_eq!(file.id, 8885210);
        assert_eq!(file.package_fingerprint, 3_100_092_788);
        assert!(file.is_release());
        assert_eq!(file.sha1(), Some("cfce17f4a407bdeef6df03e193473bc8090e1b17"));
        assert_eq!(file.md5(), Some("2e3ac64efe345af7e4cada78ccd85079"));
        assert_eq!(file.game_version().as_deref(), Some("26.3"));
        assert_eq!(serde_json::to_value(&file).unwrap(), java_file);
    }

    #[test]
    fn project_written_back_equals_what_java_wrote() {
        let (java_project, _) = java_objects();
        let p: Project = serde_json::from_value(java_project.clone()).unwrap();
        assert_eq!(p.id, 263420);
        assert_eq!(p.root_category_id(), CLASS_MODS);
        assert_eq!(p.instance_folder(), "mods");
        assert_eq!(p.mod_type(), "mods");
        assert_eq!(p.website_url(), Some("https://www.curseforge.com/minecraft/mc-mods/xaeros-minimap"));
        assert_eq!(serde_json::to_value(&p).unwrap(), java_project);
    }

    #[test]
    fn what_java_leaves_out_is_left_out_but_still_read_from_the_api() {
        // the API sends these; Java reads them but never writes them
        let api = r#"{"id":1,"name":"X","downloadCount":12345.0,"latestFiles":[{"id":9,"downloadUrl":"https://edge.forgecdn.net/files/1/2/x.jar","downloadCount":5}],
            "latestFilesIndexes":[{"gameVersion":"1.20.1","fileId":9,"filename":"x.jar","releaseType":1,"gameVersionTypeId":1,"modLoader":4}],
            "links":{"websiteUrl":"https://example","wikiUrl":null},"classId":6,"categories":null,"authors":null}"#;
        let p: Project = serde_json::from_str(api).unwrap();
        assert_eq!(p.latest_files[0].download_url.as_deref(), Some("https://edge.forgecdn.net/files/1/2/x.jar"));
        assert_eq!(p.latest_files_indexes[0].file_id, 9);
        assert_eq!(p.download_count, 12345.0);
        let out = serde_json::to_value(&p).unwrap();
        for k in ["downloadCount", "latestFiles", "latestFilesIndexes"] {
            assert!(out.get(k).is_none(), "{k} must not be saved");
        }
        assert!(out["links"].get("wikiUrl").is_none(), "null links are not saved");
        let f: File = serde_json::from_str(r#"{"id":2,"downloadUrl":null,"gameVersion":["1.20.1"],"packageFingerprint":77,"projectId":5,"releaseType":2}"#).unwrap();
        assert!(f.download_url.is_none());
        assert_eq!((f.mod_id, f.package_fingerprint, f.game_versions.len()), (5, 77, 1));
        let out = serde_json::to_value(&f).unwrap();
        assert!(out.get("downloadUrl").is_none());
        assert_eq!(out["fileFingerprint"], 77);
        assert_eq!(out["modId"], 5);
        assert!(f.is_beta());
        assert_eq!(File { display_name: "Pack 1.0".into(), ..f }.label(), "Pack 1.0 (Beta)");
    }

    #[test]
    fn social_link_types_are_saved_as_text() {
        let p: Project =
            serde_json::from_str(r#"{"id":1,"socialLinks":[{"type":2,"url":"https://discord.gg/abc"}]}"#).unwrap();
        assert_eq!(p.social_link(SOCIAL_DISCORD), Some("https://discord.gg/abc"));
        assert_eq!(serde_json::to_value(&p).unwrap()["socialLinks"][0]["type"], "2");
        // and read back from the text form
        let p2: Project = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(p2.social_link(SOCIAL_DISCORD), Some("https://discord.gg/abc"));
    }

    #[test]
    fn project_classes_pick_types_and_folders() {
        let mk = |class: i64| Project { class_id: class, primary_category_id: 0, ..Default::default() };
        assert_eq!(
            (mk(CLASS_RESOURCE_PACKS).mod_type(), mk(CLASS_RESOURCE_PACKS).instance_folder()),
            ("resourcepack", "resourcepacks")
        );
        assert_eq!(
            (mk(CLASS_SHADER_PACKS).mod_type(), mk(CLASS_SHADER_PACKS).instance_folder()),
            ("shaderpack", "shaderpacks")
        );
        assert_eq!(
            (mk(CLASS_DATA_PACKS).mod_type(), mk(CLASS_DATA_PACKS).instance_folder()),
            ("datapack", "datapacks")
        );
        assert_eq!((mk(CLASS_WORLDS).install_type(), mk(CLASS_WORLDS).instance_folder()), ("worlds", "saves"));
        assert_eq!(mk(CLASS_WORLDS).mod_type(), "mods");
        assert_eq!(mk(CLASS_MODS).install_type(), "mods");
        assert_eq!(mk(CLASS_PLUGINS).instance_folder(), "plugins");
    }

    #[test]
    fn browser_pages_follow_the_java_edition() {
        let mut p = Project { slug: "some-mod".into(), class_id: CLASS_MODS, ..Default::default() };
        let f = File { id: 77, ..Default::default() };
        assert_eq!(p.browser_download_url(&f), "https://www.curseforge.com/minecraft/mc-mods/some-mod/download/77");
        p.links.insert("websiteUrl".into(), "https://www.curseforge.com/minecraft/mc-mods/some-mod".into());
        assert_eq!(p.browser_download_url(&f), "https://www.curseforge.com/minecraft/mc-mods/some-mod/download/77");
        assert_eq!(p.file_page_url(&f), "https://www.curseforge.com/minecraft/mc-mods/some-mod/files/77");
    }

    #[test]
    fn newest_minecraft_version_of_a_file_wins() {
        let f = File {
            game_versions: vec!["Forge".into(), "1.7.10".into(), "1.12.2".into(), "1.9.4".into(), "24w14a".into()],
            ..Default::default()
        };
        assert_eq!(f.game_version().as_deref(), Some("1.12.2"));
        let g = File { game_versions: vec!["Client".into()], ..Default::default() };
        assert_eq!(g.game_version().as_deref(), Some("Client"));
    }
}
