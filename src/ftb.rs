//! Feed The Beast modpacks: the public listing, pack details and versions, installing a pack as an instance,
//! and checking for / applying updates. Everything is written into `instance.json` the way the Java edition
//! writes it (`launcher.ftbPackManifest`, `launcher.ftbPackVersionManifest`, `launcher.overridePaths`), so an
//! instance made by one edition updates and plays in the other.
//!
//! A few FTB mod files have no download address of their own and live on CurseForge. They are resolved through
//! [`crate::cf_seam::CurseForgeSource`], the seam to the CurseForge package.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::cf_seam::{CfFile, CfProject, CurseForgeSource};
use crate::http;
use crate::install::{self, Job, Progress};
use crate::instances::{self, Instance};
use crate::loaders::{self, Loader};
use crate::modcheck;
use crate::modrecord::ModRecord;
use crate::packutil;
use crate::paths::Dirs;

pub const API: &str = "https://api.feed-the-beast.com/v1/modpacks/public";
/// Packs shown per page in the browser (`Constants.FTB_PAGINATION_SIZE`).
pub const PAGE_SIZE: usize = 20;

const LIST_TTL: Duration = Duration::from_secs(600);
const PACK_TTL: Duration = Duration::from_secs(3600);

// ---------------------------------------------------------------------------------------------------------
// The data the FTB API sends, with the Java edition's field names (so what is saved reads back in both).

/// Reads a value that may be a name this program doesn't know: unknown names become `None`, like Gson's enums.
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// Reads a whole number that may arrive as a number, a number in quotes, or nothing, as Gson does.
fn num<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Value::String(s) => s.trim().parse::<i64>().or_else(|_| s.trim().parse::<f64>().map(|f| f as i64)).unwrap_or(0),
        Value::Bool(b) => b as i64,
        _ => 0,
    })
}

fn opt_num<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    })
}

