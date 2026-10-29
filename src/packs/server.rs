//! Making a server from an ATLauncher pack version: the pieces that differ from a client install (the server jar,
//! the loader's server files, the start scripts and `server.json`). The mods, configs and actions are placed by
//! the same code as for a client.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::fsx;
use super::json::{ModKind, PackJson, PackMod};
use super::loader::Kind;
use crate::instances::LoaderVersion;
use crate::model::{Artifact, Library, VersionJson};
use crate::modrecord::ModRecord;
use crate::paths::Dirs;

const LAUNCH_SH: &str = include_str!("../../assets/server-scripts/LaunchServer.sh");
const LAUNCH_BAT: &str = include_str!("../../assets/server-scripts/LaunchServer.bat");
const README: &str = include_str!("../../assets/server-scripts/readme.txt");

/// Mojang's server jar for a version: the address, size and checksum.
pub fn server_download(game: &VersionJson) -> Option<Artifact> {
    serde_json::from_value(game.downloads.as_ref()?.extras.get("server")?.clone()).ok()
}

/// Where the shared copy of the server jar lives (the Java edition keeps it with the libraries).
pub fn server_jar_library(dirs: &Dirs, mc: &str) -> PathBuf {
    dirs.libraries.join("net").join("minecraft").join("server").join(mc).join(format!("server-{mc}.jar"))
}

/// Minecraft versions from 1.7 up to 1.18.0 can be attacked through log4j (Log4Shell).
pub fn log4shell_exploitable(mc: &str) -> bool {
    super::loader::match_version(mc, "1.7", false, true)
        && !super::loader::match_version(mc, "1.18.1", false, true)
        && !mc.contains('_')
}

fn log4shell_arguments(mc: &str) -> &'static str {
    if log4shell_exploitable(mc) { "-Dlog4j2.formatMsgNoLookups=true" } else { "" }
}

/// Libraries the loader brought, not the ones Minecraft needs: servers take only these (and the pack's own).
pub fn loader_only_libraries(merged: &VersionJson, vanilla: &VersionJson) -> Vec<Library> {
    let vanilla_names: HashSet<&str> = vanilla.libraries.iter().map(|l| l.name.as_str()).collect();
    merged.libraries.iter().filter(|l| !vanilla_names.contains(l.name.as_str())).cloned().collect()
}

/// The name of the jar the start script runs.
pub fn server_jar_name(
    mc: &str,
    loader: Option<(Kind, &LoaderVersion)>,
    selected: &[PackMod],
    forge_jar: Option<&str>,
) -> String {
    match loader {
        Some((Kind::Fabric, _)) => return "fabric-server-launch.jar".into(),
        Some((Kind::Forge, _)) | Some((Kind::LegacyForge, _)) => {
            if let Some(j) = forge_jar {
                return j.to_string();
            }
        }
        _ => {}
    }
    let mcpc = selected.iter().find(|m| m.kind == Some(ModKind::Mcpc));
    let forge = selected.iter().find(|m| m.kind == Some(ModKind::Forge));
    if let Some(m) = mcpc {
        return m.file_name();
    }
    if let Some(m) = forge {
        return m.file_name();
    }
    format!("minecraft_server.{mc}.jar")
}

/// Copy libraries into the server's own `libraries/` folder (servers do not share the launcher's).
pub fn copy_libraries(dirs: &Dirs, root: &Path, libs: &[Library]) -> Result<()> {
    for lib in libs {
        let Some(a) = lib.artifact() else { continue };
        let Some(path) = a.path else { continue };
        let from = dirs.libraries.join(&path);
        if from.is_file() {
            fsx::copy_file(&from, &root.join("libraries").join(&path), true)?;
        }
    }
    Ok(())
}

/// Build `fabric-server-launch.jar`: a jar that starts Fabric's server launcher. For Fabric loaders newer than
/// 0.12.5 the loader's libraries are packed into it.
pub fn make_fabric_launch_jar(
    root: &Path,
    libs: &[PathBuf],
    main_class: &str,
    loader_version: &str,
    server_jar_name: &str,
) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let out = root.join("fabric-server-launch.jar");
    fsx::delete_path(&out);
    let shade = !loader_version.starts_with("0.12.5");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&out)?);
    let opts = SimpleFileOptions::default();
    let mut added: HashSet<String> = HashSet::new();
    added.insert("META-INF/MANIFEST.MF".into());
    w.start_file("META-INF/MANIFEST.MF", opts)?;
    let mut manifest =
        String::from("Manifest-Version: 1.0\r\nMain-Class: net.fabricmc.loader.launch.server.FabricServerLauncher\r\n");
    if !shade {
        let cp: Vec<String> = libs
            .iter()
            .filter_map(|l| l.strip_prefix(root).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
            .collect();
        manifest.push_str(&format!("Class-Path: {}\r\n", cp.join(" ")));
    }
    manifest.push_str("\r\n");
    w.write_all(manifest.as_bytes())?;
    added.insert("fabric-server-launch.properties".into());
    w.start_file("fabric-server-launch.properties", opts)?;
    w.write_all(format!("launch.mainClass={main_class}\n").as_bytes())?;
    if shade {
        for lib in libs {
            let Ok(f) = std::fs::File::open(lib) else { continue };
            let Ok(mut archive) = zip::ZipArchive::new(f) else { continue };
            for i in 0..archive.len() {
                let mut e = archive.by_index(i)?;
                let Ok(name) = e.name().map(|n| n.to_string()) else { continue };
                let signature =
                    name.starts_with("META-INF/") && [".SF", ".DSA", ".RSA", ".EC"].iter().any(|x| name.ends_with(x));
                if e.is_dir() || added.contains(&name) || signature {
                    continue;
                }
                w.start_file(&name, opts)?;
                w.write_all(&crate::packutil::zip_entry_bytes(&mut e)?)?;
                added.insert(name);
            }
        }
    }
    w.finish()?;
    std::fs::write(root.join("fabric-server-launcher.properties"), format!("serverJar={server_jar_name}\n"))?;
    Ok(())
}

