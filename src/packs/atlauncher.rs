//! ATLauncher's packs: the list (`packsnew.json`), who may see which pack (`users.json`), pack codes, and the
//! checks the Java edition makes before it installs or updates one. Installing itself is in [`super::engine`].
//!
//! The launcher API keeps its name "ATLauncher": these are ATLauncher's servers and they answer to that name.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::json::{JavaRequirement, PackJson};
use super::{DOWNLOAD_SERVER, safe_name};
use crate::http;
use crate::instances::Instance;
use crate::paths::Dirs;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PackType {
    #[default]
    Public,
    Private,
    Semipublic,
}

/// One version of a pack in the list. A `hash` makes it a development version (only testers see those).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PackVersion {
    pub version: String,
    /// The Minecraft version id this pack version is for.
    pub minecraft: String,
    pub hash: Option<String>,
    pub can_update: bool,
    pub is_recommended: bool,
    pub has_loader: bool,
    pub has_choosable_loader: bool,
    pub loader_type: Option<String>,
}

impl Default for PackVersion {
    fn default() -> Self {
        PackVersion {
            version: String::new(),
            minecraft: String::new(),
            hash: None,
            can_update: true,
            is_recommended: true,
            has_loader: false,
            has_choosable_loader: false,
            loader_type: None,
        }
    }
}

impl PackVersion {
    /// Development versions carry a hash.
    pub fn is_dev(&self) -> bool {
        self.hash.is_some()
    }

    pub fn safe_version(&self) -> String {
        safe_name(&self.version)
    }

    pub fn version_matches(&self, version: &str) -> bool {
        self.version.eq_ignore_ascii_case(version)
    }

    pub fn hash_matches(&self, hash: &str) -> bool {
        self.is_dev() && self.hash.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(hash))
    }

    /// How the version reads in a list: the pack's version and, when it differs from it, the Minecraft version.
    /// `mc_kind` is what the Minecraft manifest calls that version (`release`, `snapshot`, ...), `None` when the
    /// manifest does not know it.
    pub fn label(&self, mc_kind: Option<&str>) -> String {
        let snapshot = matches!(mc_kind, Some("snapshot"));
        if mc_kind.is_none()
            || self.minecraft.is_empty()
            || (self.minecraft.eq_ignore_ascii_case(&self.version) && !snapshot)
        {
            return self.version.clone();
        }
        if self.minecraft.eq_ignore_ascii_case(&self.version) {
            return format!("{} (Snapshot)", self.version);
        }
        if snapshot {
            return format!("{} ({}) (Snapshot)", self.version, self.minecraft);
        }
        format!("{} ({})", self.version, self.minecraft)
    }
}

/// A pack from `packsnew.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Pack {
    pub id: i64,
    pub external_id: i64,
    pub vanilla_instance: bool,
    pub position: i64,
    #[serde(deserialize_with = "super::json::nullable_string")]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: PackType,
    /// For semi-public packs: the MD5 of the code a player types in.
    pub code: Option<String>,
    pub versions: Vec<PackVersion>,
    pub dev_versions: Vec<PackVersion>,
    pub create_server: bool,
    pub logging: bool,
    pub featured: bool,
    pub system: bool,
    pub has_discord_image: bool,
    #[serde(deserialize_with = "super::json::nullable_string")]
    pub description: String,
    #[serde(rename = "discordInviteURL")]
    pub discord_invite_url: Option<String>,
    #[serde(rename = "supportURL")]
    pub support_url: Option<String>,
    #[serde(rename = "websiteURL")]
    pub website_url: Option<String>,
    /// Filled in from `users.json`.
    #[serde(skip)]
    pub testers: Vec<String>,
    #[serde(skip)]
    pub allowed_players: Vec<String>,
}

/// Who is asking and which pack codes they have added; decides which packs they can see and install.
#[derive(Clone, Debug, Default)]
pub struct UserCtx {
    /// The selected Minecraft account's name, if any.
    pub username: Option<String>,
    /// Pack codes the player typed in before (`addedPacks` in the settings).
    pub added_codes: Vec<String>,
}

