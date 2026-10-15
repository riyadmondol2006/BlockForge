//! Which Java runs a game, and whether it is a good choice. The Java edition's `Instance.resolveJavaPath` and
//! `RuntimeResolver`: Mojang's own Java for the game's version wins when "Use Java provided by Minecraft" is on (or
//! the instance names a runtime of its own); otherwise it is the Java in the settings. The reasons and the warnings
//! come back as text so the log and the window can both say them.

use std::path::{Path, PathBuf};

use crate::java;
use crate::launch::settings::{LaunchSettings, java_arch};
use crate::model::VersionJson;
use crate::paths::Dirs;
use crate::platform;

/// The processors a Java can be built for, named the way the Java edition names them.
pub const X64: &str = "x64";
pub const ARM64: &str = "arm64";
pub const X86: &str = "x86";

/// `aarch64` -> `arm64`, `amd64` -> `x64`, `i386` -> `x86`; anything else lower-cased.
pub fn normalise_arch(arch: &str) -> String {
    match arch.to_lowercase().as_str() {
        "aarch64" | "arm64" => ARM64.into(),
        "x86_64" | "amd64" | "x64" => X64.into(),
        "x86" | "i386" | "i586" | "i686" => X86.into(),
        other => other.into(),
    }
}

/// The processor this computer has, in the same words.
pub fn host_arch() -> String {
    normalise_arch(platform::arch())
}

#[derive(Clone, Debug)]
pub struct JavaPick {
    /// The Java's folder; None means whatever `java` is on the PATH.
    pub home: Option<PathBuf>,
    /// The program to start (`javaw` on Windows when it exists, like the Java edition).
    pub binary: PathBuf,
    pub major: Option<u32>,
    pub arch: Option<String>,
    /// It is Mojang's own Java for this game.
    pub is_minecraft_runtime: bool,
    /// The Mojang runtime this game wants (`java-runtime-gamma`), when one should be used.
    pub component: Option<String>,
    /// The wanted runtime is not on the disk yet.
    pub runtime_missing: bool,
    /// Why this one, for the log.
    pub reasons: Vec<String>,
    /// What looks wrong about it.
    pub warnings: Vec<String>,
}

/// The Mojang runtime this instance would use, if it uses one: its own override, or the one the game's version names
/// (old versions that name none use `jre-legacy`, Java 8).
pub fn wanted_component(version: &VersionJson, ls: &LaunchSettings) -> Option<String> {
    if let Some(over) = &ls.java_runtime_override {
        return Some(over.clone());
    }
    if !ls.use_java_provided_by_minecraft {
        return None;
    }
    Some(
        version
            .java_version
            .as_ref()
            .map(|j| j.component.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "jre-legacy".to_string()),
    )
}

/// Picks the Java without downloading anything.
pub fn pick(dirs: &Dirs, ls: &LaunchSettings, version: &VersionJson) -> JavaPick {
    let component = wanted_component(version, ls);
    let mut reasons = Vec::new();
    let mut home: Option<PathBuf> = None;
    let mut is_runtime = false;
    let mut missing = false;
    if let Some(c) = &component {
        if java::runtime_ready(dirs, c) {
            home = Some(java::runtime_home(dirs, c));
            is_runtime = true;
            reasons.push(if ls.java_runtime_override.is_some() {
                format!("using the overridden Java runtime {c}")
            } else {
                format!("it's the Java Minecraft provides for this version ({c}), so it is the one it was tested with")
            });
        } else {
            missing = true;
        }
    }
    if home.is_none() {
        if let Some(configured) = &ls.java_path {
            let h = PathBuf::from(configured);
            if java::java_binary(&h).is_file() {
                reasons.push("you picked this Java in the settings".to_string());
                home = Some(h);
            }
        }
    }
    let binary = match &home {
        Some(h) => binary_in(h),
        None => PathBuf::from(platform::java_exe()),
    };
    if home.is_none() {
        reasons.push("no other Java was set, so the `java` on the PATH is used".to_string());
    }
    let major = home.as_deref().and_then(java::major_version);
    let arch = home.as_deref().and_then(java_arch).map(|a| normalise_arch(&a));
    let required = version.java_version.as_ref().map(|j| j.major_version).filter(|m| *m > 0);
    let warnings = check(home.is_some(), major, arch.as_deref(), required);
    JavaPick {
        home,
        binary,
        major,
        arch,
        is_minecraft_runtime: is_runtime,
        component,
        runtime_missing: missing,
        reasons,
        warnings,
    }
}

