//! The CurseForge web API (`api.curseforge.com/v1`), with the `x-api-key` header the Java edition sends.
//! Answers are kept in memory for a few minutes (like the Java edition's HTTP cache) so paging back and forth
//! in Discover does not ask again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::model::*;
use super::{
    API_KEY_HEADER, API_URL, CLASS_DATA_PACKS, CLASS_MODPACKS, CLASS_MODS, CLASS_PLUGINS, CLASS_RESOURCE_PACKS,
    CLASS_SHADER_PACKS, CLASS_WORLDS, GAME_ID, PAGE_SIZE, api_key,
};
use crate::http::client;

const SHORT: Duration = Duration::from_secs(10 * 60);
const LONG: Duration = Duration::from_secs(60 * 60);

type Cache = Mutex<HashMap<String, (Instant, std::sync::Arc<Vec<u8>>)>>;

fn cache() -> &'static Cache {
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Forget everything remembered, so the next call asks CurseForge again (after "Refresh").
pub fn clear_cache() {
    if let Ok(mut c) = cache().lock() {
        c.clear();
    }
}

async fn call(
    post: Option<&Value>,
    path: &str,
    query: &[(&str, String)],
    ttl: Option<Duration>,
) -> Result<std::sync::Arc<Vec<u8>>> {
    let url = format!("{API_URL}{path}");
    let key = if post.is_none() {
        let q: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        Some(format!("{url}?{}", q.join("&")))
    } else {
        None
    };
    if let (Some(key), Some(ttl)) = (&key, ttl) {
        if let Ok(c) = cache().lock() {
            if let Some((at, bytes)) = c.get(key) {
                if at.elapsed() < ttl {
                    return Ok(bytes.clone());
                }
            }
        }
    }

    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..4u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt - 1))).await;
        }
        let mut req = match post {
            Some(body) => client().post(&url).json(body),
            None => client().get(&url),
        };
        if !query.is_empty() {
            req = req.query(query);
        }
        req = req.header(API_KEY_HEADER, api_key()).header("Accept", "application/json");
        match req.send().await {
            Ok(r) if r.status().is_success() => match r.bytes().await {
                Ok(b) => {
                    let bytes = std::sync::Arc::new(b.to_vec());
                    if let (Some(key), Some(_)) = (key, ttl) {
                        if let Ok(mut c) = cache().lock() {
                            if c.len() > 400 {
                                c.clear();
                            }
                            c.insert(key, (Instant::now(), bytes.clone()));
                        }
                    }
                    return Ok(bytes);
                }
                Err(e) => last = Some(e.into()),
            },
            Ok(r) => {
                let status = r.status();
                match status.as_u16() {
                    401 | 403 => bail!(
                        "CurseForge refused the request (HTTP {status}). The API key may be wrong or blocked; set BLOCKFORGE_CF_KEY or curseForgeApiKey in the settings to use your own."
                    ),
                    404 => bail!("CurseForge does not have that (HTTP 404)"),
                    429 => {
                        last = Some(anyhow!(
                            "CurseForge says too many requests were made (HTTP 429); try again in a moment"
                        ))
                    }
                    s if s >= 500 => {
                        last =
                            Some(anyhow!("CurseForge is having problems (HTTP {status}); try again in a few minutes"))
                    }
                    _ => bail!("CurseForge answered HTTP {status} for {path}"),
                }
            }
            Err(e) => last = Some(anyhow::Error::from(e).context("could not reach CurseForge")),
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("CurseForge did not answer")))
}

async fn get<T: DeserializeOwned>(path: &str, query: &[(&str, String)], ttl: Duration) -> Result<Response<T>> {
    let bytes = call(None, path, query, Some(ttl)).await?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading CurseForge's answer for {path}"))
}

async fn post<T: DeserializeOwned>(path: &str, body: Value) -> Result<Response<T>> {
    let bytes = call(Some(&body), path, &[], None).await?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading CurseForge's answer for {path}"))
}

// ---------------------------------------------------------------------------------------------------------
// search

/// The kinds of thing CurseForge lists for Minecraft, which the Java edition's add-mods dialog offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Section {
    Mods,
    Modpacks,
    ResourcePacks,
    Worlds,
    ShaderPacks,
    DataPacks,
    Plugins,
}

impl Section {
    /// What a player installs into an instance (plugins are for servers and are left out of the instance list).
    pub const FOR_INSTANCE: [Section; 5] =
        [Section::Mods, Section::ResourcePacks, Section::Worlds, Section::ShaderPacks, Section::DataPacks];
    /// Everything Discover can browse.
    pub const ALL: [Section; 6] = [
        Section::Mods,
        Section::Modpacks,
        Section::ResourcePacks,
        Section::Worlds,
        Section::ShaderPacks,
        Section::DataPacks,
    ];

