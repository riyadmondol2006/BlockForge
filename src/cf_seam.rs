//! THE CURSEFORGE SEAM.
//!
//! Another work package writes the real `curseforge.rs`. FTB packs (a few files are hosted on CurseForge), the
//! CurseForge export and the CurseForge page importer only need three things from it, so they ask for them through
//! this small trait instead of depending on that module. [`MinimalCurseForge`] is a private, self-contained
//! implementation of the three calls with the same public API key the Java edition ships; once `curseforge.rs`
//! exists, the lead can implement [`CurseForgeSource`] on its client and drop the minimal one.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::http::client;

const API: &str = "https://api.curseforge.com/v1";
/// The public key the Java edition uses for CurseForge's core API (`Constants.CURSEFORGE_CORE_API_KEY`).
const API_KEY: &str = "$2a$10$.7CSxLm/lnj5lCBSM5jGQ.3SICSX4j9r661AgoB1Rc4Nw8jCMKcv2";

/// A CurseForge project (mod, modpack, ...), keeping the whole object the API sent so `instance.json` can hold it
/// the way the Java edition does.
#[derive(Clone, Debug, Default)]
pub struct CfProject {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub summary: String,
    /// `links.websiteUrl`
    pub website_url: Option<String>,
    /// CurseForge's own moderation state; 4 means "approved".
    pub status: i64,
    pub class_id: i64,
    pub allow_mod_distribution: Option<bool>,
    pub raw: Value,
}

impl CfProject {
    pub fn from_json(v: &Value) -> CfProject {
        CfProject {
            id: v["id"].as_i64().unwrap_or(0),
            name: v["name"].as_str().unwrap_or("").to_string(),
            slug: v["slug"].as_str().unwrap_or("").to_string(),
            summary: v["summary"].as_str().unwrap_or("").to_string(),
            website_url: v["links"]["websiteUrl"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
            status: v["status"].as_i64().unwrap_or(0),
            class_id: v["classId"].as_i64().unwrap_or(0),
            allow_mod_distribution: v["allowModDistribution"].as_bool(),
            raw: v.clone(),
        }
    }

    /// The page a player can open to download `file_id` by hand (`CurseForgeProject.getBrowserDownloadUrl`).
    pub fn browser_download_url(&self, file_id: i64) -> String {
        match &self.website_url {
            Some(site) => format!("{site}/download/{file_id}"),
            None => {
                let class = match self.class_id {
                    12 => "texture-packs",
                    17 => "worlds",
                    4471 => "modpacks",
                    6552 => "shaders",
                    6945 => "data-packs",
                    _ => "mc-mods",
                };
                format!("https://www.curseforge.com/minecraft/{class}/{}/download/{file_id}", self.slug)
            }
        }
    }
}

/// One uploaded file of a CurseForge project.
#[derive(Clone, Debug, Default)]
pub struct CfFile {
    pub id: i64,
    pub project_id: i64,
    pub display_name: String,
    pub file_name: String,
    /// `None` when the author turned off downloads by other launchers.
    pub download_url: Option<String>,
    pub file_length: i64,
    pub is_available: bool,
    pub sha1: Option<String>,
    pub md5: Option<String>,
    pub fingerprint: u32,
    pub raw: Value,
}

impl CfFile {
    pub fn from_json(v: &Value) -> CfFile {
        let hash = |algo: i64| {
            v["hashes"]
                .as_array()
                .and_then(|a| a.iter().find(|h| h["algo"].as_i64() == Some(algo)))
                .and_then(|h| h["value"].as_str())
                .map(str::to_string)
        };
        CfFile {
            id: v["id"].as_i64().unwrap_or(0),
            project_id: v["modId"].as_i64().or_else(|| v["projectId"].as_i64()).unwrap_or(0),
            display_name: v["displayName"].as_str().unwrap_or("").to_string(),
            file_name: v["fileName"].as_str().unwrap_or("").to_string(),
            download_url: v["downloadUrl"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
            file_length: v["fileLength"].as_i64().unwrap_or(0),
            is_available: v["isAvailable"].as_bool().unwrap_or(true),
            sha1: hash(1),
            md5: hash(2),
            fingerprint: v["fileFingerprint"].as_u64().or_else(|| v["packageFingerprint"].as_u64()).unwrap_or(0) as u32,
            raw: v.clone(),
        }
    }

    /// The object kept in `instance.json`: like the Java edition, without the download address and count (they
    /// change, and the address is not meant to be stored).
    pub fn for_instance_json(&self) -> Value {
        let mut v = self.raw.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("downloadUrl");
            o.remove("downloadCount");
        }
        v
    }
}

/// A file found by its fingerprint.
#[derive(Clone, Debug)]
pub struct CfMatch {
    pub fingerprint: u32,
    pub project: CfProject,
    pub file: CfFile,
}

/// What this crate needs from CurseForge. Implement it on the real client when `curseforge.rs` lands.
pub trait CurseForgeSource: Send + Sync {
    /// One file and the project it belongs to.
    fn file(&self, project_id: i64, file_id: i64) -> BoxFuture<'_, Result<(CfProject, CfFile)>>;

