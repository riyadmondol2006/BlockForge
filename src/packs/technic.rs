//! Technic: search, modpack details, Solder builds and update checks. A Technic pack is one of two kinds: a plain
//! zip that is unpacked into the instance, or a Solder pack made of many small mod zips listed per build. Both are
//! installed by [`super::engine`].
//!
//! The Technic platform API is asked as the "atlauncher" build, the name ATLauncher registered with it.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use super::json::{DownloadKind, ModKind, PackMod};
use crate::http;
use crate::instances::Instance;
use crate::paths::Dirs;

pub const API_URL: &str = "https://api.technicpack.net";
/// The build name the Technic API knows this launcher family by.
pub const API_BUILD: &str = "atlauncher";
pub const PAGE_SIZE: usize = 20;

const CACHE_TTL: Duration = Duration::from_secs(600);

fn flex_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn flex_opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Null => None,
        other => Some(other.to_string()),
    })
}

/// A search or trending result.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Slim {
    #[serde(deserialize_with = "flex_string")]
    pub id: String,
    #[serde(deserialize_with = "super::json::nullable_string")]
    pub name: String,
    #[serde(deserialize_with = "super::json::nullable_string")]
    pub slug: String,
    pub url: Option<String>,
    pub icon_url: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct SearchResults {
    modpacks: Vec<Slim>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Asset {
    pub url: Option<String>,
    pub md5: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Feed {
    pub user: Option<String>,
    pub date: i64,
    pub content: Option<String>,
    pub avatar: Option<String>,
    pub url: Option<String>,
}

/// A modpack as the Technic API describes it. This is also what an instance keeps in `launcher.technicModpack`,
/// so unknown fields are carried along untouched.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Modpack {
    pub id: i64,
    /// The slug: the short name used in addresses.
    #[serde(deserialize_with = "super::json::nullable_string")]
    pub name: String,
    #[serde(deserialize_with = "super::json::nullable_string")]
    pub display_name: String,
    pub user: Option<String>,
    /// For plain-zip packs the address of the zip; `null` for Solder packs.
    pub url: Option<String>,
    pub platform_url: Option<String>,
    pub minecraft: Option<String>,
    pub ratings: i64,
    #[serde(alias = "installs")]
    pub downloads: i64,
    pub runs: i64,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub is_server: bool,
    pub is_official: bool,
    /// The newest version of a plain-zip pack.
    pub version: Option<String>,
    pub force_dir: bool,
    pub feed: Vec<Feed>,
    pub icon: Option<Asset>,
    pub logo: Option<Asset>,
    pub background: Option<Asset>,
    /// The Solder API address, `null` for plain-zip packs.
    pub solder: Option<String>,
    pub discord_server_id: Option<String>,
    pub server_pack_url: Option<String>,
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

impl Modpack {
    pub fn is_solder(&self) -> bool {
        self.solder.as_deref().is_some_and(|s| !s.is_empty())
    }

    pub fn title(&self) -> &str {
        if self.display_name.is_empty() { &self.name } else { &self.display_name }
    }

    pub fn icon_url(&self) -> Option<&str> {
        self.icon.as_ref().and_then(|a| a.url.as_deref()).filter(|u| !u.is_empty())
    }

    pub fn logo_url(&self) -> Option<&str> {
        self.logo.as_ref().and_then(|a| a.url.as_deref()).filter(|u| !u.is_empty())
    }
}

/// A Solder pack's list of builds.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SolderModpack {
    pub name: String,
    #[serde(alias = "display_name")]
    pub display_name: Option<String>,
    pub recommended: Option<String>,
    pub latest: Option<String>,
    pub builds: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SolderMod {
    pub name: String,
    pub version: String,
    pub md5: Option<String>,
    pub url: String,
    pub filesize: Option<i64>,
}

impl SolderMod {
    /// The mod as a pack mod: a direct download of a zip that unpacks into the instance (`mods/...`).
    pub fn to_pack_mod(&self) -> PackMod {
        let filename = self.url.rsplit('/').next().unwrap_or(&self.url).to_string();
        PackMod {
            name: self.name.clone(),
            version: self.version.clone(),
            url: self.url.clone(),
            file: filename,
            path: Some("mods/".into()),
            md5: self.md5.clone().filter(|m| !m.is_empty()),
            filesize: self.filesize.unwrap_or(0),
            download: Some(DownloadKind::Direct),
            kind: Some(ModKind::Mods),
            client: true,
            server: true,
            optional: false,
            ..Default::default()
        }
    }
}

/// One build of a Solder pack.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SolderManifest {
    #[serde(deserialize_with = "flex_opt_string")]
    pub minecraft: Option<String>,
    #[serde(deserialize_with = "flex_opt_string")]
    pub minecraft_md5: Option<String>,
    #[serde(deserialize_with = "flex_opt_string")]
    pub forge: Option<String>,
    #[serde(deserialize_with = "flex_opt_string")]
    pub java: Option<String>,
    #[serde(deserialize_with = "flex_opt_string")]
    pub memory: Option<String>,
    pub mods: Vec<SolderMod>,
}

/// A version a player can pick for a Technic pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    pub recommended: bool,
    pub latest: bool,
}

