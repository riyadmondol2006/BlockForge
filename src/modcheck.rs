//! Recognising the mods in an instance on Modrinth (by SHA-1) and CurseForge (by fingerprint), and finding where
//! the file behind a `launcher.mods` record lives. Installers use it to fill in the records of pack mods; the
//! exporters use it to decide which mods can be listed by hash instead of being packed into the archive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::cf_seam::{self, CurseForgeSource};
use crate::http::client;
use crate::modrecord::ModRecord;

const MODRINTH: &str = "https://api.modrinth.com/v2";

/// The folder a record of this `kind` lives in, relative to the instance (`DisableableMod.getFile`).
/// `mc` is the instance's Minecraft version, which `dependency` mods are filed under.
pub fn kind_dir(kind: &str, mc: &str) -> Option<String> {
    Some(match kind {
        "jar" | "forge" | "mcpc" => "jarmods".into(),
        "texturepack" => "texturepacks".into(),
        "resourcepack" => "resourcepacks".into(),
        "mods" => "mods".into(),
        "plugins" => "plugins".into(),
        "ic2lib" => "mods/ic2".into(),
        "denlib" => "mods/denlib".into(),
        "coremods" => "coremods".into(),
        "shaderpack" => "shaderpacks".into(),
        "datapack" => "datapacks".into(),
        "dependency" if !mc.is_empty() => format!("mods/{mc}"),
        _ => return None,
    })
}

/// Where the file of `rec` is while it is enabled (`DisableableMod.getFile`).
pub fn enabled_file(root: &Path, mc: &str, rec: &ModRecord) -> Option<PathBuf> {
    let dir = match rec.path.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => crate::packutil::safe_relative(p)?,
        None => PathBuf::from(kind_dir(&rec.kind, mc)?),
    };
    let file = Path::new(&rec.file);
    if rec.file.is_empty() || file.components().count() != 1 {
        return None;
    }
    Some(root.join(dir).join(file))
}

/// Where the file of `rec` is while it is disabled.
pub fn disabled_file(root: &Path, rec: &ModRecord) -> Option<PathBuf> {
    let file = Path::new(&rec.file);
    if rec.file.is_empty() || file.components().count() != 1 {
        return None;
    }
    Some(root.join("disabledmods").join(file))
}

/// Where the file of `rec` is right now (`DisableableMod.getPath`).
pub fn current_file(root: &Path, mc: &str, rec: &ModRecord) -> Option<PathBuf> {
    if rec.disabled { disabled_file(root, rec) } else { enabled_file(root, mc, rec) }
}

pub fn is_from_curseforge(rec: &ModRecord) -> bool {
    rec.curse_forge_project_id.is_some() && rec.curse_forge_file_id.is_some()
}

pub fn has_full_curseforge_info(rec: &ModRecord) -> bool {
    rec.curse_forge_project.as_ref().is_some_and(|v| !v.is_null())
        && rec.curse_forge_file.as_ref().is_some_and(|v| !v.is_null())
}

pub fn is_from_modrinth(rec: &ModRecord) -> bool {
    rec.modrinth_project.as_ref().is_some_and(|v| !v.is_null())
        && rec.modrinth_version.as_ref().is_some_and(|v| !v.is_null())
}

/// Modrinth versions for these SHA-1 hashes: hash -> version object. Hashes Modrinth doesn't know are missing.
pub async fn modrinth_versions_by_sha1(hashes: &[String]) -> Result<HashMap<String, Value>> {
    if hashes.is_empty() {
        return Ok(HashMap::new());
    }
    let r = client()
        .post(format!("{MODRINTH}/version_files"))
        .json(&json!({ "hashes": hashes, "algorithm": "sha1" }))
        .send()
        .await?;
    if !r.status().is_success() {
        bail!("Modrinth answered HTTP {}", r.status());
    }
    let v: Value = r.json().await?;
    Ok(v.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default())
}

/// Modrinth projects by id: id -> project object.
pub async fn modrinth_projects(ids: &[String]) -> Result<HashMap<String, Value>> {
    let mut out = HashMap::new();
    for chunk in ids.chunks(100) {
        let r = client()
            .get(format!("{MODRINTH}/projects"))
            .query(&[("ids", serde_json::to_string(chunk)?)])
            .send()
            .await?;
        if !r.status().is_success() {
            bail!("Modrinth answered HTTP {}", r.status());
        }
        let list: Vec<Value> = r.json().await?;
        for p in list {
            if let Some(id) = p["id"].as_str() {
                out.insert(id.to_string(), p.clone());
            }
        }
    }
    Ok(out)
}

/// The file in a Modrinth version object that has `sha1` (or the primary one).
pub fn modrinth_file_by_sha1<'a>(version: &'a Value, sha1: &str) -> Option<&'a Value> {
    let files = version["files"].as_array()?;
    files
        .iter()
        .find(|f| f["hashes"]["sha1"].as_str().is_some_and(|h| h.eq_ignore_ascii_case(sha1)))
        .or_else(|| files.iter().find(|f| f["primary"].as_bool() == Some(true)))
        .or_else(|| files.first())
}

