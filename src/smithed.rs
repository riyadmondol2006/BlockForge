//! Smithed (smithed.net): the data pack site. Searching it, and adding a pack to an instance's worlds. Its
//! download link bundles a pack with the packs it needs, so one file is all a world needs.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::http::client;
use crate::install::{self, Job, Progress};
use crate::instances::Instance;
use crate::modrecord::ModRecord;
use crate::modrinth::Sort;

const API: &str = "https://api.smithed.dev/v2";

/// A pack in Smithed's search results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pack {
    /// The id in its page's address (`myriad`), which its downloads are asked for by.
    pub id: String,
    pub name: String,
    pub summary: String,
    pub icon: Option<String>,
    pub downloads: u64,
    pub categories: Vec<String>,
    /// Every Minecraft version some version of the pack is made for.
    pub supports: Vec<String>,
}

impl Pack {
    pub fn page_url(&self) -> String {
        format!("https://smithed.net/packs/{}", self.id)
    }

    /// The name the pack's zip gets in a world's `datapacks` folder.
    pub fn file_name(&self) -> String {
        let id: String =
            self.id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        format!("{}.zip", if id.is_empty() { "pack" } else { &id })
    }
}

#[derive(Deserialize, Default)]
struct RawPack {
    #[serde(default, rename = "displayName")]
    display_name: String,
    #[serde(default)]
    data: RawData,
    #[serde(default)]
    meta: RawMeta,
}

#[derive(Deserialize, Default)]
struct RawData {
    #[serde(default)]
    id: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    display: RawDisplay,
    #[serde(default)]
    versions: Vec<RawVersion>,
}

#[derive(Deserialize, Default)]
struct RawDisplay {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    hidden: bool,
}

#[derive(Deserialize, Default)]
struct RawVersion {
    #[serde(default)]
    supports: Vec<String>,
}

#[derive(Deserialize, Default)]
struct RawMeta {
    #[serde(default, rename = "rawId")]
    raw_id: String,
    #[serde(default)]
    stats: RawStats,
}

#[derive(Deserialize, Default)]
struct RawStats {
    #[serde(default)]
    downloads: RawDownloads,
}

#[derive(Deserialize, Default)]
struct RawDownloads {
    #[serde(default)]
    total: u64,
}

impl RawPack {
    fn into_pack(self) -> Option<Pack> {
        let id = if self.meta.raw_id.is_empty() { self.data.id } else { self.meta.raw_id };
        if id.is_empty() || self.data.display.hidden {
            return None;
        }
        let name = if self.data.display.name.is_empty() { self.display_name } else { self.data.display.name };
        let mut supports: Vec<String> = Vec::new();
        for v in self.data.versions {
            for mc in v.supports {
                if !supports.contains(&mc) {
                    supports.push(mc);
                }
            }
        }
        let icon = Some(self.data.display.icon).filter(|u| u.starts_with("https://"));
        Some(Pack {
            id,
            name,
            summary: self.data.display.description,
            icon,
            downloads: self.meta.stats.downloads.total,
            categories: self.data.categories,
            supports,
        })
    }
}

/// Smithed's order for one of Discover's; `None` is its own order for the words searched for.
fn sort_name(sort: Sort) -> Option<&'static str> {
    match sort {
        Sort::Relevance => None,
        Sort::Downloads => Some("downloads"),
        Sort::Updated => Some("newest"),
    }
}

fn query(text: &str, mc: Option<&str>, sort: Sort) -> Vec<(&'static str, String)> {
    let mut q = Vec::new();
    if !text.trim().is_empty() {
        q.push(("search", text.trim().to_string()));
    }
    if let Some(mc) = mc.filter(|m| !m.is_empty()) {
        q.push(("version", mc.to_string()));
    }
    if let Some(s) = sort_name(sort) {
        q.push(("sort", s.to_string()));
    }
    q
}

/// One page (`page` counts from 0) of the packs that match `text`, for Minecraft `mc` when given, and how many
/// match in all.
pub async fn search(text: &str, mc: Option<&str>, sort: Sort, page: u32, limit: u32) -> Result<(Vec<Pack>, u64)> {
    let mut q = query(text, mc, sort);
    let count = q.clone();
    q.push(("limit", limit.to_string()));
    q.push(("page", (page + 1).to_string()));
    for scope in ["data.id", "data.categories", "data.display", "data.versions.supports", "meta.rawId", "meta.stats"] {
        q.push(("scope", scope.to_string()));
    }
    let list = async {
        let r = client().get(format!("{API}/packs")).query(&q).send().await?;
        if !r.status().is_success() {
            bail!("Smithed search failed: HTTP {}", r.status());
        }
        let raw: Vec<RawPack> = r.json().await?;
        Ok(raw.into_iter().filter_map(RawPack::into_pack).collect::<Vec<Pack>>())
    };
    let total = async {
        let r = client().get(format!("{API}/packs/count")).query(&count).send().await?;
        if !r.status().is_success() {
            bail!("Smithed search failed: HTTP {}", r.status());
        }
        Ok(r.text().await?.trim().parse::<u64>().unwrap_or(0))
    };
    let (list, total) = tokio::try_join!(list, total)?;
    let total = total.max(u64::from(page) * u64::from(limit) + list.len() as u64);
    Ok((list, total))
}

