//! Deciding whether a release is newer than the running copy, how this copy was installed and which of the
//! release's files fits this computer. Nothing here touches the network, the disk or the window, so all of it is
//! tested directly.

use std::path::{Path, PathBuf};

use super::release::{Asset, Release};

/// How this copy of BlockForge was installed, which decides what file an update needs and how it is applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A Linux AppImage: the update replaces the AppImage file itself.
    AppImage,
    /// The Debian/Ubuntu package.
    Deb,
    /// The Fedora/openSUSE package.
    Rpm,
    /// The Windows setup program (an installed copy with an uninstaller).
    WindowsInstaller,
    /// The Windows `.exe` that was unpacked from the portable zip.
    WindowsPortable,
    /// The macOS app, updated from the disk image.
    MacApp,
    /// Can't be updated in place (running from source, Flatpak, a distribution package...): only point at the page.
    PageOnly,
}

impl Kind {
    /// What the release file for this kind ends with, in lower case. `None` when there is no file to fetch.
    pub fn suffix(self) -> Option<&'static str> {
        match self {
            Kind::AppImage => Some(".appimage"),
            Kind::Deb => Some(".deb"),
            Kind::Rpm => Some(".rpm"),
            Kind::WindowsInstaller => Some(".exe"),
            Kind::WindowsPortable => Some(".zip"),
            Kind::MacApp => Some(".dmg"),
            Kind::PageOnly => None,
        }
    }

    /// Whether the update can be downloaded and applied from inside the launcher.
    pub fn can_install(self) -> bool {
        self != Kind::PageOnly
    }
}

/// The operating systems the release has files for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum System {
    Linux,
    Windows,
    Mac,
}

impl System {
    pub fn current() -> System {
        match crate::platform::os() {
            crate::platform::Os::Linux => System::Linux,
            crate::platform::Os::Windows => System::Windows,
            crate::platform::Os::Mac => System::Mac,
        }
    }
}

/// Which package manager owns `/usr/bin/blockforge`, when `dpkg -S` or `rpm -qf` could say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackageOwner {
    Dpkg,
    Rpm,
}

/// Everything [`detect_kind`] looks at, gathered by the caller so the decision itself can be tested.
#[derive(Clone, Debug)]
pub struct InstallEnv {
    pub system: System,
    /// The running program (`std::env::current_exe`).
    pub exe: Option<PathBuf>,
    /// The `APPIMAGE` environment variable: the AppImage file BlockForge was started from.
    pub appimage: Option<PathBuf>,
    /// Whether it runs inside Flatpak (`FLATPAK_ID` is set).
    pub flatpak: bool,
    /// The contents of `/etc/os-release` on Linux.
    pub os_release: Option<String>,
    /// Whether the Windows setup's uninstaller (`unins000.exe`) sits next to the program.
    pub has_uninstaller: bool,
    /// The package manager that owns the program file, when known.
    pub package_owner: Option<PackageOwner>,
}

/// Where the Linux packages put the program.
const PACKAGE_BINARY: &str = "/usr/bin/blockforge";

/// Works out how this copy was installed.
pub fn detect_kind(env: &InstallEnv) -> Kind {
    if env.flatpak {
        return Kind::PageOnly;
    }
    let exe = env.exe.as_deref();
    if exe.is_some_and(is_cargo_build) {
        return Kind::PageOnly;
    }
    match env.system {
        System::Linux => {
            if env.appimage.as_deref().is_some_and(|p| !p.as_os_str().is_empty()) {
                return Kind::AppImage;
            }
            if exe != Some(Path::new(PACKAGE_BINARY)) {
                return Kind::PageOnly;
            }
            match env.package_owner {
                Some(PackageOwner::Dpkg) => return Kind::Deb,
                Some(PackageOwner::Rpm) => return Kind::Rpm,
                None => {}
            }
            let family = env.os_release.as_deref().map(linux_family).unwrap_or_default();
            if contains_word(&family, &["debian", "ubuntu"]) {
                Kind::Deb
            } else if contains_word(&family, &["fedora", "rhel", "centos", "suse", "opensuse", "mageia"]) {
                Kind::Rpm
            } else {
                Kind::PageOnly
            }
        }
        System::Windows => match exe {
            Some(exe) if exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) => {
                if env.has_uninstaller {
                    Kind::WindowsInstaller
                } else {
                    Kind::WindowsPortable
                }
            }
            _ => Kind::PageOnly,
        },
        System::Mac => {
            let in_bundle = exe.is_some_and(|e| {
                let text = e.to_string_lossy();
                text.contains(".app/Contents/MacOS/")
            });
            if in_bundle { Kind::MacApp } else { Kind::PageOnly }
        }
    }
}

/// Whether `exe` is a program that `cargo build` or `cargo run` made (`.../target/debug/blockforge`).
fn is_cargo_build(exe: &Path) -> bool {
    let names: Vec<String> =
        exe.components().rev().skip(1).take(3).map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    let is_profile = |n: &str| n == "debug" || n == "release";
    match names.as_slice() {
        [profile, target, ..] if is_profile(profile) && target == "target" => true,
        [deps, profile, target] if deps == "deps" && is_profile(profile) && target == "target" => true,
        [profile, triple, target] if is_profile(profile) && triple.contains('-') && target == "target" => true,
        _ => false,
    }
}