/// Reads a flag that may arrive as a boolean, "true"/"false" or 0/1.
fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::String(s) => s.eq_ignore_ascii_case("true") || s == "1",
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    })
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ArtType {
    Splash,
    Square,
    Logo,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum LinkType {
    Website,
    Video,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VersionType {
    #[serde(rename = "alpha", alias = "Alpha")]
    Alpha,
    #[serde(rename = "beta", alias = "Beta")]
    Beta,
    #[serde(rename = "release", alias = "Release")]
    Release,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Config,
    Mod,
    Resource,
    Script,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TargetType {
    Modloader,
    Game,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PackArt {
    #[serde(deserialize_with = "num")]
    pub width: i64,
    #[serde(deserialize_with = "num")]
    pub height: i64,
    #[serde(deserialize_with = "flag")]
    pub compressed: bool,
    pub url: String,
    pub sha1: String,
    #[serde(deserialize_with = "num")]
    pub size: i64,
    #[serde(deserialize_with = "num")]
    pub id: i64,
    #[serde(rename = "type", deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub kind: Option<ArtType>,
    #[serde(deserialize_with = "num")]
    pub updated: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PackAuthor {
    #[serde(deserialize_with = "num")]
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub name: String,
    #[serde(rename = "type", deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(deserialize_with = "num")]
    pub updated: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PackLink {
    #[serde(deserialize_with = "num")]
    pub id: i64,
    pub name: String,
    pub link: String,
    #[serde(rename = "type", deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub kind: Option<LinkType>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PackRating {
    #[serde(deserialize_with = "num")]
    pub id: i64,
    #[serde(deserialize_with = "flag")]
    pub configured: bool,
    #[serde(deserialize_with = "flag")]
    pub verified: bool,
    #[serde(deserialize_with = "num")]
    pub age: i64,
    #[serde(deserialize_with = "flag")]
    pub gambling: bool,
    #[serde(deserialize_with = "flag")]
    pub frightening: bool,
    #[serde(deserialize_with = "flag")]
    pub alcoholdrugs: bool,
    #[serde(deserialize_with = "flag")]
    pub nuditysexual: bool,
    #[serde(deserialize_with = "flag")]
    pub sterotypeshate: bool,
    #[serde(deserialize_with = "flag")]
    pub languageviolence: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PackTag {
    #[serde(deserialize_with = "num")]
    pub id: i64,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
#[serde(default)]
pub struct VersionSpecs {
    #[serde(deserialize_with = "num")]
    pub id: i64,
    #[serde(deserialize_with = "num")]
    pub minimum: i64,
    #[serde(deserialize_with = "num")]
    pub recommended: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PackVersion {
    #[serde(deserialize_with = "num")]
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specs: Option<VersionSpecs>,
    pub name: String,
    #[serde(rename = "type", deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub kind: Option<VersionType>,
    #[serde(deserialize_with = "num")]
    pub updated: i64,
}

impl PackVersion {
    /// "1.0.1 (Beta)" for pre-releases, the plain name for releases.
    pub fn label(&self) -> String {
        match self.kind {
            Some(VersionType::Beta) => format!("{} (Beta)", self.name),
            Some(VersionType::Alpha) => format!("{} (Alpha)", self.name),
            _ => self.name.clone(),
        }
    }
}

/// `GET /modpack/{id}`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PackManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synopsis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub art: Vec<PackArt>,
    pub links: Vec<PackLink>,
    pub authors: Vec<PackAuthor>,
    pub versions: Vec<PackVersion>,
    #[serde(deserialize_with = "num")]
    pub installs: i64,
    #[serde(deserialize_with = "num")]
    pub plays: i64,
    #[serde(deserialize_with = "flag")]
    pub featured: bool,
    #[serde(deserialize_with = "num")]
    pub refreshed: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<PackRating>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(deserialize_with = "num")]
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(deserialize_with = "num")]
    pub updated: i64,
    pub tags: Vec<PackTag>,
}

impl PackManifest {
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t.name == tag)
    }

    /// The pack's page on feed-the-beast.com.
    pub fn website_url(&self) -> String {
        format!("https://feed-the-beast.com/modpacks/{}-{}", self.id, self.slug())
    }

    pub fn slug(&self) -> String {
        let lower = self.name.replace('+', " Plus").to_lowercase();
        let dashed: String =
            lower.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' }).collect();
        let mut out = String::new();
        for c in dashed.chars() {
            if c == '-' && out.ends_with('-') {
                continue;
            }
            out.push(c);
        }
        out
    }

    /// The square logo, which is what the instance takes as its picture.
    pub fn square_art(&self) -> Option<&PackArt> {
        self.art.iter().find(|a| a.kind == Some(ArtType::Square))
    }

    /// The first "website" link, if the pack has one.
    pub fn website_link(&self) -> Option<&str> {
        self.links.iter().find(|l| l.kind == Some(LinkType::Website)).map(|l| l.link.as_str())
    }

    /// Versions newest first, the order the installer offers them in.
    pub fn versions_newest_first(&self) -> Vec<&PackVersion> {
        let mut v: Vec<&PackVersion> = self.versions.iter().collect();
        v.sort_by(|a, b| b.updated.cmp(&a.updated));
        v
    }

    /// The version with the highest id, which is what update checks compare against.
    pub fn latest_version(&self) -> Option<&PackVersion> {
        self.versions.iter().max_by_key(|v| v.id)
    }

    /// The Minecraft version of a pack's newest version, when the listing shows it (it does not; see [`version_manifest`]).
    pub fn newest_release(&self) -> Option<&PackVersion> {
        self.versions_newest_first().into_iter().find(|v| v.kind == Some(VersionType::Release))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct FileCurseForge {
    #[serde(deserialize_with = "opt_num")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<i64>,
    #[serde(deserialize_with = "opt_num")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct FileHashes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    #[serde(deserialize_with = "num")]
    pub murmur: i64,
    #[serde(rename = "cfMurmur", deserialize_with = "num")]
    pub cf_murmur: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ManifestFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub mirrors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hashes: Option<FileHashes>,
    #[serde(deserialize_with = "num")]
    pub size: i64,
    pub tags: Vec<Value>,
    #[serde(deserialize_with = "flag")]
    pub clientonly: bool,
    #[serde(deserialize_with = "flag")]
    pub serveronly: bool,
    #[serde(deserialize_with = "flag")]
    pub optional: bool,
    #[serde(deserialize_with = "num")]
    pub id: i64,
    pub name: String,
    #[serde(rename = "type", deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub kind: Option<FileType>,
    #[serde(deserialize_with = "num")]
    pub updated: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curseforge: Option<FileCurseForge>,
}

impl ManifestFile {
    /// The folder the file goes in, always ending with `/` (or empty), the way `FTBPackVersionManifestFile.getPath` does.
    pub fn dir(&self) -> String {
        let path = self.path.clone().unwrap_or_default();
        let clean = path.strip_prefix("./").unwrap_or(&path);
        if clean.is_empty() || clean.ends_with('/') { clean.to_string() } else { format!("{clean}/") }
    }

    /// Where the file goes, relative to the instance (`dir + name`).
    pub fn relative_path(&self) -> String {
        format!("{}{}", self.dir(), self.name)
    }

    pub fn has_url(&self) -> bool {
        self.url.as_deref().is_some_and(|u| !u.is_empty())
    }

    pub fn is_mod(&self) -> bool {
        self.kind == Some(FileType::Mod)
    }

    pub fn curse_ids(&self) -> Option<(i64, i64)> {
        let cf = self.curseforge.as_ref()?;
        Some((cf.project?, cf.file?))
    }

    pub fn sha1_hash(&self) -> Option<&str> {
        self.sha1
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.hashes.as_ref().and_then(|h| h.sha1.as_deref()).filter(|s| !s.is_empty()))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ManifestTarget {
    pub version: String,
    #[serde(deserialize_with = "num")]
    pub id: i64,
    pub name: String,
    #[serde(rename = "type", deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub kind: Option<TargetType>,
    #[serde(deserialize_with = "num")]
    pub updated: i64,
}

/// `GET /modpack/{id}/{version}`: everything one version of a pack needs.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct VersionManifest {
    pub files: Vec<ManifestFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specs: Option<VersionSpecs>,
    pub targets: Vec<ManifestTarget>,
    #[serde(deserialize_with = "num")]
    pub installs: i64,
    #[serde(deserialize_with = "num")]
    pub plays: i64,
    #[serde(deserialize_with = "num")]
    pub refreshed: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
    #[serde(deserialize_with = "num")]
    pub parent: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification: Option<String>,
    pub links: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(deserialize_with = "num")]
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(deserialize_with = "num")]
    pub updated: i64,
}

impl VersionManifest {
    pub fn target(&self, kind: TargetType) -> Option<&ManifestTarget> {
        self.targets.iter().find(|t| t.kind == Some(kind))
    }

    pub fn minecraft_version(&self) -> Option<&str> {
        self.target(TargetType::Game).map(|t| t.version.as_str())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ManifestMod {
    #[serde(rename = "fileId", deserialize_with = "num")]
    pub file_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synopsis: Option<String>,
    #[serde(rename = "curseSlug")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curse_slug: Option<String>,
    #[serde(rename = "curseProject", deserialize_with = "opt_num")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curse_project: Option<i64>,
    #[serde(rename = "curseFile", deserialize_with = "opt_num")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curse_file: Option<i64>,
    #[serde(deserialize_with = "num")]
    pub stored: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// `GET /modpack/{id}/{version}/mods`: the mods of a version with their CurseForge ids.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ModsManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specs: Option<VersionSpecs>,
    pub targets: Vec<ManifestTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(deserialize_with = "num")]
    pub id: i64,
    pub name: String,
    pub mods: Vec<ManifestMod>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackList {
    packs: Vec<i64>,
    status: Option<String>,
}

// ---------------------------------------------------------------------------------------------------------
// Talking to the API

/// How the public pack list is ordered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Installs,
    Plays,
    Updated,
    Featured,
    All,
}

impl Sort {
    pub const ALL: [Sort; 5] = [Sort::Installs, Sort::Plays, Sort::Updated, Sort::Featured, Sort::All];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Installs => "Most installed",
            Sort::Plays => "Most played",
            Sort::Updated => "Recently updated",
            Sort::Featured => "Featured",
            Sort::All => "All packs",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Sort::Installs => "popular/installs",
            Sort::Plays => "popular/plays",
            Sort::Updated => "updated",
            Sort::Featured => "featured",
            Sort::All => "all",
        }
    }
}

fn cache(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("http")
}

/// The ids of the packs in a listing. Errors from the service come back as an empty list, as in the Java edition.
pub async fn list_ids(dirs: &Dirs, sort: Sort) -> Result<Vec<i64>> {
    let list: PackList =
        http::get_json_cached(&format!("{API}/modpack/{}/1000", sort.path()), &cache(dirs), LIST_TTL).await?;
    Ok(if list.status.as_deref() == Some("error") { Vec::new() } else { list.packs })
}

/// The ids of the packs whose name matches `query`.
pub async fn search_ids(dirs: &Dirs, query: &str) -> Result<Vec<i64>> {
    let query = query.trim();
    if query.is_empty() {
        return list_ids(dirs, Sort::All).await;
    }
    let url = reqwest::Url::parse_with_params(&format!("{API}/modpack/search/1000"), &[("term", query)])?;
    let list: PackList = http::get_json_cached(url.as_str(), &cache(dirs), LIST_TTL).await?;
    Ok(if list.status.as_deref() == Some("error") { Vec::new() } else { list.packs })
}

/// One pack's details. `None` when FTB doesn't know it (or answers with an error).
pub async fn pack(dirs: &Dirs, id: i64) -> Result<Option<PackManifest>> {
    match http::get_json_cached::<PackManifest>(&format!("{API}/modpack/{id}"), &cache(dirs), PACK_TTL).await {
        Ok(p) if p.status.as_deref() == Some("error") => Ok(None),
        Ok(p) => Ok(Some(p)),
        Err(e) if format!("{e:#}").contains("HTTP 404") => Ok(None),
        Err(e) => Err(e),
    }
}

/// The details of the packs `ids` (at most [`PAGE_SIZE`] of them are fetched), several at once.
/// Packs that can't be read, or have no versions, are left out.
pub async fn packs(dirs: &Dirs, ids: &[i64]) -> Vec<PackManifest> {
    use futures::StreamExt;
    let mut out: Vec<(usize, PackManifest)> = futures::stream::iter(ids.iter().copied().enumerate())
        .map(|(i, id)| async move { pack(dirs, id).await.ok().flatten().map(|p| (i, p)) })
        .buffer_unordered(8)
        .filter_map(|x| async move { x })
        .collect()
        .await;
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, p)| p).filter(|p| !p.versions.is_empty()).collect()
}

/// Page `page` (starting at 1) of a list of ids, as full packs.
pub async fn page(dirs: &Dirs, ids: &[i64], page: usize) -> Vec<PackManifest> {
    let start = page.saturating_sub(1) * PAGE_SIZE;
    let slice: Vec<i64> = ids.iter().copied().skip(start).take(PAGE_SIZE).collect();
    packs(dirs, &slice).await
}

/// One version of a pack, with its files and the Minecraft and loader it needs.
pub async fn version_manifest(dirs: &Dirs, pack_id: i64, version_id: i64) -> Result<VersionManifest> {
    let m: VersionManifest =
        http::get_json_cached(&format!("{API}/modpack/{pack_id}/{version_id}"), &cache(dirs), PACK_TTL)
            .await
            .with_context(|| format!("reading version {version_id} of FTB pack {pack_id}"))?;
    if m.status.as_deref() == Some("error") {
        bail!("FTB doesn't know version {version_id} of pack {pack_id}");
    }
    Ok(m)
}

/// The mods list of a version (CurseForge ids by file name). `None` when FTB has none.
pub async fn mods_manifest(dirs: &Dirs, pack_id: i64, version_id: i64) -> Option<ModsManifest> {
    let m: ModsManifest =
        http::get_json_cached(&format!("{API}/modpack/{pack_id}/{version_id}/mods"), &cache(dirs), PACK_TTL)
            .await
            .ok()?;
    if m.status.as_deref() == Some("error") { None } else { Some(m) }
}

// ---------------------------------------------------------------------------------------------------------
// Reading what an FTB instance remembers

fn extra<'a>(instance: &'a Instance, key: &str) -> Option<&'a Value> {
    instance.file.launcher.extras.get(key).filter(|v| !v.is_null())
}

