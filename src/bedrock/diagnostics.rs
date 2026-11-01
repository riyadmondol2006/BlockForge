//! Telling the player what went wrong, from the logs and the host (package E of PORT-PLAN.md).
//!
//! Ported from BedrockOnLinux `bol/gamesetup.py` (`_DIAG_RULES`, `diagnose`) and `bol/doctor.py` with the checks it
//! runs (`hostarch`, `ntsync`, `perfcheck`), reworded for BlockForge. Two kinds of result:
//!
//! - **Findings**: causes found in a session's logs, each with a kind so the window and the error type can tell
//!   *unsupported driver*, *lost display*, *missing RNG*, *licence refused* and *Xbox pre-sign-in failure* apart (the
//!   plan insists they are reported separately). A successful wrapper start proves nothing; the logs decide.
//! - **Checks**: facts about this computer (processor, memory, RAM file system, kernel features, the graphics safety
//!   state) with a verdict each, for the diagnostics screen. They never open a GPU: the same rule as
//!   `gpu_safety.rs`.
//!
//! Nothing is repaired or relaunched from here: a diagnosis only diagnoses (a GPU session is never retried
//! automatically).

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::DiagnosticReport;
#[cfg(test)]
use super::{BedrockError, ErrorKind};

/// What a finding is about. The first group stops the game from being played at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// Microsoft refused the game licence because the account has no free Store device
    LicenceRefused,
    /// the licence could not be fetched: no connection to Microsoft
    LicenceUnreachable,
    /// the licensing service answered, and the answer was not a licence
    LicenceServiceRefused,
    /// Wine could not open a window: no display server
    LostDisplay,
    /// the graphics driver can't do what the game needs (device generated commands, Vulkan 1.3 ...)
    UnsupportedDriver,
    /// the graphics driver crashed or the device was lost while the game ran
    GpuCrash,
    /// the game is drawing on the CPU
    SoftwareRendering,
    /// `SystemFunction036` could not be resolved: Wine's services abort and every symptom looks like something else
    MissingRng,
    /// the Wine prefix is broken (explorer could not start)
    BrokenPrefix,
    /// the engine is damaged or from a mixed build
    EngineProblem,
    /// the sign-in material was loaded but the game identity wasn't (Xbox pre-sign-in)
    XboxPreauth,
    /// the account sign-in was refused in the game
    SignInFailed,
    OutOfMemory,
    /// the game itself crashed (an access violation inside it)
    GameCrash,
    /// a Realms/LAN/version mismatch advice
    Multiplayer,
    /// the game drew on another card than the one chosen
    AdapterMismatch,
    /// a problem with the player's own custom environment
    CustomEnvironment,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub text: String,
}

impl Finding {
    fn new(kind: FindingKind, text: impl Into<String>) -> Self {
        Finding { kind, text: text.into() }
    }

    /// The error a session that never played should give (licensing and display problems are the player's to fix).
    #[cfg(test)]
    pub fn into_error(self, log_path: Option<PathBuf>) -> BedrockError {
        let kind = match self.kind {
            FindingKind::LicenceRefused | FindingKind::LicenceServiceRefused => ErrorKind::NotEntitled,
            FindingKind::LicenceUnreachable => ErrorKind::LicenseUnavailable,
            FindingKind::SignInFailed => ErrorKind::SignInRequired,
            FindingKind::BrokenPrefix | FindingKind::EngineProblem | FindingKind::MissingRng => ErrorKind::Prefix,
            _ => ErrorKind::Runtime,
        };
        BedrockError { kind, message: self.text, log_path }
    }

    /// Does this finding mean the game never started (so the session is a failure and not just a note)?
    pub fn is_fatal(&self) -> bool {
        matches!(
            self.kind,
            FindingKind::LicenceRefused | FindingKind::LicenceUnreachable | FindingKind::LicenceServiceRefused
        )
    }
}

pub const LICENCE_REFUSED: &str = "Microsoft refused the game licence: this account has reached its limit of ten Microsoft Store download devices. Remove the ones you no longer use at https://account.microsoft.com/devices/content, then play again.";
pub const LICENCE_UNREACHABLE: &str = "Minecraft could not start because its licence could not be fetched: no connection to Microsoft. A game downloaded from the Microsoft Store stays encrypted on disk, and Microsoft hands out the key that decrypts it each time it starts, so it needs an internet connection to start, single-player included. Connect to the internet and play again.";
pub const LICENCE_SERVICE_REFUSED: &str =
    "Microsoft's licensing service refused the licence Minecraft needs to start, so the game never opened";

struct Rule {
    pattern: Regex,
    kind: FindingKind,
    text: &'static str,
}