impl Build {
    pub fn label(&self) -> String {
        if self.recommended {
            format!("{} (Recommended)", self.version)
        } else if self.latest {
            format!("{} (Latest)", self.version)
        } else {
            self.version.clone()
        }
    }
}

fn cache(dirs: &Dirs) -> std::path::PathBuf {
    dirs.cache.join("http")
}

/// The Technic home page list.
pub async fn trending(dirs: &Dirs) -> Result<Vec<Slim>> {
    let r: SearchResults =
        http::get_json_cached(&format!("{API_URL}/trending?build={API_BUILD}"), &cache(dirs), CACHE_TTL).await?;
    Ok(r.modpacks)
}

pub async fn search(dirs: &Dirs, query: &str) -> Result<Vec<Slim>> {
    let q: String = url_encode(query.trim());
    let r: SearchResults =
        http::get_json_cached(&format!("{API_URL}/search?q={q}&build={API_BUILD}"), &cache(dirs), CACHE_TTL).await?;
    Ok(r.modpacks)
}

/// A modpack by its slug (the `name` field), as Technic describes it today.
pub async fn modpack(dirs: &Dirs, slug: &str) -> Result<Modpack> {
    http::get_json_cached(&format!("{API_URL}/modpack/{slug}?build={API_BUILD}"), &cache(dirs), CACHE_TTL)
        .await
        .with_context(|| format!("getting the Technic modpack {slug}"))
}

/// Same as [`modpack`] but never from the saved copy, and a missing pack is reported as `Ok(None)`. Used for
/// update checks, which must see the pack as it is right now.
pub async fn modpack_now(slug: &str) -> Result<Option<Modpack>> {
    let url = format!("{API_URL}/modpack/{slug}?build={API_BUILD}");
    let r = http::client().get(&url).send().await?;
    if r.status().as_u16() == 404 {
        return Ok(None);
    }
    if !r.status().is_success() {
        anyhow::bail!("{url}: HTTP {}", r.status());
    }
    Ok(Some(r.json().await.with_context(|| format!("reading the answer from {url}"))?))
}

fn normalize_solder(url: &str) -> &str {
    url.strip_suffix('/').unwrap_or(url)
}

pub async fn solder_modpack(dirs: &Dirs, solder: &str, slug: &str) -> Result<SolderModpack> {
    http::get_json_cached(&format!("{}/modpack/{slug}", normalize_solder(solder)), &cache(dirs), CACHE_TTL)
        .await
        .with_context(|| format!("getting the builds of {slug} from its Solder server"))
}

pub async fn solder_manifest(dirs: &Dirs, solder: &str, slug: &str, build: &str) -> Result<SolderManifest> {
    http::get_json_cached(&format!("{}/modpack/{slug}/{build}", normalize_solder(solder)), &cache(dirs), CACHE_TTL)
        .await
        .with_context(|| format!("getting build {build} of {slug} from its Solder server"))
}

/// The versions to offer for a pack, newest first. A Solder pack has one per build; a plain-zip pack has just the
/// current version.
pub async fn builds(dirs: &Dirs, pack: &Modpack) -> Result<Vec<Build>> {
    if let Some(solder) = pack.solder.as_deref().filter(|s| !s.is_empty()) {
        let sp = solder_modpack(dirs, solder, &pack.name).await?;
        return Ok(builds_from(&sp));
    }
    Ok(vec![Build { version: pack.version.clone().unwrap_or_default(), recommended: false, latest: false }])
}

pub fn builds_from(sp: &SolderModpack) -> Vec<Build> {
    let mut out: Vec<Build> = sp
        .builds
        .iter()
        .map(|b| Build {
            version: b.clone(),
            recommended: sp.recommended.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(b)),
            latest: sp.latest.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(b)),
        })
        .collect();
    out.reverse();
    out
}

/// Which build to start on in the picker: the recommended one, else the first.
pub fn default_build(builds: &[Build]) -> Option<usize> {
    builds.iter().position(|b| b.recommended).or(if builds.is_empty() { None } else { Some(0) })
}

