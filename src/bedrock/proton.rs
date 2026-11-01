//! Prefix compatibility setup ported from native21 Proton (Valve, GPL-3.0)
//! and BedrockOnLinux bol/prefix.py (MIT). No Python entrypoint is used.
use super::{
    Result,
    artifact::{self, invalid, io},
    engine::Engine,
};
use std::os::unix::fs::symlink;
use std::{collections::BTreeMap, fs, path::Path};

const PAYLOAD: &[&str] = &[
    "libasprintf-0.dll",
    "libbrotlicommon.dll",
    "libbrotlidec.dll",
    "libcares-2.dll",
    "libcharset-1.dll",
    "libcrypto-3-x64.dll",
    "libiconv-2.dll",
    "libidn2-0.dll",
    "libintl-8.dll",
    "libnghttp2-14.dll",
    "libnghttp3-9.dll",
    "libngtcp2_crypto_gnutls-8.dll",
    "libngtcp2_crypto_ossl.dll",
    "libngtcp2_crypto_ossl-0.dll",
    "libngtcp2-16.dll",
    "libpsl-5.dll",
    "libssh2-1.dll",
    "libssl-3-x64.dll",
    "libunistring-5.dll",
    "libzstd.dll",
    "zlib1.dll",
    "xgameruntime.dll",
    "xgameruntime.dll.threading",
    "windows.ui.core.textinput.dll",
    "windows.devices.enumeration.dll",
];

/// The Direct3D DLLs Proton's setup puts in both system folders in place of Wine's own: DXVK's and vkd3d-proton's,
/// by engine subfolder. The game's overrides load them as native only, so a Wine-built copy in their place (which a
/// refresh of the engine's DLLs, or anything else that treats them as Wine's, leaves) is not loaded at all and the
/// game finds no `dxgi.dll`.
pub const GRAPHICS_DLLS: [(&str, &[&str]); 2] =
    [("dxvk", &["d3d11", "d3d10core", "d3d9", "dxgi"]), ("vkd3d-proton", &["d3d12", "d3d12core"])];

/// The NVAPI DLLs Proton's setup adds when NVAPI is on (also not Wine's).
pub const NVAPI_DLLS: [&str; 3] = ["nvapi64", "nvofapi64", "nvapi"];

/// Whether every DLL of [`GRAPHICS_DLLS`] in the prefix's system folders is a native build, not a Wine one. A few
/// small reads: the warm start checks this before it skips the full pass.
fn graphics_dlls_in_place(drive: &Path) -> bool {
    ["system32", "syswow64"].iter().all(|dir| {
        GRAPHICS_DLLS.iter().flat_map(|(_, names)| names.iter()).all(|name| {
            let path = drive.join("windows").join(dir).join(format!("{name}.dll"));
            path.is_file() && !builtin(&path)
        })
    })
}

