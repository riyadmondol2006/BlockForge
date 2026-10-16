//! Which graphics card Minecraft uses, for laptops (and some desktops) with two of them: a small one built into the
//! processor that saves power, and a big one that is faster.
//!
//! Systems normally pick the small one for everything except programs they know to be games, and they don't know
//! Java. The setting here tells the system which to use for the game:
//!
//! * Linux: environment variables for the game, the same ones `switcherooctl launch` sets (taken from it when it is
//!   installed, worked out from `/sys/class/drm` otherwise): `DRI_PRIME` for open-source drivers, the PRIME render
//!   offload variables for NVIDIA's driver.
//! * Windows: the per-program preference that Windows keeps under Settings > System > Display > Graphics, written for
//!   the Java program that runs the game.
//! * macOS: nothing to choose; the system switches by itself.
//!
//! The pieces that decide things are plain functions on plain values, so they can be tested without a second card.

/// How a card is made, which matters for the variables the driver wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

impl Vendor {
    pub fn from_pci_id(id: u32) -> Vendor {
        match id {
            0x10de => Vendor::Nvidia,
            0x1002 | 0x1022 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            _ => Vendor::Other,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Other => "",
        }
    }
}

/// One graphics card the system has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gpu {
    /// What is saved in the settings to mean this card (Linux only).
    pub id: String,
    /// The name to show.
    pub name: String,
    pub vendor: Vendor,
    /// A fast card for demanding programs, as opposed to the small one that saves power.
    pub discrete: bool,
    /// The card the system uses for programs that don't ask for one.
    pub default: bool,
    /// The environment variables that make a program use this card.
    pub env: Vec<(String, String)>,
}

/// What the player chose in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Choice {
    /// Leave it to the system.
    #[default]
    Auto,
    /// The card that saves power.
    Integrated,
    /// The fast card.
    Dedicated,
    /// One card by its id (Linux).
    Specific(String),
}

impl Choice {
    /// The setting's text: `auto`, `integrated`, `dedicated`, or a card's id. Anything empty means `auto`.
    pub fn parse(text: &str) -> Choice {
        match text.trim() {
            "" | "auto" => Choice::Auto,
            "integrated" => Choice::Integrated,
            "dedicated" => Choice::Dedicated,
            id => Choice::Specific(id.to_string()),
        }
    }

    pub fn as_setting(&self) -> String {
        match self {
            Choice::Auto => "auto".into(),
            Choice::Integrated => "integrated".into(),
            Choice::Dedicated => "dedicated".into(),
            Choice::Specific(id) => id.clone(),
        }
    }
}

/// What to do for one start of the game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    /// Variables to give the game (Linux). The player's own environment variables still win over these.
    pub env: Vec<(String, String)>,
    /// The card's name, for the log; `None` when nothing is changed.
    pub gpu: Option<String>,
    /// A sentence for the log when the choice can't be followed.
    pub note: Option<String>,
}

/// The card a choice means, if there is one to choose. Nothing to do with fewer than two cards.
pub fn resolve<'a>(choice: &Choice, gpus: &'a [Gpu]) -> Option<&'a Gpu> {
    if gpus.len() < 2 {
        return None;
    }
    match choice {
        Choice::Auto => None,
        Choice::Dedicated => {
            gpus.iter().find(|g| g.discrete && !g.default).or_else(|| gpus.iter().find(|g| g.discrete))
        }
        Choice::Integrated => {
            gpus.iter().find(|g| !g.discrete && g.default).or_else(|| gpus.iter().find(|g| !g.discrete))
        }
        Choice::Specific(id) => gpus.iter().find(|g| &g.id == id),
    }
}