fn rules() -> &'static [Rule] {
    static RULES: std::sync::OnceLock<Vec<Rule>> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        let r = |p: &str, kind, text| Rule { pattern: Regex::new(&format!("(?is){p}")).unwrap(), kind, text };
        vec![
            r(
                r"d3d12_command_signature_init_state_template_dgc_(?:ext|nv):.*Cannot implement command signature|d3d12_command_signature_create: Device generated commands is not supported by implementation",
                FindingKind::UnsupportedDriver,
                "Minecraft's menu needs Vulkan Device Generated Commands, which this graphics driver did not provide. Repair the game engine first (the common cause); if that changes nothing the driver itself can't provide it: on an Intel discrete card it is exposed only under the 'xe' kernel driver, not 'i915'; otherwise update the Vulkan driver or use a card that can.",
            ),
            r(r"Unimplemented function\s+combase\.dll\.RoOriginateErrorW?\b", FindingKind::EngineProblem, "The engine's combase patch is missing; repair the game engine."),
            r(r"Unimplemented function\s+ntdll\.dll\.NtQueryWnfStateData\b", FindingKind::EngineProblem, "The engine's ntdll patch is missing; repair the game engine."),
            r(
                r"Loading library user32\.dll[^\r\n]*failed \(error c0000020\)",
                FindingKind::BrokenPrefix,
                "Wine's user32.dll in the prefix could not be loaded (invalid image); repairing the game repairs the prefix's engine files.",
            ),
            r(
                r"vkGetPhysicalDeviceSurfaceFormatsKHR|Can't open display|x11drv: Can't",
                FindingKind::LostDisplay,
                "The display is unavailable: no X or Wayland server answered, or it went away while the game ran.",
            ),
            r(
                r"VK_ERROR_DEVICE_LOST|device removed|DXGI_ERROR|vkd3d.*fatal|VK_ERROR_OUT_OF_DEVICE_MEMORY",
                FindingKind::GpuCrash,
                "The graphics driver crashed or ran out of video memory: update the driver or lower the graphics settings.",
            ),
            r(r"Cannot allocate memory|OutOfMemory|std::bad_alloc", FindingKind::OutOfMemory, "Out of memory (RAM or video memory)."),
            r(
                r"wineserver.*version mismatch|wineserver binary was not upgraded",
                FindingKind::EngineProblem,
                "The engine is a mix of two builds (wine and wineserver differ); repair the game engine.",
            ),
            // Wine prints these when it is about to hand the process to the crash debugger: the game's own fatal fault.
            r(
                r"wine: Unhandled page fault on \w+ access to [0-9A-Fa-f]+ at address|wine: Unhandled exception 0xc0000005",
                FindingKind::GameCrash,
                "Minecraft itself crashed with a memory access violation. The launcher, the prefix and the graphics driver are not the failing part, so repairing the prefix changes nothing: note the faulting address and the engine revision from the log, try another Minecraft version, and attach both to a bug report.",
            ),
            r(r"Authentication failed|invalid_grant|login.*failed", FindingKind::SignInFailed, "Microsoft sign-in failed in the game: sign in again."),
            r(r"Device group is full", FindingKind::LicenceRefused, LICENCE_REFUSED),
            r(
                r"panicked at [^\n]*license\.rs[^\n]*\n[^\n]*(?:dns error|failed to lookup address|ConnectError|Connect,|timed out|Connection refused|Network is unreachable|error sending request)",
                FindingKind::LicenceUnreachable,
                LICENCE_UNREACHABLE,
            ),
            r(
                r"\bInitialConnection[-_: ]*13(?:\D|$)",
                FindingKind::Multiplayer,
                "LAN InitialConnection-13: check that the host's firewall allows Minecraft's inbound RakNet UDP 19132 and, on Windows, that its network profile is Private.",
            ),
            r(
                r"\bInitialConnection[-_: ]*25(?:\D|$)",
                FindingKind::Multiplayer,
                "LAN InitialConnection-25 ('world full'): the host may have a stale session rather than a real capacity limit. On a Windows host, change its network profile from Public to Private, toggle Multiplayer Game off and on and restart both games.",
            ),
            r(
                r"\b(?:IncompatibleVersion|VersionMismatch|ProtocolVersionMismatch)\b|\b(?:outdated client|outdated server)\b|\bversion\s+mismatch\b",
                FindingKind::Multiplayer,
                "Minecraft client and host versions differ: pick exactly the host's Bedrock version before joining.",
            ),
            // before the window rule: with the RNG unresolved every service aborts and the symptom is a missing window
            r(
                r"unimplemented function advapi32\.dll\.SystemFunction036|forward 'cryptbase\.SystemFunction036'|module not found for forward 'cryptbase",
                FindingKind::MissingRng,
                "Wine's random number source is unresolved (cryptbase.SystemFunction036), so its services abort. This is not a broken prefix or a graphics fault: repair the game so the verified native one is installed, then play again.",
            ),
            r(
                r"nodrv_CreateWindow|no driver could be loaded|explorer process failed to start",
                FindingKind::BrokenPrefix,
                "The Wine prefix is broken: Wine couldn't open a window. Use Repair once; the game is not restarted automatically.",
            ),
            r(
                r"(?:XUserAddAsync|xgameruntime:.*XUser\w*).*(?:unimpl|stub|not implemented|E_NOTIMPL|0x80004001)|QueryApiImpl.*(?:unimpl|stub|not implemented|E_NOTIMPL|0x80004001)",
                FindingKind::EngineProblem,
                "The engine in use has no XUser support; repair the game engine.",
            ),
        ]
    })
}

fn licence_service_refusal(text: &str) -> Option<String> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?is)the licensing service refused the request \(([^)\n]{1,40})\):?[ \t]*([^\n]{0,160})|error decoding response body for url \(https://licensing\.mp\.microsoft\.com").unwrap()
    });
    let c = re.captures(text)?;
    let status = c.get(1).map(|m| m.as_str()).unwrap_or("");
    let reason = c.get(2).map(|m| m.as_str().trim()).unwrap_or("");
    let detail = if status.is_empty() {
        String::new()
    } else {
        format!(" ({status}{})", if reason.is_empty() { String::new() } else { format!(": {reason}") })
    };
    Some(format!(
        "{LICENCE_SERVICE_REFUSED}{detail}. Sign in to the Microsoft Store again with the account that owns Minecraft, then play. If it is refused again, attach the game log to a bug report."
    ))
}