impl UserCtx {
    /// Read from the shared settings (`addedPacks` is a list of codes in the Java edition's settings file).
    pub fn from_settings(settings: &crate::config::Settings, username: Option<&str>) -> UserCtx {
        UserCtx {
            username: username.map(str::to_string),
            added_codes: settings
                .extras
                .get("addedPacks")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
        }
    }
}

fn md5_hex(text: &str) -> String {
    use md5::{Digest, Md5};
    hex::encode(Md5::digest(text.as_bytes()))
}

impl Pack {
    /// Letters and digits only: the folder name on the servers and the image file name.
    pub fn safe_name(&self) -> String {
        safe_name(&self.name)
    }

    pub fn is_public(&self) -> bool {
        self.kind == PackType::Public
    }

    pub fn is_semi_public(&self) -> bool {
        self.kind == PackType::Semipublic
    }

    pub fn is_private(&self) -> bool {
        self.kind == PackType::Private
    }

    pub fn has_versions(&self) -> bool {
        !self.versions.is_empty()
    }

    pub fn has_dev_versions(&self) -> bool {
        !self.dev_versions.is_empty()
    }

    pub fn latest_version(&self) -> Option<&PackVersion> {
        self.versions.first()
    }

    pub fn latest_dev_version(&self) -> Option<&PackVersion> {
        self.dev_versions.first()
    }

    pub fn version_by_name(&self, name: &str) -> Option<&PackVersion> {
        self.versions.iter().find(|v| v.version_matches(name))
    }

    pub fn dev_version_by_name(&self, name: &str) -> Option<&PackVersion> {
        self.dev_versions.iter().find(|v| v.version_matches(name))
    }

    /// The latest version cannot be updated to (the pack author turned updates off or has not recommended it).
    pub fn latest_version_no_update(&self) -> bool {
        match self.latest_version() {
            None => false,
            Some(v) => !v.can_update || !v.is_recommended,
        }
    }

    pub fn is_tester(&self, user: &UserCtx) -> bool {
        match &user.username {
            Some(name) => self.testers.iter().any(|t| t.eq_ignore_ascii_case(name)),
            None => false,
        }
    }

    pub fn is_allowed_player(&self, user: &UserCtx) -> bool {
        if !self.is_private() {
            return true;
        }
        match &user.username {
            Some(name) => self.allowed_players.iter().any(|p| p.eq_ignore_ascii_case(name)),
            None => false,
        }
    }

    /// Whether this semi-public pack's code has been added by the player.
    pub fn code_added(&self, user: &UserCtx) -> bool {
        let Some(code) = self.code.as_deref() else { return false };
        user.added_codes.iter().any(|c| md5_hex(c).eq_ignore_ascii_case(code))
    }

    /// The Java edition's `canInstall`: testers always; private packs need the player on the allowed list;
    /// semi-public packs need their code added; public packs just need a version.
    pub fn can_install(&self, user: &UserCtx) -> bool {
        match self.kind {
            PackType::Private => self.is_tester(user) || (self.has_versions() && self.is_allowed_player(user)),
            PackType::Semipublic if self.code.is_some() => {
                self.is_tester(user) || (self.has_versions() && self.code_added(user))
            }
            _ => self.is_tester(user) || self.has_versions(),
        }
    }

    /// The versions a player may pick, newest first: development versions (testers only) ahead of the rest.
    /// For servers, versions whose Minecraft has no server download are left out (`server_ok` says which).
    pub fn pickable_versions(
        &self,
        user: &UserCtx,
        is_server: bool,
        server_ok: &dyn Fn(&str) -> bool,
    ) -> Vec<PackVersion> {
        let mut out = Vec::new();
        if self.is_tester(user) {
            for v in &self.dev_versions {
                if !is_server || server_ok(&v.minecraft) {
                    out.push(v.clone());
                }
            }
        }
        for v in &self.versions {
            if !is_server || server_ok(&v.minecraft) {
                out.push(v.clone());
            }
        }
        out
    }

    /// The version to start on in the picker: the first recommended release for a new install.
    pub fn default_version(&self, versions: &[PackVersion]) -> Option<usize> {
        versions.iter().position(|v| v.is_recommended && !v.is_dev())
    }

    pub fn image_url(&self) -> String {
        format!("{DOWNLOAD_SERVER}/launcher/images/{}.png", self.safe_name().to_lowercase())
    }