/// Whether `instance` came from an FTB pack (`Instance.isFTBPack`). Instances saved by older versions used the
/// `modpacksChPackManifest` names; both are read.
pub fn is_ftb_instance(instance: &Instance) -> bool {
    pack_manifest_of(instance).is_some() && version_manifest_of(instance).is_some()
}

pub fn pack_manifest_of(instance: &Instance) -> Option<PackManifest> {
    let v = extra(instance, "ftbPackManifest").or_else(|| extra(instance, "modpacksChPackManifest"))?;
    serde_json::from_value(v.clone()).ok()
}

pub fn version_manifest_of(instance: &Instance) -> Option<VersionManifest> {
    let v = extra(instance, "ftbPackVersionManifest").or_else(|| extra(instance, "modpacksChPackVersionManifest"))?;
    serde_json::from_value(v.clone()).ok()
}

/// Is the newest FTB version newer than the one installed? `Some(version)` when it is.
pub fn newer_version<'a>(installed: &VersionManifest, pack: &'a PackManifest) -> Option<&'a PackVersion> {
    pack.latest_version().filter(|v| v.id != installed.id)
}

/// Check one FTB instance for a newer pack version (`FTBUpdateManager`).
pub async fn check_update(dirs: &Dirs, instance: &Instance) -> Result<Option<PackVersion>> {
    let (Some(installed), Some(local)) = (version_manifest_of(instance), pack_manifest_of(instance)) else {
        return Ok(None);
    };
    let Some(pack) = pack(dirs, local.id).await? else { return Ok(None) };
    Ok(newer_version(&installed, &pack).cloned())
}

/// Check every FTB instance (one request per distinct pack). Returns folder -> newest version, for the
/// instances that have an update.
pub async fn check_updates(dirs: &Dirs, instances: &[Instance]) -> HashMap<String, PackVersion> {
    let mut by_pack: HashMap<i64, Option<PackManifest>> = HashMap::new();
    let mut out = HashMap::new();
    for inst in instances.iter().filter(|i| is_ftb_instance(i)) {
        let (Some(installed), Some(local)) = (version_manifest_of(inst), pack_manifest_of(inst)) else { continue };
        if let std::collections::hash_map::Entry::Vacant(e) = by_pack.entry(local.id) {
            e.insert(pack(dirs, local.id).await.ok().flatten());
        }
        if let Some(Some(latest)) = by_pack.get(&local.id) {
            if let Some(v) = newer_version(&installed, latest) {
                out.insert(inst.folder.clone(), v.clone());
            }
        }
    }
    out
}

/// The ids in `launcher.ignoredUpdates`; an update to one of them is not offered again.
pub fn update_ignored(instance: &Instance, version_id: i64) -> bool {
    let l = &instance.file.launcher.extras;
    l.get("ignoreAllUpdates").and_then(Value::as_bool).unwrap_or(false)
        || l.get("ignoredUpdates")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(&version_id.to_string())))
}

// ---------------------------------------------------------------------------------------------------------
// Installing

/// What the player can choose when installing.
#[derive(Clone, Debug)]
pub struct InstallOptions {
    /// The instance's name; the pack's name when empty.
    pub name: Option<String>,
    /// Look the pack's mods up on CurseForge afterwards (off when `dontCheckModsOnCurseForge` is set).
    pub check_curseforge: bool,
    /// Look the pack's mods up on Modrinth afterwards (off when `dontCheckModsOnModrinth` is set).
    pub check_modrinth: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        InstallOptions { name: None, check_curseforge: true, check_modrinth: true }
    }
}

impl InstallOptions {
    /// Options following the launcher's settings (`dontCheckModsOnCurseForge`, `dontCheckModsOnModrinth`).
    pub fn from_settings(settings: &crate::config::Settings, name: Option<String>) -> InstallOptions {
        let flag = |k: &str| settings.extras.get(k).and_then(Value::as_bool).unwrap_or(false);
        InstallOptions {
            name,
            check_curseforge: !flag("dontCheckModsOnCurseForge"),
            check_modrinth: !flag("dontCheckModsOnModrinth"),
        }
    }
}

/// A mod CurseForge won't let other launchers download. The player has to fetch it in a browser.
#[derive(Clone, Debug)]
pub struct ManualDownload {
    pub mod_name: String,
    pub file_name: String,
    /// The page to open in the browser.
    pub url: String,
}

/// Returned (inside an `anyhow::Error`) when some mods must be downloaded by hand. The installer looks in the
/// player's Downloads folder for them; when they are all there the same install succeeds.
#[derive(Debug)]
pub struct ManualDownloadsNeeded(pub Vec<ManualDownload>);