/// Names of the graphics devices the game's graphics layer listed (`Found device: ...`).
pub fn adapters_in_log(text: &str) -> Vec<String> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)Found device:\s*([^\r\n]+)").unwrap());
    let mut out: Vec<String> = Vec::new();
    for c in re.captures_iter(text) {
        let name = c[1].trim().to_string();
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Does a device name from the log fit the card that was chosen? Compared on the distinctive words (the model number
/// and vendor), not the exact marketing text, which differs between the PCI list and the driver.
pub fn adapter_fits(chosen: &str, logged: &str) -> bool {
    const GENERIC: &[&str] = &[
        "graphics",
        "mobile",
        "series",
        "corporation",
        "technologies",
        "inc",
        "ltd",
        "device",
        "gpu",
        "with",
        "design",
        "max",
        "radeon",
        "geforce",
        "nvidia",
        "amd",
        "intel",
        "ati",
    ];
    let words = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| w.len() >= 3)
            .map(str::to_string)
            .collect()
    };
    let logged_l = logged.to_lowercase();
    let distinctive: Vec<String> = words(chosen).into_iter().filter(|w| !GENERIC.contains(&w.as_str())).collect();
    if distinctive.is_empty() {
        // only a vendor to go on
        return words(chosen).iter().any(|w| logged_l.contains(w.as_str()));
    }
    distinctive.iter().any(|w| logged_l.contains(w.as_str()))
}

/// Facts the log can't give, supplied by the caller.
#[derive(Clone, Debug, Default)]
pub struct LogContext {
    /// the card the game was told to use (its name), to compare with the one it drew on
    pub expected_gpu: Option<String>,
    /// the host draws through a virtual GPU (checked by [`host_has_virtio_gpu`])
    pub virtual_gpu: bool,
    /// variables in the player's own environment that override what BlockForge sets
    pub env_overrides: Vec<String>,
}

/// The likely causes in the text of a session's logs. Order: the cause nearest the start of the rule list first.
pub fn diagnose_text(text: &str, ctx: &LogContext) -> Vec<Finding> {
    let mut hits: Vec<Finding> = Vec::new();
    for rule in rules() {
        if rule.pattern.is_match(text) {
            hits.push(Finding::new(rule.kind, rule.text));
        }
    }
    let has = |hits: &[Finding], k: FindingKind| hits.iter().any(|h| h.kind == k);
    // the game never ran: the licence request died before sign-in could have, and its panic names login.live.com
    if has(&hits, FindingKind::LicenceUnreachable) {
        hits.retain(|h| h.kind != FindingKind::SignInFailed);
    }
    if let Some(refusal) = licence_service_refusal(text) {
        if !has(&hits, FindingKind::LicenceRefused) {
            hits.push(Finding::new(FindingKind::LicenceServiceRefused, refusal));
        }
    }
    // positive evidence wins: if the engine logged its XUser patches or a good pre-sign-in, XUser is there
    static PATCHED: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let patched =
        PATCHED.get_or_init(|| Regex::new(r"(?i)InitializeApiImplEx2 patched|preauth: loaded user/XSTS").unwrap());
    if patched.is_match(text) {
        hits.retain(|h| !h.text.contains("no XUser support"));
    }
    static LOADED: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static IDENT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if LOADED.get_or_init(|| Regex::new(r"(?i)preauth: loaded user/XSTS").unwrap()).is_match(text)
        && !IDENT.get_or_init(|| Regex::new(r"(?i)native XGame identity loaded").unwrap()).is_match(text)
    {
        hits.push(Finding::new(
            FindingKind::XboxPreauth,
            "Xbox credentials loaded, but the game's identity did not initialise. Repair the game engine and attach the diagnostics log if the online tabs stay locked.",
        ));
    }
    let devices = adapters_in_log(text);
    let software_only = !devices.is_empty() && devices.iter().all(|d| d.to_lowercase().contains("llvmpipe"));
    if software_only {
        hits.push(Finding::new(
            FindingKind::SoftwareRendering,
            "The game runs on software rendering (llvmpipe): the graphics card's Vulkan driver isn't active, so the game runs on the CPU (slow, and the Play screen may render black). Reboot, or install or enable the card's Vulkan drivers.",
        ));
    } else if let Some(chosen) = ctx.expected_gpu.as_deref() {
        if !devices.is_empty() && !devices.iter().any(|d| adapter_fits(chosen, d)) {
            hits.push(Finding::new(
                FindingKind::AdapterMismatch,
                format!("The game was told to use {chosen}, but its graphics layer listed {}. The card choice did not reach the game; check the Vulkan environment inside the container.", devices.join(", ")),
            ));
        }
    }
    static NO13: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static NODXVK: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let no13 = NO13.get_or_init(|| {
        Regex::new(r"(?i)Skipping:\s*Device does not support Vulkan 1\.3|A Vulkan 1\.3 capable setup is required")
            .unwrap()
    });
    let nodxvk = NODXVK.get_or_init(|| Regex::new(r"(?i)DXVK:\s*No adapters found|Failed to initialize DXVK").unwrap());
    if no13.is_match(text) && nodxvk.is_match(text) && !software_only {
        hits.push(Finding::new(
            FindingKind::UnsupportedDriver,
            "No usable Vulkan 1.3 adapter was found: this graphics driver can't provide what Minecraft's Direct3D 12 renderer needs. Update the Vulkan driver; a legacy renderer exists only as a last resort and loses ray tracing.",
        ));
    }
    if has(&hits, FindingKind::GameCrash) && ctx.virtual_gpu {
        hits.push(Finding::new(
            FindingKind::UnsupportedDriver,
            "This computer draws through a virtual GPU (virtio-gpu, as in a virtual machine or ChromeOS's Linux container), which Minecraft's Direct3D 12 renderer can crash on. A legacy renderer is the way to try; it costs visual quality and ray tracing.",
        ));
    }
    if !ctx.env_overrides.is_empty() {
        hits.push(Finding::new(
            FindingKind::CustomEnvironment,
            format!("Custom environment variables override BlockForge's settings: {}. Clear them and play again before reinstalling anything.", ctx.env_overrides.join(", ")),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    hits.retain(|h| seen.insert(h.text.clone()));
    hits
}

/// A log file's start and end (a crash is near the end, proof of initialisation near the start); the middle of a
/// huge diagnostic trace is left out rather than loaded.
pub fn read_edges(path: &Path, edge: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len() as usize;
    let mut raw = Vec::new();
    if size <= edge * 2 {
        f.read_to_end(&mut raw).ok()?;
    } else {
        let mut head = vec![0; edge];
        f.read_exact(&mut head).ok()?;
        raw.extend_from_slice(&head);
        raw.extend_from_slice(b"\n[... log middle omitted ...]\n");
        f.seek(SeekFrom::End(-(edge as i64))).ok()?;
        let mut tail = vec![0; edge];
        f.read_exact(&mut tail).ok()?;
        raw.extend_from_slice(&tail);
    }
    Some(String::from_utf8_lossy(&raw).into_owned())
}

/// The logs of one session folder that findings come from.
pub const LOG_FILES: &[&str] = &["runtime.log", "game.log", "setup.log", "license.log"];

/// Read a session's logs (`<log_dir>/{runtime,game,setup,license}.log`) and diagnose them.
pub fn diagnose_dir(log_dir: &Path, ctx: &LogContext) -> Vec<Finding> {
    let text: String =
        LOG_FILES.iter().filter_map(|n| read_edges(&log_dir.join(n), 200_000)).collect::<Vec<_>>().join("\n");
    diagnose_text(&text, ctx)
}

/// Whether a virtio GPU is among the host's graphics devices (a virtual machine or ChromeOS's container).
pub fn host_has_virtio_gpu() -> bool {
    std::fs::read_dir("/sys/class/drm")
        .map(|rd| {
            rd.flatten().any(|e| {
                std::fs::read_to_string(e.path().join("device/vendor"))
                    .is_ok_and(|v| v.trim().eq_ignore_ascii_case("0x1af4"))
            })
        })
        .unwrap_or(false)
}

// ---- the host

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Ok,
    /// worth knowing; the game can still start
    Warn,
    /// the game will not start or not run properly
    Fail,
    /// a fact, not a verdict
    Info,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub verdict: Verdict,
    pub text: String,
}

/// What was read from the computer. Filled by [`HostFacts::probe`]; a test fills it by hand.
#[derive(Clone, Debug, Default)]
pub struct HostFacts {
    pub machine: String,
    /// an interpreter registered for x86-64 programs (FEX, Box64, qemu) when the machine isn't x86-64
    pub x86_emulator: Option<String>,
    pub kernel_release: String,
    pub mem_available_mib: Option<u64>,
    pub mem_total_mib: Option<u64>,
    pub swap_used_mib: Option<u64>,
    pub free_disk_mib: Option<u64>,
    pub data_dir: Option<PathBuf>,
    /// RAM file system for the decrypted game image: its root, and its free space
    pub staging_root: Option<PathBuf>,
    pub staging_free_mib: Option<u64>,
    pub ntsync_device: bool,
    /// `Some(true)` when the engine's wineserver was built with ntsync support, `None` when unknown
    pub engine_has_ntsync: Option<bool>,
    pub no_ntsync_env: bool,
    pub session_type: String,
    pub has_display: bool,
    pub has_xrandr: bool,
    pub has_journalctl: bool,
    pub vulkan_icd_found: bool,
    pub gpus: Vec<String>,
}

/// The game settles around this much memory once a world is loaded.
pub const LOW_MEMORY_MIB: u64 = 2048;
pub const LOW_DISK_MIB: u64 = 5 * 1024;
/// The decrypted executable and DLLs of a build: well under this, measured by the plan's own sizes at launch time.
pub const MIN_STAGING_MIB: u64 = 512;

fn meminfo_mib(text: &str, field: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(field)?.strip_prefix(':')?;
        let kb: u64 = rest.trim().strip_suffix("kB")?.trim().parse().ok()?;
        Some(kb / 1024)
    })
}