/// The effect of a choice on Linux: the variables of the card, or a note when it is not there.
pub fn effect(choice: &Choice, gpus: &[Gpu]) -> Effect {
    if *choice == Choice::Auto {
        return Effect::default();
    }
    match resolve(choice, gpus) {
        Some(g) => Effect { env: g.env.clone(), gpu: Some(g.name.clone()), note: None },
        None if gpus.len() < 2 => Effect::default(),
        None => Effect {
            note: Some(
                match choice {
                    Choice::Specific(_) => {
                        "The graphics card chosen in Settings isn't in this computer; using the automatic choice"
                    }
                    Choice::Dedicated => "No separate fast graphics card was found; using the automatic choice",
                    _ => "No power-saving graphics card was found; using the automatic choice",
                }
                .to_string(),
            ),
            ..Effect::default()
        },
    }
}

// ---- Linux: switcherooctl

/// The cards `switcherooctl list` prints:
///
/// ```text
/// Device: 0
///   Name:        Advanced Micro Devices, Inc. [AMD/ATI] Cezanne [Radeon Vega Series]
///   Default:     yes
///   Discrete:    no
///   Environment: DRI_PRIME=pci-0000_06_00_0 VK_LOADER_DRIVERS_SELECT=*radeon*
/// ```
pub fn parse_switcherooctl(text: &str) -> Vec<Gpu> {
    struct Block {
        index: String,
        name: String,
        default: bool,
        discrete: bool,
        env: Vec<(String, String)>,
    }
    let mut blocks: Vec<Block> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Device:") {
            blocks.push(Block {
                index: rest.trim().to_string(),
                name: String::new(),
                default: false,
                discrete: false,
                env: Vec::new(),
            });
            continue;
        }
        let Some(block) = blocks.last_mut() else { continue };
        if let Some(v) = line.strip_prefix("Name:") {
            block.name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Default:") {
            block.default = v.trim().eq_ignore_ascii_case("yes");
        } else if let Some(v) = line.strip_prefix("Discrete:") {
            block.discrete = v.trim().eq_ignore_ascii_case("yes");
        } else if let Some(v) = line.strip_prefix("Environment:") {
            // values have no spaces in them, so splitting on spaces is how switcherooctl's own users do it too
            block.env = v
                .split_whitespace()
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        }
    }
    blocks
        .into_iter()
        .filter(|b| !b.name.is_empty())
        .map(|b| {
            let vendor = vendor_from_name(&b.name);
            let id = env_id(&b.env).unwrap_or_else(|| format!("gpu{}", b.index));
            Gpu { id, name: short_name(&b.name), vendor, discrete: b.discrete, default: b.default, env: b.env }
        })
        .collect()
}

/// A stable name for a card out of its variables: the PCI address in `DRI_PRIME`, or `nvidia` for NVIDIA's driver.
fn env_id(env: &[(String, String)]) -> Option<String> {
    if let Some((_, v)) = env.iter().find(|(k, _)| k == "DRI_PRIME").filter(|(_, v)| v.starts_with("pci-")) {
        return Some(v.clone());
    }
    env.iter().any(|(k, v)| k == "__NV_PRIME_RENDER_OFFLOAD" && v == "1").then(|| "nvidia".to_string())
}

fn vendor_from_name(name: &str) -> Vendor {
    let n = name.to_ascii_lowercase();
    if n.contains("nvidia") {
        Vendor::Nvidia
    } else if n.contains("intel") {
        Vendor::Intel
    } else if n.contains("amd") || n.contains("radeon") || n.contains("ati technologies") {
        Vendor::Amd
    } else {
        Vendor::Other
    }
}