/// Hold the prefix's exclusive idle lease and engine lease before calling any
/// setup functions. Package C owns registry edits after wineboot + server wait.
/// Without a bundled template, call native headless Wineboot once before this
/// function and again after it. Never write a template into the engine itself.
pub fn prepare(engine: &Engine, prefix: &Path, nvapi: bool) -> Result<()> {
    if !prefix.is_absolute() || prefix.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(invalid("Prefix path must be absolute without parent traversal"));
    }
    if engine.revision() != super::pins::ENGINE.revision {
        return Err(invalid("Changed engine revision requires verified reinstall"));
    }
    for relative in [
        "",
        "drive_c",
        "drive_c/windows",
        "drive_c/windows/system32",
        "drive_c/windows/syswow64",
        "drive_c/windows/Fonts",
        "drive_c/users",
        "drive_c/users/steamuser",
        "dosdevices",
    ] {
        let path = if relative.is_empty() { prefix.to_path_buf() } else { prefix.join(relative) };
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink() || !m.is_dir()) {
            return Err(invalid("Prefix setup refuses a linked or invalid system directory"));
        }
        fs::create_dir_all(path).map_err(io)?;
    }
    for name in ["system.reg", "user.reg", "userdef.reg", ".bf-proton-files.json"] {
        if fs::symlink_metadata(prefix.join(name)).is_ok_and(|m| m.is_symlink()) {
            return Err(invalid("Prefix setup refuses linked hives or tracking files"));
        }
    }
    let had_hives = prefix.join("system.reg").is_file();
    let template = engine.root().join("files/share/default_pfx");
    if !prefix.join("system.reg").exists() && template.is_dir() {
        copy_template(&template, prefix, engine)?;
        seed_machine_guid(&prefix.join("system.reg"))?;
    }
    migrate_user_paths(prefix)?;
    if let Some(login) = login_name() {
        link_login_user(prefix, &login)?;
    }
    let drive = prefix.join("drive_c");
    fs::create_dir_all(&drive).map_err(io)?;
    fs::create_dir_all(prefix.join("dosdevices")).map_err(io)?;
    link_preserve(&prefix.join("dosdevices/c:"), Path::new("../drive_c"))?;
    link_preserve(&prefix.join("dosdevices/z:"), Path::new("/"))?;
    let revision = format!("{}:{}:nvapi={nvapi}", engine.revision(), super::pins::ENGINE.sha256);
    let revision_path = prefix.join(".bf-proton-revision");
    if had_hives
        && fs::read_to_string(&revision_path).is_ok_and(|old| old == revision)
        && ["xgameruntime.dll", "xgameruntime.dll.threading", "cryptbase.dll"]
            .iter()
            .all(|f| drive.join("windows/system32").join(f).is_file())
        && graphics_dlls_in_place(&drive)
    {
        return Ok(());
    }
    let track_path = prefix.join(".bf-proton-files.json");
    let mut tracked: BTreeMap<String, String> = if track_path.exists() {
        serde_json::from_slice(&fs::read(&track_path).map_err(io)?).map_err(io)?
    } else {
        BTreeMap::new()
    };
    let lib = engine.root().join("files/lib/wine");
    for (arch, dir) in [("x86_64-windows", "system32"), ("i386-windows", "syswow64")] {
        let dst = drive.join("windows").join(dir);
        fs::create_dir_all(&dst).map_err(io)?;
        for entry in fs::read_dir(lib.join(arch)).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".dll") || !entry.path().is_file() {
                continue;
            }
            let target = dst.join(&name);
            // This includes the full Proton redist set. Materialize other
            // builtins too: Wineboot must never write through engine links.
            install_file(&entry.path(), &target, prefix, &mut tracked, false)?;
        }
        for (sub, names) in GRAPHICS_DLLS {
            for name in names {
                let source = lib.join(sub).join(arch).join(if sub == "vkd3d-proton" {
                    format!("{name}.dll.bol-ext-dgc")
                } else {
                    format!("{name}.dll")
                });
                install_file(&source, &dst.join(format!("{name}.dll")), prefix, &mut tracked, true)?;
            }
        }
        for name in ["icuin68", "icuuc68", "icudt68"] {
            let source = lib.join("icu").join(arch).join(format!("{name}.dll"));
            if source.exists() {
                link_preserve(&dst.join(format!("{name}.dll")), &source)?;
            }
        }
        if nvapi {
            let names: &[&str] = if arch == "x86_64-windows" { &["nvapi64", "nvofapi64"] } else { &["nvapi"] };
            for name in names {
                install_file(
                    &lib.join("nvapi").join(arch).join(format!("{name}.dll")),
                    &dst.join(format!("{name}.dll")),
                    prefix,
                    &mut tracked,
                    true,
                )?;
            }
        }
    }
    for arch_dir in ["system32", "syswow64"] {
        let arch = if arch_dir == "system32" { "x86_64-windows" } else { "i386-windows" };
        install_file(
            &lib.join(arch).join("cryptbase.dll"),
            &drive.join("windows").join(arch_dir).join("cryptbase.dll"),
            prefix,
            &mut tracked,
            true,
        )?;
    }
    for name in PAYLOAD {
        let source = lib.join("x86_64-windows").join(name);
        if source.is_file() {
            install_file(&source, &drive.join("windows/system32").join(name), prefix, &mut tracked, true)?;
        } else if name.starts_with("xgameruntime") {
            return Err(invalid("Missing pinned GDK payload"));
        }
    }
    let fonts = drive.join("windows/Fonts");
    fs::create_dir_all(&fonts).map_err(io)?;
    for dir in ["files/share/fonts", "files/share/wine/fonts"] {
        for entry in fs::read_dir(engine.root().join(dir)).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name();
            if matches!(entry.path().extension().and_then(|s| s.to_str()), Some("ttf" | "ttc")) {
                link_preserve(&fonts.join(name), &entry.path())?;
            }
        }
    }
    let stamp = fs::metadata(engine.root().join("files/share/wine/wine.inf"))
        .map_err(io)?
        .modified()
        .map_err(io)?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io)?
        .as_secs();
    atomic_file(&prefix.join(".update-timestamp"), stamp.to_string().as_bytes())?;
    atomic_file(&track_path, &serde_json::to_vec(&tracked).map_err(io)?)?;
    atomic_file(&revision_path, revision.as_bytes())?;
    Ok(())
}