    pub fn class_id(self) -> i64 {
        match self {
            Section::Mods => CLASS_MODS,
            Section::Modpacks => CLASS_MODPACKS,
            Section::ResourcePacks => CLASS_RESOURCE_PACKS,
            Section::Worlds => CLASS_WORLDS,
            Section::ShaderPacks => CLASS_SHADER_PACKS,
            Section::DataPacks => CLASS_DATA_PACKS,
            Section::Plugins => CLASS_PLUGINS,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Section::Mods => "Mods",
            Section::Modpacks => "Modpacks",
            Section::ResourcePacks => "Resource packs",
            Section::Worlds => "Worlds",
            Section::ShaderPacks => "Shaders",
            Section::DataPacks => "Data packs",
            Section::Plugins => "Plugins",
        }
    }

    /// Whether the Minecraft version of an instance narrows this section. The Java edition does not narrow
    /// resource packs and shaders (their pages rarely list versions correctly) and modpacks are narrowed only
    /// by an explicit version.
    pub fn uses_game_version(self) -> bool {
        matches!(self, Section::Mods | Section::Worlds | Section::DataPacks | Section::Plugins | Section::Modpacks)
    }

    /// Only mods are narrowed by the mod loader.
    pub fn uses_loader(self) -> bool {
        self == Section::Mods
    }
}