    pub fn json_url(&self, version: &str) -> String {
        json_url(&self.name, version)
    }

    pub fn configs_zip_url(&self, version: &str) -> String {
        format!("{DOWNLOAD_SERVER}/packs/{}/versions/{version}/Configs.zip", self.safe_name())
    }
}

pub fn json_url(pack_name: &str, version: &str) -> String {
    format!("{DOWNLOAD_SERVER}/packs/{}/versions/{version}/Configs.json", safe_name(pack_name))
}

/// Pack users from `users.json`.
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct PackUsers {
    pack: i64,
    testers: Vec<String>,
    #[serde(rename = "allowedPlayers")]
    allowed_players: Vec<String>,
}

/// Every pack the servers know of.
#[derive(Clone, Debug, Default)]
pub struct PackList {
    pub packs: Vec<Pack>,
}

fn json_dir(dirs: &Dirs) -> PathBuf {
    dirs.configs.join("json")
}

#[derive(Deserialize)]
struct FileEntry {
    name: String,
    folder: String,
    #[serde(default)]
    sha1: Option<String>,
}

impl PackList {
    /// Parse `packsnew.json` and `users.json`.
    pub fn parse(packs_json: &[u8], users_json: Option<&[u8]>) -> Result<PackList> {
        let mut packs: Vec<Pack> = serde_json::from_slice(packs_json).context("reading the pack list")?;
        if let Some(users) = users_json {
            let users: Vec<PackUsers> = serde_json::from_slice(users).context("reading the pack users")?;
            for u in users {
                if let Some(p) = packs.iter_mut().find(|p| p.id == u.pack) {
                    p.testers.extend(u.testers);
                    p.allowed_players.extend(u.allowed_players);
                }
            }
        }
        Ok(PackList { packs })
    }

    /// Read the copy saved in the data folder (the same files the Java edition keeps in `configs/json`).
    pub fn load_cached(dirs: &Dirs) -> Option<PackList> {
        let dir = json_dir(dirs);
        let packs = std::fs::read(dir.join("packsnew.json")).ok()?;
        let users = std::fs::read(dir.join("users.json")).ok();
        PackList::parse(&packs, users.as_deref()).ok()
    }

    /// Bring the saved copy up to date (only what changed on the server is downloaded, decided by the checksums
    /// in `files.json`, like the Java edition) and read it. Works offline from the saved copy.
    pub async fn load(dirs: &Dirs) -> Result<PackList> {
        match refresh_files(dirs).await {
            Ok(()) => {}
            Err(e) => {
                if PackList::load_cached(dirs).is_none() {
                    return Err(e).context("couldn't download the ATLauncher pack list");
                }
            }
        }
        PackList::load_cached(dirs).context("the ATLauncher pack list is missing")
    }

    pub fn by_id(&self, id: i64) -> Option<&Pack> {
        self.packs.iter().find(|p| p.id == id)
    }

    pub fn by_name(&self, name: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn by_safe_name(&self, name: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.safe_name().eq_ignore_ascii_case(name))
    }

    /// The semi-public pack a typed code belongs to.
    pub fn semi_public_by_code(&self, code: &str) -> Option<&Pack> {
        let hash = md5_hex(code);
        self.packs
            .iter()
            .find(|p| p.is_semi_public() && p.code.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&hash)))
    }

    /// Add a pack code. Returns the code to save in `addedPacks` when it opens a pack the player did not
    /// already have; `None` when no pack uses it, it is already added, or the player is a tester of that pack.
    pub fn add_code(&self, code: &str, user: &UserCtx) -> Option<String> {
        let pack = self.semi_public_by_code(code)?;
        if pack.is_tester(user) || pack.code_added(user) {
            return None;
        }
        Some(code.to_string())
    }

    /// The packs a player sees, in the order wanted. System packs are never shown. With `featured_only` only
    /// featured packs are listed.
    pub fn visible(&self, user: &UserCtx, featured_only: bool, order: PackOrder) -> Vec<&Pack> {
        let mut v: Vec<&Pack> =
            self.packs.iter().filter(|p| !p.system && (!featured_only || p.featured) && p.can_install(user)).collect();
        match order {
            PackOrder::Position => v.sort_by_key(|p| p.position),
            PackOrder::NameAscending => v.sort_by_key(|p| p.name.to_lowercase()),
            PackOrder::NameDescending => {
                v.sort_by_key(|p| p.name.to_lowercase());
                v.reverse();
            }
        }
        v
    }

    /// Search by name or description (case-insensitive) among the packs a player can see.
    pub fn search(&self, user: &UserCtx, query: &str, order: PackOrder) -> Vec<&Pack> {
        let q = query.trim().to_lowercase();
        self.visible(user, false, order)
            .into_iter()
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q) || p.description.to_lowercase().contains(&q))
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackOrder {
    Position,
    NameAscending,
    NameDescending,
}