impl std::fmt::Display for ManualDownloadsNeeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} mods can only be downloaded from the CurseForge website: ", self.0.len())?;
        write!(f, "{}", self.0.iter().map(|m| m.file_name.as_str()).collect::<Vec<_>>().join(", "))
    }
}

impl std::error::Error for ManualDownloadsNeeded {}

fn same_file_name(m: &ManifestMod, f: &ManifestFile) -> bool {
    let Some(name) = m.filename.as_deref() else { return false };
    name.eq_ignore_ascii_case(&f.name)
        || name.eq_ignore_ascii_case(&f.name.replace('_', ""))
        || name.replace('_', " ").eq_ignore_ascii_case(&f.name)
}

/// The CurseForge project and file for a mod that FTB doesn't host itself: its own `curseforge` ids, or the
/// ones the mods list has under the same file name.
fn curse_ids_for(f: &ManifestFile, mods: Option<&ModsManifest>) -> Option<(i64, i64)> {
    if let Some(ids) = f.curse_ids() {
        return Some(ids);
    }
    let m = mods?.mods.iter().find(|m| same_file_name(m, f))?;
    Some((m.curse_project?, m.curse_file?))
}

/// Install version `version_id` of FTB pack `pack_id` as a new instance.
pub async fn install(
    dirs: &Dirs,
    pack_id: i64,
    version_id: i64,
    opts: &InstallOptions,
    cf: &dyn CurseForgeSource,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    progress.stage("Reading the pack");
    let pack = pack(dirs, pack_id).await?.with_context(|| format!("FTB has no pack {pack_id}"))?;
    let vm = version_manifest(dirs, pack_id, version_id).await?;
    let version = pack.versions.iter().find(|v| v.id == version_id).cloned();
    run_install(dirs, &pack, &vm, version.as_ref(), None, opts, cf, progress).await
}

/// Move an FTB instance to another version of its pack (usually the newest). The pack's own files are replaced;
/// worlds, settings and mods the player added are kept.
pub async fn update(
    dirs: &Dirs,
    instance: &Instance,
    version_id: i64,
    opts: &InstallOptions,
    cf: &dyn CurseForgeSource,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    let _ = version_manifest_of(instance).context("this instance wasn't installed from an FTB pack")?;
    let local = pack_manifest_of(instance).context("this instance wasn't installed from an FTB pack")?;
    progress.stage("Reading the pack");
    let pack = pack(dirs, local.id).await?.unwrap_or(local);
    let vm = version_manifest(dirs, pack.id, version_id).await?;
    let version = pack.versions.iter().find(|v| v.id == version_id).cloned();
    run_install(dirs, &pack, &vm, version.as_ref(), Some(instance.clone()), opts, cf, progress).await
}

