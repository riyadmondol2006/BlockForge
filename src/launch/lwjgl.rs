//! Running Minecraft on computers its own libraries don't cover: ARM processors (a Raspberry Pi, Windows or Linux on
//! ARM), where Mojang ships no LWJGL natives for older versions. The Java edition has a list of replacement LWJGL
//! builds (`configs/json/lwjgl.json`, downloaded with the launcher's other files) and swaps them in when the launcher's
//! downloaded settings turn `useLwjglReplacement` on and "ARM support" is on in the settings. Also here: dropping the
//! game's own GLFW or OpenAL so the system's are used (`useSystemGlfw`, `useSystemOpenAl`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::launch::legacy::match_whole_version;
use crate::launch::settings::remote_config_bool;
use crate::model::{Artifact, Library, VersionJson};
use crate::paths::Dirs;
use crate::platform;

/// One downloadable LWJGL file.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct LwjglLibrary {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct LwjglVersion {
    #[serde(default)]
    pub version: String,
    /// Library name ("lwjgl", "lwjgl-glfw") then classifier ("linux-arm64", or "*" for the main jar) to file.
    #[serde(default)]
    pub libraries: HashMap<String, HashMap<String, LwjglLibrary>>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct LwjglMajor {
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub versions: Vec<LwjglVersion>,
}

/// The contents of `lwjgl.json`.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct LwjglVersions {
    /// The Minecraft versions that use LWJGL 2.
    #[serde(default)]
    pub legacy_lwjgl_versions: Vec<String>,
    #[serde(default)]
    pub versions: Vec<LwjglMajor>,
}