/// Download `packsnew.json` and `users.json` when the server's checksum differs from the saved file's.
async fn refresh_files(dirs: &Dirs) -> Result<()> {
    let dir = json_dir(dirs);
    std::fs::create_dir_all(&dir)?;
    let files: Vec<FileEntry> = http::get_json(&format!("{DOWNLOAD_SERVER}/launcher/json/files.json")).await?;
    for wanted in ["packsnew.json", "users.json"] {
        let Some(entry) = files.iter().find(|f| f.name == wanted && f.folder.eq_ignore_ascii_case("json")) else {
            continue;
        };
        let local = dir.join(wanted);
        let current = match (&entry.sha1, std::fs::read(&local)) {
            (Some(sha1), Ok(bytes)) => http::sha1_hex(&bytes).eq_ignore_ascii_case(sha1),
            _ => false,
        };
        if current {
            continue;
        }
        let bytes = http::get_bytes(&format!("{DOWNLOAD_SERVER}/launcher/json/{wanted}")).await?;
        if let Some(sha1) = &entry.sha1 {
            let got = http::sha1_hex(&bytes);
            if !got.eq_ignore_ascii_case(sha1) {
                anyhow::bail!("{wanted} arrived damaged (expected {sha1}, got {got})");
            }
        }
        super::write_bytes_atomic(&local, &bytes)?;
    }
    Ok(())
}

/// The version's `Configs.json`. Cached for ten minutes unless `fresh`; testers always get a fresh copy (the pack
/// author may be changing it).
pub async fn fetch_pack_json(dirs: &Dirs, pack_name: &str, version: &str, fresh: bool) -> Result<(PackJson, Vec<u8>)> {
    let url = json_url(pack_name, version);
    let bytes = if fresh {
        http::get_bytes(&url).await?
    } else {
        let file = dirs.cache.join("packs").join(format!("{}.json", http::sha1_hex(url.as_bytes())));
        let age = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok());
        match age {
            Some(a) if a < Duration::from_secs(600) => std::fs::read(&file)?,
            _ => {
                let bytes = http::get_bytes(&url).await?;
                let _ = std::fs::create_dir_all(file.parent().unwrap_or(&dirs.cache));
                let _ = std::fs::write(&file, &bytes);
                bytes
            }
        }
    };
    let parsed = PackJson::parse(&bytes).with_context(|| format!("reading the definition of {pack_name} {version}"))?;
    Ok((parsed, bytes))
}

// ---------------------------------------------------------------------------------------------------------
// what an installed instance says about its pack

/// What kind of install an instance is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstanceOrigin {
    /// A pack from ATLauncher's own list.
    Atl,
    Technic,
    TechnicSolder,
    /// Plain Minecraft with an optional loader.
    Vanilla,
    /// CurseForge, Modrinth, FTB, MultiMC or anything else this module does not manage.
    Other,
}

pub fn origin_of(instance: &Instance) -> InstanceOrigin {
    let l = &instance.file.launcher;
    if l.vanilla_instance {
        return InstanceOrigin::Vanilla;
    }
    if let Some(t) = l.extras.get("technicModpack").filter(|v| !v.is_null()) {
        return if t.get("solder").is_some_and(|s| s.is_string()) {
            InstanceOrigin::TechnicSolder
        } else {
            InstanceOrigin::Technic
        };
    }
    const EXTERNAL: [&str; 7] = [
        "curseForgeProject",
        "curseForgeManifest",
        "modrinthProject",
        "modrinthManifest",
        "ftbPackManifest",
        "multiMCManifest",
        "curseManifest",
    ];
    if EXTERNAL.iter().any(|k| l.extras.get(*k).is_some_and(|v| !v.is_null())) {
        return InstanceOrigin::Other;
    }
    if l.pack_id != 0 { InstanceOrigin::Atl } else { InstanceOrigin::Other }
}