#[cfg(target_os = "linux")]
impl HostFacts {
    pub fn probe(data_dir: &Path, engine_root: Option<&Path>, gpus: &[crate::gpu::Gpu]) -> HostFacts {
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let swap_total = meminfo_mib(&meminfo, "SwapTotal");
        let swap_free = meminfo_mib(&meminfo, "SwapFree");
        let staging = super::supervisor::staging_system_root();
        let machine = std::env::consts::ARCH.to_string();
        HostFacts {
            x86_emulator: if machine == "x86_64" {
                None
            } else {
                x86_64_emulator(Path::new("/proc/sys/fs/binfmt_misc"))
            },
            machine,
            kernel_release: std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            mem_available_mib: meminfo_mib(&meminfo, "MemAvailable"),
            mem_total_mib: meminfo_mib(&meminfo, "MemTotal"),
            swap_used_mib: swap_total.zip(swap_free).map(|(t, f)| t.saturating_sub(f)),
            free_disk_mib: super::supervisor::free_space(data_dir).map(|(free, _)| free >> 20),
            data_dir: Some(data_dir.to_path_buf()),
            staging_free_mib: staging
                .as_ref()
                .and_then(|s| super::supervisor::free_space(s.parent().unwrap_or(s)))
                .map(|(free, _)| free >> 20),
            staging_root: staging,
            ntsync_device: Path::new("/dev/ntsync").exists(),
            engine_has_ntsync: engine_root.and_then(engine_supports_ntsync),
            no_ntsync_env: std::env::var("PROTON_NO_NTSYNC")
                .is_ok_and(|v| !matches!(v.trim().to_lowercase().as_str(), "" | "0" | "no" | "off" | "false")),
            session_type: std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
            has_display: std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some(),
            has_xrandr: crate::launch::executable_in_path("xrandr"),
            has_journalctl: crate::launch::executable_in_path("journalctl"),
            vulkan_icd_found: ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d", "/usr/local/share/vulkan/icd.d"]
                .iter()
                .any(|d| std::fs::read_dir(d).is_ok_and(|mut rd| rd.next().is_some())),
            gpus: gpus.iter().map(|g| g.name.clone()).collect(),
        }
    }
}