/// Fill in the Modrinth project and version of every record in `records` whose file Modrinth recognises.
/// Records that already have them are left alone. Returns how many were found.
pub async fn attach_modrinth(root: &Path, mc: &str, records: &mut [ModRecord], only_enabled: bool) -> Result<usize> {
    let mut by_hash: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, rec) in records.iter().enumerate() {
        if is_from_modrinth(rec) || (only_enabled && rec.disabled) {
            continue;
        }
        let Some(file) = current_file(root, mc, rec).filter(|f| f.is_file()) else { continue };
        if let Ok(bytes) = std::fs::read(file) {
            by_hash.entry(crate::http::sha1_hex(&bytes)).or_default().push(i);
        }
    }
    if by_hash.is_empty() {
        return Ok(0);
    }
    let hashes: Vec<String> = by_hash.keys().cloned().collect();
    let versions = modrinth_versions_by_sha1(&hashes).await?;
    if versions.is_empty() {
        return Ok(0);
    }
    let mut project_ids: Vec<String> =
        versions.values().filter_map(|v| v["project_id"].as_str().map(str::to_string)).collect();
    project_ids.sort();
    project_ids.dedup();
    let projects = modrinth_projects(&project_ids).await.unwrap_or_default();
    let mut found = 0;
    for (hash, idx) in by_hash {
        let Some(version) = versions.get(&hash) else { continue };
        for i in idx {
            records[i].modrinth_version = Some(version.clone());
            if let Some(p) = version["project_id"].as_str().and_then(|id| projects.get(id)) {
                records[i].modrinth_project = Some(p.clone());
            }
            found += 1;
        }
    }
    Ok(found)
}

/// Fill in the CurseForge project and file of every record in `records` whose file CurseForge recognises
/// (by fingerprint), only for projects CurseForge has approved. With `rename`, the record also takes the
/// project's name and summary, as the installers do. Returns how many were found.
pub async fn attach_curseforge(
    root: &Path,
    mc: &str,
    records: &mut [ModRecord],
    cf: &dyn CurseForgeSource,
    rename: bool,
) -> Result<usize> {
    let mut by_print: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, rec) in records.iter().enumerate() {
        if rec.disabled || rec.kind == "worlds" || (rec.curse_forge_project.is_some() && rec.curse_forge_file.is_some())
        {
            continue;
        }
        let Some(file) = enabled_file(root, mc, rec).filter(|f| f.is_file()) else { continue };
        if let Ok(bytes) = std::fs::read(file) {
            by_print.entry(cf_seam::fingerprint(&bytes)).or_default().push(i);
        }
    }
    if by_print.is_empty() {
        return Ok(0);
    }
    let matches = cf.fingerprints(by_print.keys().copied().collect()).await?;
    let mut found = 0;
    for m in matches {
        if m.project.status != 4 {
            continue;
        }
        let Some(idx) = by_print.get(&m.file.fingerprint) else { continue };
        for &i in idx {
            let rec = &mut records[i];
            rec.curse_forge_project_id = Some(m.project.id);
            rec.curse_forge_file_id = Some(m.file.id);
            rec.curse_forge_project = Some(m.project.raw.clone());
            rec.curse_forge_file = Some(m.file.for_instance_json());
            if rename {
                rec.name = m.project.name.clone();
                rec.description = m.project.summary.clone();
            }
            found += 1;
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: &str, file: &str) -> ModRecord {
        ModRecord { kind: kind.into(), file: file.into(), ..Default::default() }
    }

    #[test]
    fn files_are_found_where_java_puts_them() {
        let root = Path::new("/i");
        assert_eq!(enabled_file(root, "1.20.1", &rec("mods", "a.jar")), Some(PathBuf::from("/i/mods/a.jar")));
        assert_eq!(
            enabled_file(root, "1.20.1", &rec("dependency", "a.jar")),
            Some(PathBuf::from("/i/mods/1.20.1/a.jar"))
        );
        assert_eq!(
            enabled_file(root, "1.20.1", &rec("resourcepack", "p.zip")),
            Some(PathBuf::from("/i/resourcepacks/p.zip"))
        );
        assert_eq!(enabled_file(root, "1.20.1", &rec("worlds", "w")), None);
        let mut with_path = rec("mods", "a.jar");
        with_path.path = Some("mods/".into());
        assert_eq!(enabled_file(root, "x", &with_path), Some(PathBuf::from("/i/mods/a.jar")));
        let mut bad = rec("mods", "a.jar");
        bad.path = Some("../outside".into());
        assert_eq!(enabled_file(root, "x", &bad), None);
        assert_eq!(enabled_file(root, "x", &rec("mods", "../x.jar")), None);
        let mut off = rec("mods", "a.jar");
        off.disabled = true;
        assert_eq!(current_file(root, "x", &off), Some(PathBuf::from("/i/disabledmods/a.jar")));
    }

    #[test]
    fn modrinth_file_prefers_the_matching_hash() {
        let v =
            json!({"files":[{"hashes":{"sha1":"aa"},"primary":true,"url":"u1"},{"hashes":{"sha1":"bb"},"url":"u2"}]});
        assert_eq!(modrinth_file_by_sha1(&v, "BB").unwrap()["url"], "u2");
        assert_eq!(modrinth_file_by_sha1(&v, "zz").unwrap()["url"], "u1");
    }
}