impl LwjglVersions {
    /// Reads `configs/json/lwjgl.json`; nothing at all when it isn't there (then no replacement happens).
    pub fn load(dirs: &Dirs) -> LwjglVersions {
        std::fs::read(dirs.configs.join("json").join("lwjgl.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// If this Minecraft version uses LWJGL 2.
    pub fn uses_legacy_lwjgl(&self, minecraft_version: &str) -> bool {
        self.legacy_lwjgl_versions.iter().any(|v| v == minecraft_version)
    }

    /// The replacement LWJGL 2 natives for this computer (a file to unpack, not a jar for the class path).
    pub fn legacy_lwjgl_library(&self, classifier: &str) -> Option<LwjglLibrary> {
        let major = self.versions.iter().find(|v| v.version == 2)?;
        major.versions.first()?.libraries.get("lwjgl")?.get(classifier).cloned()
    }
}

/// The name this computer has in the replacement list: `linux-arm64`, `windows-arm64`, `macos-arm64`, `linux-arm32`,
/// `windows-x86`, ...
pub fn classifier() -> String {
    let os = match platform::os() {
        platform::Os::Windows => "windows",
        platform::Os::Mac => "macos",
        platform::Os::Linux => "linux",
    };
    match platform::arch() {
        "aarch64" => format!("{os}-arm64"),
        a if a.starts_with("arm") => format!("{os}-arm32"),
        "x86" => format!("{os}-x86"),
        _ => os.to_string(),
    }
}

pub fn is_arm() -> bool {
    let a = platform::arch();
    a.starts_with("arm") || a == "aarch64"
}

fn is_mac_arm() -> bool {
    platform::os() == platform::Os::Mac && is_arm()
}

/// What decides whether the replacements are used for a launch.
#[derive(Clone, Debug)]
pub struct LwjglPlan {
    /// The launcher's downloaded settings allow replacing LWJGL at all (`useLwjglReplacement`).
    pub allowed_remotely: bool,
    /// "ARM support" in the settings.
    pub arm_support: bool,
    pub versions: LwjglVersions,
}

impl LwjglPlan {
    pub fn new(dirs: &Dirs, arm_support: bool) -> LwjglPlan {
        LwjglPlan {
            allowed_remotely: remote_config_bool(dirs, "useLwjglReplacement", false),
            arm_support,
            versions: LwjglVersions::load(dirs),
        }
    }

    /// LWJGL 2 is only replaced on Linux on ARM.
    pub fn should_use_legacy_lwjgl(&self, minecraft_version: &str) -> bool {
        self.allowed_remotely
            && self.arm_support
            && self.versions.uses_legacy_lwjgl(minecraft_version)
            && is_arm()
            && platform::os() == platform::Os::Linux
    }

    /// LWJGL 3 is replaced on ARM, except on a Mac with an Apple chip when Minecraft already provides natives for it.
    pub fn should_replace_lwjgl3(&self, version: &VersionJson) -> bool {
        self.allowed_remotely
            && self.arm_support
            && !self.versions.uses_legacy_lwjgl(version.game_version())
            && is_arm()
            && (!is_mac_arm()
                || !version
                    .libraries
                    .iter()
                    .any(|l| l.name.starts_with("org.lwjgl:lwjgl") && l.name.ends_with("natives-macos-arm64")))
    }

    /// The version with every LWJGL 3 library swapped for its replacement, when that applies. The rest of the program
    /// (downloads, class path, natives) then simply works from the changed list.
    pub fn apply(&self, version: &VersionJson) -> VersionJson {
        if !self.should_replace_lwjgl3(version) {
            return version.clone();
        }
        let mut out = version.clone();
        out.libraries = version.libraries.iter().map(|l| self.replacement_lwjgl3_library(l)).collect();
        out
    }

    /// Swaps one LWJGL 3 library for the replacement build that has ARM natives. Anything that is not LWJGL 3, or has
    /// no replacement, comes back as it was.
    pub fn replacement_lwjgl3_library(&self, library: &Library) -> Library {
        if !library.name.contains("lwjgl") || !library.name.contains(":3") {
            return library.clone();
        }
        let Some(major) = self.versions.versions.iter().find(|v| v.version == 3) else { return library.clone() };
        let mut parts = library.name.split(':');
        let (Some(_group), Some(library_name), Some(lwjgl_version)) = (parts.next(), parts.next(), parts.next()) else {
            return library.clone();
        };
        // use 3.3.1 at a minimum, else match to the same version as Minecraft
        let wanted = if match_whole_version(lwjgl_version, "3.3.1", true) { lwjgl_version } else { "3.3.1" };
        let Some(entry) =
            major.versions.iter().find(|v| v.version == wanted).and_then(|v| v.libraries.get(library_name))
        else {
            return library.clone();
        };
        let classifier = classifier();
        let mut out = library.clone();
        let no_natives = library.natives.as_ref().is_none_or(|n| n.is_empty());
        let same_file =
            |theirs: &LwjglLibrary| library.artifact().and_then(|a| a.sha1).is_some_and(|s| s == theirs.sha1);
        let set = |target: &mut Artifact, theirs: &LwjglLibrary| {
            target.path = Some(theirs.path.clone());
            target.sha1 = Some(theirs.sha1.clone());
            target.url = theirs.url.clone();
            target.size = Some(theirs.size);
        };

        // the base library (1.19-pre1 and onwards have no natives classifiers; this is the jar without natives)
        if no_natives && !library.name.contains(":natives-") {
            let Some(theirs) = entry.get("*") else { return library.clone() };
            if same_file(theirs) {
                return library.clone();
            }
            let downloads = out.downloads.get_or_insert_with(Default::default);
            set(downloads.artifact.get_or_insert_with(Default::default), theirs);
            out.name = theirs.name.clone();
        }
        // 1.19-pre1 format natives: their own library entry per platform
        if no_natives && library.name.contains(":natives-") {
            let Some(theirs) = entry.get(&classifier) else { return library.clone() };
            if same_file(theirs) {
                return library.clone();
            }
            let downloads = out.downloads.get_or_insert_with(Default::default);
            set(downloads.artifact.get_or_insert_with(Default::default), theirs);
            out.name = theirs.name.clone();
        }
        // the old natives format: a classifier jar chosen by the `natives` table
        if !no_natives {
            let Some(theirs) = entry.get(&classifier) else { return library.clone() };
            if same_file(theirs) {
                return library.clone();
            }
            if let Some(key) = library.natives.as_ref().and_then(|n| n.get(platform::os_rule_name())) {
                let key = key.replace("${arch}", if cfg!(target_pointer_width = "32") { "32" } else { "64" });
                if let Some(download) =
                    out.downloads.as_mut().and_then(|d| d.classifiers.as_mut()).and_then(|c| c.get_mut(&key))
                {
                    set(download, theirs);
                }
            }
        }
        out
    }
}

/// Unpacks the old-natives jars of a version into `dest` the way the Java edition does: every file goes to the top of
/// the folder (no sub-folders), except `META-INF`, which stays; folders and the library's own exclude list are left
/// out. With `use_system_glfw` or `use_system_openal` the matching libraries are skipped so the system's are found.
/// Returns the names of the libraries that were skipped on purpose.
pub fn extract_natives(
    dirs: &Dirs,
    version: &VersionJson,
    dest: &Path,
    use_system_glfw: bool,
    use_system_openal: bool,
) -> Result<Vec<String>> {
    let features = crate::model::Features::default();
    std::fs::create_dir_all(dest)?;
    let mut skipped = Vec::new();
    for lib in version.active_libraries(&features) {
        let Some(native) = lib.native() else { continue };
        if lib.name.contains("glfw") && use_system_glfw {
            skipped.push(format!("useSystemGlfw was enabled, not using glfw natives from Minecraft ({})", lib.name));
            continue;
        }
        if lib.name.contains("openal") && use_system_openal {
            skipped
                .push(format!("useSystemOpenAl was enabled, not using openal natives from Minecraft ({})", lib.name));
            continue;
        }
        let Some(path) = native.path else { continue };
        let jar = dirs.libraries.join(path);
        let file = std::fs::File::open(&jar).with_context(|| format!("natives jar {} is missing", jar.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let excludes = lib.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_default();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let Ok(name) = entry.name().map(|n| n.to_string()) else { continue };
            let Some(target) = native_target(&name, &excludes) else { continue };
            let out = dest.join(&target);
            // never write outside the folder, whatever the jar says
            if !out.starts_with(dest) || target.contains("..") {
                continue;
            }
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
        }
    }
    Ok(skipped)
}

/// Where a file of a natives jar goes inside the natives folder, or None to skip it.
pub fn native_target(name: &str, excludes: &[String]) -> Option<String> {
    if excludes.iter().any(|ex| name.starts_with(ex.as_str())) {
        return None;
    }
    // keep META-INF folder as per normal
    if name.starts_with("META-INF") {
        return if name.ends_with('/') { None } else { Some(name.to_string()) };
    }
    // don't extract folders
    if name.ends_with('/') {
        return None;
    }
    // if it has a / then extract just to root
    Some(name.rsplit('/').next().unwrap_or(name).to_string())
}

/// Downloads LWJGL 2's replacement natives (if needed) and unpacks them into `dest`, for `-Dorg.lwjgl.librarypath`.
pub async fn prepare_legacy_lwjgl(
    dirs: &Dirs,
    library: &LwjglLibrary,
    dest: &Path,
    progress: &std::sync::Arc<crate::install::Progress>,
) -> Result<()> {
    let jar: PathBuf = dirs.libraries.join(&library.path);
    let job = crate::install::Job::new(&library.url, &jar).checked(Some(&library.sha1), Some(library.size));
    crate::install::download_all(vec![job], progress, false).await?;
    std::fs::create_dir_all(dest)?;
    let file = std::fs::File::open(&jar).with_context(|| format!("{} is missing", jar.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"legacyLwjglVersions":["1.8.9","1.12.2"],"versions":[
        {"version":2,"versions":[{"version":"2","libraries":{"lwjgl":{"linux-arm64":{"url":"https://x/lwjgl2.jar","path":"com/atlauncher/lwjgl2-arm-fix/2.0.0/a.jar","size":10,"sha1":"aa"}}}}]},
        {"version":3,"versions":[{"version":"3.3.1","libraries":{"lwjgl":{
            "linux-arm64":{"url":"https://x/n.jar","path":"org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux-arm64.jar","size":5,"sha1":"bb","name":"org.lwjgl:lwjgl:3.3.1:natives-linux-arm64"},
            "*":{"url":"https://x/b.jar","path":"org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar","size":6,"sha1":"cc","name":"org.lwjgl:lwjgl:3.3.1"}}}}]}]}"#;

    fn plan() -> LwjglPlan {
        LwjglPlan { allowed_remotely: true, arm_support: true, versions: serde_json::from_str(LIST).unwrap() }
    }

    fn lib(json: &str) -> Library {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn reads_the_java_editions_list() {
        let v = plan().versions;
        assert!(v.uses_legacy_lwjgl("1.8.9") && !v.uses_legacy_lwjgl("1.21.1"));
        assert_eq!(v.legacy_lwjgl_library("linux-arm64").unwrap().sha1, "aa");
        assert!(v.legacy_lwjgl_library("windows-arm64").is_none());
    }

    #[test]
    fn swaps_the_base_library_and_new_style_natives() {
        let p = plan();
        let base = lib(
            r#"{"name":"org.lwjgl:lwjgl:3.3.3","downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar","sha1":"old","size":1,"url":"https://old/b.jar"}}}"#,
        );
        // a version the list doesn't have is left alone
        assert_eq!(p.replacement_lwjgl3_library(&base).name, "org.lwjgl:lwjgl:3.3.3");
        // an older one than 3.3.1 gets 3.3.1
        let older = lib(
            r#"{"name":"org.lwjgl:lwjgl:3.2.2","downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.2.2/lwjgl-3.2.2.jar","sha1":"old","size":1,"url":"https://old/b.jar"}}}"#,
        );
        let out = p.replacement_lwjgl3_library(&older);
        assert_eq!(out.name, "org.lwjgl:lwjgl:3.3.1");
        assert_eq!(out.artifact().unwrap().url, "https://x/b.jar");
        assert_eq!(out.artifact().unwrap().path.as_deref(), Some("org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar"));
        // the file already is the replacement: untouched
        let same = lib(
            r#"{"name":"org.lwjgl:lwjgl:3.3.1","downloads":{"artifact":{"path":"p","sha1":"cc","size":1,"url":"u"}}}"#,
        );
        assert_eq!(p.replacement_lwjgl3_library(&same).artifact().unwrap().url, "u");
        // other libraries and LWJGL 2 are never touched
        let other = lib(
            r#"{"name":"com.google.guava:guava:31.0","downloads":{"artifact":{"path":"p","sha1":"zz","size":1,"url":"u"}}}"#,
        );
        assert_eq!(p.replacement_lwjgl3_library(&other).name, "com.google.guava:guava:31.0");
        let two = lib(r#"{"name":"org.lwjgl.lwjgl:lwjgl:2.9.4","downloads":{}}"#);
        assert_eq!(p.replacement_lwjgl3_library(&two).name, "org.lwjgl.lwjgl:lwjgl:2.9.4");
        // a library the list doesn't know
        let unknown = lib(
            r#"{"name":"org.lwjgl:lwjgl-nanovg:3.3.3","downloads":{"artifact":{"path":"p","sha1":"zz","size":1,"url":"u"}}}"#,
        );
        assert_eq!(p.replacement_lwjgl3_library(&unknown).name, "org.lwjgl:lwjgl-nanovg:3.3.3");
    }

    #[test]
    fn natives_go_to_the_top_of_the_folder() {
        let ex = vec!["META-INF/".to_string()];
        assert_eq!(native_target("liblwjgl.so", &[]).as_deref(), Some("liblwjgl.so"));
        assert_eq!(native_target("linux/x64/liblwjgl.so", &[]).as_deref(), Some("liblwjgl.so"));
        assert_eq!(native_target("native/", &[]), None);
        assert_eq!(native_target("META-INF/MANIFEST.MF", &[]).as_deref(), Some("META-INF/MANIFEST.MF"));
        assert_eq!(native_target("META-INF/MANIFEST.MF", &ex), None);
        assert_eq!(native_target("META-INF/", &[]), None);
    }

    #[test]
    fn replacement_needs_the_remote_switch_and_arm() {
        let mut p = plan();
        let v: VersionJson = serde_json::from_str(r#"{"id":"1.20.1","libraries":[]}"#).unwrap();
        // on this (non-ARM) computer nothing is replaced, whatever the switches say
        assert_eq!(p.should_replace_lwjgl3(&v), is_arm());
        p.allowed_remotely = false;
        assert!(!p.should_replace_lwjgl3(&v));
        assert!(!p.should_use_legacy_lwjgl("1.8.9"));
        assert_eq!(p.apply(&v).libraries.len(), 0);
    }

    #[test]
    fn classifier_names() {
        let c = classifier();
        assert!(c.starts_with("linux") || c.starts_with("windows") || c.starts_with("macos"));
    }
}