/// The program to start inside a Java folder. On Windows `javaw` (no console window) is used when it is there.
pub fn binary_in(home: &Path) -> PathBuf {
    let java = java::java_binary(home);
    if platform::os() == platform::Os::Windows {
        let w = home.join("bin").join("javaw.exe");
        if w.is_file() {
            return w;
        }
    }
    java
}

/// What looks wrong about a Java for a game that needs `required` (the Java edition's `RuntimeResolver.check`).
pub fn check(found: bool, major: Option<u32>, arch: Option<&str>, required: Option<u32>) -> Vec<String> {
    let mut w = Vec::new();
    if !found {
        // the PATH is trusted only as a last resort; nothing to say about a Java we have not looked at
        return w;
    }
    if let (Some(req), Some(have)) = (required, major) {
        if have < req {
            w.push(format!(
                "this Minecraft needs Java {req} or newer but this is Java {have}, so it will probably fail to start"
            ));
        }
    }
    let host = host_arch();
    if let Some(arch) = arch {
        if arch != host {
            if host == ARM64 && arch == X64 {
                w.push(
                    "this Java is built for Intel/AMD processors and runs through emulation on this computer, which makes Minecraft noticeably slower. An ARM Java is recommended."
                        .to_string(),
                );
            } else if arch == X86 && host != X86 {
                w.push("this is a 32-bit Java, which limits Minecraft to about 1.5 GB of memory".to_string());
            } else {
                w.push(format!("this Java is built for {arch} but this computer is {host}"));
            }
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::instances::{Instance, InstanceFile};

    fn inst(json: &str) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher = serde_json::from_str(json).unwrap();
        file.version = serde_json::from_str(
            r#"{"id":"1.21.1","javaVersion":{"component":"java-runtime-delta","majorVersion":21}}"#,
        )
        .unwrap();
        Instance { folder: "x".into(), mod_count: 0, root: "/nonexistent".into(), file }
    }

    #[test]
    fn architectures_are_normalised() {
        assert_eq!(normalise_arch("aarch64"), "arm64");
        assert_eq!(normalise_arch("amd64"), "x64");
        assert_eq!(normalise_arch("x86_64"), "x64");
        assert_eq!(normalise_arch("i386"), "x86");
        assert_eq!(normalise_arch("riscv64"), "riscv64");
    }

    #[test]
    fn warnings_match_the_java_edition() {
        let host = host_arch();
        assert!(
            check(true, Some(17), None, Some(21))[0]
                .starts_with("this Minecraft needs Java 21 or newer but this is Java 17")
        );
        assert!(check(true, Some(21), Some(&host), Some(21)).is_empty());
        assert!(check(false, Some(8), None, Some(21)).is_empty());
        let other = if host == X86 { X64 } else { X86 };
        assert!(!check(true, Some(21), Some(other), Some(8)).is_empty());
        if host == X64 {
            assert!(check(true, Some(21), Some(X86), None)[0].contains("32-bit Java"));
        }
    }

    #[test]
    fn the_runtime_component_follows_the_settings() {
        let s = Settings::default();
        let i = inst(r#"{"name":"A"}"#);
        let ls = LaunchSettings::resolve(&s, &i);
        assert_eq!(wanted_component(&i.file.version, &ls).as_deref(), Some("java-runtime-delta"));
        let off = LaunchSettings::resolve(&s, &inst(r#"{"name":"A","useJavaProvidedByMinecraft":false}"#));
        assert_eq!(wanted_component(&i.file.version, &off), None);
        // an override is used even when Minecraft's Java is switched off
        let over = LaunchSettings::resolve(
            &s,
            &inst(r#"{"name":"A","useJavaProvidedByMinecraft":false,"javaRuntimeOverride":"java-runtime-alpha"}"#),
        );
        assert_eq!(wanted_component(&i.file.version, &over).as_deref(), Some("java-runtime-alpha"));
        // a version that names no Java gets the legacy one
        let bare: VersionJson = serde_json::from_str(r#"{"id":"1.8.9"}"#).unwrap();
        assert_eq!(wanted_component(&bare, &ls).as_deref(), Some("jre-legacy"));
    }

    #[test]
    fn nothing_installed_falls_back_to_the_path() {
        let dir = std::env::temp_dir().join(format!("bf-javapick-{}", std::process::id()));
        let dirs = Dirs::at(&dir);
        let s = Settings::default();
        let i = inst(r#"{"name":"A"}"#);
        let ls = LaunchSettings::resolve(&s, &i);
        let p = pick(&dirs, &ls, &i.file.version);
        assert!(p.home.is_none() && p.runtime_missing);
        assert_eq!(p.component.as_deref(), Some("java-runtime-delta"));
        assert_eq!(p.binary, PathBuf::from(platform::java_exe()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