/// The ID and ID_LIKE values from `/etc/os-release`, lower case and space separated.
pub fn linux_family(os_release: &str) -> String {
    let mut ids = String::new();
    for line in os_release.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("ID=").or_else(|| line.strip_prefix("ID_LIKE=")) {
            ids.push(' ');
            ids.push_str(&value.replace(['"', '\''], "").to_ascii_lowercase());
        }
    }
    ids
}

/// Looks at the real computer: the environment, the program's own path, `/etc/os-release` and, for a copy in
/// `/usr/bin`, the package databases.
pub fn current_env() -> InstallEnv {
    let system = System::current();
    let exe = std::env::current_exe().ok();
    let env_path = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let in_usr_bin = exe.as_deref() == Some(Path::new(PACKAGE_BINARY));
    InstallEnv {
        system,
        has_uninstaller: exe
            .as_deref()
            .and_then(Path::parent)
            .is_some_and(|dir| system == System::Windows && dir.join("unins000.exe").is_file()),
        appimage: env_path("APPIMAGE").filter(|p| p.is_file()),
        flatpak: std::env::var_os("FLATPAK_ID").is_some_and(|v| !v.is_empty()),
        os_release: (system == System::Linux).then(|| std::fs::read_to_string("/etc/os-release").ok()).flatten(),
        package_owner: if in_usr_bin { package_owner() } else { None },
        exe,
    }
}

/// Asks `dpkg -S` and then `rpm -qf` which package owns the program file (neither may be installed).
fn package_owner() -> Option<PackageOwner> {
    let owns = |tool: &str, args: &[&str]| {
        crate::platform::command(tool)
            .args(args)
            .arg(PACKAGE_BINARY)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    if owns("dpkg", &["-S"]) {
        Some(PackageOwner::Dpkg)
    } else if owns("rpm", &["-qf"]) {
        Some(PackageOwner::Rpm)
    } else {
        None
    }
}

/// How this copy was installed.
pub fn install_kind() -> Kind {
    detect_kind(&current_env())
}

const ARM_TOKENS: &[&str] = &["arm64", "aarch64"];
const X64_TOKENS: &[&str] = &["x64", "x86_64", "amd64", "intel"];

/// Reads `v3.4.42.0`, `3.4.42` or `v3.4.42.0-rc1` into four numbers (missing ones are 0). `None` when the tag isn't
/// a version, like `nightly`.
pub fn parse_version(tag: &str) -> Option<[u32; 4]> {
    let mut clean = tag.trim();
    clean = clean.strip_prefix(['v', 'V']).unwrap_or(clean);
    if let Some(cut) = clean.find(['-', '+', ' ']) {
        clean = &clean[..cut];
    }
    if clean.to_ascii_lowercase().ends_with(".beta") {
        clean = &clean[..clean.len() - ".beta".len()];
    }
    let parts: Vec<&str> = clean.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut version = [0u32; 4];
    for (slot, part) in version.iter_mut().zip(&parts) {
        if part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    Some(version)
}

/// True when the release tag is a higher version than `current`. The same version is never an update.
pub fn is_newer(tag: &str, current: &[u32; 4]) -> bool {
    parse_version(tag).is_some_and(|other| other > *current)
}

/// The version of the running launcher.
pub fn current_version() -> [u32; 4] {
    parse_version(crate::VERSION).unwrap_or_default()
}

/// A tag shown to people: `v4.0.0` becomes `4.0.0`.
pub fn display_version(tag: &str) -> &str {
    let tag = tag.trim();
    tag.strip_prefix(['v', 'V']).unwrap_or(tag)
}

/// Picks the release file for this kind of install and CPU. Prefers a file named for this CPU, accepts one that
/// doesn't name a CPU, and never picks one built for a different CPU. `arch` is `x86_64`, `aarch64`, `amd64`...
pub fn pick_asset<'a>(release: &'a Release, kind: Kind, arch: &str) -> Option<&'a Asset> {
    let suffix = kind.suffix()?;
    let arch = arch.to_ascii_lowercase();
    let arm = ARM_TOKENS.iter().any(|t| arch.contains(t));
    let (mine, other) = if arm { (ARM_TOKENS, X64_TOKENS) } else { (X64_TOKENS, ARM_TOKENS) };

    let mut neutral = None;
    for asset in &release.assets {
        if asset.name.is_empty() || asset.browser_download_url.is_empty() {
            continue;
        }
        let name = asset.name.to_ascii_lowercase();
        if !name.ends_with(suffix) || name.contains("sources") || name.contains("javadoc") {
            continue;
        }
        // the installer and the portable copy share a system, so the setup program is told apart by name
        let portable = name.contains("portable");
        if (kind == Kind::WindowsInstaller && portable) || (kind == Kind::WindowsPortable && !portable) {
            continue;
        }
        if has_token(&name, mine) {
            return Some(asset);
        }
        if !has_token(&name, other) && neutral.is_none() {
            neutral = Some(asset);
        }
    }
    neutral
}

/// Whether any of `tokens` appears in `name` as a whole word (not touching letters or digits on either side).
fn has_token(name: &str, tokens: &[&str]) -> bool {
    contains_word(name, tokens)
}

fn contains_word(haystack: &str, words: &[&str]) -> bool {
    let bytes = haystack.as_bytes();
    let is_word_byte = |b: u8| b.is_ascii_alphanumeric();
    words.iter().any(|word| {
        haystack.match_indices(word).any(|(start, _)| {
            let before = start == 0 || !is_word_byte(bytes[start - 1]);
            let end = start + word.len();
            let after = end >= bytes.len() || !is_word_byte(bytes[end]);
            before && after
        })
    })
}