/// The address the pack's data pack for Minecraft `mc` is downloaded from, with the packs it needs inside.
fn download_url(id: &str, mc: &str) -> String {
    let mut url = reqwest::Url::parse(&format!("{API}/download")).expect("a fixed address");
    url.query_pairs_mut().append_pair("pack", id).append_pair("version", mc).append_pair("mode", "datapack");
    url.to_string()
}

/// Adds a pack to each of `worlds` (`saves/<world>/datapacks`) and records it in the instance's list, in place of
/// the copy that was there.
pub async fn install(instance: &mut Instance, pack: &Pack, worlds: &[String], progress: &Arc<Progress>) -> Result<()> {
    if worlds.is_empty() {
        bail!(
            "Data packs are added to a world, and {} has none yet. Play it once to make a world, then add the data pack again.",
            instance.name()
        );
    }
    let mc = instance.mc_version().to_string();
    if !pack.supports.is_empty() && !pack.supports.contains(&mc) {
        bail!("{} has no version for Minecraft {mc}", pack.name);
    }
    let file = pack.file_name();
    progress.stage(pack.name.clone());
    let first = instance.root.join("saves").join(&worlds[0]).join("datapacks").join(&file);
    install::download_all(vec![Job::new(download_url(&pack.id, &mc), &first)], progress, true)
        .await
        .map_err(|e| e.context(format!("Couldn't download {} from Smithed. Please try again.", pack.name)))?;
    check_zip(&first)?;
    let mut records = instance.mod_records();
    for world in worlds {
        let rel = format!("saves/{world}/datapacks");
        let dest = instance.root.join(&rel).join(&file);
        if dest != first {
            std::fs::create_dir_all(instance.root.join(&rel))?;
            std::fs::copy(&first, &dest)?;
        }
        records.retain(|m| !(m.kind == "datapack" && m.file == file && m.path.as_deref() == Some(rel.as_str())));
        let mut rec = ModRecord::new(&pack.name, &file, "datapack");
        rec.description = pack.summary.clone();
        rec.version = "Unknown".into();
        rec.optional = true;
        rec.user_added = true;
        rec.was_selected = true;
        rec.path = Some(rel);
        records.push(rec);
    }
    instance.set_mod_records(&records);
    instance.save()
}

/// Smithed answers a pack it can't build with an error page; that is not a data pack.
fn check_zip(path: &Path) -> Result<()> {
    let mut head = [0u8; 4];
    let ok = std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok()
        && head == *b"PK\x03\x04";
    if !ok {
        let _ = std::fs::remove_file(path);
        bail!("Smithed couldn't put that pack together. Try again later.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_results_are_read() {
        let json = r#"[{"displayName":"Myriad","id":"ssIS","data":{"id":"myriad","categories":["QoL"],
            "display":{"name":"Myriad","description":"More vanilla","icon":"https://x/logo.png","hidden":false},
            "versions":[{"supports":["1.21"]},{"supports":["1.21","1.21.1"]}]},
            "meta":{"rawId":"myriad","stats":{"downloads":{"total":32604}}}},
            {"displayName":"Hidden","id":"h","data":{"display":{"hidden":true}},"meta":{"rawId":"hidden"}}]"#;
        let raw: Vec<RawPack> = serde_json::from_str(json).unwrap();
        let packs: Vec<Pack> = raw.into_iter().filter_map(RawPack::into_pack).collect();
        assert_eq!(packs.len(), 1, "a hidden pack is left out");
        let p = &packs[0];
        assert_eq!((p.id.as_str(), p.name.as_str(), p.downloads), ("myriad", "Myriad", 32604));
        assert_eq!(p.supports, ["1.21", "1.21.1"]);
        assert_eq!(p.icon.as_deref(), Some("https://x/logo.png"));
        assert_eq!(p.page_url(), "https://smithed.net/packs/myriad");
    }

    #[test]
    fn the_file_name_is_safe() {
        let p = Pack { id: "gm4/../trees!".into(), ..Default::default() };
        assert_eq!(p.file_name(), "gm4____trees_.zip");
        assert!(crate::packutil::safe_file_name(&p.file_name()));
    }

    #[test]
    fn the_download_asks_for_the_data_pack_of_the_version() {
        assert_eq!(
            download_url("my pack", "1.21.1"),
            "https://api.smithed.dev/v2/download?pack=my+pack&version=1.21.1&mode=datapack"
        );
    }

    #[test]
    fn a_search_says_its_words_version_and_order() {
        let q = query(" trees ", Some("1.21"), Sort::Downloads);
        assert_eq!(
            q,
            vec![("search", "trees".to_string()), ("version", "1.21".to_string()), ("sort", "downloads".to_string())]
        );
        assert!(query("", None, Sort::Relevance).is_empty());
    }
}