/// "NVIDIA Corporation TU117M [GeForce GTX 1650 Mobile / Max-Q]" becomes "NVIDIA GeForce GTX 1650 Mobile".
pub fn short_name(full: &str) -> String {
    let vendor = vendor_from_name(full);
    let cleaned: String = full.chars().filter(|c| !matches!(c, '\u{2122}' | '\u{00AE}')).collect();
    // the last bracket holds the marketing name; the one before may be the vendor's ("[AMD/ATI]")
    let marketing = cleaned.rsplit_once('[').and_then(|(before, rest)| {
        let inside = rest.split(']').next()?.trim();
        // "[AMD/ATI]" alone is a vendor tag, not a name
        (!inside.contains("AMD/ATI") || before.contains('[')).then(|| inside.to_string())
    });
    let base = match marketing {
        Some(m) => m.split(" / ").next().unwrap_or(&m).trim().to_string(),
        None => cleaned
            .replace("Corporation", "")
            .replace("Advanced Micro Devices, Inc.", "")
            .replace("[AMD/ATI]", "")
            .replace("Intel", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    };
    let label = vendor.label();
    if label.is_empty() || base.to_ascii_lowercase().starts_with(&label.to_ascii_lowercase()) {
        base
    } else {
        format!("{label} {base}")
    }
}

// ---- Linux: /sys/class/drm

/// A card as `/sys/class/drm/card*/device` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysCard {
    /// `0000:06:00.0`
    pub pci_address: String,
    pub vendor: u32,
    pub device: u32,
    /// The card the firmware started the screen with: the one the system uses by default.
    pub boot_vga: bool,
}

/// The cards for sysfs entries. `name_of` gives a card's name when the system's PCI list has it; `nvidia_egl` is the
/// path of NVIDIA's EGL vendor file when it exists (Wayland and EGL programs need it next to the other variables).
pub fn from_sysfs(
    cards: &[SysCard],
    name_of: impl Fn(u32, u32) -> Option<String>,
    nvidia_egl: Option<&str>,
) -> Vec<Gpu> {
    let any_boot = cards.iter().any(|c| c.boot_vga);
    let mut gpus: Vec<Gpu> = cards
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let vendor = Vendor::from_pci_id(c.vendor);
            let tag = format!("pci-{}", c.pci_address.replace([':', '.'], "_"));
            // NVIDIA's driver is for fast cards only here; for the others a card is "separate" when the screen
            // wasn't started with it, which on a laptop is the second card
            let default = if any_boot { c.boot_vga } else { i == 0 };
            let discrete = match vendor {
                Vendor::Nvidia => true,
                Vendor::Intel => false,
                _ => cards.len() > 1 && !default,
            };
            let name = name_of(c.vendor, c.device)
                .map(|n| short_name(&n))
                .unwrap_or_else(|| format!("{} graphics ({:04x}:{:04x})", vendor.label(), c.vendor, c.device));
            let env = match vendor {
                Vendor::Nvidia => {
                    let mut env = vec![
                        ("__NV_PRIME_RENDER_OFFLOAD".to_string(), "1".to_string()),
                        ("__GLX_VENDOR_LIBRARY_NAME".to_string(), "nvidia".to_string()),
                        ("__VK_LAYER_NV_optimus".to_string(), "NVIDIA_only".to_string()),
                    ];
                    if let Some(file) = nvidia_egl {
                        env.push(("__EGL_VENDOR_LIBRARY_FILENAMES".to_string(), file.to_string()));
                    }
                    env
                }
                _ => vec![("DRI_PRIME".to_string(), tag.clone())],
            };
            let id = if vendor == Vendor::Nvidia { "nvidia".to_string() } else { tag };
            Gpu { id, name, vendor, discrete, default, env }
        })
        .collect();
    gpus.sort_by_key(|g| !g.default);
    gpus
}

/// Where NVIDIA's EGL vendor file is, if the driver has put one there.
#[cfg(target_os = "linux")]
fn nvidia_egl_file() -> Option<String> {
    ["/usr/share/glvnd/egl_vendor.d/10_nvidia.json", "/etc/glvnd/egl_vendor.d/10_nvidia.json"]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
        .map(str::to_string)
}

/// A card's name out of the system's PCI list (`pci.ids`), if the file is there.
#[cfg(target_os = "linux")]
fn pci_name(vendor: u32, device: u32) -> Option<String> {
    let text = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"]
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p).ok())?;
    pci_ids_lookup(&text, vendor, device)
}