/// Take away what the previous version of the pack put on disk, once the new version is known: its pack
/// files (configs, scripts, resources), and the mods the new version doesn't have again. Mods that are in both
/// versions stay (the download step checks them), and so do worlds, settings and mods the player added.
fn remove_previous_version(instance: &mut Instance, old: &VersionManifest, plans: &[ModPlan]) {
    let root = instance.root.clone();
    let mc = instance.mc_version().to_string();
    let keep_paths: HashSet<PathBuf> = plans
        .iter()
        .filter_map(|p| packutil::safe_relative(&format!("{}{}", p.dir, p.file)))
        .map(|r| root.join(r))
        .collect();
    let records = instance.mod_records();
    for rec in records.iter().filter(|r| !r.user_added) {
        if let Some(path) = modcheck::current_file(&root, &mc, rec) {
            if !keep_paths.contains(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let mut dirs = HashSet::new();
    for f in old.files.iter().filter(|f| !f.is_mod()) {
        if let Some(rel) = packutil::safe_relative(&f.relative_path()) {
            let path = root.join(&rel);
            if path.is_file() {
                let _ = std::fs::remove_file(&path);
            }
            if let Some(parent) = path.parent() {
                dirs.insert(parent.to_path_buf());
            }
        }
    }
    let mut sorted: Vec<PathBuf> = dirs.into_iter().collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in sorted {
        packutil::remove_empty_parents(&root, &d);
    }
    let kept: Vec<ModRecord> = records.into_iter().filter(|r| r.user_added).collect();
    instance.set_mod_records(&kept);
}

/// Where a loader name from FTB's targets maps to.
fn loader_of(name: &str) -> Result<Loader> {
    match name.to_ascii_lowercase().as_str() {
        "forge" => Ok(Loader::Forge),
        "neoforge" => Ok(Loader::NeoForge),
        "fabric" => Ok(Loader::Fabric),
        "quilt" => Ok(Loader::Quilt),
        other => bail!("this pack uses a mod loader BlockForge doesn't know ({other})"),
    }
}

/// FTB lists Forge as `47.2.0`; the Forge installer wants `1.20.1-47.2.0`.
async fn loader_version_for(dirs: &Dirs, loader: Loader, mc: &str, wanted: &str) -> String {
    let known = loaders::versions(dirs, loader, mc).await.unwrap_or_default();
    packutil::match_loader_version(loader, mc, wanted, &known).unwrap_or_else(|| match loader {
        Loader::Forge if !wanted.starts_with(&format!("{mc}-")) => format!("{mc}-{wanted}"),
        _ => wanted.to_string(),
    })
}

/// `config/memorysettings.json` in a pack can say how much memory it needs; it wins over the version's number.
fn required_memory(root: &std::path::Path, recommended: i64) -> i64 {
    std::fs::read(root.join("config").join("memorysettings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["minimumClient"]["minimumClient"].as_i64())
        .unwrap_or(recommended)
}

struct ModPlan {
    /// File name inside its folder.
    file: String,
    dir: String,
    url: String,
    sha1: Option<String>,
    size: Option<i64>,
    record: ModRecord,
}

#[allow(clippy::too_many_arguments)]
async fn run_install(
    dirs: &Dirs,
    pack: &PackManifest,
    vm: &VersionManifest,
    version: Option<&PackVersion>,
    existing: Option<Instance>,
    opts: &InstallOptions,
    cf: &dyn CurseForgeSource,
    progress: &Arc<Progress>,
) -> Result<Instance> {
    let mc = vm.target(TargetType::Game).map(|t| t.version.clone()).context("Minecraft target couldn't be found.")?;
    let loader_target =
        vm.target(TargetType::Modloader).context("This pack has no mod loader listed, so it can't be installed.")?;
    let loader = loader_of(&loader_target.name)?;
    let loader_version = loader_version_for(dirs, loader, &mc, &loader_target.version).await;

    // everything that can fail before touching the disk is checked first: which mods can be downloaded at all
    progress.stage("Working out where each mod comes from");
    let mods = mods_manifest(dirs, pack.id, vm.id).await;
    let plans = plan_mods(dirs, vm, mods.as_ref(), cf, progress).await?;

    let name = match (&existing, &opts.name) {
        (Some(e), _) => e.name().to_string(),
        (None, Some(n)) if !n.trim().is_empty() => n.trim().to_string(),
        _ => pack.name.clone(),
    };
    let is_new = existing.is_none();
    let mut instance = match existing {
        Some(mut inst) => {
            progress.stage("Removing the old version's files");
            if let Some(old) = version_manifest_of(&inst) {
                remove_previous_version(&mut inst, &old, &plans);
            }
            let (merged, info) = packutil::prepare_version(dirs, &mc, loader, Some(&loader_version), progress).await?;
            inst.file.version = merged;
            inst.file.launcher.loader_version = info;
            inst
        }
        None => {
            let name = packutil::unique_name(&name, |n| packutil::instance_name_taken(dirs, n));
            instances::create(dirs, &name, &mc, loader, Some(&loader_version), progress).await?
        }
    };

    match install_files(&mut instance, pack, vm, version, plans, opts, cf, progress).await {
        Ok(()) => Ok(instance),
        Err(e) => {
            if is_new {
                let _ = std::fs::remove_dir_all(&instance.root);
            }
            Err(e)
        }
    }
}

/// Decide, for every mod of the version, where its file comes from: FTB's own address, a CurseForge address,
/// the same file on Modrinth, or the player's browser.
async fn plan_mods(
    dirs: &Dirs,
    vm: &VersionManifest,
    mods: Option<&ModsManifest>,
    cf: &dyn CurseForgeSource,
    progress: &Arc<Progress>,
) -> Result<Vec<ModPlan>> {
    let mut plans = Vec::new();
    let mut hosted_elsewhere: Vec<(&ManifestFile, i64, i64)> = Vec::new();
    let mut unavailable = 0usize;
    for f in vm.files.iter().filter(|f| f.is_mod()) {
        if f.has_url() {
            let mut record = ModRecord::new(&f.name, &f.name, "mods");
            record.version = f.version.clone().unwrap_or_default();
            record.optional = f.optional;
            record.path = Some(f.dir());
            record.user_added = false;
            plans.push(ModPlan {
                file: f.name.clone(),
                dir: f.dir(),
                url: f.url.clone().unwrap_or_default(),
                sha1: f.sha1_hash().map(str::to_string),
                size: Some(f.size).filter(|s| *s > 0),
                record,
            });
        } else if let Some((p, file)) = curse_ids_for(f, mods) {
            hosted_elsewhere.push((f, p, file));
        } else {
            unavailable += 1;
        }
    }
    if unavailable > 0 {
        bail!(
            "{unavailable} mods for this pack are not available to download. At this time you can only install this pack through the FTB launcher."
        );
    }
    if hosted_elsewhere.is_empty() {
        return Ok(plans);
    }

    progress.stage(format!("Asking CurseForge about {} mods", hosted_elsewhere.len()));
    let found = cf.files(hosted_elsewhere.iter().map(|(_, p, f)| (*p, *f)).collect()).await?;
    let mut manual = Vec::new();
    let mut need_modrinth: Vec<(ModPlan, CfFile, CfProject)> = Vec::new();
    for (f, project_id, file_id) in hosted_elsewhere {
        let (project, file) = match found.get(&file_id) {
            Some(x) => x.clone(),
            None => cf.file(project_id, file_id).await.with_context(|| format!("finding {} on CurseForge", f.name))?,
        };
        // the folder comes from FTB's manifest, like for the files FTB hosts itself: without it the jar landed in
        // the instance folder, where the game never loads it
        let dir = match f.dir() {
            d if d.is_empty() => "mods/".to_string(),
            d => d,
        };
        let mut record = ModRecord::new(&project.name, &file.file_name, "mods");
        record.version = file.display_name.clone();
        record.description = project.summary.clone();
        record.optional = f.optional;
        record.path = Some(dir.clone());
        record.user_added = false;
        record.curse_forge_project_id = Some(project.id);
        record.curse_forge_file_id = Some(file.id);
        record.curse_forge_project = Some(project.raw.clone());
        record.curse_forge_file = Some(file.for_instance_json());
        let plan = ModPlan {
            file: file.file_name.clone(),
            dir,
            url: file.download_url.clone().unwrap_or_default(),
            sha1: file.sha1.clone(),
            size: Some(file.file_length).filter(|s| *s > 0),
            record,
        };
        if plan.url.is_empty() {
            need_modrinth.push((plan, file, project));
        } else {
            plans.push(plan);
        }
    }

    // files CurseForge won't hand out: the same file is often on Modrinth, found by its SHA-1
    if !need_modrinth.is_empty() {
        progress.stage(format!("{} mods can't be downloaded from CurseForge, trying Modrinth", need_modrinth.len()));
        let hashes: Vec<String> = need_modrinth.iter().filter_map(|(_, f, _)| f.sha1.clone()).collect();
        let versions = modcheck::modrinth_versions_by_sha1(&hashes).await.unwrap_or_default();
        let ids: Vec<String> = versions.values().filter_map(|v| v["project_id"].as_str().map(str::to_string)).collect();
        let projects = modcheck::modrinth_projects(&ids).await.unwrap_or_default();
        let downloads = packutil::downloads_dir();
        for (mut plan, file, project) in need_modrinth {
            let version = file.sha1.as_deref().and_then(|h| versions.get(h));
            let url = version
                .and_then(|v| modcheck::modrinth_file_with_sha1(v, file.sha1.as_deref().unwrap_or("")))
                .and_then(|f| f["url"].as_str());
            if let (Some(url), Some(version)) = (url, version) {
                plan.url = url.to_string();
                plan.record.modrinth_version = Some(version.clone());
                plan.record.modrinth_project = version["project_id"].as_str().and_then(|id| projects.get(id)).cloned();
                plans.push(plan);
                continue;
            }
            // last resort: the player downloads it in a browser; it may already be in the Downloads folder
            let wanted = [file.file_name.clone(), file.file_name.replace(' ', "+")];
            if let Some(found) = wanted.iter().map(|n| downloads.join(n)).find(|p| p.is_file()) {
                let dest = dirs.temp.join("ftb-manual").join(&file.file_name);
                if let Some(dir) = dest.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::copy(&found, &dest)?;
                plan.url = format!("file://{}", dest.display());
                plan.record.skipped = false;
                plans.push(plan);
            } else {
                manual.push(ManualDownload {
                    mod_name: project.name.clone(),
                    file_name: file.file_name.clone(),
                    url: project.browser_download_url(file.id),
                });
            }
        }
    }
    if !manual.is_empty() {
        return Err(ManualDownloadsNeeded(manual).into());
    }
    Ok(plans)
}

/// Download everything and write the instance's records.
async fn install_files(
    instance: &mut Instance,
    pack: &PackManifest,
    vm: &VersionManifest,
    version: Option<&PackVersion>,
    plans: Vec<ModPlan>,
    opts: &InstallOptions,
    cf: &dyn CurseForgeSource,
    progress: &Arc<Progress>,
) -> Result<()> {
    let root = instance.root.clone();
    std::fs::create_dir_all(root.join("mods"))?;

    // the pack's mods
    let mut jobs = Vec::new();
    let mut local_copies = Vec::new();
    let mut records = instance.mod_records();
    for plan in &plans {
        let rel = packutil::safe_relative(&format!("{}{}", plan.dir, plan.file))
            .with_context(|| format!("the pack has a file path that points outside the instance: {}", plan.file))?;
        let dest = root.join(&rel);
        if let Some(local) = plan.url.strip_prefix("file://") {
            local_copies.push((PathBuf::from(local), dest));
        } else {
            let mut job = Job::new(&plan.url, dest);
            job.sha1 = plan.sha1.clone();
            job.size = plan.size.map(|s| s as u64);
            jobs.push(job);
        }
        let mut record = plan.record.clone();
        record.file = plan.file.clone();
        records.retain(|r| r.file != record.file || r.path != record.path);
        records.push(record);
    }
    progress.stage(format!("Downloading {} mods", jobs.len()));
    let mut warnings = download_tolerant(jobs, progress).await?;
    for (from, to) in local_copies {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(&from, &to)?;
    }

    // configs, scripts, resources
    let mut override_paths = Vec::new();
    let mut jobs = Vec::new();
    for f in vm.files.iter().filter(|f| !f.is_mod()) {
        let rel_text = f.relative_path();
        let Some(rel) = packutil::safe_relative(&rel_text) else {
            warnings.push(format!("{rel_text} was skipped because its path points outside the instance"));
            continue;
        };
        override_paths.push(rel_text);
        if !f.has_url() {
            continue;
        }
        // FTB's hashes are wrong quite often, so files are checked by size only
        let mut job = Job::new(f.url.clone().unwrap_or_default(), root.join(rel));
        job.size = Some(f.size).filter(|s| *s > 0).map(|s| s as u64);
        jobs.push(job);
    }
    progress.stage(format!("Downloading {} pack files", jobs.len()));
    warnings.extend(download_tolerant(jobs, progress).await?);

    // the picture
    progress.stage("Downloading the picture");
    if let Some(art) = pack.square_art().filter(|a| !a.url.is_empty()) {
        let _ = install::download_all(vec![Job::new(&art.url, root.join("instance.png"))], progress, false).await;
    }

    // look the mods up on the other platforms, like the Java installer does
    if (opts.check_modrinth || opts.check_curseforge) && !records.is_empty() {
        let mc = instance.mc_version().to_string();
        if opts.check_curseforge {
            progress.stage("Checking the mods on CurseForge");
            let _ = modcheck::attach_curseforge(&root, &mc, &mut records, cf, true).await;
        }
        if opts.check_modrinth {
            progress.stage("Checking the mods on Modrinth");
            let _ = modcheck::attach_modrinth(&root, &mc, &mut records, true).await;
        }
    }

    // `instance.json`, the way the Java installer writes it for FTB packs
    let l = &mut instance.file.launcher;
    l.pack = pack.name.clone();
    l.description = pack.description.clone().unwrap_or_default();
    l.pack_id = 0;
    l.external_pack_id = pack.id;
    l.version = version.map(|v| v.name.clone()).unwrap_or_else(|| vm.name.clone());
    l.enable_curse_forge_integration = true;
    l.enable_editing_mods = true;
    l.vanilla_instance = false;
    l.required_memory = required_memory(&root, vm.specs.map(|s| s.recommended).unwrap_or(0));
    l.extras.insert("isPlayable".into(), json!(true));
    l.extras.insert("isDev".into(), json!(false));
    l.extras.insert("overridePaths".into(), json!(override_paths));
    l.extras.insert("ftbPackManifest".into(), serde_json::to_value(pack)?);
    l.extras.insert("ftbPackVersionManifest".into(), serde_json::to_value(vm)?);
    instance.set_mod_records(&records);
    instance.save().context("saving instance.json")?;
    for w in &warnings {
        eprintln!("[ftb] {w}");
    }
    Ok(())
}

/// Download `jobs`; a file whose checksum doesn't match is tried again without the check (FTB's are often
/// wrong), and files that still fail are reported as warnings instead of failing the whole install.
async fn download_tolerant(jobs: Vec<Job>, progress: &Arc<Progress>) -> Result<Vec<String>> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    if progress.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        bail!("cancelled");
    }
    match install::download_all(jobs.clone(), progress, true).await {
        Ok(_) => Ok(Vec::new()),
        Err(first) => {
            let unchecked: Vec<Job> = jobs
                .into_iter()
                .map(|mut j| {
                    j.sha1 = None;
                    j
                })
                .collect();
            match install::download_all(unchecked, progress, false).await {
                Ok(_) => Ok(vec![format!("some files had checksums that didn't match, kept anyway ({first:#})")]),
                Err(e) => Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed copies of what api.feed-the-beast.com returned on 2026-10-16.
    const PACK: &str = r#"{"status":"success","id":100,"name":"FTB StoneBlock 3","slug":"ftb-stoneblock-3",
        "description":"d","synopsis":"In a world surrounded by stone","type":"release","released":1667330107,"updated":1667243244,
        "private":false,"featured":false,"plays":14396036,"installs":2510927,"plays_14d":7424876,
        "tags":[{"id":9,"name":"FTB"},{"id":12,"name":"Magic"}],
        "links":[{"id":-1,"name":"Wiki","link":"https://docs.feed-the-beast.com","type":"misc"},{"id":-1,"name":"Site","link":"https://x","type":"website"}],
        "art":[{"url":"https://cdn/x.png","type":"square","updated":0,"width":512,"height":512,"compressed":false,"mirrors":[],"id":-1,"sha1":"08e2","title":"Logo","order":0},
               {"url":"https://cdn/y.png","type":"splash","width":1920,"height":1080,"id":-1,"sha1":"ac05"}],
        "authors":[{"id":-1,"website":"https://www.feed-the-beast.com/","name":"FTB Team","type":"team","updated":0}],
        "versions":[{"specs":{"minimum":4096,"recommended":6144,"id":-1},"targets":[],"id":2275,"name":"1.0.0","type":"beta","updated":1667330107,"released":1667330107,"private":false},
                    {"specs":{"minimum":4096,"recommended":6144,"id":-1},"id":2276,"name":"1.0.1","type":"Release","updated":1667420169}],
        "provider":"modpacks.ch","notification":"","rating":{"id":-1,"configured":false,"verified":false,"age":7,"gambling":false,"frightening":true,"alcoholdrugs":false,"nuditysexual":false,"sterotypeshate":false,"language":false,"violence":true},
        "meta":{}}"#;

    const VERSION: &str = r#"{"status":"success","updated":1667420169,"refreshed":0,"type":"release","plays":129062,"installs":95830,
        "changelog":"https://api.feed-the-beast.com/v1/modpacks/modpack/100/2276/changelog","parent":100,"notification":"","links":[],"id":2276,"name":"1.0.1","private":false,
        "targets":[{"id":-1,"name":"minecraft","version":"1.18.2","type":"game","updated":0},{"id":-1,"name":"forge","version":"40.1.84","type":"modloader","updated":0},{"id":-1,"name":"java","version":"17.0.2+8","type":"runtime","updated":0}],
        "specs":{"id":-1,"minimum":4096,"recommended":6144},
        "files":[
         {"id":683606289,"version":"0","path":"./config/Advancedperipherals/","url":"https://files.feed-the-beast.com/blob/7e/7e44.toml","mirrors":[],"tags":[],"sha1":"8439d14fe27656b5e6963bfacb04e77c178e267d",
          "hashes":{"sha1":"8439d14fe27656b5e6963bfacb04e77c178e267d","sha256":"18cc","sha512":"9355","murmur":869666281,"cfMurmur":3885360454},"size":3624,"clientonly":false,"serveronly":false,"optional":false,"name":"peripherals.toml","type":"config","updated":1652272633},
         {"id":2461656446,"version":"11.5.0","path":"./mods/","url":"","mirrors":[],"tags":[],"sha1":"","size":211920,"clientonly":false,"serveronly":false,"optional":false,"name":"AE2WTLib-11.5.0.jar","type":"mod","updated":1,"curseforge":{"project":459929,"file":4013966}},
         {"id":5,"path":"./mods","url":"https://files/x.jar","hashes":{"sha1":"abcd","murmur":1,"cfMurmur":2},"size":10,"name":"x.jar","type":"mod"},
         {"id":6,"path":"./kubejs/","url":"https://files/y.js","size":1,"name":"y.js","type":"somethingnew"}]}"#;

    #[test]
    fn pack_details_read_and_write_back_in_javas_shape() {
        let p: PackManifest = serde_json::from_str(PACK).unwrap();
        assert_eq!(p.id, 100);
        assert_eq!(p.versions.len(), 2);
        assert_eq!(p.versions[1].kind, Some(VersionType::Release), "capitalised version types are read too");
        assert_eq!(p.versions[0].label(), "1.0.0 (Beta)");
        assert_eq!(p.latest_version().unwrap().id, 2276);
        assert_eq!(p.versions_newest_first()[0].id, 2276);
        assert_eq!(p.square_art().unwrap().url, "https://cdn/x.png");
        assert_eq!(p.website_link(), Some("https://x"), "unknown link types are ignored");
        assert!(p.has_tag("FTB") && !p.has_tag("Tech"));
        assert_eq!(p.website_url(), "https://feed-the-beast.com/modpacks/100-ftb-stoneblock-3");
        let back = serde_json::to_value(&p).unwrap();
        assert_eq!(back["versions"][0]["type"], "beta");
        assert_eq!(back["art"][0]["type"], "square");
        assert_eq!(back["links"][0].get("type"), None, "a link type Java doesn't know is not written");
        assert_eq!(back["rating"]["age"], 7);
        assert!(back.get("plays_14d").is_none());
        let again: PackManifest = serde_json::from_value(back).unwrap();
        assert_eq!(again.id, 100);
    }

    #[test]
    fn numbers_in_quotes_are_read_like_gson_does() {
        // seen in the live answer for FTB StoneBlock 3 version 100359: an `id` sent as a string
        let f: ManifestFile = serde_json::from_str(r#"{"id":"609977","size":"12","name":"a.jar","type":"mod","clientonly":"true","curseforge":{"project":"5","file":6}}"#).unwrap();
        assert_eq!((f.id, f.size, f.clientonly), (609977, 12, true));
        assert_eq!(f.curse_ids(), Some((5, 6)));
        let m: ManifestMod = serde_json::from_str(r#"{"fileId":"7","curseProject":"8","curseFile":null}"#).unwrap();
        assert_eq!((m.file_id, m.curse_project, m.curse_file), (7, Some(8), None));
    }

    #[test]
    fn slugs_match_javas() {
        let mut p = PackManifest { id: 1, name: "FTB Presents: Direwolf20 1.20".into(), ..Default::default() };
        assert_eq!(p.slug(), "ftb-presents-direwolf20-1-20");
        p.name = "Sky+Block".into();
        assert_eq!(p.slug(), "sky-plusblock", "like Java, the + becomes \" Plus\" with no space after it");
        p.name = "Sky+ Block".into();
        assert_eq!(p.slug(), "sky-plus-block");
    }

    #[test]
    fn version_files_resolve_paths_and_origins() {
        let v: VersionManifest = serde_json::from_str(VERSION).unwrap();
        assert_eq!(v.minecraft_version(), Some("1.18.2"));
        let t = v.target(TargetType::Modloader).unwrap();
        assert_eq!((t.name.as_str(), t.version.as_str()), ("forge", "40.1.84"));
        assert_eq!(v.files[0].dir(), "config/Advancedperipherals/");
        assert_eq!(v.files[0].relative_path(), "config/Advancedperipherals/peripherals.toml");
        assert_eq!(v.files[2].dir(), "mods/", "a folder without a trailing slash gets one");
        assert!(v.files[0].has_url() && !v.files[1].has_url());
        assert_eq!(v.files[1].curse_ids(), Some((459929, 4013966)));
        assert_eq!(v.files[2].sha1_hash(), Some("abcd"), "the sha1 inside hashes is used when the plain one is empty");
        assert_eq!(v.files[3].kind, None, "unknown file types are not mods");
        assert!(v.files[1].is_mod() && !v.files[3].is_mod());
        assert_eq!(v.files[0].hashes.as_ref().unwrap().cf_murmur, 3885360454);
    }

    #[test]
    fn mods_without_an_address_find_their_curseforge_ids() {
        let v: VersionManifest = serde_json::from_str(VERSION).unwrap();
        let mods: ModsManifest = serde_json::from_str(
            r#"{"status":"success","id":2276,"name":"1.0.1","mods":[
                {"fileId":1,"name":"A","curseProject":10,"curseFile":20,"filename":"somemod-1.0.jar"},
                {"fileId":3,"name":"C","curseProject":11,"curseFile":21,"filename":"another_mod.jar"},
                {"fileId":2,"name":"B","curseProject":null,"curseFile":null,"filename":"nope.jar"}]}"#,
        )
        .unwrap();
        let named = |n: &str| ManifestFile { name: n.into(), kind: Some(FileType::Mod), ..Default::default() };
        // FTB's own ids win
        assert_eq!(curse_ids_for(&v.files[1], Some(&mods)), Some((459929, 4013966)));
        // otherwise the mods list is searched by file name, ignoring case; a name that differs only by
        // underscores (taken out, or turned into spaces) matches too
        assert_eq!(curse_ids_for(&named("SomeMod-1.0.jar"), Some(&mods)), Some((10, 20)));
        assert_eq!(curse_ids_for(&named("some_mod-1.0.jar"), Some(&mods)), Some((10, 20)));
        assert_eq!(curse_ids_for(&named("another mod.jar"), Some(&mods)), Some((11, 21)));
        assert_eq!(curse_ids_for(&named("nope.jar"), Some(&mods)), None);
        assert_eq!(curse_ids_for(&named("other.jar"), None), None);
    }

    #[test]
    fn update_checks_compare_the_highest_version_id() {
        let p: PackManifest = serde_json::from_str(PACK).unwrap();
        let old = VersionManifest { id: 2275, ..Default::default() };
        let current = VersionManifest { id: 2276, ..Default::default() };
        assert_eq!(newer_version(&old, &p).map(|v| v.id), Some(2276));
        assert!(newer_version(&current, &p).is_none());
    }

    #[test]
    fn loaders_map_from_ftb_names() {
        assert_eq!(loader_of("Forge").unwrap(), Loader::Forge);
        assert_eq!(loader_of("neoforge").unwrap(), Loader::NeoForge);
        assert!(loader_of("liteloader").is_err());
    }

    #[test]
    fn memory_follows_the_packs_own_file() {
        let dir = std::env::temp_dir().join(format!("bf-ftb-mem-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        assert_eq!(required_memory(&dir, 6144), 6144);
        std::fs::write(dir.join("config/memorysettings.json"), r#"{"minimumClient":{"minimumClient":8192}}"#).unwrap();
        assert_eq!(required_memory(&dir, 6144), 8192);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignored_updates_are_remembered_per_version() {
        let mut inst =
            Instance { folder: "x".into(), mod_count: 0, root: PathBuf::from("/x"), file: Default::default() };
        assert!(!update_ignored(&inst, 5));
        inst.file.launcher.extras.insert("ignoredUpdates".into(), json!(["5"]));
        assert!(update_ignored(&inst, 5) && !update_ignored(&inst, 6));
        inst.file.launcher.extras.insert("ignoreAllUpdates".into(), json!(true));
        assert!(update_ignored(&inst, 6));
    }

    // ---- live checks against the real services: `cargo test --lib -- --ignored --nocapture`

    fn live_dirs() -> Dirs {
        let d = std::env::temp_dir().join(format!("bf-ftb-live-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        Dirs::at(d)
    }

    #[tokio::test]
    #[ignore = "needs the network"]
    async fn live_listing_details_and_a_file_whose_curseforge_fingerprint_matches() {
        let dirs = live_dirs();
        let ids = list_ids(&dirs, Sort::Installs).await.unwrap();
        assert!(ids.len() > 50, "FTB lists many packs, got {}", ids.len());
        let first = page(&dirs, &ids, 1).await;
        assert_eq!(first.len(), PAGE_SIZE);
        assert!(first.iter().all(|p| !p.versions.is_empty() && !p.name.is_empty()));
        let found = search_ids(&dirs, "skyfactory").await.unwrap();
        assert!(!found.is_empty());
        assert!(pack(&dirs, 9_999_999).await.unwrap().is_none(), "an unknown pack is None, not an error");

        // StoneBlock 3, version 1.0.1
        let p = pack(&dirs, 100).await.unwrap().unwrap();
        assert_eq!(p.name, "FTB StoneBlock 3");
        let v = version_manifest(&dirs, 100, 2276).await.unwrap();
        assert_eq!(v.minecraft_version(), Some("1.18.2"));
        assert_eq!(v.target(TargetType::Modloader).unwrap().name, "forge");
        assert!(v.files.len() > 1000);

        // FTB gives CurseForge's fingerprint of each file; ours must agree
        let f = v.files.iter().find(|f| f.name == "peripherals.toml").unwrap();
        let bytes = http::get_bytes(f.url.as_deref().unwrap()).await.unwrap();
        assert_eq!(http::sha1_hex(&bytes), f.sha1_hash().unwrap());
        assert_eq!(crate::cf_seam::fingerprint(&bytes) as i64, f.hashes.as_ref().unwrap().cf_murmur);

        let mods = mods_manifest(&dirs, 100, 2276).await.unwrap();
        assert!(mods.mods.iter().any(|m| m.curse_project.is_some()));
    }

    /// CurseForge as far as one test needs it: every file can be downloaded.
    struct FakeCurseForge;

    impl CurseForgeSource for FakeCurseForge {
        fn file(
            &self,
            project_id: i64,
            file_id: i64,
        ) -> futures::future::BoxFuture<'_, Result<(crate::cf_seam::CfProject, crate::cf_seam::CfFile)>> {
            Box::pin(async move {
                let project = crate::cf_seam::CfProject::from_json(
                    &serde_json::json!({"id": project_id, "name": "AE2 Wireless Terminals", "summary": "s"}),
                );
                let file = crate::cf_seam::CfFile::from_json(&serde_json::json!({
                    "id": file_id, "modId": project_id, "displayName": "11.5.0", "fileName": "AE2WTLib-11.5.0.jar",
                    "downloadUrl": "https://edge.forgecdn.net/files/AE2WTLib-11.5.0.jar", "fileLength": 211920,
                    "isAvailable": true
                }));
                Ok((project, file))
            })
        }

        fn fingerprints(
            &self,
            _fingerprints: Vec<u32>,
        ) -> futures::future::BoxFuture<'_, Result<Vec<crate::cf_seam::CfMatch>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[tokio::test]
    async fn mods_hosted_on_curseforge_go_into_the_mods_folder() {
        let vm: VersionManifest = serde_json::from_str(VERSION).unwrap();
        let dirs = Dirs::at(std::env::temp_dir().join(format!("bf-ftb-plan-{}", std::process::id())));
        let plans = plan_mods(&dirs, &vm, None, &FakeCurseForge, &Progress::new()).await.unwrap();
        let ae2 = plans.iter().find(|p| p.file == "AE2WTLib-11.5.0.jar").expect("the CurseForge mod is planned");
        assert_eq!(ae2.dir, "mods/", "not the instance folder, where the game never loads it");
        assert_eq!(ae2.record.path.as_deref(), Some("mods/"));
        let x = plans.iter().find(|p| p.file == "x.jar").unwrap();
        assert_eq!(x.dir, ae2.dir, "both kinds of mod end up together");
    }

    #[tokio::test]
    #[ignore = "needs the network"]
    async fn live_curseforge_seam_finds_a_file_and_its_fingerprint() {
        let cf = crate::cf_seam::MinimalCurseForge;
        let (project, file) = cf.file(459929, 4013966).await.unwrap();
        assert_eq!(project.name, "Applied Energistics 2 Wireless Terminals");
        assert_eq!(file.file_name, "AE2WTLib-11.5.0.jar");
        let url = file.download_url.clone().unwrap();
        let bytes = http::get_bytes(&url).await.unwrap();
        assert_eq!(
            crate::cf_seam::fingerprint(&bytes),
            file.fingerprint,
            "the whole jar hashes to CurseForge's own fingerprint"
        );
        let matches = cf.fingerprints(vec![file.fingerprint]).await.unwrap();
        assert!(matches.iter().any(|m| m.file.id == 4013966 && m.project.id == 459929));
        let many = cf.files(vec![(459929, 4013966)]).await.unwrap();
        assert!(many.contains_key(&4013966));
    }
}