/// Percent-encode a search term for the query part of an address.
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The memory a Solder build asks for in megabytes, when it names a number.
pub fn solder_memory(m: &SolderManifest) -> Option<i64> {
    m.memory.as_deref().and_then(|s| s.trim().parse::<i64>().ok()).filter(|n| *n > 0)
}

// ---------------------------------------------------------------------------------------------------------
// updates

/// What the Technic side knows is newest for an installed pack.
#[derive(Clone, Debug)]
pub enum Latest {
    /// A plain-zip pack: its current version and the pack as it is now.
    Zip { version: Option<String>, pack: Box<Modpack> },
    /// A Solder pack: the newest build.
    Solder { latest: Option<String>, pack: Box<Modpack> },
    /// The pack no longer exists on Technic.
    Gone,
}

/// The pack recorded in an instance, if it came from Technic.
pub fn installed_pack(instance: &Instance) -> Option<Modpack> {
    serde_json::from_value(instance.file.launcher.extras.get("technicModpack")?.clone()).ok()
}

/// Ask Technic what is newest for an instance's pack.
pub async fn latest_for(instance: &Instance) -> Result<Latest> {
    let Some(recorded) = installed_pack(instance) else {
        anyhow::bail!("{} didn't come from Technic", instance.name())
    };
    let Some(now) = modpack_now(&recorded.name).await? else { return Ok(Latest::Gone) };
    if now.is_solder() && recorded.is_solder() {
        let solder = now.solder.clone().unwrap_or_default();
        let sp: SolderModpack = http::get_json(&format!("{}/modpack/{}", normalize_solder(&solder), now.name)).await?;
        return Ok(Latest::Solder { latest: sp.latest, pack: Box::new(now) });
    }
    Ok(Latest::Zip { version: now.version.clone(), pack: Box::new(now) })
}

/// The newest version when it differs from the installed one (`None` when up to date). Matches the Java
/// edition's tests: a Solder pack compares its `latest` build, a plain-zip pack its `version`.
pub fn update_version(instance: &Instance, latest: &Latest) -> Option<String> {
    let installed = &instance.file.launcher.version;
    match latest {
        Latest::Solder { latest: Some(l), .. } if !l.is_empty() && l != installed => Some(l.clone()),
        Latest::Zip { version: Some(v), .. } if !v.is_empty() && v != installed => Some(v.clone()),
        _ => None,
    }
}

/// Whether the player wants updates looked for (`checkForUpdates` on the instance, on unless turned off).
pub fn checks_for_updates(instance: &Instance) -> bool {
    instance.file.launcher.extras.get("checkForUpdates").and_then(Value::as_bool).unwrap_or(true)
}

/// A pack that has gone from Technic is not asked about again: the instance's update checks are turned off.
pub fn stop_checking(instance: &mut Instance) -> Result<()> {
    instance.file.launcher.extras.insert("checkForUpdates".into(), Value::Bool(false));
    instance.save()
}

#[cfg(test)]
mod tests {
    use super::*;

    // from https://api.technicpack.net/trending?build=atlauncher
    const TRENDING: &str = r#"{"modpacks":[{"id":"533075","name":"Yogbox 2.0","slug":"yogbox-20","url":"https://www.technicpack.net/modpack/yogbox-20.533075","iconUrl":"https://cdn.technicpack.net/platform2/pack-icons/533075.png?1779844078"},{"id":"1643990","name":"After Humans (Updated)","slug":"after-humans-updated","url":"https://www.technicpack.net/modpack/after-humans-updated.1643990","iconUrl":"https://cdn.technicpack.net/platform2/pack-icons/1643990.png?1717775129"}]}"#;

    // from https://api.technicpack.net/modpack/tekkitlite?build=atlauncher (feed shortened)
    const MODPACK: &str = r#"{"id":552675,"name":"tekkitlite","displayName":"Tekkit Lite","user":"sct","url":null,"platformUrl":"https://www.technicpack.net/modpack/tekkitlite.552675","minecraft":"1.4.7","ratings":0,"installs":1069399,"runs":9711616,"description":"The official Tekkit Lite modpack","tags":null,"isServer":false,"isOfficial":true,"version":"1.0","feed":[{"user":"Pyker","date":1707659325,"content":"Lumy Skin Patch has been updated","avatar":"https://gravatar.com/avatar/6c89","url":"https://www.technicpack.net/status/4962138"}],"icon":{"url":"https://cdn.technicpack.net/platform2/pack-icons/552675.png?1631992514"},"logo":{"url":"https://cdn.technicpack.net/platform2/pack-logos/552675.png?1631992514"},"background":{"url":"https://cdn.technicpack.net/platform2/pack-backgrounds/552675.jpg?1631992514"},"solder":"https://solder.technicpack.net/api/","brandNew":"kept"}"#;

