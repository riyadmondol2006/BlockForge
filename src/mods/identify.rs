//! Finding out what a jar is: its hash is sent to Modrinth, which knows every file it hosts. A match gives the mod
//! its real name, description, picture and an address to check for updates.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use super::ModSettings;
use super::api::{Modrinth, MrVersion};
use super::files::{self, ModKey};
use crate::instances::Instance;
use crate::modrecord::ModRecord;

/// The sha1 of a file, as lowercase hex.
pub fn sha1_file(path: &Path) -> Result<String> {
    use sha1::{Digest, Sha1};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// The sha512 of a file, as lowercase hex.
pub fn sha512_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha512};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha512::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Hashes many files on a worker thread. Files that can't be read are left out.
pub async fn hash_files(paths: Vec<PathBuf>) -> Vec<(PathBuf, String)> {
    tokio::task::spawn_blocking(move || paths.into_iter().filter_map(|p| sha1_file(&p).ok().map(|h| (p, h))).collect())
        .await
        .unwrap_or_default()
}

/// Asks Modrinth what each record's file is and fills in the Modrinth project and version, the name, the version
/// label and the description. Records Modrinth doesn't know are left as they were. Returns how many were found.
/// Nothing is sent when `dontCheckModsOnModrinth` is on.
pub async fn identify_records(instance: &Instance, records: &mut [ModRecord], settings: &ModSettings) -> Result<usize> {
    if settings.dont_check_mods_on_modrinth {
        return Ok(0);
    }
    let paths: Vec<(usize, PathBuf)> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.modrinth_project.is_none() && r.modrinth_version.is_none())
        .filter_map(|(i, r)| files::current_path(instance, r).filter(|p| p.is_file()).map(|p| (i, p)))
        .collect();
    if paths.is_empty() {
        return Ok(0);
    }
    let hashed = hash_files(paths.iter().map(|(_, p)| p.clone()).collect()).await;
    let by_hash: HashMap<String, usize> =
        hashed.iter().filter_map(|(p, h)| paths.iter().find(|(_, q)| q == p).map(|(i, _)| (h.clone(), *i))).collect();
    if by_hash.is_empty() {
        return Ok(0);
    }
    let api = Modrinth::new(settings.modrinth_api_key.clone());
    let hashes: Vec<String> = by_hash.keys().cloned().collect();
    let versions = api.versions_from_sha1(&hashes).await?;
    if versions.is_empty() {
        return Ok(0);
    }
    let project_ids: Vec<String> = versions.values().map(|v| v.project_id.clone()).collect();
    let projects = api.projects(&project_ids).await?;
    let mut found = 0;
    for (hash, version) in versions {
        let (Some(&i), Some(project)) = (by_hash.get(&hash), projects.get(&version.project_id)) else { continue };
        apply_match(&mut records[i], project, &version, settings);
        found += 1;
    }
    Ok(found)
}

/// Gives a record the Modrinth project and version it was found to be.
pub fn apply_match(rec: &mut ModRecord, project: &Value, version: &MrVersion, settings: &ModSettings) {
    rec.modrinth_project = Some(project.clone());
    rec.modrinth_version = Some(version.raw.clone());
    // a mod CurseForge also knows keeps the CurseForge name unless Modrinth is the preferred platform
    if !files::is_from_curseforge(rec) || settings.default_mod_platform == "MODRINTH" {
        if let Some(t) = project.get("title").and_then(Value::as_str) {
            rec.name = t.to_string();
        }
        if let Some(d) = project.get("description").and_then(Value::as_str) {
            rec.description = d.to_string();
        }
    }
    if !version.name.is_empty() {
        rec.version = version.name.clone();
    }
}