/// Look a vendor and device up in the text of `pci.ids`: vendor lines start in the first column, device lines with
/// one tab.
pub fn pci_ids_lookup(text: &str, vendor: u32, device: u32) -> Option<String> {
    let (vendor_key, device_key) = (format!("{vendor:04x}  "), format!("\t{device:04x}  "));
    let mut vendor_name: Option<&str> = None;
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        if !line.starts_with('\t') {
            if vendor_name.is_some() {
                return None; // the vendor's block ended without the device
            }
            if let Some(rest) = line.strip_prefix(&vendor_key) {
                vendor_name = Some(rest.trim());
            }
        } else if vendor_name.is_some() && line.starts_with(&device_key) {
            let name = line[device_key.len()..].trim();
            return Some(format!("{} {name}", vendor_name.unwrap_or_default()));
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn detect_sysfs() -> Vec<Gpu> {
    let Ok(read) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let hex = |p: &std::path::Path| {
        let t = std::fs::read_to_string(p).ok()?;
        u32::from_str_radix(t.trim().trim_start_matches("0x"), 16).ok()
    };
    let mut cards = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // "card0", not "card0-HDMI-A-1" or "renderD128"
        if !name.starts_with("card") || name[4..].chars().any(|c| !c.is_ascii_digit()) {
            continue;
        }
        let device_dir = entry.path().join("device");
        let (Some(vendor), Some(device)) = (hex(&device_dir.join("vendor")), hex(&device_dir.join("device"))) else {
            continue;
        };
        // the folder the link points to is named after the PCI address
        let Some(pci_address) = std::fs::canonicalize(&device_dir)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        else {
            continue;
        };
        if pci_address.matches(':').count() < 2 {
            continue; // not a PCI card (a virtual or platform display device)
        }
        let boot_vga = std::fs::read_to_string(device_dir.join("boot_vga")).is_ok_and(|t| t.trim() == "1");
        cards.push(SysCard { pci_address, vendor, device, boot_vga });
    }
    cards.sort_by(|a, b| a.pci_address.cmp(&b.pci_address));
    from_sysfs(&cards, pci_name, nvidia_egl_file().as_deref())
}

#[cfg(target_os = "linux")]
fn detect_switcheroo() -> Vec<Gpu> {
    let Ok(out) = crate::platform::command("switcherooctl").arg("list").output() else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let mut gpus = parse_switcherooctl(&String::from_utf8_lossy(&out.stdout));
    // NVIDIA's EGL file is what makes Wayland and EGL programs follow the other variables
    if let Some(file) = nvidia_egl_file() {
        for g in gpus.iter_mut().filter(|g| g.vendor == Vendor::Nvidia) {
            if !g.env.iter().any(|(k, _)| k == "__EGL_VENDOR_LIBRARY_FILENAMES") {
                g.env.push(("__EGL_VENDOR_LIBRARY_FILENAMES".to_string(), file.clone()));
            }
        }
    }
    gpus
}

// ---- Windows

/// The names of the display adapters out of the text `reg query ... /s /v DriverDesc` prints, without the ones that
/// aren't cards (the basic display driver, remote desktop).
pub fn parse_windows_adapters(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let Some((_, value)) = line.trim().split_once("REG_SZ") else { continue };
        if !line.trim_start().starts_with("DriverDesc") {
            continue;
        }
        let name = value.trim();
        let lower = name.to_ascii_lowercase();
        if name.is_empty()
            || lower.contains("basic display")
            || lower.contains("remote")
            || lower.contains("virtual")
            || lower.contains("displaylink")
            || names.iter().any(|n: &String| n == name)
        {
            continue;
        }
        names.push(name.to_string());
    }
    names
}