/// An update the pack list offers for an instance.
#[derive(Clone, Debug)]
pub struct Update {
    pub version: PackVersion,
    /// The player chose to ignore this update (or every update) for the instance.
    pub ignored: bool,
}

fn extra_bool(instance: &Instance, key: &str) -> bool {
    instance.file.launcher.extras.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn ignored_versions(instance: &Instance) -> Vec<String> {
    instance
        .file
        .launcher
        .extras
        .get("ignoredUpdates")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn update_ignored(instance: &Instance, version_or_hash: &str) -> bool {
    extra_bool(instance, "ignoreAllUpdates")
        || ignored_versions(instance).iter().any(|v| v.eq_ignore_ascii_case(version_or_hash))
}

/// "Ignore this update": remember the version (or the development hash) so the instance stops asking about it.
pub fn ignore_update(instance: &mut Instance, version_or_hash: &str) -> Result<()> {
    if !update_ignored(instance, version_or_hash) {
        let mut list = ignored_versions(instance);
        list.push(version_or_hash.to_string());
        instance.file.launcher.extras.insert("ignoredUpdates".into(), serde_json::json!(list));
    }
    instance.save()
}

/// "Don't remind me again": no update of this instance is offered any more.
pub fn ignore_all_updates(instance: &mut Instance) -> Result<()> {
    instance.file.launcher.extras.insert("ignoreAllUpdates".into(), Value::Bool(true));
    instance.save()
}

/// The pack an ATLauncher-list instance came from.
pub fn pack_of<'a>(list: &'a PackList, instance: &Instance) -> Option<&'a Pack> {
    if origin_of(instance) != InstanceOrigin::Atl {
        return None;
    }
    list.by_id(instance.file.launcher.pack_id)
}

/// The Java edition's update test for packs from ATLauncher's list: the newest version differs from the
/// installed one and the pack allows updating to it; a development install is compared by its hash.
pub fn update_for(list: &PackList, instance: &Instance) -> Option<Update> {
    let pack = pack_of(list, instance)?;
    let launcher = &instance.file.launcher;
    let is_dev = extra_bool(instance, "isDev");
    if pack.has_versions() && !is_dev {
        let latest = pack.latest_version()?;
        if !latest.version.eq_ignore_ascii_case(&launcher.version) && !pack.latest_version_no_update() {
            return Some(Update { ignored: update_ignored(instance, &latest.version), version: latest.clone() });
        }
    }
    if is_dev {
        if let Some(hash) = launcher.extras.get("hash").and_then(Value::as_str) {
            let dev = pack.dev_version_by_name(&launcher.version)?;
            if !dev.hash_matches(hash) {
                let key = dev.hash.clone().unwrap_or_default();
                return Some(Update { ignored: update_ignored(instance, &key), version: dev.clone() });
            }
        }
    }
    None
}

/// The pack version an instance is on, if the list still has it.
pub fn installed_version<'a>(list: &'a PackList, instance: &Instance) -> Option<&'a PackVersion> {
    let pack = pack_of(list, instance)?;
    if extra_bool(instance, "isDev") {
        pack.dev_version_by_name(&instance.file.launcher.version)
    } else {
        pack.version_by_name(&instance.file.launcher.version)
    }
}

/// Java requirement saved with the instance (`launcher.java`).
pub fn java_requirement(instance: &Instance) -> Option<JavaRequirement> {
    serde_json::from_value(instance.file.launcher.extras.get("java")?.clone()).ok()
}

/// The text of the Java edition's "Cannot launch instance due to your Java version" dialog, when the Java about
/// to be used does not fit what this pack version needs. `java_major` is the major version of that Java.
pub fn java_warning(instance: &Instance, java_major: Option<u32>) -> Option<String> {
    let req = java_requirement(instance)?;
    let major = java_major? as i64;
    if req.conforms(major) {
        return None;
    }
    Some(format!(
        "This version of the pack requires a Java version which you are not using. Please install that version of Java and try again. Java version needed: {}",
        req.describe()
    ))
}