    /// Many files at once; the default asks one by one. Files that can't be found are left out.
    fn files(&self, pairs: Vec<(i64, i64)>) -> BoxFuture<'_, Result<HashMap<i64, (CfProject, CfFile)>>> {
        Box::pin(async move {
            let mut out = HashMap::new();
            for (p, f) in pairs {
                if let Ok(found) = self.file(p, f).await {
                    out.insert(f, found);
                }
            }
            Ok(out)
        })
    }

    /// Files whose CurseForge fingerprint (the whitespace-less Murmur2 hash) is in `fingerprints`.
    fn fingerprints(&self, fingerprints: Vec<u32>) -> BoxFuture<'_, Result<Vec<CfMatch>>>;
}

/// The private, minimal client (see the module comment).
#[derive(Default, Clone, Copy)]
pub struct MinimalCurseForge;

impl MinimalCurseForge {
    async fn send(req: reqwest::RequestBuilder) -> Result<Value> {
        let r = req
            .header("x-api-key", API_KEY)
            .header("Accept", "application/json")
            .send()
            .await
            .context("asking CurseForge")?;
        if !r.status().is_success() {
            bail!("CurseForge answered HTTP {}", r.status());
        }
        let v: Value = r.json().await.context("reading CurseForge's answer")?;
        Ok(v.get("data").cloned().unwrap_or(Value::Null))
    }

    async fn projects(ids: &[i64]) -> Result<HashMap<i64, CfProject>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let data = Self::send(client().post(format!("{API}/mods")).json(&json!({ "modIds": ids }))).await?;
        Ok(data.as_array().map(|a| a.iter().map(CfProject::from_json).map(|p| (p.id, p)).collect()).unwrap_or_default())
    }
}

impl CurseForgeSource for MinimalCurseForge {
    fn file(&self, project_id: i64, file_id: i64) -> BoxFuture<'_, Result<(CfProject, CfFile)>> {
        Box::pin(async move {
            let file = Self::send(client().get(format!("{API}/mods/{project_id}/files/{file_id}"))).await?;
            if file.is_null() {
                bail!("CurseForge has no file {file_id} for project {project_id}");
            }
            let project = Self::send(client().get(format!("{API}/mods/{project_id}"))).await?;
            Ok((CfProject::from_json(&project), CfFile::from_json(&file)))
        })
    }

    fn files(&self, pairs: Vec<(i64, i64)>) -> BoxFuture<'_, Result<HashMap<i64, (CfProject, CfFile)>>> {
        Box::pin(async move {
            let mut out = HashMap::new();
            for chunk in pairs.chunks(200) {
                let ids: Vec<i64> = chunk.iter().map(|p| p.1).collect();
                let data =
                    Self::send(client().post(format!("{API}/mods/files")).json(&json!({ "fileIds": ids }))).await?;
                let files: Vec<CfFile> =
                    data.as_array().map(|a| a.iter().map(CfFile::from_json).collect()).unwrap_or_default();
                let mut project_ids: Vec<i64> = files.iter().map(|f| f.project_id).collect();
                project_ids.sort_unstable();
                project_ids.dedup();
                let mut projects = HashMap::new();
                for ids in project_ids.chunks(200) {
                    projects.extend(Self::projects(ids).await?);
                }
                for f in files {
                    if let Some(p) = projects.get(&f.project_id) {
                        out.insert(f.id, (p.clone(), f));
                    }
                }
            }
            Ok(out)
        })
    }

    fn fingerprints(&self, fingerprints: Vec<u32>) -> BoxFuture<'_, Result<Vec<CfMatch>>> {
        Box::pin(async move {
            if fingerprints.is_empty() {
                return Ok(Vec::new());
            }
            let data =
                Self::send(client().post(format!("{API}/fingerprints")).json(&json!({ "fingerprints": fingerprints })))
                    .await?;
            let exact = data["exactMatches"].as_array().cloned().unwrap_or_default();
            let mut ids: Vec<i64> = exact.iter().filter_map(|m| m["id"].as_i64()).collect();
            ids.sort_unstable();
            ids.dedup();
            let mut projects = HashMap::new();
            for chunk in ids.chunks(200) {
                projects.extend(Self::projects(chunk).await?);
            }
            Ok(exact
                .iter()
                .filter_map(|m| {
                    let file = CfFile::from_json(&m["file"]);
                    let project = projects.get(&m["id"].as_i64()?)?.clone();
                    Some(CfMatch { fingerprint: file.fingerprint, project, file })
                })
                .collect())
        })
    }
}