/// Idle-only repair requests explicitly invalidate the cheap warm-launch cache.
pub fn invalidate(prefix: &Path) -> Result<()> {
    match fs::remove_file(prefix.join(".bf-proton-revision")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
    }
}
fn link_preserve(dst: &Path, source: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(dst) {
        if !meta.is_symlink() {
            return Ok(());
        }
        fs::remove_file(dst).map_err(io)?;
    }
    symlink(source, dst).map_err(io)
}
fn builtin(path: &Path) -> bool {
    use std::io::Read;
    let mut b = [0; 96];
    fs::File::open(path).and_then(|mut f| f.read_exact(&mut b)).is_ok()
        && b.windows(b"Wine builtin DLL".len()).any(|s| s == b"Wine builtin DLL")
}
fn install_file(
    src: &Path,
    dst: &Path,
    prefix: &Path,
    tracked: &mut BTreeMap<String, String>,
    required: bool,
) -> Result<()> {
    if !src.is_file() {
        if required {
            return Err(invalid(format!("Required engine DLL missing: {}", src.display())));
        }
        return Ok(());
    }
    let key = dst.strip_prefix(prefix).map_err(io)?.display().to_string();
    if dst.exists() && !builtin(dst) {
        let actual = artifact::sha256(dst)?;
        if tracked.get(&key) != Some(&actual) {
            // A native payload copied by the immutable template is managed too.
            // Adopt only identical reviewed bytes; preserve different user DLLs.
            if actual == artifact::sha256(src)? {
                tracked.insert(key, actual);
            }
            return Ok(());
        }
    }
    if fs::symlink_metadata(dst).is_ok() {
        fs::remove_file(dst).map_err(io)?;
    }
    copy_owned(src, dst)?;
    tracked.insert(key, artifact::sha256(dst)?);
    Ok(())
}
fn copy_template(src: &Path, dst: &Path, engine: &Engine) -> Result<()> {
    copy_template_at(src, dst, engine, src, dst)
}
fn copy_template_at(src: &Path, dst: &Path, engine: &Engine, source_base: &Path, dest_base: &Path) -> Result<()> {
    if fs::symlink_metadata(dst).is_ok_and(|m| m.is_symlink()) {
        return Err(invalid("Linked template destination directory"));
    }
    fs::create_dir_all(dst).map_err(io)?;
    for entry in fs::read_dir(src).map_err(io)? {
        let entry = entry.map_err(io)?;
        let target = dst.join(entry.file_name());
        let ty = entry.file_type().map_err(io)?;
        if ty.is_dir() {
            copy_template_at(&entry.path(), &target, engine, source_base, dest_base)?;
        } else if ty.is_symlink() {
            let resolved = fs::canonicalize(entry.path()).map_err(io)?;
            if !resolved.starts_with(engine.root()) {
                return Err(invalid("Template link outside owned engine"));
            }
            if fs::symlink_metadata(&target).is_err() {
                let remapped = if let Ok(relative) = resolved.strip_prefix(source_base) {
                    dest_base.join(relative)
                } else {
                    resolved
                };
                if remapped.starts_with(dest_base) {
                    symlink(remapped, target).map_err(io)?;
                } else {
                    // Wineboot's setupapi pass can overwrite builtin files through
                    // links. Never expose the immutable engine to those writes.
                    copy_owned(&remapped, &target)?;
                }
            }
        } else if !target.exists() {
            copy_owned(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn copy_owned(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::copy(src, dst).map_err(io)?;
    let mode = fs::metadata(dst).map_err(io)?.permissions().mode();
    fs::set_permissions(dst, fs::Permissions::from_mode((mode & 0o777) | 0o200)).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_user_files_survive_and_managed_files_update() {
        let dir = crate::testutil::scratch("b-proton");
        let prefix = dir.join("pfx");
        fs::create_dir_all(&prefix).unwrap();
        let source = dir.join("source.dll");
        let dest = prefix.join("native.dll");
        fs::write(&source, b"version one").unwrap();
        let mut track = BTreeMap::new();
        let template_native = prefix.join("template-native.dll");
        fs::copy(&source, &template_native).unwrap();
        install_file(&source, &template_native, &prefix, &mut track, true).unwrap();
        install_file(&source, &dest, &prefix, &mut track, true).unwrap();
        fs::write(&source, b"version two").unwrap();
        install_file(&source, &dest, &prefix, &mut track, true).unwrap();
        install_file(&source, &template_native, &prefix, &mut track, true).unwrap();
        assert_eq!(fs::read(&template_native).unwrap(), b"version two");
        assert_eq!(fs::read(&dest).unwrap(), b"version two");
        fs::write(&dest, b"user replacement").unwrap();
        install_file(&source, &dest, &prefix, &mut track, true).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"user replacement");
        link_preserve(&dest, &source).unwrap();
        assert!(!fs::symlink_metadata(&dest).unwrap().is_symlink());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn font_links_refresh_and_engine_change_requires_verification() {
        let dir = crate::testutil::scratch("b-fonts");
        let source = dir.join("font.ttf");
        fs::write(&source, b"font").unwrap();
        let link = dir.join("arial.ttf");
        link_preserve(&link, &source).unwrap();
        assert_eq!(fs::read(&link).unwrap(), b"font");
        let newer = dir.join("new.ttf");
        fs::write(&newer, b"new font").unwrap();
        link_preserve(&link, &newer).unwrap();
        assert_eq!(fs::read(&link).unwrap(), b"new font");
        let e = Engine::reviewed_as(dir.join("engine"), "unreviewed");
        assert!(prepare(&e, &dir.join("prefix"), false).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

/// Only used on a freshly copied immutable template, while the prefix is idle.
/// Registry maintenance after boot belongs to package C's full hive editor.
fn seed_machine_guid(hive: &Path) -> Result<()> {
    let text = fs::read_to_string(hive).map_err(io)?;
    if !text.starts_with("WINE REGISTRY Version 2") {
        return Err(invalid("Invalid template registry"));
    }
    let mut output = String::new();
    let mut section = false;
    let mut changed = false;
    for line in text.lines() {
        if line.starts_with('[') {
            section = line.starts_with("[Software\\\\Microsoft\\\\Cryptography]");
        }
        if section && line.starts_with("\"MachineGuid\"=") {
            if changed {
                return Err(invalid("Duplicate template MachineGuid"));
            }
            output.push_str(&format!("\"MachineGuid\"=\"{}\"\n", uuid::Uuid::new_v4()));
            changed = true;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    if !changed {
        return Err(invalid("Template MachineGuid missing"));
    }
    atomic_file(hive, output.as_bytes())
}

fn atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(io)?;
        f.write_all(bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
        drop(f);
        fs::rename(&tmp, path).map_err(io)?;
        fs::File::open(path.parent().unwrap()).and_then(|f| f.sync_all()).map_err(io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

/// The name Wine gives the Windows user: the login the session passes it as `USER` (or `LOGNAME`). None when it is
/// not a plain folder name.
fn login_name() -> Option<String> {
    ["USER", "LOGNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|n| !n.is_empty() && n != "." && n != ".." && !n.contains(['/', '\\', '\0']) && n.len() <= 255)
}

/// Make `drive_c/users/<login>` the same folder as `steamuser`, as umu does for a Proton prefix (umu_run.py
/// `setup_pfx`). This Wine names the user's folders after the login, while the game's data (worlds, options, the
/// backups of them) is kept under `steamuser`; without the link the game wrote to a folder nothing else reads.
/// A login folder that already holds something is left as it is.
fn link_login_user(prefix: &Path, login: &str) -> Result<()> {
    if login.eq_ignore_ascii_case("steamuser") {
        return Ok(());
    }
    let users = prefix.join("drive_c/users");
    let steamuser = users.join("steamuser");
    let user = users.join(login);
    if fs::symlink_metadata(&user).is_err() && fs::symlink_metadata(&steamuser).is_ok_and(|m| m.is_dir()) {
        symlink("steamuser", &user).map_err(io)?;
    }
    Ok(())
}

fn migrate_user_paths(prefix: &Path) -> Result<()> {
    let user = prefix.join("drive_c/users/steamuser");
    for (old, new, link) in [
        ("Local Settings/Application Data", "AppData/Local", "../AppData/Local"),
        ("Application Data", "AppData/Roaming", "./AppData/Roaming"),
        ("My Documents", "Documents", "./Documents"),
    ] {
        let old_path = user.join(old);
        let new_path = user.join(new);
        if let Ok(target) = fs::read_link(&new_path) {
            if target.ends_with(old) {
                fs::remove_file(&new_path).map_err(io)?;
            } else {
                return Err(invalid("User path migration refuses an unexpected target link"));
            }
        }
        if fs::symlink_metadata(&old_path).is_ok_and(|m| !m.is_symlink()) {
            let backup = old_path.with_file_name(format!("{} BACKUP", old_path.file_name().unwrap().to_string_lossy()));
            if backup.exists() {
                return Err(invalid("User path migration backup already exists"));
            }
            merge_user_dir(&old_path, &new_path)?;
            fs::rename(&old_path, backup).map_err(io)?;
        }
        fs::create_dir_all(&new_path).map_err(io)?;
        fs::create_dir_all(old_path.parent().unwrap()).map_err(io)?;
        link_preserve(&old_path, Path::new(link))?;
    }
    Ok(())
}
fn merge_user_dir(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).map_err(io)?;
    for entry in fs::read_dir(src).map_err(io)? {
        let entry = entry.map_err(io)?;
        let target = dst.join(entry.file_name());
        let kind = entry.file_type().map_err(io)?;
        if kind.is_symlink() {
            return Err(invalid("User path migration requires review of symbolic links"));
        }
        if kind.is_dir() {
            merge_user_dir(&entry.path(), &target)?;
        } else if target.exists() {
            if artifact::sha256(&entry.path())? != artifact::sha256(&target)? {
                return Err(invalid("User path migration found conflicting files; originals retained"));
            }
        } else {
            fs::copy(entry.path(), target).map_err(io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn the_login_folder_is_steamuser_unless_it_already_holds_something() {
        let dir = crate::testutil::scratch("b-login");
        let users = dir.join("drive_c/users");
        fs::create_dir_all(users.join("steamuser/AppData/Roaming")).unwrap();
        link_login_user(&dir, "player").unwrap();
        assert_eq!(fs::read_link(users.join("player")).unwrap(), Path::new("steamuser"));
        assert!(users.join("player/AppData/Roaming").is_dir(), "the login's folders are steamuser's");
        link_login_user(&dir, "player").unwrap();
        link_login_user(&dir, "steamuser").unwrap();
        assert!(fs::symlink_metadata(users.join("steamuser")).unwrap().is_dir());
        fs::create_dir_all(users.join("other/AppData")).unwrap();
        link_login_user(&dir, "other").unwrap();
        assert!(!fs::symlink_metadata(users.join("other")).unwrap().is_symlink(), "a real folder is kept");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn unique_template_guid_preserved_on_repair_and_user_aliases_keep_data() {
        let dir = crate::testutil::scratch("b-guid");
        let hive = dir.join("system.reg");
        let original =
            "WINE REGISTRY Version 2\n\n[Software\\\\Microsoft\\\\Cryptography] 0\n\"MachineGuid\"=\"template-guid\"\n";
        fs::write(&hive, original).unwrap();
        seed_machine_guid(&hive).unwrap();
        let first = fs::read_to_string(&hive).unwrap();
        assert!(!first.contains("template-guid"));
        fs::write(&hive, original).unwrap();
        seed_machine_guid(&hive).unwrap();
        assert_ne!(fs::read_to_string(&hive).unwrap(), first);
        let old = dir.join("drive_c/users/steamuser/Application Data");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("world"), b"world data").unwrap();
        migrate_user_paths(&dir).unwrap();
        migrate_user_paths(&dir).unwrap();
        assert_eq!(fs::read(old.join("world")).unwrap(), b"world data");
        assert!(old.with_file_name("Application Data BACKUP").join("world").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;
    #[test]
    fn template_links_are_remapped_and_engine_files_are_materialized() {
        let dir = crate::testutil::scratch("b-template");
        let engine = Engine::synthetic(dir.join("engine"));
        let template = engine.root().join("files/share/default_pfx");
        let pe = engine.root().join("files/lib/wine/x86_64-windows/kernel32.dll");
        fs::create_dir_all(pe.parent().unwrap()).unwrap();
        fs::write(&pe, b"immutable engine DLL").unwrap();
        let system32 = template.join("drive_c/windows/system32");
        fs::create_dir_all(&system32).unwrap();
        symlink(&pe, system32.join("kernel32.dll")).unwrap();
        symlink("system32", template.join("drive_c/windows/alias")).unwrap();
        let prefix = dir.join("prefix");
        copy_template(&template, &prefix, &engine).unwrap();
        let owned = prefix.join("drive_c/windows/system32/kernel32.dll");
        assert!(!fs::symlink_metadata(&owned).unwrap().is_symlink());
        fs::write(&owned, b"wineboot update").unwrap();
        assert_eq!(fs::read(pe).unwrap(), b"immutable engine DLL");
        assert_eq!(
            fs::canonicalize(prefix.join("drive_c/windows/alias")).unwrap(),
            prefix.join("drive_c/windows/system32")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn linked_prefix_or_system_directory_cannot_write_outside_owned_prefix() {
        let dir = crate::testutil::scratch("b-prefix-path");
        let outside = dir.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"user data").unwrap();
        let prefix = dir.join("prefix");
        symlink(&outside, &prefix).unwrap();
        let engine = Engine::synthetic(dir.join("engine"));
        assert!(prepare(&engine, &prefix, false).is_err());
        fs::remove_file(&prefix).unwrap();
        fs::create_dir_all(prefix.join("drive_c/windows")).unwrap();
        symlink(&outside, prefix.join("drive_c/windows/system32")).unwrap();
        assert!(prepare(&engine, &prefix, false).is_err());
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"user data");
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn warm_setup_skips_bulk_payload_work_until_repair_invalidates_it() {
        let dir = crate::testutil::scratch("b-warm");
        let e = Engine::synthetic(dir.join("engine"));
        let p = dir.join("prefix");
        for arch in ["x86_64-windows", "i386-windows"] {
            for (sub, names) in [
                ("", vec!["cryptbase.dll"]),
                ("dxvk", vec!["d3d11.dll", "d3d10core.dll", "d3d9.dll", "dxgi.dll"]),
                ("vkd3d-proton", vec!["d3d12.dll.bol-ext-dgc", "d3d12core.dll.bol-ext-dgc"]),
            ] {
                let d = if sub.is_empty() {
                    e.root().join("files/lib/wine").join(arch)
                } else {
                    e.root().join("files/lib/wine").join(sub).join(arch)
                };
                fs::create_dir_all(&d).unwrap();
                for name in names {
                    fs::write(d.join(name), b"synthetic payload").unwrap();
                }
            }
        }
        for name in ["xgameruntime.dll", "xgameruntime.dll.threading"] {
            fs::write(e.root().join("files/lib/wine/x86_64-windows").join(name), b"synthetic payload").unwrap();
        }
        for d in ["files/share/fonts", "files/share/wine/fonts"] {
            fs::create_dir_all(e.root().join(d)).unwrap();
        }
        fs::write(e.root().join("files/share/wine/wine.inf"), b"synthetic inf").unwrap();
        prepare(&e, &p, false).unwrap();
        fs::write(p.join("system.reg"), b"synthetic completed boot").unwrap();
        fs::rename(e.root().join("files/lib/wine"), e.root().join("files/lib/not-scanned")).unwrap();
        prepare(&e, &p, false).unwrap();
        invalidate(&p).unwrap();
        assert!(prepare(&e, &p, false).is_err());
        // A prefix last prepared by another engine revision (or NVAPI policy) gets the full pass again, too.
        fs::rename(e.root().join("files/lib/not-scanned"), e.root().join("files/lib/wine")).unwrap();
        prepare(&e, &p, false).unwrap();
        fs::write(p.join(".bf-proton-revision"), "wow64-archs-native20:old-archive:nvapi=false").unwrap();
        fs::rename(e.root().join("files/lib/wine"), e.root().join("files/lib/not-scanned")).unwrap();
        assert!(prepare(&e, &p, false).is_err());
        fs::rename(e.root().join("files/lib/not-scanned"), e.root().join("files/lib/wine")).unwrap();
        prepare(&e, &p, false).unwrap();
        fs::rename(e.root().join("files/lib/wine"), e.root().join("files/lib/not-scanned")).unwrap();
        assert!(prepare(&e, &p, true).is_err(), "turning NVAPI on is a different prefix revision");
        // A Wine-built DLL where DXVK's belongs (what an engine refresh used to leave) is not warm: the full pass
        // puts DXVK's back.
        fs::rename(e.root().join("files/lib/not-scanned"), e.root().join("files/lib/wine")).unwrap();
        prepare(&e, &p, false).unwrap();
        let dxgi = p.join("drive_c/windows/syswow64/dxgi.dll");
        let mut wine_built = vec![0u8; 0x40];
        wine_built.extend_from_slice(b"Wine builtin DLL");
        wine_built.resize(0x100, 0);
        fs::write(&dxgi, &wine_built).unwrap();
        prepare(&e, &p, false).unwrap();
        assert_eq!(fs::read(&dxgi).unwrap(), b"synthetic payload");
        fs::remove_dir_all(dir).unwrap();
    }
}