/// Why a pack version cannot be installed right now, `None` when it can. Covers the pack-user rules, the Minecraft
/// version existing, and (for servers) the version having a server.
pub fn why_not_installable(
    pack: &Pack,
    version: &PackVersion,
    user: &UserCtx,
    is_server: bool,
    mc_known: &dyn Fn(&str) -> bool,
    mc_has_server: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if !pack.can_install(user) {
        return Some(match pack.kind {
            PackType::Private => {
                format!("{} is a private pack and your account isn't on its list of players.", pack.name)
            }
            PackType::Semipublic => format!("{} needs a pack code. Add it first.", pack.name),
            PackType::Public => format!("{} has no versions to install yet.", pack.name),
        });
    }
    if version.is_dev() && !pack.is_tester(user) {
        return Some("Development versions are only for the pack's testers.".to_string());
    }
    if !mc_known(&version.minecraft) {
        return Some(format!(
            "Minecraft {} isn't available, so {} {} can't be installed.",
            version.minecraft, pack.name, version.version
        ));
    }
    if is_server && !pack.create_server {
        return Some(format!("{} doesn't offer a server.", pack.name));
    }
    if is_server && !mc_has_server(&version.minecraft) {
        return Some(format!(
            "Minecraft {} has no server, so a {} server can't be made from this version.",
            version.minecraft, pack.name
        ));
    }
    None
}