/// An interpreter registered with the kernel for x86-64 programs (an ARM computer runs the game through one).
pub fn x86_64_emulator(binfmt: &Path) -> Option<String> {
    let mut entries: Vec<_> = std::fs::read_dir(binfmt).ok()?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if name == "register" || name == "status" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some("enabled") {
            continue;
        }
        let (mut magic, mut interp) = (String::new(), String::new());
        for l in lines {
            if let Some((k, v)) = l.split_once(' ') {
                match k.trim_end_matches(':') {
                    "magic" => magic = v.replace(' ', "").to_lowercase(),
                    "interpreter" => interp = v.trim().to_string(),
                    _ => {}
                }
            }
        }
        // an x86-64 ELF: 64-bit, with e_machine 0x3e at byte 18
        if magic.starts_with("7f454c4602") && magic.get(36..40) == Some("3e00") {
            return Some(if interp.is_empty() { name } else { interp });
        }
    }
    None
}

/// Whether the engine's wineserver mentions `/dev/ntsync` (the literal is compiled in only when the build found the
/// kernel header). `None` when no wineserver could be read.
#[cfg(target_os = "linux")]
pub fn engine_supports_ntsync(engine_root: &Path) -> Option<bool> {
    use std::io::Read;
    const NEEDLE: &[u8] = b"/dev/ntsync";
    let mut verdict = None;
    for rel in ["files/bin-wow64/wineserver", "files/bin/wineserver"] {
        let Ok(mut f) = std::fs::File::open(engine_root.join(rel)) else { continue };
        let mut tail: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 1 << 20];
        let mut found = false;
        while let Ok(n) = f.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut window = std::mem::take(&mut tail);
            window.extend_from_slice(&chunk[..n]);
            if window.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
                found = true;
                break;
            }
            tail = window[window.len().saturating_sub(NEEDLE.len() - 1)..].to_vec();
        }
        if found {
            return Some(true);
        }
        verdict = Some(false);
    }
    verdict
}

/// The verdicts on a computer's facts. Pure, so a test can try every case.
pub fn evaluate_host(f: &HostFacts) -> Vec<Check> {
    let mut out = Vec::new();
    let mut add = |name: &str, verdict, text: String| out.push(Check { name: name.into(), verdict, text });
    // the game, the engine and the runtime are x86-64 programs
    if matches!(f.machine.as_str(), "x86_64" | "amd64") {
        add("processor", Verdict::Ok, format!("OK ({})", f.machine));
    } else if let Some(emu) = &f.x86_emulator {
        add("processor", Verdict::Warn, format!("{}: x86-64 programs run through {emu} (experimental)", f.machine));
    } else {
        add(
            "processor",
            Verdict::Fail,
            format!(
                "This computer's processor is {}, and Minecraft for Windows, Wine and the runtime are x86-64 programs. On an ARM computer they run through an x86-64 emulator registered with the kernel, and none is. Install FEX-Emu with its x86-64 RootFS, or Box64, then try again.",
                if f.machine.is_empty() { "unknown" } else { &f.machine }
            ),
        );
    }
    match f.mem_available_mib {
        Some(m) if m < LOW_MEMORY_MIB => {
            let swap = f
                .swap_used_mib
                .filter(|s| *s > 0)
                .map(|s| format!(" {:.1} GiB is already in swap.", s as f64 / 1024.0))
                .unwrap_or_default();
            add(
                "memory",
                Verdict::Warn,
                format!(
                    "Only {m} MiB of memory is available, and Minecraft settles around {LOW_MEMORY_MIB} MiB once a world is loaded.{swap} Close what you are not using before playing, or lower the render distance."
                ),
            );
        }
        Some(m) => add(
            "memory",
            Verdict::Ok,
            format!("OK ({m} MiB available{})", f.mem_total_mib.map(|t| format!(" of {t}")).unwrap_or_default()),
        ),
        None => add("memory", Verdict::Info, "unknown".into()),
    }
    match f.free_disk_mib {
        Some(m) if m < LOW_DISK_MIB => add(
            "disk",
            Verdict::Warn,
            format!(
                "Only {m} MiB is free where the game lives{}. Shader caches can't grow there and a world save or update can fail outright. Free a few GiB before playing.",
                f.data_dir.as_ref().map(|d| format!(" ({})", d.display())).unwrap_or_default()
            ),
        ),
        Some(m) => add("disk", Verdict::Ok, format!("OK ({m} MiB free)")),
        None => add("disk", Verdict::Info, "unknown".into()),
    }
    // the decrypted image goes to RAM only; with no RAM file system the game can't start
    match (&f.staging_root, f.staging_free_mib) {
        (None, _) => add("game files in RAM", Verdict::Fail, "No RAM file system (tmpfs) is available for the game's decrypted files, and BlockForge never writes them to disk. Mount /dev/shm or use a desktop session with XDG_RUNTIME_DIR.".into()),
        (Some(root), Some(m)) if m < MIN_STAGING_MIB => add(
            "game files in RAM",
            Verdict::Fail,
            format!("Only {m} MiB is free in {} for the game's decrypted files; it needs at least {MIN_STAGING_MIB} MiB. Close programs or enlarge the tmpfs.", root.display()),
        ),
        (Some(root), free) => add("game files in RAM", Verdict::Ok, format!("OK ({}{})", root.display(), free.map(|m| format!(", {m} MiB free")).unwrap_or_default())),
    }
    // Wine 11 has no esync/fsync: without ntsync every wait is a wineserver round trip
    if f.no_ntsync_env {
        add("fast sync", Verdict::Warn, "PROTON_NO_NTSYNC is set, which turns off Wine's in-process synchronization. This engine has no fallback, so every wait becomes a wineserver round trip: expect low frame rates and stuttering chunk generation. Clear it.".into());
    } else {
        match (f.engine_has_ntsync, f.ntsync_device) {
            (Some(false), _) => add("fast sync", Verdict::Warn, "The installed engine was built without Wine's in-process synchronization (ntsync) and has no fallback, so every wait becomes a wineserver round trip. Repair the game to fetch an engine built with it.".into()),
            (Some(true), false) => add(
                "fast sync",
                Verdict::Warn,
                format!(
                    "This kernel does not provide /dev/ntsync, so Wine falls back to wineserver round trips: low frame rates and stuttering chunk generation. The driver is in mainline Linux 6.14 and later (some distributions backport it); this kernel is {}. If it is a module, `sudo modprobe ntsync` and make that persistent.",
                    if f.kernel_release.is_empty() { "unknown" } else { &f.kernel_release }
                ),
            ),
            (Some(true), true) => add("fast sync", Verdict::Ok, "OK (ntsync)".into()),
            (None, true) => add("fast sync", Verdict::Ok, "OK (kernel has ntsync)".into()),
            (None, false) => add("fast sync", Verdict::Info, "unknown (no engine to inspect, and /dev/ntsync is absent)".into()),
        }
    }
    let display = if f.session_type.is_empty() { "unknown session" } else { &f.session_type };
    if f.has_display {
        add("display", Verdict::Ok, format!("OK ({display}; the game uses X11/XWayland by default)"));
    } else {
        add("display", Verdict::Fail, "No display server is reachable (neither DISPLAY nor WAYLAND_DISPLAY is set), so the game window can't open.".into());
    }
    if !f.gpus.is_empty() {
        add("graphics cards", Verdict::Info, f.gpus.join(", "));
    }
    if !f.vulkan_icd_found {
        add("vulkan", Verdict::Warn, "No Vulkan driver description was found (/usr/share/vulkan/icd.d is empty or missing). Install your card's Vulkan driver; nothing was opened to check.".into());
    }
    if f.session_type == "x11" && !f.has_xrandr {
        add("xrandr", Verdict::Warn, "xrandr isn't installed, so BlockForge can't confirm the X11 session has a hardware GPU provider before starting.".into());
    }
    if !f.has_journalctl {
        add(
            "kernel log",
            Verdict::Warn,
            "journalctl isn't available, so earlier graphics-driver faults can't be checked before starting.".into(),
        );
    }
    out
}

