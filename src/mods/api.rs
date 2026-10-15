//! The parts of the Modrinth API that mod management uses: finding out what a file is from its hash, looking for
//! the newest version of each file in one request, and fetching a project's versions.
//!
//! Projects and versions are kept as the JSON Modrinth sent (`raw`), because that is exactly what the Java edition
//! saves in `instance.json` for each mod, and what it reads back.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::versions::VersionLike;
use crate::retry;

pub const API: &str = "https://api.modrinth.com/v2";

/// Projects asked for in one request; the address would get too long beyond this.
const PROJECT_CHUNK: usize = 100;
/// Hashes sent in one request.
const HASH_CHUNK: usize = 500;

#[derive(Clone, Debug)]
pub struct MrFile {
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
    pub hashes: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct MrDependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    /// `required`, `optional`, `incompatible` or `embedded`.
    pub dependency_type: String,
}

/// One version of a Modrinth project.
#[derive(Clone, Debug)]
pub struct MrVersion {
    pub raw: Value,
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub date_published: String,
    /// `release`, `beta` or `alpha`.
    pub version_type: Option<String>,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub files: Vec<MrFile>,
    pub dependencies: Vec<MrDependency>,
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn string(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

impl MrVersion {
    pub fn from_value(raw: Value) -> Option<MrVersion> {
        let id = raw.get("id")?.as_str()?.to_string();
        let files = raw
            .get("files")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|f| MrFile {
                        url: string(f, "url"),
                        filename: string(f, "filename"),
                        primary: f.get("primary").and_then(Value::as_bool).unwrap_or(false),
                        size: f.get("size").and_then(Value::as_u64).unwrap_or(0),
                        hashes: f
                            .get("hashes")
                            .and_then(Value::as_object)
                            .map(|h| h.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let dependencies = raw
            .get("dependencies")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|d| MrDependency {
                        project_id: d.get("project_id").and_then(Value::as_str).map(str::to_string),
                        version_id: d.get("version_id").and_then(Value::as_str).map(str::to_string),
                        dependency_type: string(d, "dependency_type"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(MrVersion {
            id,
            project_id: string(&raw, "project_id"),
            name: string(&raw, "name"),
            version_number: string(&raw, "version_number"),
            date_published: string(&raw, "date_published"),
            version_type: raw.get("version_type").and_then(Value::as_str).map(str::to_string),
            loaders: strings(&raw, "loaders"),
            game_versions: strings(&raw, "game_versions"),
            files,
            dependencies,
            raw,
        })
    }

    pub fn primary_file(&self) -> Option<&MrFile> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }

    /// The file whose sha1 is `sha1`, or the primary one when none matches.
    pub fn file_by_sha1(&self, sha1: &str) -> Option<&MrFile> {
        self.files
            .iter()
            .find(|f| f.hashes.get("sha1").is_some_and(|h| h.eq_ignore_ascii_case(sha1)))
            .or_else(|| self.primary_file())
    }

    /// What the version is called in lists: "Sodium 0.9.2 (Release)".
    pub fn label(&self) -> String {
        match self.version_type.as_deref() {
            None => self.name.clone(),
            Some("alpha") => format!("{} (Alpha)", self.name),
            Some("beta") => format!("{} (Beta)", self.name),
            Some(_) => format!("{} (Release)", self.name),
        }
    }
}

impl VersionLike for MrVersion {
    fn id(&self) -> &str {
        &self.id
    }
    fn date_published(&self) -> &str {
        &self.date_published
    }
    fn version_type(&self) -> Option<&str> {
        self.version_type.as_deref()
    }
    fn loaders(&self) -> &[String] {
        &self.loaders
    }
    fn game_versions(&self) -> &[String] {
        &self.game_versions
    }
}

/// A connection to Modrinth, carrying the player's API key when they set one.
#[derive(Clone, Debug, Default)]
pub struct Modrinth {
    pub api_key: Option<String>,
}

impl Modrinth {
    pub fn new(api_key: Option<String>) -> Modrinth {
        Modrinth { api_key }
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let r = crate::http::client().get(url);
        match &self.api_key {
            Some(k) => r.header("Authorization", k),
            None => r,
        }
    }

    fn post(&self, url: &str, body: &Value) -> reqwest::RequestBuilder {
        let r = crate::http::client().post(url).json(body);
        match &self.api_key {
            Some(k) => r.header("Authorization", k),
            None => r,
        }
    }

    /// The version each hash belongs to (`POST /version_files`). Files Modrinth doesn't know are left out.
    pub async fn versions_from_hashes(&self, hashes: &[String], algorithm: &str) -> Result<HashMap<String, MrVersion>> {
        let mut out = HashMap::new();
        for chunk in hashes.chunks(HASH_CHUNK) {
            let body = json!({"hashes": chunk, "algorithm": algorithm});
            let answer = self.json(retry::send(|| self.post(&format!("{API}/version_files"), &body)).await?).await?;
            out.extend(parse_version_map(&answer));
        }
        Ok(out)
    }

    pub async fn versions_from_sha1(&self, hashes: &[String]) -> Result<HashMap<String, MrVersion>> {
        self.versions_from_hashes(hashes, "sha1").await
    }

    pub async fn versions_from_sha512(&self, hashes: &[String]) -> Result<HashMap<String, MrVersion>> {
        self.versions_from_hashes(hashes, "sha512").await
    }

    /// The version a single hash belongs to (`GET /version_file/<hash>`), or `None` when Modrinth doesn't know it.
    pub async fn version_from_hash(&self, hash: &str, algorithm: &str) -> Result<Option<MrVersion>> {
        let r = retry::send(|| self.get(&format!("{API}/version_file/{hash}?algorithm={algorithm}"))).await?;
        if r.status().as_u16() == 404 {
            return Ok(None);
        }
        Ok(MrVersion::from_value(self.json(r).await?))
    }

    /// The newest version of each file's project (`POST /version_files/update`), in one request. `loaders` and
    /// `game_versions` narrow it down; empty means any.
    pub async fn latest_from_sha1(
        &self,
        hashes: &[String],
        loaders: &[String],
        game_versions: &[String],
    ) -> Result<HashMap<String, MrVersion>> {
        let mut out = HashMap::new();
        for chunk in hashes.chunks(HASH_CHUNK) {
            let mut body = json!({"hashes": chunk, "algorithm": "sha1"});
            if !loaders.is_empty() {
                body["loaders"] = json!(loaders);
            }
            if !game_versions.is_empty() {
                body["game_versions"] = json!(game_versions);
            }
            let answer =
                self.json(retry::send(|| self.post(&format!("{API}/version_files/update"), &body)).await?).await?;
            out.extend(parse_version_map(&answer));
        }
        Ok(out)
    }

    /// Projects by id, as Modrinth's JSON.
    pub async fn projects(&self, ids: &[String]) -> Result<HashMap<String, Value>> {
        let mut unique: Vec<&String> = Vec::new();
        for id in ids {
            if !unique.contains(&id) {
                unique.push(id);
            }
        }
        let mut out = HashMap::new();
        for chunk in unique.chunks(PROJECT_CHUNK) {
            let list = serde_json::to_string(&chunk)?;
            let answer = self
                .json(retry::send(|| self.get(&format!("{API}/projects")).query(&[("ids", list.as_str())])).await?)
                .await?;
            for p in answer.as_array().into_iter().flatten() {
                if let Some(id) = p.get("id").and_then(Value::as_str) {
                    out.insert(id.to_string(), p.clone());
                }
            }
        }
        Ok(out)
    }

    /// One project, by id or slug.
    pub async fn project(&self, id: &str) -> Result<Value> {
        let id = id.replace("local-", "");
        self.json(retry::send(|| self.get(&format!("{API}/project/{id}"))).await?).await
    }

    /// Every version of a project, optionally only those for a game version and some loaders. Order is Modrinth's.
    pub async fn versions(
        &self,
        project: &str,
        game_version: Option<&str>,
        loaders: &[String],
    ) -> Result<Vec<MrVersion>> {
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(g) = game_version {
            query.push(("game_versions", serde_json::to_string(&[g])?));
        }
        if !loaders.is_empty() {
            query.push(("loaders", serde_json::to_string(loaders)?));
        }
        let answer = self
            .json(retry::send(|| self.get(&format!("{API}/project/{project}/version")).query(&query)).await?)
            .await?;
        Ok(answer.as_array().into_iter().flatten().filter_map(|v| MrVersion::from_value(v.clone())).collect())
    }

    /// A single version by id.
    pub async fn version(&self, id: &str) -> Result<MrVersion> {
        let v = self.json(retry::send(|| self.get(&format!("{API}/version/{id}"))).await?).await?;
        MrVersion::from_value(v).with_context(|| format!("Modrinth sent something unexpected for version {id}"))
    }

    async fn json(&self, r: reqwest::Response) -> Result<Value> {
        let status = r.status();
        if !status.is_success() {
            bail!("Modrinth answered HTTP {status}");
        }
        r.json().await.context("reading Modrinth's answer")
    }
}

fn parse_version_map(answer: &Value) -> HashMap<String, MrVersion> {
    answer
        .as_object()
        .map(|o| o.iter().filter_map(|(hash, v)| Some((hash.clone(), MrVersion::from_value(v.clone())?))).collect())
        .unwrap_or_default()
}

/// The loaders to ask Modrinth for when listing versions of a mod for an instance (what the Java edition's
/// `ModrinthApi.getVersions` sends). Plain Minecraft sends none.
pub fn loaders_for_listing(
    loader_key: Option<&str>,
    minecraft_version: &str,
    neoforge_forge_versions: &[String],
) -> Vec<String> {
    match loader_key {
        Some("forge") => vec!["forge".into()],
        Some("neoforge") => {
            let mut v = Vec::new();
            if neoforge_forge_versions.iter().any(|x| x == minecraft_version) {
                v.push("forge".to_string());
            }
            v.push("neoforge".into());
            v
        }
        Some("fabric") => vec!["fabric".into()],
        Some("quilt") => vec!["fabric".into(), "quilt".into()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_version_as_modrinth_sends_it() {
        let v = MrVersion::from_value(json!({
            "id": "b4sCL1Qe", "project_id": "wdLuzzEP", "name": "Gamma Utils 3.2.0", "version_number": "3.2.0+Fabric",
            "date_published": "2026-09-15T21:57:29.549563Z", "version_type": "release",
            "files": [{"hashes": {"sha1": "ab35", "sha512": "ff"}, "url": "https://cdn/x.jar", "filename": "x.jar", "primary": true, "size": 10}],
            "dependencies": [{"project_id": "9s6osm5g", "dependency_type": "required"}, {"file_name": "x", "dependency_type": "embedded"}],
            "game_versions": ["26.3"], "loaders": ["fabric"]
        }))
        .unwrap();
        assert_eq!(v.primary_file().unwrap().filename, "x.jar");
        assert_eq!(v.file_by_sha1("AB35").unwrap().url, "https://cdn/x.jar");
        assert_eq!(v.dependencies.len(), 2);
        assert_eq!(v.dependencies[0].project_id.as_deref(), Some("9s6osm5g"));
        assert_eq!(v.label(), "Gamma Utils 3.2.0 (Release)");
        assert_eq!(v.raw["project_id"], "wdLuzzEP");
        assert!(MrVersion::from_value(json!({"name": "no id"})).is_none());
    }

    #[test]
    fn listing_loaders_follow_the_java_edition() {
        let compat = vec!["1.20.1".to_string()];
        assert_eq!(loaders_for_listing(Some("quilt"), "1.21", &compat), vec!["fabric", "quilt"]);
        assert_eq!(loaders_for_listing(Some("neoforge"), "1.20.1", &compat), vec!["forge", "neoforge"]);
        assert_eq!(loaders_for_listing(Some("neoforge"), "1.21", &compat), vec!["neoforge"]);
        assert!(loaders_for_listing(None, "1.21", &compat).is_empty());
    }
}
