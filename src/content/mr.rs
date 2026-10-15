//! The parts of Modrinth's API the content manager needs, with the objects kept whole: what is saved in an
//! instance's mod list is the project and version exactly as Modrinth described them (as the Java edition saves
//! them), so the types here carry every field they don't name in `extra`.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::http::client;

pub const API: &str = "https://api.modrinth.com/v2";
/// Search results come this many to a page.
pub const PAGE_SIZE: u32 = 20;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct File {
    #[serde(default)]
    pub hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Dependency {
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    /// `required`, `optional`, `incompatible` or `embedded`
    #[serde(default)]
    pub dependency_type: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Version {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_number: String,
    /// ISO 8601 in UTC, so it sorts as text
    #[serde(default)]
    pub date_published: Option<String>,
    /// `release`, `beta` or `alpha`
    #[serde(default)]
    pub version_type: Option<String>,
    #[serde(default)]
    pub files: Vec<File>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Version {
    /// The file to download: the one marked primary, otherwise the first.
    pub fn primary_file(&self) -> Option<&File> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }

    /// Whether this is a full release (a version with no stated type counts as one).
    pub fn is_release(&self) -> bool {
        self.version_type.as_deref().is_none_or(|t| t == "release")
    }

    /// What goes into an instance's mod list for this version: as the API gave it, minus the fields the Java
    /// edition leaves out.
    pub fn record_value(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(map) = value.as_object_mut() {
            for key in ["featured", "downloads"] {
                map.remove(key);
            }
        }
        value
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Project {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default = "mod_type")]
    pub project_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn mod_type() -> String {
    "mod".to_string()
}

impl Project {
    /// What goes into an instance's mod list for this project: as the API gave it, minus the fields the Java
    /// edition leaves out.
    pub fn record_value(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(map) = value.as_object_mut() {
            for key in ["moderator_message", "downloads", "followers", "versions"] {
                map.remove(key);
            }
        }
        value
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct SearchHit {
    #[serde(default)]
    pub project_id: String,
    #[serde(default = "mod_type")]
    pub project_type: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub follows: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub date_created: Option<String>,
    #[serde(default)]
    pub date_modified: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub client_side: Option<String>,
    #[serde(default)]
    pub server_side: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct SearchResult {
    #[serde(default)]
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: u32,
    #[serde(default)]
    pub total_hits: u64,
}

/// Talks to Modrinth. Carries the player's API key when they have set one.
#[derive(Clone, Debug, Default)]
pub struct Api {
    pub key: Option<String>,
}

impl Api {
    fn get(&self, url: String) -> reqwest::RequestBuilder {
        let r = client().get(url);
        match self.key.as_deref().filter(|k| !k.is_empty()) {
            Some(key) => r.header("Authorization", key),
            None => r,
        }
    }

    fn post(&self, url: String) -> reqwest::RequestBuilder {
        let r = client().post(url);
        match self.key.as_deref().filter(|k| !k.is_empty()) {
            Some(key) => r.header("Authorization", key),
            None => r,
        }
    }

    async fn read<T: serde::de::DeserializeOwned>(r: reqwest::Response, what: &str) -> Result<T> {
        if !r.status().is_success() {
            bail!("Modrinth: HTTP {} {what}", r.status());
        }
        r.json().await.with_context(|| format!("reading what Modrinth said {what}"))
    }

    /// Searches one kind of project.
    ///
    /// `game_versions` are alternatives (any one will do); `categories` is a list of "any of" groups that must all
    /// match (a mod loader group, for example).
    pub async fn search(
        &self,
        game_versions: Option<&[String]>,
        query: &str,
        page: u32,
        index: &str,
        categories: &[Vec<String>],
        project_type: Option<&str>,
    ) -> Result<SearchResult> {
        let mut facets: Vec<Vec<String>> = Vec::new();
        if let Some(gv) = game_versions.filter(|g| !g.is_empty()) {
            facets.push(gv.iter().map(|g| format!("versions:{g}")).collect());
        }
        for group in categories {
            facets.push(group.iter().map(|c| format!("categories:{c}")).collect());
        }
        if let Some(t) = project_type {
            facets.push(vec![format!("project_type:{}", t.to_lowercase())]);
        }
        let mut req = self.get(format!("{API}/search")).query(&[
            ("limit", PAGE_SIZE.to_string()),
            ("offset", (page * PAGE_SIZE).to_string()),
            ("query", query.to_string()),
            ("index", index.to_string()),
        ]);
        if !facets.is_empty() {
            req = req.query(&[("facets", serde_json::to_string(&facets)?)]);
        }
        let r = req.send().await.context("Couldn't reach Modrinth")?;
        Self::read(r, "for the search").await
    }

    pub async fn project(&self, id: &str) -> Result<Project> {
        let r = self
            .get(format!("{API}/project/{}", id.replace("local-", "")))
            .send()
            .await
            .context("Couldn't reach Modrinth")?;
        Self::read(r, &format!("for project {id}")).await
    }

    /// Every version of a project, newest first as Modrinth lists them.
    pub async fn versions(&self, id: &str) -> Result<Vec<Version>> {
        let r = self.get(format!("{API}/project/{id}/version")).send().await.context("Couldn't reach Modrinth")?;
        Self::read(r, &format!("for the versions of {id}")).await
    }

    /// Several projects at once.
    pub async fn projects(&self, ids: &[String]) -> Result<Vec<Project>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let r = self
            .get(format!("{API}/projects"))
            .query(&[("ids", serde_json::to_string(ids)?)])
            .send()
            .await
            .context("Couldn't reach Modrinth")?;
        Self::read(r, "for the projects").await
    }

    /// The version each file is, by the sha1 of the file. Files Modrinth doesn't know are left out.
    pub async fn versions_from_sha1(&self, hashes: &[String]) -> Result<HashMap<String, Version>> {
        if hashes.is_empty() {
            return Ok(HashMap::new());
        }
        let r = self
            .post(format!("{API}/version_files"))
            .json(&json!({ "hashes": hashes, "algorithm": "sha1" }))
            .send()
            .await
            .context("Couldn't reach Modrinth")?;
        Self::read(r, "for the files").await
    }

    /// The newest version of each file's project in one request, optionally only for some loaders and Minecraft
    /// versions. Files Modrinth doesn't know are left out.
    pub async fn latest_from_sha1(
        &self,
        hashes: &[String],
        loaders: Option<&[String]>,
        game_versions: Option<&[String]>,
    ) -> Result<HashMap<String, Version>> {
        if hashes.is_empty() {
            return Ok(HashMap::new());
        }
        let mut body = json!({ "hashes": hashes, "algorithm": "sha1" });
        if let Some(l) = loaders.filter(|l| !l.is_empty()) {
            body["loaders"] = json!(l);
        }
        if let Some(g) = game_versions.filter(|g| !g.is_empty()) {
            body["game_versions"] = json!(g);
        }
        let r = self
            .post(format!("{API}/version_files/update"))
            .json(&body)
            .send()
            .await
            .context("Couldn't reach Modrinth")?;
        Self::read(r, "for the updates").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_keep_every_field_they_dont_name() {
        let text = r#"{"id":"v1","project_id":"p","name":"Sodium 0.6","version_number":"0.6","date_published":"2026-01-01T00:00:00Z",
            "version_type":"release","featured":true,"downloads":9,"changelog":"x","files":[{"hashes":{"sha1":"aa"},"url":"https://x/a.jar","filename":"a.jar","primary":true,"size":5,"file_type":null}],
            "dependencies":[{"version_id":null,"project_id":"q","file_name":null,"dependency_type":"required"}],"game_versions":["1.21.1"],"loaders":["fabric"],"status":"listed"}"#;
        let v: Version = serde_json::from_str(text).unwrap();
        assert!(v.is_release());
        assert_eq!(v.primary_file().unwrap().filename, "a.jar");
        let rec = v.record_value();
        assert_eq!(rec["changelog"], "x");
        assert_eq!(rec["status"], "listed");
        assert!(rec.get("featured").is_none() && rec.get("downloads").is_none());
        assert_eq!(rec["files"][0]["file_type"], Value::Null);
    }

    #[test]
    fn projects_drop_the_fields_the_java_edition_leaves_out() {
        let p: Project = serde_json::from_str(r#"{"id":"AANobbMI","slug":"sodium","project_type":"mod","title":"Sodium","description":"d","downloads":5,"followers":6,"versions":["a"],"body":"b"}"#).unwrap();
        let rec = p.record_value();
        assert_eq!(rec["id"], "AANobbMI");
        assert_eq!(rec["body"], "b");
        for gone in ["downloads", "followers", "versions"] {
            assert!(rec.get(gone).is_none());
        }
    }
}