/// Write `LaunchServer.sh`, `LaunchServer.bat` and the readme.
pub fn write_scripts(root: &Path, json: &PackJson, mc: &str, server_jar: &str) -> Result<()> {
    let fill = |t: &str| {
        t.replace("%%SERVERJAR%%", server_jar)
            .replace("%%ARGUMENTS%%", &json.server_arguments)
            .replace("%%LOG4SHELLARGUMENTS%%", log4shell_arguments(mc))
    };
    std::fs::write(root.join("LaunchServer.sh"), fill(LAUNCH_SH))?;
    std::fs::write(root.join("LaunchServer.bat"), fill(LAUNCH_BAT))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(root.join("LaunchServer.sh"), std::fs::Permissions::from_mode(0o755));
    }
    std::fs::write(root.join("_readme.txt"), README)?;
    Ok(())
}

/// `server.json`, the Java edition's record of a server.
#[allow(clippy::too_many_arguments)]
pub fn write_server_json(
    root: &Path,
    name: &str,
    pack: &str,
    pack_id: i64,
    external_id: i64,
    mc: &str,
    version: &str,
    hash: Option<&str>,
    is_dev: bool,
    loader: Option<&LoaderVersion>,
    java: Option<&crate::model::JavaVersion>,
    mods: &[ModRecord],
    previous: Option<&Value>,
) -> Result<()> {
    let mut v = previous.cloned().filter(|p| p.is_object()).unwrap_or_else(|| json!({}));
    let o = v.as_object_mut().context("server.json is not an object")?;
    o.insert("name".into(), json!(name));
    o.insert("pack".into(), json!(pack));
    o.insert("packId".into(), json!(pack_id));
    o.insert("externalPackId".into(), json!(external_id));
    o.insert("vanillaInstance".into(), json!(false));
    o.insert("minecraftVersion".into(), json!(mc));
    o.insert("version".into(), json!(version));
    o.insert("isPatchedForLog4Shell".into(), json!(true));
    o.insert("isDev".into(), json!(is_dev));
    match hash.filter(|_| is_dev) {
        Some(h) => {
            o.insert("hash".into(), json!(h));
        }
        None => {
            o.remove("hash");
        }
    }
    o.insert("loaderVersion".into(), serde_json::to_value(loader)?);
    if let Some(j) = java {
        o.insert("javaVersion".into(), serde_json::to_value(j)?);
    }
    o.insert("mods".into(), serde_json::to_value(mods)?);
    o.entry("ignoredUpdates").or_insert(json!([]));
    crate::config::write_json_atomic(&root.join("server.json"), &v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log4shell_range_is_1_7_to_1_18_0() {
        assert!(log4shell_exploitable("1.12.2"));
        assert!(log4shell_exploitable("1.18"));
        assert!(!log4shell_exploitable("1.18.1"));
        assert!(!log4shell_exploitable("1.6.4"));
        assert!(!log4shell_exploitable("1.21.1"));
    }

    #[test]
    fn the_server_jar_is_picked_like_the_java_edition_does() {
        let forge = PackMod {
            name: "Forge".into(),
            file: "forge-universal.jar".into(),
            kind: Some(ModKind::Forge),
            ..Default::default()
        };
        let mcpc =
            PackMod { name: "MCPC".into(), file: "mcpc.jar".into(), kind: Some(ModKind::Mcpc), ..Default::default() };
        assert_eq!(server_jar_name("1.6.4", None, &[], None), "minecraft_server.1.6.4.jar");
        assert_eq!(server_jar_name("1.6.4", None, std::slice::from_ref(&forge), None), "forge-universal.jar");
        assert_eq!(server_jar_name("1.6.4", None, &[forge, mcpc], None), "mcpc.jar");
        let lv = LoaderVersion { kind: "Fabric".into(), ..Default::default() };
        assert_eq!(server_jar_name("1.20.1", Some((Kind::Fabric, &lv)), &[], None), "fabric-server-launch.jar");
    }

    #[test]
    fn scripts_get_the_jar_and_arguments() {
        let dir = std::env::temp_dir().join(format!("bf-srv-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let json = PackJson { server_arguments: "-Dfoo=bar".into(), ..Default::default() };
        write_scripts(&dir, &json, "1.12.2", "forge.jar").unwrap();
        let sh = std::fs::read_to_string(dir.join("LaunchServer.sh")).unwrap();
        assert!(sh.contains("-jar forge.jar"));
        assert!(sh.contains("-Dfoo=bar -Dlog4j2.formatMsgNoLookups=true"));
        assert!(!sh.contains("%%"));
        assert!(dir.join("_readme.txt").is_file());
        let _ = std::fs::remove_dir_all(dir);
    }
}
