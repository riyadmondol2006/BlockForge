//! Choosing the Java that Minecraft runs on, and remembering why, so the launcher can say what it chose and why
//! instead of leaving people to guess.

use std::path::{Path, PathBuf};

pub const X64: &str = "x64";
pub const ARM64: &str = "arm64";
pub const X86: &str = "x86";

/// A Java installation as far as choosing one is concerned: where it is, which version and what kind of processor it
/// is built for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaRuntime {
    pub home: PathBuf,
    /// 8, 11, 17, 21 ... or -1 if unknown.
    pub major: i32,
    /// `x64`, `arm64`, `x86` or `None` if unknown.
    pub arch: Option<String>,
    /// Where it came from, shown to the player: "chosen in settings", "Minecraft's own Java" ...
    pub source: String,
}

impl JavaRuntime {
    pub fn new(home: impl Into<PathBuf>, major: i32, arch: Option<&str>, source: &str) -> JavaRuntime {
        JavaRuntime { home: home.into(), major, arch: arch.map(str::to_string), source: source.to_string() }
    }

    /// Reads a Java home's `release` file, which every JDK and most JREs ship, without starting Java.
    pub fn from_home(home: &Path, source: &str) -> JavaRuntime {
        let unknown = || JavaRuntime::new(home, -1, None, source);
        let Ok(text) = std::fs::read_to_string(home.join("release")) else { return unknown() };
        let props = parse_properties(&text);
        let get = |key: &str| props.iter().find(|(k, _)| k == key).map(|(_, v)| v.replace('"', "").trim().to_string());
        JavaRuntime::new(
            home,
            parse_major(get("JAVA_VERSION").as_deref()),
            normalise_arch(get("OS_ARCH").as_deref()).as_deref(),
            source,
        )
    }

    pub fn label(&self) -> String {
        format!(
            "{}{} at {}",
            if self.major > 0 { format!("Java {}", self.major) } else { "Java".to_string() },
            self.arch.as_ref().map(|a| format!(", {a}")).unwrap_or_default(),
            self.home.display()
        )
    }
}

/// `KEY=value` lines, `#` and `!` comments, as Java's properties files have them.
fn parse_properties(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
        .filter_map(|l| {
            let at = l.find(['=', ':'])?;
            Some((l[..at].trim().to_string(), l[at + 1..].trim().to_string()))
        })
        .collect()
}

/// "1.8.0_392" is 8, "17.0.9" is 17.
pub fn parse_major(version: Option<&str>) -> i32 {
    let Some(v) = version else { return -1 };
    let v = v.trim();
    // the old numbering ("1.8") counts after its leading "1."
    let rest = v.strip_prefix("1.").filter(|r| r.starts_with(|c: char| c.is_ascii_digit())).unwrap_or(v);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(-1)
}

pub fn normalise_arch(arch: Option<&str>) -> Option<String> {
    let a = arch?.to_lowercase();
    Some(
        match a.as_str() {
            "aarch64" | "arm64" => ARM64,
            "x86_64" | "amd64" | "x64" => X64,
            "x86" | "i386" | "i586" | "i686" => X86,
            other => return Some(other.to_string()),
        }
        .to_string(),
    )
}

/// The processor this computer has, as `x64`, `arm64` or `x86`.
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => ARM64,
        "x86" => X86,
        _ => X64,
    }
}

/// What the choice is based on.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// A Java the player chose, in the instance or in settings; `None` if they left it automatic.
    pub user_choice: Option<JavaRuntime>,
    /// The Java that Mojang provides for this Minecraft version, if it's installed and allowed.
    pub minecraft_runtime: Option<JavaRuntime>,
    /// Javas found on the computer.
    pub installed: Vec<JavaRuntime>,
    /// The Java that launches BlockForge itself, as a last resort.
    pub launcher_java: Option<JavaRuntime>,
    /// The version Minecraft needs at least, or -1 if unknown.
    pub required_major: i32,
    /// The processor this computer has: `x64`, `arm64` or `x86`.
    pub host_arch: Option<String>,
}

impl Request {
    pub fn new() -> Request {
        Request { required_major: -1, ..Default::default() }
    }
}