/// CurseForge's file fingerprint: Murmur2 (seed 1) over the file with tabs, line breaks and spaces removed.
pub fn fingerprint(data: &[u8]) -> u32 {
    let filtered: Vec<u8> = data.iter().copied().filter(|b| !matches!(b, 9 | 10 | 13 | 32)).collect();
    murmur2(&filtered, 1)
}

/// MurmurHash2 (32 bit).
pub fn murmur2(data: &[u8], seed: u32) -> u32 {
    const M: u32 = 0x5bd1_e995;
    const R: u32 = 24;
    let mut h: u32 = seed ^ (data.len() as u32);
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        let mut k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M);
        h ^= k;
    }
    let rest = chunks.remainder();
    if rest.len() >= 3 {
        h ^= (rest[2] as u32) << 16;
    }
    if rest.len() >= 2 {
        h ^= (rest[1] as u32) << 8;
    }
    if !rest.is_empty() {
        h ^= rest[0] as u32;
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur2_matches_the_reference_values() {
        // reference values of MurmurHash2 (seed 0) from the original implementation's test vectors
        assert_eq!(murmur2(b"", 0), 0);
        assert_eq!(murmur2(b"a", 0), 0x92685f5e);
        assert_eq!(murmur2(b"abc", 0), 0x13577c9b);
    }

    #[test]
    fn fingerprint_ignores_whitespace() {
        assert_eq!(fingerprint(b"a b\r\n\tc"), murmur2(b"abc", 1));
    }

    #[test]
    fn file_and_project_read_java_written_objects() {
        // trimmed from a real `launcher.mods` entry written by the Java edition
        let file = CfFile::from_json(
            &json!({"id":8885210,"gameId":432,"isAvailable":true,"displayName":"x.jar","fileName":"x.jar",
            "fileLength":2229181,"downloadUrl":"https://edge.forgecdn.net/files/8885/210/x.jar","downloadCount":3,
            "hashes":[{"value":"cfce17f4a407bdeef6df03e193473bc8090e1b17","algo":1},{"value":"2e3ac64efe345af7e4cada78ccd85079","algo":2}],
            "fileFingerprint":3100092788u64,"modId":263420}),
        );
        assert_eq!(file.fingerprint, 3100092788);
        assert_eq!(file.project_id, 263420);
        assert_eq!(file.sha1.as_deref(), Some("cfce17f4a407bdeef6df03e193473bc8090e1b17"));
        assert!(file.for_instance_json().get("downloadUrl").is_none());
        let project =
            CfProject::from_json(&json!({"id":263420,"name":"Xaero's Minimap","slug":"xaeros-minimap","status":4,
            "links":{"websiteUrl":"https://www.curseforge.com/minecraft/mc-mods/xaeros-minimap"}}));
        assert_eq!(
            project.browser_download_url(8885210),
            "https://www.curseforge.com/minecraft/mc-mods/xaeros-minimap/download/8885210"
        );
        let bare = CfProject { slug: "pack".into(), class_id: 4471, ..Default::default() };
        assert_eq!(bare.browser_download_url(5), "https://www.curseforge.com/minecraft/modpacks/pack/download/5");
    }
}