/// The cards on Windows. Windows picks a card by kind (power saving or high performance), not by name, so the
/// names are for showing what the computer has.
pub fn windows_gpus(names: &[String]) -> Vec<Gpu> {
    let first_intel = names.iter().position(|n| vendor_from_name(n) == Vendor::Intel);
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let vendor = vendor_from_name(n);
            // an Intel card next to another card is the small one; an AMD or NVIDIA one is the fast one, except an
            // AMD card "Graphics" (Vega, Radeon Graphics) that is built into the processor
            let integrated = vendor == Vendor::Intel
                || (vendor == Vendor::Amd && (n.ends_with("Graphics") || n.contains("Vega")))
                || (first_intel.is_none() && names.len() > 1 && i == 0 && vendor != Vendor::Nvidia);
            Gpu {
                id: format!("gpu{i}"),
                name: short_name(n),
                vendor,
                discrete: !integrated,
                default: integrated,
                env: Vec::new(),
            }
        })
        .collect()
}

/// The preference Windows keeps for a program: 1 power saving, 2 high performance. `None` for the choices that
/// leave it alone.
pub fn windows_preference(choice: &Choice) -> Option<u8> {
    match choice {
        Choice::Integrated => Some(1),
        Choice::Dedicated => Some(2),
        Choice::Auto | Choice::Specific(_) => None,
    }
}

/// The new data of a program's preference value, given what it holds now. The data is a list of `Name=value;` pairs
/// (Windows also keeps things like `AutoHDREnable=2097;` there), and only `GpuPreference` is touched. `None` means
/// there is nothing left and the value should be deleted.
pub fn merge_preference(existing: Option<&str>, preference: Option<u8>) -> Option<String> {
    let mut parts: Vec<String> = existing
        .unwrap_or("")
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.to_ascii_lowercase().starts_with("gpupreference="))
        .map(str::to_string)
        .collect();
    if let Some(p) = preference {
        parts.push(format!("GpuPreference={p}"));
    }
    (!parts.is_empty()).then(|| parts.join(";") + ";")
}

/// The data `reg query` printed for one value: `    C:\x\javaw.exe    REG_SZ    GpuPreference=2;`.
pub fn parse_reg_value(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(name) {
            if let Some((_, data)) = rest.split_once("REG_SZ") {
                return Some(data.trim().to_string());
            }
        }
    }
    None
}

#[cfg(windows)]
const PREFERENCES_KEY: &str = r"HKCU\Software\Microsoft\DirectX\UserGpuPreferences";

#[cfg(windows)]
fn detect_windows() -> Vec<Gpu> {
    let class = r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11cd-bfc1-08002be10318}";
    let Ok(out) = crate::platform::command("reg").args(["query", class, "/s", "/v", "DriverDesc"]).output() else {
        return Vec::new();
    };
    windows_gpus(&parse_windows_adapters(&String::from_utf8_lossy(&out.stdout)))
}

/// Tell Windows which card the Java program at `exe` should use, keeping its other graphics settings. With
/// `Auto`, only a preference this program wrote earlier is taken away again, so what the player set in Windows'
/// own Settings is left alone. Never fails: if Windows says no, the game starts on the card it would have used.
#[cfg(windows)]
pub fn apply_windows_preference(dirs: &crate::paths::Dirs, exe: &std::path::Path, choice: &Choice) {
    let name = exe.to_string_lossy().to_string();
    let marker = dirs.cache.join("gpu-preference-paths.txt");
    let mut ours: Vec<String> =
        std::fs::read_to_string(&marker).unwrap_or_default().lines().map(str::to_string).collect();
    let preference = windows_preference(choice);
    if preference.is_none() && !ours.contains(&name) {
        return;
    }
    let current = crate::platform::command("reg")
        .args(["query", PREFERENCES_KEY, "/v", &name])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| parse_reg_value(&String::from_utf8_lossy(&o.stdout), &name));
    let result = match merge_preference(current.as_deref(), preference) {
        Some(data) => crate::platform::command("reg")
            .args(["add", PREFERENCES_KEY, "/v", &name, "/t", "REG_SZ", "/d", &data, "/f"])
            .output(),
        None => crate::platform::command("reg").args(["delete", PREFERENCES_KEY, "/v", &name, "/f"]).output(),
    };
    if !result.is_ok_and(|o| o.status.success()) {
        crate::logs::debug("Graphics card preference: Windows didn't accept the change");
        return;
    }
    if preference.is_some() {
        if !ours.contains(&name) {
            ours.push(name);
        }
    } else {
        ours.retain(|p| p != &name);
    }
    let _ = std::fs::create_dir_all(&dirs.cache);
    let _ = std::fs::write(&marker, ours.join("\n"));
}

