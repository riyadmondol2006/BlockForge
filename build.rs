//! Finds the translation files in `assets/lang` (`de-DE.po`, the Java edition's naming) and lists them for
//! `src/i18n.rs` to compile into the program. With no files the list is empty and only English is offered.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    embed_windows_resources();
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let dir = root.join("assets").join("lang");
    println!("cargo:rerun-if-changed=assets/lang");
    println!("cargo:rerun-if-changed=build.rs");

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "po")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                // de_DE.po and de-DE.po both mean German (Germany)
                files.push((stem.replace('_', "-"), path));
            }
        }
    }
    files.sort();

    let mut code = String::from("pub static EMBEDDED_LANGUAGES: &[(&str, &[u8])] = &[\n");
    for (tag, path) in &files {
        let escaped = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(code, "    (\"{tag}\", include_bytes!(\"{escaped}\")),");
        println!("cargo:rerun-if-changed={}", path.display());
    }
    code.push_str("];\n");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("embedded_languages.rs");
    std::fs::write(out, code).expect("writing the list of translation files");
}

/// Puts the icon and the version information (what Explorer shows under Properties > Details, and what the installer
/// and Task Manager read) into `blockforge.exe`. Does nothing for other systems. It needs the Windows resource
/// compiler (`rc.exe` from the Windows SDK, which Visual Studio installs, or `windres` for the gnu target), so a
/// Windows target has to be built on Windows.
#[cfg(windows)]
fn embed_windows_resources() {
    println!("cargo:rerun-if-changed=packaging/windows/blockforge.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut res = winresource::WindowsResource::new();
    res.set_icon("packaging/windows/blockforge.ico")
        .set_language(0x0409) // English (United States)
        .set("ProductName", "BlockForge")
        .set("FileDescription", "BlockForge - a fast Minecraft launcher")
        .set("CompanyName", "BlockForge contributors")
        .set("LegalCopyright", "Copyright ATLauncher and BlockForge contributors, GPL-3.0")
        .set("OriginalFilename", "blockforge.exe")
        .set("InternalName", "blockforge")
        .set("ProductVersion", &version)
        .set("FileVersion", &version);
    if let Err(e) = res.compile() {
        // a missing icon or resource compiler must stop the build: an exe without them looks broken to people
        panic!("embedding the Windows icon and version information failed: {e}");
    }
}

#[cfg(not(windows))]
fn embed_windows_resources() {}