    // from https://solder.technicpack.net/api/modpack/tekkitlite and .../tekkitlite/0.6.6 (shortened)
    const SOLDER_PACK: &str = r#"{"name":"tekkitlite","display_name":"Tekkit Lite","recommended":"0.6.6","latest":"0.6.6","builds":["0.5.9","0.5.8.1","0.6.5","0.6.6"]}"#;
    const SOLDER_BUILD: &str = r#"{"minecraft":"1.4.7","forge":null,"java":"","memory":0,"mods":[{"name":"applied-energistics","version":"v9.i","md5":"304a959df8c60a7fb4fc0b3a3c446fe3","url":"https://mirror-mods.technicpack.net/mods/applied-energistics/applied-energistics-v9.i.zip","filesize":528077},{"name":"nomd5","version":"1","md5":null,"url":"https://x.example/m/nomd5-1.zip"}]}"#;

    #[test]
    fn search_results_parse() {
        let r: SearchResults = serde_json::from_str(TRENDING).unwrap();
        assert_eq!(r.modpacks.len(), 2);
        assert_eq!(r.modpacks[0].slug, "yogbox-20");
        assert_eq!(r.modpacks[0].id, "533075");
        assert!(r.modpacks[1].icon_url.as_deref().unwrap().contains("pack-icons"));
    }

    #[test]
    fn modpack_details_parse_and_keep_unknown_fields() {
        let m: Modpack = serde_json::from_str(MODPACK).unwrap();
        assert_eq!(m.id, 552675);
        assert_eq!(m.name, "tekkitlite");
        assert_eq!(m.title(), "Tekkit Lite");
        assert!(m.is_solder());
        assert_eq!(m.minecraft.as_deref(), Some("1.4.7"));
        assert_eq!(m.downloads, 1069399, "the API calls it installs");
        assert_eq!(m.version.as_deref(), Some("1.0"));
        assert!(m.url.is_none());
        assert_eq!(m.feed[0].user.as_deref(), Some("Pyker"));
        assert_eq!(m.logo_url(), Some("https://cdn.technicpack.net/platform2/pack-logos/552675.png?1631992514"));
        // what an instance saves can be read back by both editions
        let back = serde_json::to_value(&m).unwrap();
        assert_eq!(back["displayName"], "Tekkit Lite");
        assert_eq!(back["brandNew"], "kept");
        assert_eq!(back["solder"], "https://solder.technicpack.net/api/");
    }

    #[test]
    fn solder_builds_are_listed_newest_first_with_labels() {
        let sp: SolderModpack = serde_json::from_str(SOLDER_PACK).unwrap();
        let builds = builds_from(&sp);
        assert_eq!(builds[0].version, "0.6.6");
        assert_eq!(builds[0].label(), "0.6.6 (Recommended)");
        assert_eq!(builds[3].version, "0.5.9");
        assert_eq!(default_build(&builds), Some(0));
        assert_eq!(default_build(&[]), None);
    }

    #[test]
    fn solder_builds_turn_into_direct_mod_downloads() {
        let m: SolderManifest = serde_json::from_str(SOLDER_BUILD).unwrap();
        assert_eq!(m.minecraft.as_deref(), Some("1.4.7"));
        assert_eq!(m.forge, None);
        assert_eq!(solder_memory(&m), None);
        let first = m.mods[0].to_pack_mod();
        assert_eq!(first.file, "applied-energistics-v9.i.zip");
        assert_eq!(first.path.as_deref(), Some("mods/"));
        assert_eq!(first.md5.as_deref(), Some("304a959df8c60a7fb4fc0b3a3c446fe3"));
        assert_eq!(first.filesize, 528077);
        assert_eq!(first.download, Some(DownloadKind::Direct));
        assert_eq!(
            first.download_url(),
            "https://mirror-mods.technicpack.net/mods/applied-energistics/applied-energistics-v9.i.zip"
        );
        assert!(!first.optional);
        let second = m.mods[1].to_pack_mod();
        assert!(second.md5.is_none());
        let with_memory: SolderManifest =
            serde_json::from_str(r#"{"minecraft":"1.7.10","memory":"4096","mods":[]}"#).unwrap();
        assert_eq!(solder_memory(&with_memory), Some(4096));
    }

    #[test]
    fn search_terms_are_encoded() {
        assert_eq!(url_encode("sky factory"), "sky%20factory");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(url_encode("tech-pack_1.0"), "tech-pack_1.0");
    }
}