// ---- all systems

/// The graphics cards this computer has. Empty where there is nothing to choose between (macOS), or when they
/// can't be found out. Starts a program, so it is for Settings and for starts where a card was chosen, not for
/// every start.
pub fn detect() -> Vec<Gpu> {
    #[cfg(target_os = "linux")]
    {
        let gpus = detect_switcheroo();
        if gpus.len() >= 2 { gpus } else { detect_sysfs() }
    }
    #[cfg(windows)]
    {
        detect_windows()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

/// Whether the setting means anything on this system.
pub fn supported() -> bool {
    !cfg!(target_os = "macos")
}

/// Whether a single card can be picked by name (Linux); Windows has only power saving and high performance.
pub fn can_pick_by_name() -> bool {
    cfg!(target_os = "linux")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SWITCHEROO: &str = "Device: 0
  Name:        Advanced Micro Devices, Inc. [AMD/ATI] Cezanne [Radeon\u{2122} Vega Series / Radeon\u{2122} Vega Mobile Series]
  Default:     yes
  Discrete:    no
  Environment: DRI_PRIME=pci-0000_06_00_0 VK_LOADER_DRIVERS_SELECT=*radeon*

Device: 1
  Name:        NVIDIA Corporation TU117M [GeForce GTX 1650 Mobile / Max-Q]
  Default:     no
  Discrete:    yes
  Environment: __GLX_VENDOR_LIBRARY_NAME=nvidia __NV_PRIME_RENDER_OFFLOAD=1 __VK_LAYER_NV_optimus=NVIDIA_only VK_LOADER_DRIVERS_SELECT=*nvidia*
";

    fn two() -> Vec<Gpu> {
        parse_switcherooctl(SWITCHEROO)
    }

    fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn switcherooctl_output_is_read() {
        let gpus = two();
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "AMD Radeon Vega Series");
        assert!(gpus[0].default && !gpus[0].discrete);
        assert_eq!(gpus[0].id, "pci-0000_06_00_0");
        assert_eq!(gpus[1].name, "NVIDIA GeForce GTX 1650 Mobile");
        assert!(gpus[1].discrete && !gpus[1].default);
        assert_eq!(gpus[1].id, "nvidia");
        assert_eq!(get(&gpus[1].env, "__NV_PRIME_RENDER_OFFLOAD"), Some("1"));
        assert_eq!(get(&gpus[0].env, "DRI_PRIME"), Some("pci-0000_06_00_0"));
        assert!(parse_switcherooctl("").is_empty());
        assert!(parse_switcherooctl("Device: 0\n").is_empty());
    }

    #[test]
    fn names_are_shortened() {
        assert_eq!(
            short_name("NVIDIA Corporation TU117M [GeForce GTX 1650 Mobile / Max-Q]"),
            "NVIDIA GeForce GTX 1650 Mobile"
        );
        assert_eq!(short_name("Intel Corporation UHD Graphics 630 (Mobile)"), "Intel UHD Graphics 630 (Mobile)");
        assert_eq!(short_name("NVIDIA GeForce RTX 3060 Laptop GPU"), "NVIDIA GeForce RTX 3060 Laptop GPU");
        assert_eq!(short_name("AMD Radeon(TM) Graphics"), "AMD Radeon(TM) Graphics");
    }

    #[test]
    fn choices_are_read_and_written() {
        assert_eq!(Choice::parse(""), Choice::Auto);
        assert_eq!(Choice::parse("auto"), Choice::Auto);
        assert_eq!(Choice::parse(" dedicated "), Choice::Dedicated);
        assert_eq!(Choice::parse("integrated"), Choice::Integrated);
        assert_eq!(Choice::parse("nvidia"), Choice::Specific("nvidia".into()));
        for c in [Choice::Auto, Choice::Integrated, Choice::Dedicated, Choice::Specific("pci-0000_01_00_0".into())] {
            assert_eq!(Choice::parse(&c.as_setting()), c);
        }
    }

    #[test]
    fn dedicated_uses_the_fast_card_and_integrated_the_small_one() {
        let gpus = two();
        let fast = effect(&Choice::Dedicated, &gpus);
        assert_eq!(fast.gpu.as_deref(), Some("NVIDIA GeForce GTX 1650 Mobile"));
        assert_eq!(get(&fast.env, "__NV_PRIME_RENDER_OFFLOAD"), Some("1"));
        assert_eq!(get(&fast.env, "__GLX_VENDOR_LIBRARY_NAME"), Some("nvidia"));
        assert_eq!(get(&fast.env, "DRI_PRIME"), None);
        let small = effect(&Choice::Integrated, &gpus);
        assert_eq!(small.gpu.as_deref(), Some("AMD Radeon Vega Series"));
        assert_eq!(get(&small.env, "DRI_PRIME"), Some("pci-0000_06_00_0"));
        // the small card's setting never carries NVIDIA's offload variables
        assert_eq!(get(&small.env, "__NV_PRIME_RENDER_OFFLOAD"), None);
        let named = effect(&Choice::Specific("nvidia".into()), &gpus);
        assert_eq!(named, fast);
    }

    #[test]
    fn automatic_and_single_card_change_nothing() {
        assert_eq!(effect(&Choice::Auto, &two()), Effect::default());
        let one = vec![two().remove(0)];
        assert_eq!(effect(&Choice::Dedicated, &one), Effect::default());
        assert_eq!(effect(&Choice::Dedicated, &[]), Effect::default());
    }

    #[test]
    fn a_missing_card_gives_a_note_and_no_variables() {
        let e = effect(&Choice::Specific("pci-0000_99_00_0".into()), &two());
        assert!(e.env.is_empty() && e.gpu.is_none());
        assert!(e.note.unwrap().contains("isn't in this computer"));
        // two small cards: nothing for "dedicated" to pick
        let mut gpus = two();
        gpus[1].discrete = false;
        assert!(effect(&Choice::Dedicated, &gpus).note.is_some());
    }

    #[test]
    fn sysfs_cards_get_the_right_variables() {
        let cards = [
            SysCard { pci_address: "0000:01:00.0".into(), vendor: 0x10de, device: 0x1f9d, boot_vga: false },
            SysCard { pci_address: "0000:06:00.0".into(), vendor: 0x1002, device: 0x1638, boot_vga: true },
        ];
        let gpus = from_sysfs(&cards, |_, _| None, Some("/usr/share/glvnd/egl_vendor.d/10_nvidia.json"));
        // the default card comes first
        assert_eq!(gpus[0].id, "pci-0000_06_00_0");
        assert!(gpus[0].default && !gpus[0].discrete);
        assert_eq!(gpus[0].env, vec![("DRI_PRIME".to_string(), "pci-0000_06_00_0".to_string())]);
        assert_eq!(gpus[0].name, "AMD graphics (1002:1638)");
        assert!(gpus[1].discrete);
        assert_eq!(get(&gpus[1].env, "__NV_PRIME_RENDER_OFFLOAD"), Some("1"));
        assert_eq!(
            get(&gpus[1].env, "__EGL_VENDOR_LIBRARY_FILENAMES"),
            Some("/usr/share/glvnd/egl_vendor.d/10_nvidia.json")
        );
        assert_eq!(get(&effect(&Choice::Dedicated, &gpus).env, "__VK_LAYER_NV_optimus"), Some("NVIDIA_only"));
    }

    #[test]
    fn an_intel_and_an_amd_card_are_small_and_fast() {
        let cards = [
            SysCard { pci_address: "0000:00:02.0".into(), vendor: 0x8086, device: 0x9a49, boot_vga: true },
            SysCard { pci_address: "0000:03:00.0".into(), vendor: 0x1002, device: 0x73ff, boot_vga: false },
        ];
        let gpus = from_sysfs(&cards, |_, _| None, None);
        assert!(!gpus[0].discrete && gpus[0].vendor == Vendor::Intel);
        assert!(gpus[1].discrete);
        assert_eq!(get(&effect(&Choice::Dedicated, &gpus).env, "DRI_PRIME"), Some("pci-0000_03_00_0"));
    }

    #[test]
    fn pci_list_lookup() {
        let ids = "# comment\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t1638  Cezanne [Radeon Vega Series]\n10de  NVIDIA Corporation\n\t1f9d  TU117M [GeForce GTX 1650 Mobile / Max-Q]\n\t1f9e  Other\n";
        assert_eq!(
            pci_ids_lookup(ids, 0x10de, 0x1f9d).as_deref(),
            Some("NVIDIA Corporation TU117M [GeForce GTX 1650 Mobile / Max-Q]")
        );
        assert_eq!(pci_ids_lookup(ids, 0x10de, 0x9999), None);
        assert_eq!(pci_ids_lookup(ids, 0x1234, 0x1638), None);
        // a device of another vendor with the same number is not taken
        assert_eq!(pci_ids_lookup(ids, 0x10de, 0x1638), None);
    }

    #[test]
    fn windows_adapters_are_read() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\...\\0000\r\n    DriverDesc    REG_SZ    Intel(R) UHD Graphics 630\r\n\r\nHKEY_LOCAL_MACHINE\\...\\0001\r\n    DriverDesc    REG_SZ    NVIDIA GeForce GTX 1650\r\n\r\n...\\0002\r\n    DriverDesc    REG_SZ    Microsoft Basic Display Adapter\r\n";
        let names = parse_windows_adapters(out);
        assert_eq!(names, vec!["Intel(R) UHD Graphics 630", "NVIDIA GeForce GTX 1650"]);
        let gpus = windows_gpus(&names);
        assert!(!gpus[0].discrete && gpus[1].discrete);
        assert_eq!(windows_preference(&Choice::Dedicated), Some(2));
        assert_eq!(windows_preference(&Choice::Integrated), Some(1));
        assert_eq!(windows_preference(&Choice::Auto), None);
    }

    #[test]
    fn windows_preference_keeps_other_settings() {
        assert_eq!(merge_preference(None, Some(2)).as_deref(), Some("GpuPreference=2;"));
        assert_eq!(merge_preference(Some("GpuPreference=1;"), Some(2)).as_deref(), Some("GpuPreference=2;"));
        assert_eq!(
            merge_preference(Some("AutoHDREnable=2097;GpuPreference=1;"), Some(2)).as_deref(),
            Some("AutoHDREnable=2097;GpuPreference=2;")
        );
        assert_eq!(merge_preference(Some("GpuPreference=2;"), None), None);
        assert_eq!(
            merge_preference(Some("AutoHDREnable=2097;GpuPreference=2;"), None).as_deref(),
            Some("AutoHDREnable=2097;")
        );
        assert_eq!(merge_preference(None, None), None);
        assert_eq!(
            parse_reg_value(
                "\r\nHKEY_CURRENT_USER\\x\r\n    C:\\Java\\bin\\javaw.exe    REG_SZ    GpuPreference=2;\r\n",
                "C:\\Java\\bin\\javaw.exe"
            )
            .as_deref(),
            Some("GpuPreference=2;")
        );
    }
}