/// Everything the diagnostics screen shows: host checks, the graphics safety state and the findings of the last
/// session's logs, as one list of plain sentences plus the log folder.
pub struct ReportInput {
    pub facts: HostFacts,
    pub graphics_refusal: Option<String>,
    pub graphics_notes: Vec<String>,
    pub findings: Vec<Finding>,
    pub log_dir: PathBuf,
}

pub struct Report {
    pub checks: Vec<Check>,
    pub findings: Vec<Finding>,
    pub log_dir: PathBuf,
}

pub fn build_report(input: &ReportInput) -> Report {
    let mut checks = evaluate_host(&input.facts);
    match &input.graphics_refusal {
        Some(r) => {
            checks.push(Check { name: "graphics safety".into(), verdict: Verdict::Fail, text: format!("BLOCKED: {r}") })
        }
        None => checks.push(Check {
            name: "graphics safety".into(),
            verdict: Verdict::Ok,
            text: "OK (no unsafe state found; no GPU was opened to check)".into(),
        }),
    }
    for n in &input.graphics_notes {
        checks.push(Check { name: "graphics safety".into(), verdict: Verdict::Warn, text: n.clone() });
    }
    Report { checks, findings: input.findings.clone(), log_dir: input.log_dir.clone() }
}

impl Report {
    /// The public report type: every problem as one sentence (failures first), and the log folder.
    pub fn into_public(self) -> DiagnosticReport {
        let mut issues: Vec<String> = Vec::new();
        for v in [Verdict::Fail, Verdict::Warn] {
            for c in self.checks.iter().filter(|c| c.verdict == v) {
                issues.push(format!("{}: {}", c.name, c.text));
            }
        }
        issues.extend(self.findings.iter().map(|f| f.text.clone()));
        DiagnosticReport { issues, log_dir: self.log_dir }
    }

    /// The text written to `diagnostics.log` (the host section and the findings). Nothing secret is in it, but it goes
    /// through the same redaction as the other logs when written.
    pub fn text(&self) -> String {
        let mut s = String::from("BlockForge Bedrock diagnostics\n\n");
        for c in &self.checks {
            let tag = match c.verdict {
                Verdict::Ok => "ok  ",
                Verdict::Warn => "warn",
                Verdict::Fail => "FAIL",
                Verdict::Info => "info",
            };
            s.push_str(&format!("[{tag}] {:18} {}\n", c.name, c.text));
        }
        if self.findings.is_empty() {
            s.push_str("\nNo known cause found in the logs.\n");
        } else {
            s.push_str("\nLikely cause:\n");
            for f in &self.findings {
                s.push_str(&format!("  - ({:?}) {}\n", f.kind, f.text));
            }
        }
        s
    }
}

