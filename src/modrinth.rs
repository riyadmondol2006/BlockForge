//! Modrinth: searching, installing mods (with the mods they need) and importing `.mrpack` modpacks.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::http::{self, client};
use crate::install::{self, Job, Progress};
use crate::instances::{self, Instance};
use crate::loaders::Loader;
use crate::paths::Dirs;

const API: &str = "https://api.modrinth.com/v2";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Mod,
    Modpack,
    ResourcePack,
    Shader,
    DataPack,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Mod, Kind::Modpack, Kind::ResourcePack, Kind::Shader, Kind::DataPack];

    pub fn api_name(self) -> &'static str {
        match self {
            Kind::Mod => "mod",
            Kind::Modpack => "modpack",
            Kind::ResourcePack => "resourcepack",
            Kind::Shader => "shader",
            Kind::DataPack => "datapack",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Mod => "Mods",
            Kind::Modpack => "Modpacks",
            Kind::ResourcePack => "Resource packs",
            Kind::Shader => "Shaders",
            Kind::DataPack => "Data packs",
        }
    }

    /// The instance folder this kind of file goes in.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Mod => "mods",
            Kind::Modpack => "mods",
            Kind::ResourcePack => "resourcepacks",
            Kind::Shader => "shaderpacks",
            Kind::DataPack => "datapacks",
        }
    }

    pub fn from_api(s: &str) -> Kind {
        match s {
            "modpack" => Kind::Modpack,
            "resourcepack" => Kind::ResourcePack,
            "shader" => Kind::Shader,
            "datapack" => Kind::DataPack,
            _ => Kind::Mod,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct Hit {
    pub project_id: String,
    #[serde(default)]
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub project_type: String,
    #[serde(default)]
    pub categories: Vec<String>,
    /// The Minecraft versions it has files for.
    #[serde(default)]
    pub versions: Vec<String>,
}

#[derive(Deserialize, Debug)]
struct SearchAnswer {
    hits: Vec<Hit>,
    #[serde(default)]
    total_hits: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Relevance,
    Downloads,
    Updated,
}

impl Sort {
    fn api(self) -> &'static str {
        match self {
            Sort::Relevance => "relevance",
            Sort::Downloads => "downloads",
            Sort::Updated => "updated",
        }
    }
}

pub async fn search(
    query: &str,
    kind: Kind,
    mc: Option<&str>,
    loader: Loader,
    sort: Sort,
    offset: u32,
    limit: u32,
) -> Result<(Vec<Hit>, u64)> {
    let loaders: Vec<String> = loader.modrinth_name().map(str::to_string).into_iter().collect();
    search_for_loaders(query, kind, mc, &loaders, sort, offset, limit).await
}

/// Where a mod has to run, for the environment filter.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side {
    #[default]
    Any,
    /// Runs in the player's game and needs nothing on the server (a minimap, a shader loader).
    Client,
    /// Runs on a server and needs nothing in the players' games (a backup mod, a permissions mod).
    Server,
}

/// What a search is narrowed to besides the kind, the Minecraft version and the loader.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Narrow {
    /// Modrinth's category names (`optimization`, `16x`); a project needs all of them.
    pub categories: Vec<String>,
    pub side: Side,
}

/// The search filters: the kind, the Minecraft version, any one of `loaders` (a Quilt instance also runs Fabric
/// mods, so both are asked for), every category in `narrow` and where the mod runs. Loaders only filter mods and
/// modpacks.
fn search_facets(kind: Kind, mc: Option<&str>, loaders: &[String], narrow: &Narrow) -> Vec<Vec<String>> {
    let mut facets: Vec<Vec<String>> = vec![vec![format!("project_type:{}", kind.api_name())]];
    if let Some(mc) = mc.filter(|m| !m.is_empty()) {
        facets.push(vec![format!("versions:{mc}")]);
    }
    if (kind == Kind::Mod || kind == Kind::Modpack) && !loaders.is_empty() {
        facets.push(loaders.iter().map(|l| format!("categories:{l}")).collect());
    }
    // one group each: a project has to be in every category chosen, as on Modrinth's own site
    for c in narrow.categories.iter().filter(|c| !c.is_empty()) {
        facets.push(vec![format!("categories:{c}")]);
    }
    // "client" is a mod the game runs that the server can do without, "server" the other way round
    let side = |s: &str, values: [&str; 2]| values.iter().map(|v| format!("{s}_side:{v}")).collect::<Vec<_>>();
    match narrow.side {
        Side::Any => {}
        Side::Client => {
            facets.push(side("client", ["required", "optional"]));
            facets.push(side("server", ["optional", "unsupported"]));
        }
        Side::Server => {
            facets.push(side("server", ["required", "optional"]));
            facets.push(side("client", ["optional", "unsupported"]));
        }
    }
    facets
}