/// A summary of the packs by Minecraft version, handy for filters.
pub fn minecraft_versions(list: &PackList) -> Vec<String> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut out = Vec::new();
    for p in &list.packs {
        for v in &p.versions {
            if seen.insert(v.minecraft.clone(), ()).is_none() {
                out.push(v.minecraft.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // real entries from https://download.nodecdn.net/containers/atl/launcher/json/packsnew.json
    const PACKS: &str = r#"[
 {"id":1,"position":29,"name":"MoonQuest","type":"public","devVersions":[{"version":"dev","minecraft":"1.7.10","hash":"1759ad83418f1dfdab42b7ba33ed8f53"}],
  "versions":[{"version":"0.17.0","minecraft":"1.6.4","canUpdate":true,"isRecommended":true},{"version":"0.16.0","minecraft":"1.6.4","canUpdate":true,"isRecommended":true}],
  "createServer":true,"logging":true,"featured":false,"hasDiscordImage":false,"description":"Welcome to this Yogscast Inspired Unofficial Minecraft Modpack!"},
 {"id":8,"position":7,"name":"Resonant Rise","type":"public","devVersions":[],
  "versions":[{"version":"5.0.0-pre.3","minecraft":"1.12.2","canUpdate":true,"isRecommended":true,"hasLoader":true},{"version":"4.0","minecraft":"1.12.2","canUpdate":false,"isRecommended":false}],
  "createServer":true,"logging":true,"featured":true,"hasDiscordImage":false,"discordInviteURL":"https://discord.resonantri.se","supportURL":"https://forum.resonantri.se/forums/tech-support.9/","websiteURL":"https://forum.resonantri.se/","description":"Tech"},
 {"id":99,"position":1,"name":"Crundee Craft","type":"private","devVersions":[],"versions":[{"version":"1","minecraft":"1.7.10"}],"createServer":false,"logging":true,"featured":false,"hasDiscordImage":false,"description":"private"},
 {"id":100,"position":2,"name":"Secret Pack","type":"semipublic","code":"5d41402abc4b2a76b9719d911017c592","devVersions":[],"versions":[{"version":"1","minecraft":"1.7.10"}],"createServer":false,"logging":true,"featured":false,"hasDiscordImage":false,"description":"semi"}
]"#;
    const USERS: &str =
        r#"[{"pack":1,"testers":["demon012","RyanTheAllmighty"]},{"pack":99,"testers":[],"allowedPlayers":["Riyad"]}]"#;

    fn list() -> PackList {
        PackList::parse(PACKS.as_bytes(), Some(USERS.as_bytes())).unwrap()
    }

    fn user(name: Option<&str>, codes: &[&str]) -> UserCtx {
        UserCtx { username: name.map(str::to_string), added_codes: codes.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn real_pack_entries_parse() {
        let l = list();
        assert_eq!(l.packs.len(), 4);
        let rr = l.by_name("resonant rise").unwrap();
        assert_eq!(rr.id, 8);
        assert!(rr.featured);
        assert_eq!(rr.discord_invite_url.as_deref(), Some("https://discord.resonantri.se"));
        assert_eq!(rr.website_url.as_deref(), Some("https://forum.resonantri.se/"));
        assert!(rr.versions[0].has_loader);
        assert!(!rr.versions[1].can_update);
        let mq = l.by_id(1).unwrap();
        assert!(mq.dev_versions[0].is_dev());
        assert!(!mq.versions[0].is_dev());
        assert_eq!(mq.safe_name(), "MoonQuest");
        assert_eq!(
            mq.json_url("0.17.0"),
            "https://download.nodecdn.net/containers/atl/packs/MoonQuest/versions/0.17.0/Configs.json"
        );
        assert_eq!(
            l.by_id(8).unwrap().image_url(),
            "https://download.nodecdn.net/containers/atl/launcher/images/resonantrise.png"
        );
        assert_eq!(mq.testers, vec!["demon012", "RyanTheAllmighty"]);
    }

    #[test]
    fn who_can_see_what() {
        let l = list();
        let anon = user(None, &[]);
        let names = |u: &UserCtx| {
            l.visible(u, false, PackOrder::NameAscending).iter().map(|p| p.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names(&anon), vec!["MoonQuest", "Resonant Rise"]);
        // the allowed player sees the private pack
        assert_eq!(names(&user(Some("riyad"), &[])), vec!["Crundee Craft", "MoonQuest", "Resonant Rise"]);
        // a pack code opens the semi-public pack ("hello" is the code whose MD5 is in the list)
        assert_eq!(names(&user(None, &["hello"])), vec!["MoonQuest", "Resonant Rise", "Secret Pack"]);
        // testers see dev versions
        let tester = user(Some("DEMON012"), &[]);
        let mq = l.by_id(1).unwrap();
        assert!(mq.is_tester(&tester));
        let all = mq.pickable_versions(&tester, false, &|_| true);
        assert_eq!(all[0].version, "dev");
        assert_eq!(mq.pickable_versions(&anon, false, &|_| true).len(), 2);
        assert_eq!(l.visible(&anon, true, PackOrder::Position).len(), 1);
    }

    #[test]
    fn pack_codes() {
        let l = list();
        assert_eq!(l.semi_public_by_code("hello").unwrap().name, "Secret Pack");
        assert!(l.semi_public_by_code("nope").is_none());
        assert_eq!(l.add_code("hello", &user(None, &[])), Some("hello".to_string()));
        assert_eq!(l.add_code("hello", &user(None, &["hello"])), None);
        assert_eq!(l.add_code("nope", &user(None, &[])), None);
    }

    #[test]
    fn version_labels_match_the_java_edition() {
        let v = |version: &str, mc: &str| PackVersion {
            version: version.into(),
            minecraft: mc.into(),
            ..Default::default()
        };
        assert_eq!(v("1.0", "1.12.2").label(Some("release")), "1.0 (1.12.2)");
        assert_eq!(v("1.12.2", "1.12.2").label(Some("release")), "1.12.2");
        assert_eq!(v("24w01a", "24w01a").label(Some("snapshot")), "24w01a (Snapshot)");
        assert_eq!(v("2.0", "24w01a").label(Some("snapshot")), "2.0 (24w01a) (Snapshot)");
        assert_eq!(v("2.0", "9.9").label(None), "2.0");
    }

    #[test]
    fn recommended_default_and_update_rules() {
        let l = list();
        let rr = l.by_id(8).unwrap();
        assert!(!rr.latest_version_no_update());
        let mut p = rr.clone();
        p.versions[0].is_recommended = false;
        assert!(p.latest_version_no_update());
        let picks = rr.pickable_versions(&user(None, &[]), false, &|_| true);
        assert_eq!(rr.default_version(&picks), Some(0));
    }

    #[test]
    fn install_checks_explain_themselves() {
        let l = list();
        let rr = l.by_id(8).unwrap();
        let v = &rr.versions[0];
        let u = user(None, &[]);
        assert!(why_not_installable(rr, v, &u, false, &|_| true, &|_| true).is_none());
        assert!(why_not_installable(rr, v, &u, false, &|_| false, &|_| true).unwrap().contains("isn't available"));
        assert!(why_not_installable(rr, v, &u, true, &|_| true, &|_| false).unwrap().contains("no server"));
        let crundee = l.by_id(99).unwrap();
        assert!(
            why_not_installable(crundee, &crundee.versions[0], &u, false, &|_| true, &|_| true)
                .unwrap()
                .contains("private pack")
        );
        let mq = l.by_id(1).unwrap();
        assert!(
            why_not_installable(mq, &mq.dev_versions[0], &u, false, &|_| true, &|_| true).unwrap().contains("testers")
        );
    }

    fn instance(launcher: serde_json::Value) -> Instance {
        let file: crate::instances::InstanceFile =
            serde_json::from_value(serde_json::json!({"uuid": "u", "id": "1.12.2", "launcher": launcher})).unwrap();
        Instance { folder: "I".into(), mod_count: 0, root: std::path::PathBuf::from("/i"), file }
    }

    #[test]
    fn updates_are_found_from_what_the_instance_saved() {
        let l = list();
        // on an old version of Resonant Rise (pack 8), the list's newest differs
        let old =
            instance(serde_json::json!({"name":"RR","pack":"Resonant Rise","packId":8,"version":"3.0","isDev":false}));
        assert_eq!(origin_of(&old), InstanceOrigin::Atl);
        let u = update_for(&l, &old).unwrap();
        assert_eq!(u.version.version, "5.0.0-pre.3");
        assert!(!u.ignored);
        // ignoring that version, or every update, is remembered
        let ignored = instance(
            serde_json::json!({"name":"RR","pack":"Resonant Rise","packId":8,"version":"3.0","ignoredUpdates":["5.0.0-PRE.3"]}),
        );
        assert!(update_for(&l, &ignored).unwrap().ignored);
        let all = instance(
            serde_json::json!({"name":"RR","pack":"Resonant Rise","packId":8,"version":"3.0","ignoreAllUpdates":true}),
        );
        assert!(update_for(&l, &all).unwrap().ignored);
        // up to date
        let current =
            instance(serde_json::json!({"name":"RR","pack":"Resonant Rise","packId":8,"version":"5.0.0-pre.3"}));
        assert!(update_for(&l, &current).is_none());
        // a development install is compared by its hash
        let dev_same = instance(
            serde_json::json!({"name":"M","pack":"MoonQuest","packId":1,"version":"dev","isDev":true,"hash":"1759ad83418f1dfdab42b7ba33ed8f53"}),
        );
        assert!(update_for(&l, &dev_same).is_none());
        let dev_old = instance(
            serde_json::json!({"name":"M","pack":"MoonQuest","packId":1,"version":"dev","isDev":true,"hash":"0000"}),
        );
        assert_eq!(update_for(&l, &dev_old).unwrap().version.version, "dev");
        // other kinds of instance are not ours
        assert_eq!(
            origin_of(&instance(serde_json::json!({"name":"V","vanillaInstance":true}))),
            InstanceOrigin::Vanilla
        );
        assert_eq!(
            origin_of(&instance(
                serde_json::json!({"name":"T","technicModpack":{"name":"x","solder":"https://s/api/"}})
            )),
            InstanceOrigin::TechnicSolder
        );
        assert_eq!(
            origin_of(&instance(serde_json::json!({"name":"T","technicModpack":{"name":"x"}}))),
            InstanceOrigin::Technic
        );
        assert_eq!(
            origin_of(&instance(serde_json::json!({"name":"C","packId":0,"curseForgeProject":{"id":1}}))),
            InstanceOrigin::Other
        );
    }

    #[test]
    fn the_java_warning_names_the_version_needed() {
        let i = instance(serde_json::json!({"name":"L","packId":516,"version":"1","java":{"min":25}}));
        let w = java_warning(&i, Some(21)).unwrap();
        assert!(w.contains("Java 25 minimum"));
        assert!(java_warning(&i, Some(25)).is_none());
        assert!(java_warning(&i, None).is_none());
        assert!(java_warning(&instance(serde_json::json!({"name":"L","version":"1"})), Some(8)).is_none());
    }
}