/// How a search is ordered. The names are what the CurseForge API calls them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Popularity,
    LastUpdated,
    TotalDownloads,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Popularity, Sort::LastUpdated, Sort::TotalDownloads];

    pub fn api_name(self) -> &'static str {
        match self {
            Sort::Popularity => "Popularity",
            Sort::LastUpdated => "LastUpdated",
            Sort::TotalDownloads => "TotalDownloads",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Popularity => "Popularity",
            Sort::LastUpdated => "Last Updated",
            Sort::TotalDownloads => "Total Downloads",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchQuery {
    pub section: Section,
    pub text: String,
    pub game_version: Option<String>,
    /// CurseForge mod loader numbers (1 Forge, 4 Fabric, 5 Quilt, 6 NeoForge).
    pub mod_loaders: Vec<i64>,
    pub sort: Sort,
    pub descending: bool,
    pub category_id: Option<i64>,
    /// First page is 0.
    pub page: u32,
}

impl SearchQuery {
    pub fn new(section: Section) -> SearchQuery {
        SearchQuery {
            section,
            text: String::new(),
            game_version: None,
            mod_loaders: Vec::new(),
            sort: Sort::Popularity,
            descending: true,
            category_id: None,
            page: 0,
        }
    }
}

pub struct SearchPage {
    pub projects: Vec<Project>,
    /// All matches (the API caps it at 10000).
    pub total: u64,
    /// Whether a further page probably exists.
    pub has_more: bool,
}

/// One page (20 results) of a search.
pub async fn search(q: &SearchQuery) -> Result<SearchPage> {
    let mut params: Vec<(&str, String)> = vec![
        ("gameId", GAME_ID.to_string()),
        ("classId", q.section.class_id().to_string()),
        ("searchFilter", q.text.clone()),
        ("sortField", q.sort.api_name().to_string()),
        ("sortOrder", if q.descending { "desc" } else { "asc" }.to_string()),
        ("pageSize", PAGE_SIZE.to_string()),
        ("index", (q.page * PAGE_SIZE).to_string()),
    ];
    if !q.mod_loaders.is_empty() {
        params.push(("modLoaderTypes", serde_json::to_string(&q.mod_loaders)?));
    }
    if let Some(v) = q.game_version.as_deref().filter(|v| !v.is_empty()) {
        params.push(("gameVersion", v.to_string()));
    }
    if let Some(c) = q.category_id {
        params.push(("categoryId", c.to_string()));
    }
    let r: Response<Vec<Project>> = get("/mods/search", &params, SHORT).await?;
    let count = r.data.len();
    let total = r.pagination.as_ref().map(|p| p.total_count.max(0) as u64).unwrap_or(0);
    let has_more = match &r.pagination {
        Some(p) => (p.index + p.result_count) < p.total_count && count > 0,
        None => count == PAGE_SIZE as usize,
    };
    Ok(SearchPage { projects: r.data, total, has_more })
}

/// A project by its slug (the last part of its CurseForge address) inside one class.
pub async fn project_by_slug(slug: &str, section: Section) -> Result<Option<Project>> {
    let params =
        [("gameId", GAME_ID.to_string()), ("slug", slug.to_string()), ("classId", section.class_id().to_string())];
    let r: Response<Vec<Project>> = get("/mods/search", &params, SHORT).await?;
    Ok(r.data.into_iter().next())
}

// ---------------------------------------------------------------------------------------------------------
// projects and files

pub async fn project(id: i64) -> Result<Project> {
    Ok(get::<Project>(&format!("/mods/{id}"), &[], SHORT).await?.data)
}

/// Several projects at once (a modpack's mods, or the projects of installed files).
pub async fn projects(ids: &[i64]) -> Result<Vec<Project>> {
    let mut out = Vec::new();
    let mut unique: Vec<i64> = Vec::new();
    for id in ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    for chunk in unique.chunks(500) {
        let r: Response<Vec<Project>> = post("/mods", json!({ "modIds": chunk })).await?;
        out.extend(r.data);
    }
    Ok(out)
}

/// The same, as a lookup by id.
pub async fn projects_map(ids: &[i64]) -> Result<HashMap<i64, Project>> {
    Ok(projects(ids).await?.into_iter().map(|p| (p.id, p)).collect())
}

/// The long description of a project, as HTML.
pub async fn description(id: i64) -> Result<String> {
    Ok(get::<String>(&format!("/mods/{id}/description"), &[("raw", "true".to_string())], LONG).await?.data)
}

/// Every file of a project, newest first (by file id, which is how the Java edition orders them).
pub async fn files(project_id: i64) -> Result<Vec<File>> {
    const STEP: i64 = 50;
    let mut all: Vec<File> = Vec::new();
    let mut index = 0i64;
    loop {
        let params = [("pageSize", STEP.to_string()), ("index", index.to_string())];
        let r: Response<Vec<File>> = get(&format!("/mods/{project_id}/files"), &params, SHORT).await?;
        let got = r.data.len() as i64;
        all.extend(r.data);
        let total = r.pagination.map(|p| p.total_count).unwrap_or(0);
        index += got;
        // the API will not page beyond 10000 results
        if got == 0 || index >= total || index >= 10_000 {
            break;
        }
    }
    all.sort_by(|a, b| b.id.cmp(&a.id));
    all.dedup_by_key(|f| f.id);
    Ok(all)
}

pub async fn file(project_id: i64, file_id: i64) -> Result<File> {
    Ok(get::<File>(&format!("/mods/{project_id}/files/{file_id}"), &[], LONG).await?.data)
}

/// Several files by id (the files of a modpack's manifest).
pub async fn files_by_ids(ids: &[i64]) -> Result<Vec<File>> {
    let mut out = Vec::new();
    let mut unique: Vec<i64> = Vec::new();
    for id in ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    for chunk in unique.chunks(500) {
        let r: Response<Vec<File>> = post("/mods/files", json!({ "fileIds": chunk })).await?;
        out.extend(r.data);
    }
    Ok(out)
}

/// What the author wrote about a file, as HTML.
pub async fn changelog(project_id: i64, file_id: i64) -> Result<String> {
    Ok(get::<String>(&format!("/mods/{project_id}/files/{file_id}/changelog"), &[], LONG).await?.data)
}

/// Where CurseForge says a file can be fetched from; an error when the author does not allow it.
pub async fn download_url(project_id: i64, file_id: i64) -> Result<String> {
    Ok(get::<String>(&format!("/mods/{project_id}/files/{file_id}/download-url"), &[], SHORT).await?.data)
}

/// Identify files by their [fingerprint](super::fingerprint): the projects and files CurseForge knows them as.
pub async fn fingerprints(prints: &[u32]) -> Result<Fingerprint> {
    let r: Response<Fingerprint> = post("/fingerprints", json!({ "fingerprints": prints })).await?;
    Ok(r.data)
}

// ---------------------------------------------------------------------------------------------------------
// categories

/// All Minecraft categories (classes and their children), remembered for an hour.
pub async fn categories() -> Result<Vec<CategoryForGame>> {
    Ok(get::<Vec<CategoryForGame>>("/categories", &[("gameId", GAME_ID.to_string())], LONG).await?.data)
}

/// The categories of one section, sorted by name, for the category drop-down.
pub async fn categories_for(section: Section) -> Result<Vec<CategoryForGame>> {
    let mut list: Vec<CategoryForGame> =
        categories().await?.into_iter().filter(|c| c.class_id == Some(section.class_id())).collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}