/// Like [`search`], for an instance whose mods may be made for any of several loaders.
pub async fn search_for_loaders(
    query: &str,
    kind: Kind,
    mc: Option<&str>,
    loaders: &[String],
    sort: Sort,
    offset: u32,
    limit: u32,
) -> Result<(Vec<Hit>, u64)> {
    search_narrowed(query, kind, mc, loaders, &Narrow::default(), sort, offset, limit).await
}

/// Like [`search_for_loaders`], narrowed further to categories and where the mod runs.
#[allow(clippy::too_many_arguments)]
pub async fn search_narrowed(
    query: &str,
    kind: Kind,
    mc: Option<&str>,
    loaders: &[String],
    narrow: &Narrow,
    sort: Sort,
    offset: u32,
    limit: u32,
) -> Result<(Vec<Hit>, u64)> {
    let facets = search_facets(kind, mc, loaders, narrow);
    let r = client()
        .get(format!("{API}/search"))
        .query(&[
            ("query", query.to_string()),
            ("facets", serde_json::to_string(&facets)?),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
            ("index", sort.api().to_string()),
        ])
        .send()
        .await?;
    if !r.status().is_success() {
        bail!("Modrinth search failed: HTTP {}", r.status());
    }
    let mut answer: SearchAnswer = r.json().await?;
    if kind == Kind::DataPack {
        // Modrinth files a project that is both a mod and a data pack as a mod; here it was asked for as a data pack
        for hit in &mut answer.hits {
            hit.project_type = "datapack".into();
        }
    }
    Ok((answer.hits, answer.total_hits))
}

#[derive(Deserialize, Clone, Debug)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_number: String,
    #[serde(default)]
    pub version_type: String,
    #[serde(default)]
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub hashes: BTreeMap<String, String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Dependency {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub version_id: Option<String>,
    #[serde(default)]
    pub dependency_type: String,
}

impl Version {
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }
}