/// The decision and the thinking behind it.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub java: Option<JavaRuntime>,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

impl Choice {
    /// One line per fact, for the log.
    pub fn explain(&self) -> String {
        let mut s = match &self.java {
            None => "No Java found".to_string(),
            Some(j) => format!("Using {}", j.label()),
        };
        for r in &self.reasons {
            s.push_str(&format!("\n  why: {r}"));
        }
        for w in &self.warnings {
            s.push_str(&format!("\n  warning: {w}"));
        }
        s
    }
}

pub fn resolve(request: &Request) -> Choice {
    let mut choice = Choice::default();
    if let Some(j) = &request.user_choice {
        choice.java = Some(j.clone());
        choice.reasons.push("you picked this Java in the settings".into());
    } else if let Some(j) = &request.minecraft_runtime {
        choice.java = Some(j.clone());
        choice
            .reasons
            .push("it's the Java Minecraft provides for this version, so it is the one it was tested with".into());
    } else if let Some(best) = best(request) {
        choice.reasons.push(if request.required_major > 0 {
            format!(
                "it is the closest installed Java that meets Minecraft's requirement of Java {}",
                request.required_major
            )
        } else {
            "it is the newest Java installed here".into()
        });
        if request.host_arch.is_some() && request.host_arch == best.arch {
            choice
                .reasons
                .push(format!("it matches this computer's processor ({})", best.arch.clone().unwrap_or_default()));
        }
        choice.java = Some(best);
    } else if let Some(j) = &request.launcher_java {
        choice.java = Some(j.clone());
        choice.reasons.push("no other Java was found, so BlockForge's own Java is being used".into());
    }
    check(&mut choice, request);
    choice
}

/// The best installed Java: meets the requirement, matches the processor, and is as close to the requirement as
/// possible.
fn best(request: &Request) -> Option<JavaRuntime> {
    request
        .installed
        .iter()
        .filter(|j| !(j.major > 0 && request.required_major > 0 && j.major < request.required_major))
        // a Java whose version is known beats one that isn't; then the closest to the requirement, or the newest
        // when there is no requirement
        .min_by_key(|j| {
            (
                request.host_arch.is_some() && request.host_arch != j.arch,
                j.major <= 0,
                if request.required_major > 0 { j.major - request.required_major } else { -j.major },
            )
        })
        .cloned()
}