/// Write `diagnostics.log` into the session's log folder (0600).
pub fn write_report(log_dir: &Path, report: &Report) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(log_dir)?;
    let path = log_dir.join("diagnostics.log");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
        f.write_all(report.text().as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&path, report.text())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    fn kinds(text: &str) -> Vec<FindingKind> {
        diagnose_text(text, &LogContext::default()).iter().map(|f| f.kind).collect()
    }

    #[test]
    fn the_five_causes_the_plan_names_are_told_apart() {
        // unsupported driver
        let k = kinds("d3d12_command_signature_create: Device generated commands is not supported by implementation");
        assert_eq!(k, vec![FindingKind::UnsupportedDriver]);
        // lost display
        assert_eq!(kinds("x11drv: Can't open display: :0"), vec![FindingKind::LostDisplay]);
        // missing RNG: its symptom is a missing window, which must not read as a broken prefix
        let rng = "err:ntdll: unimplemented function advapi32.dll.SystemFunction036 called\nnodrv_CreateWindow";
        let k = kinds(rng);
        assert!(k.contains(&FindingKind::MissingRng) && k.contains(&FindingKind::BrokenPrefix), "{k:?}");
        // licensing refusal and unreachable service are different findings, both fatal
        let refused = diagnose_text("Error: Device group is full", &LogContext::default());
        assert_eq!(refused[0].kind, FindingKind::LicenceRefused);
        assert!(refused[0].is_fatal());
        let unreachable = diagnose_text(
            "thread 'main' panicked at crates/xodus-cli/src/license.rs:51:6:\ncalled `Result::unwrap()` on an `Err` value: Request(reqwest::Error { kind: Request, url: \"https://login.live.com/RST2.srf\", source: dns error",
            &LogContext::default(),
        );
        assert_eq!(unreachable[0].kind, FindingKind::LicenceUnreachable);
        // and sign-in failure is dropped when the licence call died first (its panic names login.live.com)
        let both = diagnose_text(
            "login.live.com login failed\nthread 'main' panicked at src/license.rs:51:6:\n failed to lookup address",
            &LogContext::default(),
        );
        assert!(both.iter().all(|f| f.kind != FindingKind::SignInFailed), "{both:?}");
        let svc = diagnose_text(
            "the licensing service refused the request (403 Forbidden): not entitled",
            &LogContext::default(),
        );
        assert_eq!(svc[0].kind, FindingKind::LicenceServiceRefused);
        assert!(svc[0].text.contains("403 Forbidden: not entitled"));
        // Xbox pre-sign-in: loaded, but no identity
        let x = kinds("preauth: loaded user/XSTS for account\nInitializeApiImplEx2 patched");
        assert_eq!(x, vec![FindingKind::XboxPreauth]);
        assert!(kinds("preauth: loaded user/XSTS\nnative XGame identity loaded").is_empty());
    }

    #[test]
    fn findings_become_the_right_errors() {
        let e = Finding::new(FindingKind::LicenceRefused, LICENCE_REFUSED).into_error(None);
        assert!(matches!(e.kind, ErrorKind::NotEntitled));
        let e = Finding::new(FindingKind::LicenceUnreachable, LICENCE_UNREACHABLE).into_error(Some("/l".into()));
        assert!(matches!(e.kind, ErrorKind::LicenseUnavailable) && e.log_path.is_some());
        assert!(matches!(Finding::new(FindingKind::MissingRng, "x").into_error(None).kind, ErrorKind::Prefix));
        assert!(matches!(Finding::new(FindingKind::LostDisplay, "x").into_error(None).kind, ErrorKind::Runtime));
        assert!(!Finding::new(FindingKind::LostDisplay, "x").is_fatal());
    }

    #[test]
    fn the_adapter_in_the_log_is_checked_against_the_choice() {
        let log = "Found device: NVIDIA GeForce GTX 1650 with Max-Q Design\nFound device: AMD Radeon Vega 8 Graphics\nFound device: NVIDIA GeForce GTX 1650 with Max-Q Design";
        assert_eq!(adapters_in_log(log).len(), 2, "each device once");
        assert!(adapter_fits("NVIDIA GeForce GTX 1650 Mobile", "NVIDIA GeForce GTX 1650 with Max-Q Design"));
        assert!(!adapter_fits("NVIDIA GeForce GTX 1650 Mobile", "AMD Radeon Vega 8 Graphics"));
        assert!(adapter_fits("AMD Radeon Vega 8", "AMD Radeon Vega 8 Graphics (RADV RAVEN)"));
        let ctx = LogContext { expected_gpu: Some("NVIDIA GeForce GTX 1650 Mobile".into()), ..Default::default() };
        // the chosen card is among the devices: fine
        assert!(diagnose_text(log, &ctx).is_empty());
        // only the other card was seen: the choice did not reach the game
        let only_amd = "Found device: AMD Radeon Vega 8 Graphics";
        assert_eq!(diagnose_text(only_amd, &ctx)[0].kind, FindingKind::AdapterMismatch);
        // software rendering is its own finding
        let k = diagnose_text("Found device: llvmpipe (LLVM 19.1.7, 256 bits)", &ctx);
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].kind, FindingKind::SoftwareRendering);
    }

    #[test]
    fn a_crash_on_a_virtual_gpu_gets_its_own_advice() {
        let t = "wine: Unhandled page fault on write access to 0000000000000010 at address 00007FF6";
        let plain = diagnose_text(t, &LogContext::default());
        assert_eq!(plain.len(), 1);
        let virt = diagnose_text(t, &LogContext { virtual_gpu: true, ..Default::default() });
        assert_eq!(virt.len(), 2);
        let ov =
            diagnose_text("", &LogContext { env_overrides: vec!["PROTON_USE_WINED3D".into()], ..Default::default() });
        assert_eq!(ov[0].kind, FindingKind::CustomEnvironment);
    }

    #[test]
    fn logs_are_read_from_both_ends() {
        let dir = scratch("diag-edges");
        let mut big = String::from("preauth: loaded user/XSTS ok\n");
        big.push_str(&"filler line\n".repeat(60_000));
        big.push_str("VK_ERROR_DEVICE_LOST at the end\n");
        std::fs::write(dir.join("runtime.log"), &big).unwrap();
        std::fs::write(dir.join("game.log"), "x11drv: Can't open display\n").unwrap();
        let text = read_edges(&dir.join("runtime.log"), 200_000).unwrap();
        assert!(
            text.contains("preauth: loaded") && text.contains("DEVICE_LOST") && text.contains("log middle omitted")
        );
        assert!(text.len() < 500_000);
        let f = diagnose_dir(&dir, &LogContext::default());
        let ks: Vec<_> = f.iter().map(|f| f.kind).collect();
        assert!(ks.contains(&FindingKind::GpuCrash) && ks.contains(&FindingKind::LostDisplay), "{ks:?}");
        // missing logs are fine
        assert!(diagnose_dir(&dir.join("nope"), &LogContext::default()).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn healthy() -> HostFacts {
        HostFacts {
            machine: "x86_64".into(),
            kernel_release: "6.17.1".into(),
            mem_available_mib: Some(9000),
            mem_total_mib: Some(15000),
            free_disk_mib: Some(120_000),
            staging_root: Some("/dev/shm/blockforge-bedrock-1000".into()),
            staging_free_mib: Some(7000),
            ntsync_device: true,
            engine_has_ntsync: Some(true),
            session_type: "wayland".into(),
            has_display: true,
            has_xrandr: true,
            has_journalctl: true,
            vulkan_icd_found: true,
            gpus: vec!["NVIDIA GeForce GTX 1650".into(), "AMD Radeon Vega 8".into()],
            ..Default::default()
        }
    }

    #[test]
    fn a_healthy_computer_passes_and_each_problem_is_named() {
        let ok = evaluate_host(&healthy());
        assert!(ok.iter().all(|c| matches!(c.verdict, Verdict::Ok | Verdict::Info)), "{ok:?}");
        let mut f = healthy();
        f.machine = "aarch64".into();
        let c = evaluate_host(&f);
        assert_eq!(c[0].verdict, Verdict::Fail);
        f.x86_emulator = Some("/usr/bin/FEXInterpreter".into());
        assert_eq!(evaluate_host(&f)[0].verdict, Verdict::Warn);
        let mut f = healthy();
        f.mem_available_mib = Some(800);
        f.swap_used_mib = Some(3000);
        f.free_disk_mib = Some(900);
        f.staging_root = None;
        f.ntsync_device = false;
        f.has_display = false;
        f.vulkan_icd_found = false;
        f.has_journalctl = false;
        let names: Vec<_> = evaluate_host(&f)
            .into_iter()
            .filter(|c| matches!(c.verdict, Verdict::Warn | Verdict::Fail))
            .map(|c| c.name)
            .collect();
        for n in ["memory", "disk", "game files in RAM", "fast sync", "display", "vulkan", "kernel log"] {
            assert!(names.iter().any(|x| x == n), "{n} missing in {names:?}");
        }
        // little RAM-disk space left
        let mut f = healthy();
        f.staging_free_mib = Some(100);
        assert!(evaluate_host(&f).iter().any(|c| c.name == "game files in RAM" && c.verdict == Verdict::Fail));
        // PROTON_NO_NTSYNC wins over everything else about sync
        let mut f = healthy();
        f.no_ntsync_env = true;
        assert!(evaluate_host(&f).iter().any(|c| c.name == "fast sync" && c.verdict == Verdict::Warn));
        let mut f = healthy();
        f.engine_has_ntsync = Some(false);
        assert!(evaluate_host(&f).iter().any(|c| c.name == "fast sync" && c.text.contains("without")));
    }

    #[test]
    fn the_report_lists_failures_first_and_writes_a_private_file() {
        let dir = scratch("diag-report");
        let mut facts = healthy();
        facts.free_disk_mib = Some(100);
        facts.has_display = false;
        let input = ReportInput {
            facts,
            graphics_refusal: Some(
                "the graphics driver has already reported a fatal kernel fault during this boot".into(),
            ),
            graphics_notes: vec![],
            findings: vec![Finding::new(FindingKind::LostDisplay, "display gone")],
            log_dir: dir.clone(),
        };
        let report = build_report(&input);
        let path = write_report(&dir, &report).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("BLOCKED") && text.contains("display gone"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let public = report.into_public();
        assert_eq!(public.log_dir, dir);
        let first_warn = public.issues.iter().position(|i| i.starts_with("disk")).unwrap();
        let first_fail = public.issues.iter().position(|i| i.starts_with("display")).unwrap();
        assert!(first_fail < first_warn, "failures come first: {:?}", public.issues);
        assert_eq!(public.issues.last().unwrap(), "display gone");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn x86_emulators_are_found_in_binfmt() {
        let dir = scratch("diag-binfmt");
        std::fs::write(dir.join("status"), "enabled\n").unwrap();
        std::fs::write(
            dir.join("python3.12"),
            "enabled\ninterpreter /usr/bin/python3.12\nflags: \nmagic 7f454c4602010100000000000000000002003e01\n",
        )
        .unwrap();
        assert_eq!(x86_64_emulator(&dir), None, "an interpreter for something else");
        std::fs::write(
            dir.join("FEX-x86_64"),
            "enabled\ninterpreter /usr/bin/FEXInterpreter\nflags: POCF\noffset 0\nmagic 7f454c4602010100000000000000000002003e00\nmask ffffffffffffff00fffffffffffffffffeffffff\n",
        )
        .unwrap();
        assert_eq!(x86_64_emulator(&dir).as_deref(), Some("/usr/bin/FEXInterpreter"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ntsync_is_found_in_the_engine_binary() {
        let root = scratch("diag-ntsync");
        let bin = root.join("files/bin-wow64");
        std::fs::create_dir_all(&bin).unwrap();
        assert_eq!(engine_supports_ntsync(&root), None, "nothing to read: unknown, not a defect");
        // the literal straddles the 1 MiB read boundary
        let mut data = vec![b'x'; (1 << 20) - 4];
        data.extend_from_slice(b"/dev/ntsync tail");
        std::fs::write(bin.join("wineserver"), &data).unwrap();
        assert_eq!(engine_supports_ntsync(&root), Some(true));
        std::fs::write(bin.join("wineserver"), vec![b'x'; 4096]).unwrap();
        assert_eq!(engine_supports_ntsync(&root), Some(false));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn meminfo_is_read() {
        let t = "MemTotal:       16000000 kB\nMemAvailable:    8192000 kB\nSwapTotal: 100 kB\n";
        assert_eq!(meminfo_mib(t, "MemAvailable"), Some(8000));
        assert_eq!(meminfo_mib(t, "SwapFree"), None);
    }
}