/// Versions of a project that run on `mc` with `loader`, newest first, releases before betas.
pub async fn versions(project: &str, mc: Option<&str>, loader: Loader, kind: Kind) -> Result<Vec<Version>> {
    let mut req = client().get(format!("{API}/project/{project}/version"));
    if let Some(mc) = mc.filter(|m| !m.is_empty()) {
        req = req.query(&[("game_versions", serde_json::to_string(&[mc])?)]);
    }
    if kind == Kind::Mod {
        if let Some(l) = loader.modrinth_name() {
            req = req.query(&[("loaders", serde_json::to_string(&[l])?)]);
        }
    } else if kind == Kind::DataPack {
        // a project that is also a mod has jars too; only its data pack zips are wanted
        req = req.query(&[("loaders", serde_json::to_string(&["datapack"])?)]);
    }
    let r = req.send().await?;
    if !r.status().is_success() {
        bail!("Modrinth: HTTP {} for {project}", r.status());
    }
    let mut list: Vec<Version> = r.json().await?;
    // stable sort keeps Modrinth's newest-first order inside each group
    list.sort_by_key(|v| match v.version_type.as_str() {
        "release" => 0,
        "beta" => 1,
        _ => 2,
    });
    Ok(list)
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Installed {
    /// project id -> file name in the instance
    pub files: BTreeMap<String, String>,
}

fn installed_file(instance: &Instance) -> std::path::PathBuf {
    instance.root.join("blockforge-modrinth.json")
}

pub fn load_installed(instance: &Instance) -> Installed {
    std::fs::read(installed_file(instance)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Put `project` into `instance`, along with the projects it requires. Returns the files that were added.
pub async fn install_into(
    instance: &Instance,
    project: &str,
    kind: Kind,
    progress: &Arc<Progress>,
) -> Result<Vec<String>> {
    let mc = instance.mc_version().to_string();
    let loader = instance.loader().and_then(|(k, _)| Loader::parse(k)).unwrap_or(Loader::Vanilla);
    let mut installed = load_installed(instance);
    let mut done: HashSet<String> = HashSet::new();
    let mut queue: Vec<(String, Kind)> = vec![(project.to_string(), kind)];
    let mut added = Vec::new();
    let mut jobs = Vec::new();
    let mut replaced = Vec::new();

    while let Some((id, k)) = queue.pop() {
        if !done.insert(id.clone()) || installed.files.contains_key(&id) && id != project {
            continue;
        }
        progress.stage(format!("Finding a version of {id}"));
        let list = versions(&id, Some(&mc), loader, k).await?;
        let Some(version) = list.first() else {
            if id == project {
                bail!(
                    "no version of this project works with Minecraft {mc}{}",
                    if loader == Loader::Vanilla { String::new() } else { format!(" and {}", loader.label()) }
                );
            }
            continue; // a dependency with no matching version: leave it to the player
        };
        let Some(file) = version.primary_file() else { continue };
        if !crate::packutil::safe_file_name(&file.filename) {
            bail!("{id} has a file name that isn't allowed: {}", file.filename);
        }
        let dest = instance.root.join(k.folder()).join(&file.filename);
        let mut job = Job::new(&file.url, &dest);
        job.sha1 = file.hashes.get("sha1").cloned();
        job.size = Some(file.size).filter(|s| *s > 0);
        jobs.push(job);
        // an older file of the same project is replaced, once the new one is downloaded
        if let Some(old) = installed.files.get(&id) {
            if old != &file.filename && crate::packutil::safe_file_name(old) {
                replaced.push(instance.root.join(k.folder()).join(old));
            }
        }
        installed.files.insert(id.clone(), file.filename.clone());
        added.push(file.filename.clone());
        for dep in &version.dependencies {
            if dep.dependency_type == "required" {
                if let Some(p) = &dep.project_id {
                    queue.push((p.clone(), Kind::Mod));
                }
            }
        }
    }
    install::download_all(jobs, progress, true).await?;
    for old in replaced {
        let _ = std::fs::remove_file(old);
    }
    crate::config::write_json_atomic(&installed_file(instance), &installed)?;
    Ok(added)
}

// ---------------------------------------------------------------------------------------------------------
// .mrpack modpacks

#[derive(Deserialize)]
struct PackIndex {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "versionId")]
    version_id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    files: Vec<PackFile>,
    dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct PackFile {
    path: String,
    hashes: BTreeMap<String, String>,
    #[serde(default)]
    env: Option<BTreeMap<String, String>>,
    downloads: Vec<String>,
    #[serde(default, rename = "fileSize")]
    file_size: u64,
}

/// Create an instance from a Modrinth modpack file.
pub async fn import_mrpack(dirs: &Dirs, pack: &Path, progress: &Arc<Progress>) -> Result<Instance> {
    progress.stage("Reading the modpack");
    let file = std::fs::File::open(pack).with_context(|| format!("opening {}", pack.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("that isn't a modpack file")?;
    let index: PackIndex = {
        let mut entry =
            archive.by_name("modrinth.index.json").context("not a Modrinth modpack (no modrinth.index.json)")?;
        let text = crate::packutil::zip_entry_text(&mut entry)?;
        serde_json::from_str(&text).context("reading modrinth.index.json")?
    };
    let mc = index.dependencies.get("minecraft").context("the modpack doesn't say which Minecraft it needs")?.clone();
    let (loader, loader_version) = if let Some(v) = index.dependencies.get("fabric-loader") {
        (Loader::Fabric, Some(v.clone()))
    } else if let Some(v) = index.dependencies.get("quilt-loader") {
        (Loader::Quilt, Some(v.clone()))
    } else if let Some(v) = index.dependencies.get("neoforge") {
        (Loader::NeoForge, Some(v.clone()))
    } else if let Some(v) = index.dependencies.get("forge") {
        (Loader::Forge, Some(format!("{mc}-{v}")))
    } else {
        (Loader::Vanilla, None)
    };

    // every path is checked before anything is made, so a bad pack is refused without leaving an instance behind
    let mut planned = Vec::new();
    for f in &index.files {
        // files meant only for servers are left out
        if f.env.as_ref().and_then(|e| e.get("client")).is_some_and(|c| c == "unsupported") {
            continue;
        }
        // `safe_relative` also refuses what Windows would read as a place on the drive (`/Users/x`, `C:x`), which
        // `is_absolute` alone lets through
        let rel = crate::packutil::safe_relative(&f.path)
            .with_context(|| format!("the modpack has a file path that points outside the instance: {}", f.path))?;
        let Some(url) = f.downloads.first() else { continue };
        planned.push((rel, url.clone(), f.hashes.get("sha1").cloned(), Some(f.file_size).filter(|s| *s > 0)));
    }

    let name = if index.name.is_empty() { "Modpack".to_string() } else { index.name.clone() };
    let mut instance = instances::create(dirs, &name, &mc, loader, loader_version.as_deref(), progress).await?;
    // a failed download half way through a 300-mod pack must not leave an instance that looks whole
    match fill_mrpack(&mut instance, &index, planned, &mut archive, &name, progress).await {
        Ok(()) => Ok(instance),
        Err(e) => {
            let _ = instances::delete(&instance);
            Err(e)
        }
    }
}

/// The part of [`import_mrpack`] after the instance exists: the pack's downloads and its own files.
async fn fill_mrpack(
    instance: &mut Instance,
    index: &PackIndex,
    planned: Vec<(std::path::PathBuf, String, Option<String>, Option<u64>)>,
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    progress: &Arc<Progress>,
) -> Result<()> {
    instance.file.launcher.pack = name.to_string();
    instance.file.launcher.vanilla_instance = false;
    if let Some(summary) = &index.summary {
        instance.file.launcher.description = summary.clone();
    }
    if !index.version_id.is_empty() {
        instance.file.launcher.version = index.version_id.clone();
    }

    let mut jobs = Vec::new();
    for (rel, url, sha1, size) in planned {
        let mut job = Job::new(&url, instance.root.join(rel));
        job.sha1 = sha1;
        job.size = size;
        job.keep_on_mismatch = true;
        jobs.push(job);
    }
    progress.stage(format!("Downloading {} files from the modpack", jobs.len()));
    install::download_all(jobs, progress, true).await?;

    progress.stage("Copying the modpack's own files");
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Ok(name) = entry.name().map(|n| n.to_string()) else { continue };
        let rel = name.strip_prefix("overrides/").or_else(|| name.strip_prefix("client-overrides/"));
        let Some(rel) = rel.filter(|r| !r.is_empty()) else { continue };
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = crate::packutil::safe_relative(rel) else { continue };
        let out = instance.root.join(rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::File::create(&out)?;
        crate::packutil::copy_zip_entry(&mut entry, &mut f).with_context(|| format!("unpacking {name}"))?;
    }
    instance.save()?;
    Ok(())
}

/// Download the newest version of a Modrinth modpack and import it.
pub async fn install_modpack(dirs: &Dirs, project: &str, progress: &Arc<Progress>) -> Result<Instance> {
    progress.stage("Finding the modpack");
    let list = versions(project, None, Loader::Vanilla, Kind::Modpack).await?;
    let version = list.first().context("this modpack has no versions")?;
    let file = version.primary_file().context("this modpack version has no file")?;
    let temp = dirs.temp.join(format!("pack-{}.mrpack", uuid::Uuid::new_v4().simple()));
    let mut job = Job::new(&file.url, &temp);
    job.sha1 = file.hashes.get("sha1").cloned();
    install::download_all(vec![job], progress, true).await?;
    let result = import_mrpack(dirs, &temp, progress).await;
    let _ = std::fs::remove_file(&temp);
    result
}

pub fn sha1_of_file(path: &Path) -> Result<String> {
    Ok(http::sha1_hex(&std::fs::read(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quilt_instance_also_finds_fabric_mods() {
        let loaders = vec!["quilt".to_string(), "fabric".to_string()];
        let none = Narrow::default();
        let f = search_facets(Kind::Mod, Some("1.21.1"), &loaders, &none);
        assert_eq!(f[0], vec!["project_type:mod"]);
        assert_eq!(f[1], vec!["versions:1.21.1"]);
        // one group: any of the loaders matches
        assert_eq!(f[2], vec!["categories:quilt", "categories:fabric"]);
        // resource packs don't have a loader
        assert_eq!(search_facets(Kind::ResourcePack, Some("1.21.1"), &loaders, &none).len(), 2);
        assert_eq!(search_facets(Kind::Mod, None, &[], &none).len(), 1);
    }

    #[test]
    fn every_category_chosen_is_needed() {
        let narrow = Narrow { categories: vec!["optimization".into(), "utility".into()], side: Side::Any };
        let f = search_facets(Kind::Mod, None, &["fabric".into()], &narrow);
        // the loader is one group with its alternatives; each category is a group of its own, so all must match
        assert_eq!(f[1], vec!["categories:fabric"]);
        assert_eq!(f[2], vec!["categories:optimization"]);
        assert_eq!(f[3], vec!["categories:utility"]);
        assert_eq!(f.len(), 4);
        // resource packs keep their categories but have no loader
        let f = search_facets(Kind::ResourcePack, Some("1.20.1"), &["fabric".into()], &narrow);
        assert_eq!(f.len(), 4);
        assert_eq!(f[2], vec!["categories:optimization"]);
    }

    #[test]
    fn the_environment_asks_for_both_sides() {
        let client = Narrow { side: Side::Client, ..Default::default() };
        let f = search_facets(Kind::Mod, None, &[], &client);
        assert_eq!(f[1], vec!["client_side:required", "client_side:optional"]);
        assert_eq!(f[2], vec!["server_side:optional", "server_side:unsupported"]);
        let server = Narrow { side: Side::Server, ..Default::default() };
        let f = search_facets(Kind::Mod, None, &[], &server);
        assert_eq!(f[1], vec!["server_side:required", "server_side:optional"]);
        assert_eq!(f[2], vec!["client_side:optional", "client_side:unsupported"]);
    }
}

#[cfg(test)]
mod mrpack_tests {
    use super::*;
    use std::io::Write;

    fn mrpack(path: &Path, index: &str, extra: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("modrinth.index.json", o).unwrap();
        z.write_all(index.as_bytes()).unwrap();
        for (name, body) in extra {
            z.start_file(*name, o).unwrap();
            z.write_all(body).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn a_pack_with_an_escaping_path_is_refused_before_any_instance_is_made() {
        let base = crate::testutil::scratch("mrpack-evil");
        let dirs = Dirs::at(base.join("data"));
        dirs.ensure().unwrap();
        for path in ["../../evil.jar", "/etc/evil", "mods/../../evil.jar", "\\\\server\\share\\x", "..\\x"] {
            let pack = base.join("evil.mrpack");
            let index = serde_json::json!({
                "name": "Evil", "dependencies": {"minecraft": "1.20.1"},
                "files": [{"path": path, "hashes": {}, "downloads": ["http://127.0.0.1:9/x"], "fileSize": 1}]
            });
            mrpack(&pack, &index.to_string(), &[]);
            let err = crate::testutil::runtime().block_on(import_mrpack(&dirs, &pack, &Progress::new()));
            assert!(err.is_err(), "{path}");
            assert!(std::fs::read_dir(&dirs.instances).unwrap().next().is_none(), "{path}: no instance left");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn broken_pack_files_give_errors_not_crashes() {
        let base = crate::testutil::scratch("mrpack-broken");
        let dirs = Dirs::at(base.join("data"));
        dirs.ensure().unwrap();
        let rt = crate::testutil::runtime();
        let pack = base.join("p.mrpack");
        // not a zip
        std::fs::write(&pack, b"this is not a zip").unwrap();
        assert!(rt.block_on(import_mrpack(&dirs, &pack, &Progress::new())).is_err());
        // a zip without the index, with an index that isn't JSON, without a Minecraft version
        for index in ["{", "[]", r#"{"name":"x","dependencies":{}}"#, r#"{"files":"nope","dependencies":{}}"#] {
            mrpack(&pack, index, &[]);
            assert!(rt.block_on(import_mrpack(&dirs, &pack, &Progress::new())).is_err(), "{index}");
        }
        let mut z = zip::ZipWriter::new(std::fs::File::create(&pack).unwrap());
        z.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.finish().unwrap();
        assert!(rt.block_on(import_mrpack(&dirs, &pack, &Progress::new())).is_err());
        // a file that doesn't exist
        assert!(rt.block_on(import_mrpack(&dirs, &base.join("missing.mrpack"), &Progress::new())).is_err());
        assert!(std::fs::read_dir(&dirs.instances).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