fn check(choice: &mut Choice, request: &Request) {
    let Some(java) = &choice.java else {
        choice
            .warnings
            .push("no Java could be found, install one or turn on \"Use Java provided by Minecraft\"".into());
        return;
    };
    if request.required_major > 0 && java.major > 0 && java.major < request.required_major {
        choice.warnings.push(format!(
            "this Minecraft needs Java {} or newer but this is Java {}, so it will probably fail to start",
            request.required_major, java.major
        ));
    }
    if let (Some(host), Some(arch)) = (&request.host_arch, &java.arch) {
        if host != arch {
            if host == ARM64 && arch == X64 {
                choice.warnings.push(
                    "this Java is built for Intel/AMD processors and runs through emulation on this computer, which makes Minecraft noticeably slower. An ARM Java is recommended."
                        .into(),
                );
            } else if arch == X86 && host != X86 {
                choice.warnings.push("this is a 32-bit Java, which limits Minecraft to about 1.5 GB of memory".into());
            } else {
                choice.warnings.push(format!("this Java is built for {arch} but this computer is {host}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn java(home: &str, major: i32, arch: &str) -> JavaRuntime {
        JavaRuntime::new(home, major, Some(arch), "test")
    }

    fn request() -> Request {
        Request::new()
    }

    #[test]
    fn reads_the_release_file() {
        let dir = std::env::temp_dir().join(format!("blockforge-release-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("release"),
            "JAVA_VERSION=\"17.0.9\"\nOS_ARCH=\"aarch64\"\nIMPLEMENTOR=\"Eclipse Adoptium\"\n",
        )
        .unwrap();
        let r = JavaRuntime::from_home(&dir, "test");
        assert_eq!(r.major, 17);
        assert_eq!(r.arch.as_deref(), Some(ARM64));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_versions_and_architectures() {
        assert_eq!(parse_major(Some("1.8.0_392")), 8);
        assert_eq!(parse_major(Some("21.0.1")), 21);
        assert_eq!(parse_major(Some("17")), 17);
        assert_eq!(parse_major(None), -1);
        assert_eq!(parse_major(Some("junk")), -1);
        assert_eq!(normalise_arch(Some("amd64")).as_deref(), Some(X64));
        assert_eq!(normalise_arch(Some("arm64")).as_deref(), Some(ARM64));
        assert_eq!(normalise_arch(Some("i386")).as_deref(), Some(X86));
        assert_eq!(normalise_arch(None), None);
        assert_eq!(JavaRuntime::from_home(Path::new("/does/not/exist"), "x").major, -1);
    }

    #[test]
    fn the_players_choice_wins_but_is_checked() {
        let mut req = request();
        req.user_choice = Some(java("/jdk8", 8, "x64"));
        req.minecraft_runtime = Some(java("/mc17", 17, "x64"));
        req.required_major = 17;
        req.host_arch = Some("x64".into());
        let c = resolve(&req);
        assert_eq!(c.java.unwrap().home, PathBuf::from("/jdk8"));
        assert!(c.warnings[0].contains("needs Java 17"));
    }

    #[test]
    fn minecrafts_own_java_is_preferred_when_nothing_is_chosen() {
        let mut req = request();
        req.minecraft_runtime = Some(java("/mc17", 17, "arm64"));
        req.installed.push(java("/jdk21", 21, "arm64"));
        req.required_major = 17;
        req.host_arch = Some("arm64".into());
        let c = resolve(&req);
        assert_eq!(c.java.as_ref().unwrap().major, 17);
        assert!(c.warnings.is_empty());
        assert!(c.explain().contains("why:"));
    }

    #[test]
    fn picks_the_closest_installed_java_that_meets_the_requirement() {
        let mut req = request();
        req.installed = vec![java("/jdk8", 8, "x64"), java("/jdk21", 21, "x64"), java("/jdk17", 17, "x64")];
        req.required_major = 17;
        req.host_arch = Some("x64".into());
        assert_eq!(resolve(&req).java.unwrap().major, 17);
    }

    #[test]
    fn picks_the_newest_java_when_minecraft_does_not_say_what_it_needs() {
        let mut req = request();
        req.installed = vec![java("/jdk8", 8, "x64"), java("/jdk21", 21, "x64"), java("/jdk17", 17, "x64")];
        req.host_arch = Some("x64".into());
        let c = resolve(&req);
        assert_eq!(c.java.unwrap().major, 21);
        assert!(c.reasons[0].contains("newest"));
    }

    #[test]
    fn a_java_of_unknown_version_is_only_a_last_resort() {
        let mut req = request();
        req.installed = vec![java("/unknown", -1, "x64"), java("/jdk21", 21, "x64")];
        req.required_major = 17;
        req.host_arch = Some("x64".into());
        assert_eq!(resolve(&req).java.unwrap().major, 21);
    }

    #[test]
    fn prefers_the_native_processor_on_arm_computers() {
        let mut req = request();
        req.installed = vec![java("/x64-17", 17, "x64"), java("/arm-21", 21, "arm64")];
        req.required_major = 17;
        req.host_arch = Some("arm64".into());
        let c = resolve(&req);
        assert_eq!(c.java.unwrap().arch.as_deref(), Some(ARM64));
        assert!(c.warnings.is_empty());
    }

    #[test]
    fn warns_about_emulation_and_falls_back_to_the_launcher_java() {
        let mut arm_with_intel = request();
        arm_with_intel.user_choice = Some(java("/x64-17", 17, "x64"));
        arm_with_intel.host_arch = Some("arm64".into());
        assert!(resolve(&arm_with_intel).warnings[0].contains("emulation"));

        let mut nothing = request();
        nothing.launcher_java = Some(java("/launcher", 21, "x64"));
        assert_eq!(resolve(&nothing).java.unwrap().home, PathBuf::from("/launcher"));

        let none = resolve(&request());
        assert!(none.java.is_none());
        assert!(none.warnings[0].contains("no Java"));
    }
}
