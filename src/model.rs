//! Mojang's version JSON: what a Minecraft version needs to run. The Java edition saves the same shape into
//! each `instance.json`, with an extra `launcher` block, so one set of types reads both.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{paths, platform};

pub type Extras = Map<String, Value>;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub main_class: String,
    /// Old versions (before 1.13) give their game arguments as one string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Arguments>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<VersionDownloads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Logging>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_version: Option<JavaVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    #[serde(flatten)]
    pub extras: Extras,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Arg>,
    #[serde(default)]
    pub jvm: Vec<Arg>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Arg {
    Plain(String),
    Ruled { rules: Vec<Rule>, value: OneOrMany },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Rule {
    #[serde(default)]
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<BTreeMap<String, Value>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct OsRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The "features" a launch has, which some game arguments depend on.
#[derive(Clone, Debug, Default)]
pub struct Features {
    pub custom_resolution: bool,
    pub demo: bool,
}

impl Features {
    fn get(&self, name: &str) -> bool {
        match name {
            "has_custom_resolution" => self.custom_resolution,
            "is_demo_user" => self.demo,
            _ => false,
        }
    }
}

impl OsRule {
    fn matches(&self) -> bool {
        if let Some(name) = &self.name {
            if name != platform::os_rule_name() {
                return false;
            }
        }
        if let Some(arch) = &self.arch {
            let ok = match arch.as_str() {
                "x86" => platform::arch() == "x86",
                "x86_64" | "amd64" => platform::arch() == "x86_64",
                "arm64" | "aarch64" => platform::arch() == "aarch64",
                _ => false,
            };
            if !ok {
                return false;
            }
        }
        // `version` only ever appears on rules for very old macOS releases
        self.version.is_none()
    }
}

impl Rule {
    fn matches(&self, features: &Features) -> bool {
        if let Some(map) = &self.features {
            // a rule that lists features applies when each one is on; the Java edition's saved files lose the
            // names and leave an empty list, which must not switch anything on
            if map.is_empty() {
                return false;
            }
            let all = map.iter().all(|(k, v)| {
                let wanted = match v {
                    Value::Bool(b) => *b,
                    Value::String(s) => s == "true",
                    _ => false,
                };
                features.get(k) == wanted
            });
            if !all {
                return false;
            }
        }
        self.os.as_ref().is_none_or(OsRule::matches)
    }
}

pub fn rules_allow(rules: &[Rule], features: &Features) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(features) {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub total_size: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VersionDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Artifact>,
    #[serde(flatten)]
    pub extras: Extras,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Artifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Logging {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<LoggingClient>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LoggingClient {
    #[serde(default)]
    pub argument: String,
    #[serde(default)]
    pub file: LoggingFile,
    #[serde(default, rename = "type")]
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LoggingFile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    #[serde(default)]
    pub component: String,
    #[serde(default)]
    pub major_version: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Library {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibDownloads>,
    /// Maven repository base, used by loader profiles that list a name and a URL only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extract>,
    #[serde(flatten)]
    pub extras: Extras,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LibDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<BTreeMap<String, Artifact>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

const MOJANG_LIBRARIES: &str = "https://libraries.minecraft.net/";

impl Library {
    pub fn applies(&self, features: &Features) -> bool {
        rules_allow(&self.rules, features)
    }

    /// The jar that goes on the class path, with its path under `libraries/` always filled in.
    pub fn artifact(&self) -> Option<Artifact> {
        if let Some(a) = self.downloads.as_ref().and_then(|d| d.artifact.clone()) {
            let mut a = a;
            if a.path.is_none() {
                a.path = paths::maven_path(&self.name);
            }
            // a path from the file that would lead out of `libraries/` is never used
            a.path = a.path.filter(|p| crate::packutil::safe_relative(p).is_some());
            if a.path.is_some() {
                return Some(a);
            }
        }
        if self.downloads.as_ref().is_some_and(|d| d.classifiers.is_some()) && self.natives.is_some() {
            return None; // old style: only the natives jar exists
        }
        let path = paths::maven_path(&self.name)?;
        let base = self.url.clone().unwrap_or_else(|| MOJANG_LIBRARIES.to_string());
        let base = if base.ends_with('/') { base } else { format!("{base}/") };
        let sha1 = self.extras.get("sha1").and_then(Value::as_str).map(str::to_string);
        let size = self.extras.get("size").and_then(Value::as_i64);
        Some(Artifact { url: format!("{base}{path}"), path: Some(path), sha1, size })
    }

    /// For old versions: the natives jar to unpack for this computer.
    pub fn native(&self) -> Option<Artifact> {
        let classifier = self.natives.as_ref()?.get(platform::os_rule_name())?;
        let classifier = classifier.replace("${arch}", if cfg!(target_pointer_width = "32") { "32" } else { "64" });
        let mut a = self.downloads.as_ref()?.classifiers.as_ref()?.get(&classifier)?.clone();
        if a.path.is_none() {
            a.path = paths::maven_path(&format!("{}:{}", self.name, classifier));
        }
        a.path = a.path.filter(|p| crate::packutil::safe_relative(p).is_some());
        Some(a)
    }

    /// `group:artifact` plus classifier, to tell apart libraries that appear twice.
    pub fn dedupe_key(&self) -> String {
        let mut parts = self.name.split(':');
        let (g, a, _v) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next());
        match parts.next() {
            Some(c) => format!("{g}:{a}:{c}"),
            None => format!("{g}:{a}"),
        }
    }

    pub fn version(&self) -> &str {
        self.name.split(':').nth(2).unwrap_or("")
    }
}

/// Compare dotted versions the way libraries need: numbers by value, the rest by text.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> Vec<String> { s.split(['.', '-', '+', '_']).map(str::to_string).collect() };
    let (a, b) = (split(a), split(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).map(String::as_str).unwrap_or("0"), b.get(i).map(String::as_str).unwrap_or("0"));
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    std::cmp::Ordering::Equal
}

impl VersionJson {
    /// Put a loader profile (Fabric, Quilt, Forge) on top of the Minecraft version it is built for.
    pub fn merged_over(mut self, parent: VersionJson) -> VersionJson {
        let mut out = parent;
        out.main_class = if self.main_class.is_empty() { out.main_class } else { self.main_class };
        if self.minecraft_arguments.is_some() {
            out.minecraft_arguments = self.minecraft_arguments.take();
            if self.arguments.is_none() {
                out.arguments = None;
            }
        }
        if let Some(child) = self.arguments.take() {
            let base = out.arguments.get_or_insert_with(Arguments::default);
            base.game.extend(child.game);
            base.jvm.extend(child.jvm);
        }
        let mut libs = std::mem::take(&mut self.libraries);
        libs.extend(std::mem::take(&mut out.libraries));
        out.libraries = libs;
        if self.java_version.is_some() {
            out.java_version = self.java_version.take();
        }
        for (k, v) in self.extras {
            out.extras.entry(k).or_insert(v);
        }
        out.inherits_from = None;
        out
    }

    /// Libraries for this computer, with duplicates reduced to the newest version in the first one's place.
    pub fn active_libraries(&self, features: &Features) -> Vec<&Library> {
        let mut out: Vec<&Library> = Vec::with_capacity(self.libraries.len());
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for lib in self.libraries.iter().filter(|l| l.applies(features)) {
            let key = format!("{}|{}", lib.dedupe_key(), lib.native().is_some());
            match seen.get(&key) {
                Some(&i) => {
                    if compare_versions(lib.version(), out[i].version()).is_gt() {
                        out[i] = lib;
                    }
                }
                None => {
                    seen.insert(key, out.len());
                    out.push(lib);
                }
            }
        }
        out
    }

    /// Forge and NeoForge (1.17 and newer) start through a module bootstrap and bring their own patched game jar;
    /// the vanilla jar must stay off the class path or the two collide as modules. They announce themselves with
    /// an `-DignoreList=` Java option.
    pub fn uses_module_bootstrap(&self) -> bool {
        let Some(args) = &self.arguments else { return false };
        args.jvm.iter().any(|a| match a {
            Arg::Plain(s) => s.starts_with("-DignoreList="),
            Arg::Ruled { value, .. } => value.to_vec().iter().any(|s| s.starts_with("-DignoreList=")),
        })
    }

    pub fn game_version(&self) -> &str {
        self.inherits_from.as_deref().unwrap_or(&self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let f = Features::default();
        let allow_linux: Vec<Rule> = serde_json::from_str(r#"[{"action":"allow","os":{"name":"linux"}}]"#).unwrap();
        assert_eq!(rules_allow(&allow_linux, &f), cfg!(target_os = "linux"));
        let not_osx: Vec<Rule> =
            serde_json::from_str(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#).unwrap();
        assert_eq!(rules_allow(&not_osx, &f), !cfg!(target_os = "macos"));
        // the Java edition's files keep an empty feature list; that must never switch --demo on
        let demo: Vec<Rule> = serde_json::from_str(r#"[{"action":"allow","features":{}}]"#).unwrap();
        assert!(!rules_allow(&demo, &f));
        let res: Vec<Rule> =
            serde_json::from_str(r#"[{"action":"allow","features":{"has_custom_resolution":"true"}}]"#).unwrap();
        assert!(!rules_allow(&res, &f));
        assert!(rules_allow(&res, &Features { custom_resolution: true, demo: false }));
    }

    #[test]
    fn library_without_downloads_uses_its_maven_url() {
        let lib: Library =
            serde_json::from_str(r#"{"name":"net.fabricmc:fabric-loader:0.16.0","url":"https://maven.fabricmc.net/"}"#)
                .unwrap();
        let a = lib.artifact().unwrap();
        assert_eq!(a.url, "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.0/fabric-loader-0.16.0.jar");
    }

    #[test]
    fn versions_compare() {
        assert!(compare_versions("9.11", "9.9").is_gt());
        assert!(compare_versions("1.2", "1.2.0").is_eq());
    }
}