/// The Edit Mods window's "Refresh Metadata": asks Modrinth again about the chosen mods, whatever they were before.
/// Returns how many were recognised.
pub async fn refresh_metadata(instance: &mut Instance, keys: &[ModKey], settings: &ModSettings) -> Result<usize> {
    let mut list = instance.mod_records();
    let mut chosen: Vec<ModRecord> = list.iter().filter(|m| keys.contains(&ModKey::of(m))).cloned().collect();
    for r in &mut chosen {
        r.modrinth_project = None;
        r.modrinth_version = None;
    }
    let found = identify_records(instance, &mut chosen, settings).await?;
    for r in chosen {
        if let Some(slot) = list.iter_mut().find(|m| m.file == r.file && m.kind == r.kind) {
            if r.modrinth_project.is_some() {
                *slot = r;
            }
        }
    }
    instance.set_mod_records(&list);
    instance.save()?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_known_values() {
        let p = std::env::temp_dir().join(format!("blockforge-hash-{}.bin", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha1_file(&p).unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha512_file(&p).unwrap(),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_match_names_the_record_after_the_project() {
        let project = serde_json::json!({"id": "p1", "title": "Sodium", "description": "Fast renderer"});
        let version =
            MrVersion::from_value(serde_json::json!({"id": "v1", "project_id": "p1", "name": "Sodium 0.9.2"})).unwrap();
        let mut r = ModRecord::new("sodium-fabric.jar", "sodium-fabric.jar", "mods");
        apply_match(&mut r, &project, &version, &ModSettings::default());
        assert_eq!(
            (r.name.as_str(), r.version.as_str(), r.description.as_str()),
            ("Sodium", "Sodium 0.9.2", "Fast renderer")
        );
        assert_eq!(r.modrinth_project_id(), Some("p1"));
        assert_eq!(r.modrinth_version_id(), Some("v1"));

        // known to CurseForge too, and CurseForge preferred: the name stays
        let mut both = ModRecord::new("curse name", "x.jar", "mods");
        both.curse_forge_project_id = Some(1);
        both.curse_forge_file_id = Some(2);
        let mut s = ModSettings::default();
        s.default_mod_platform = "CURSEFORGE".into();
        apply_match(&mut both, &project, &version, &s);
        assert_eq!(both.name, "curse name");
        assert_eq!(both.modrinth_project_id(), Some("p1"));
    }
}

/// Checks against the jars of the Java edition's instances on this computer (read only) and against the live
/// Modrinth service. The jar checks are skipped when there are no such instances; the network check only runs with
/// `cargo test -- --ignored`.
#[cfg(test)]
mod real_data {
    use super::*;

    fn real_jars() -> Vec<PathBuf> {
        let Some(home) = std::env::var_os("HOME") else { return Vec::new() };
        let instances = PathBuf::from(home).join(".local/share/blockforge/instances");
        let mut out = Vec::new();
        for inst in std::fs::read_dir(instances).into_iter().flatten().flatten() {
            for jar in std::fs::read_dir(inst.path().join("mods")).into_iter().flatten().flatten() {
                if jar.path().extension().is_some_and(|e| e == "jar") {
                    out.push(jar.path());
                }
            }
        }
        out
    }

    #[test]
    fn real_mods_are_read_and_found_clean() {
        let jars = real_jars();
        for jar in &jars {
            let d = super::super::metadata::detect(jar);
            assert_eq!(d.kind, super::super::metadata::FileKind::Mod, "{}", jar.display());
            assert!(
                d.loader.is_some() && d.name.is_some() && d.version.is_some(),
                "{} gave {:?}",
                jar.display(),
                d.info
            );
            assert_eq!(super::super::scan::scan_jar(jar), Ok(false), "{}", jar.display());
        }
    }

    #[tokio::test]
    #[ignore = "asks the real Modrinth"]
    async fn real_mods_are_recognised_by_hash() {
        let jars = real_jars();
        if jars.is_empty() {
            return;
        }
        let hashes: Vec<String> = jars.iter().filter_map(|j| sha1_file(j).ok()).collect();
        let api = Modrinth::default();
        let found = api.versions_from_sha1(&hashes).await.unwrap();
        assert!(!found.is_empty(), "Modrinth knew none of {} real mods", hashes.len());
        // the same file by its sha512 gives the same version
        let (hash1, version) = found.iter().next().unwrap();
        let jar = jars.iter().find(|j| &sha1_file(j).unwrap() == hash1).unwrap();
        let again = api.version_from_hash(&sha512_file(jar).unwrap(), "sha512").await.unwrap().unwrap();
        assert_eq!(again.id, version.id);
        let projects = api.projects(&found.values().map(|v| v.project_id.clone()).collect::<Vec<_>>()).await.unwrap();
        assert!(!projects.is_empty());
    }
}
