//! Fail-closed graphics checks that never open a GPU device, the crash marker that makes an interrupted game visible
//! to the next start, and the choice of the graphics card for the game (package E of PORT-PLAN.md).
//!
//! Ported from BedrockOnLinux `bol/gpu_safety.py` (MIT): the launcher must not call Vulkan or OpenGL to decide
//! whether Vulkan is safe, because on a broken proprietary driver that probe can itself take the machine down.
//! Everything here comes from the X server's own protocol (through `xrandr`), sysfs, `/proc` and the kernel's text
//! log. Differences from BOL, on purpose:
//!
//! - The marker is BlockForge's own file (`<bedrock>/gpu/launch-in-progress.json`, format 3), owned by the **session
//!   supervisor** and not by the window, so it stays truthful when the window closes. BOL's markers are never read.
//! - One marker for the whole computer: v1 allows one Bedrock game at a time (and the same hybrid-GPU machine is
//!   behind every profile).
//! - A missing `xrandr` program no longer refuses a start on its own (BOL could ask libXrandr through `ctypes`);
//!   the check says it could not verify and carries on, and the doctor says so.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{BedrockError, ErrorKind};
use crate::gpu;

const STATE_VERSION: u32 = 3;

/// Where the marker and the acknowledgement live.
#[derive(Clone, Debug)]
pub struct Paths {
    pub marker: PathBuf,
    pub ack: PathBuf,
}

impl Paths {
    /// `bedrock_root` is `Dirs.base/bedrock`.
    pub fn at(bedrock_root: &Path) -> Paths {
        let dir = bedrock_root.join("gpu");
        Paths { marker: dir.join("launch-in-progress.json"), ack: dir.join("safety-ack.json") }
    }
}

// ---- the host: everything that comes from outside, so a test can stand in for it

pub trait Host: Send + Sync {
    fn boot_id(&self) -> String;
    /// start time of a process in clock ticks, `None` when there is no such process
    fn process_start(&self, pid: u32) -> Option<u64>;
    fn now(&self) -> i64;
    /// the kernel log of boot `boot` (0 is this one, -1 the one before), the newest lines
    fn kernel_log(&self, boot: i32) -> Option<String>;
    /// how many GPU providers the X server has, `None` when that can't be asked
    fn x_providers(&self, env: &Env) -> Option<u32>;
    /// the X server's own log says it fell back to a software framebuffer
    fn xorg_software_fallback(&self) -> bool;
    /// the NVIDIA kernel driver is loaded while the GLX libraries are Mesa's (a state Debian derivatives get into)
    fn nvidia_with_mesa_glx(&self) -> bool;
    /// the display is a nested gamescope one (asked only when the environment didn't settle it)
    fn gamescope_atoms(&self, _env: &Env) -> bool {
        false
    }
}

/// Environment variables as pairs (the process's own, or a test's).
#[derive(Clone, Debug, Default)]
pub struct Env(pub Vec<(String, String)>);

impl Env {
    pub fn from_process() -> Env {
        Env(std::env::vars().collect())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn text(&self, key: &str) -> String {
        self.get(key).unwrap_or("").trim().to_string()
    }
}

pub struct SystemHost;

fn run_for_text(program: &str, args: &[&str], env: Option<&Env>, timeout: std::time::Duration) -> Option<String> {
    use std::io::Read;
    let mut cmd = crate::platform::command(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(env) = env {
        cmd.env_clear();
        for (k, v) in &env.0 {
            cmd.env(k, v);
        }
    }
    let mut child = cmd.spawn().ok()?;
    // read both pipes as the tool writes: a full pipe would otherwise stop a tool that prints a lot (the kernel log)
    let mut readers = Vec::new();
    if let Some(mut o) = child.stdout.take() {
        readers.push(std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = o.read_to_end(&mut b);
            b
        }));
    }
    if let Some(mut e) = child.stderr.take() {
        readers.push(std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = e.read_to_end(&mut b);
            b
        }));
    }
    let started = std::time::Instant::now();
    // wait with a limit; these tools answer at once or are broken
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(_) => return None,
        }
    };
    let mut parts = readers.into_iter().map(|h| h.join().unwrap_or_default());
    let (stdout, stderr) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    if !status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&stderr));
    Some(text)
}

impl Host for SystemHost {
    fn boot_id(&self) -> String {
        super::supervisor::boot_id()
    }

    fn process_start(&self, pid: u32) -> Option<u64> {
        super::supervisor::process_start(pid)
    }

    fn now(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
    }

    fn kernel_log(&self, boot: i32) -> Option<String> {
        let b = boot.to_string();
        let text = run_for_text(
            "journalctl",
            &["-k", "-b", &b, "--no-pager", "-o", "cat", "-n", "5000"],
            None,
            std::time::Duration::from_secs(5),
        )?;
        // keep the end: a fault early in a long boot must not hide behind chatter, but cap the work
        let keep = 2_000_000;
        Some(if text.len() > keep { text[text.len() - keep..].to_string() } else { text })
    }

    fn x_providers(&self, env: &Env) -> Option<u32> {
        let text = run_for_text("xrandr", &["--listproviders"], Some(env), std::time::Duration::from_secs(4))?;
        parse_xrandr_providers(&text)
    }

    fn xorg_software_fallback(&self) -> bool {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let candidates =
            [home.map(|h| h.join(".local/share/xorg/Xorg.0.log")), Some(PathBuf::from("/var/log/Xorg.0.log"))];
        candidates
            .into_iter()
            .flatten()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .any(|t| xorg_log_says_software(&t))
    }

    fn nvidia_with_mesa_glx(&self) -> bool {
        let Ok(rd) = std::fs::read_dir("/sys/class/drm") else { return false };
        let nvidia = rd.flatten().any(|e| {
            let dev = e.path().join("device");
            std::fs::read_to_string(dev.join("vendor")).is_ok_and(|v| v.trim().eq_ignore_ascii_case("0x10de"))
                && std::fs::canonicalize(dev.join("driver"))
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n == "nvidia"))
                    .unwrap_or(false)
        });
        nvidia
            && std::fs::read_link("/etc/alternatives/glx").is_ok_and(|t| t.to_string_lossy().contains("mesa-diverted"))
    }
}

pub fn parse_xrandr_providers(text: &str) -> Option<u32> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)Providers:\s*number\s*:\s*(\d+)").unwrap());
    re.captures(text)?.get(1)?.as_str().parse().ok()
}

pub fn xorg_log_says_software(text: &str) -> bool {
    text.contains("FBDEV(0):") && text.contains("DRISWRAST GL provider") && text.contains("Failed to open DRM device")
}

// ---- kernel log: recognising a GPU failure without blaming unrelated messages

fn compiled(cell: &'static std::sync::OnceLock<Vec<Regex>>, patterns: &[&str]) -> &'static [Regex] {
    cell.get_or_init(|| patterns.iter().map(|p| Regex::new(p).unwrap()).collect())
}

/// Whether the kernel log shows a fatal graphics fault (a GPU that fell off the bus, a reset that failed, a
/// lock-up next to graphics-driver lines).
pub fn gpu_fault_in_text(text: &str) -> bool {
    static DIRECT: std::sync::OnceLock<Vec<Regex>> = std::sync::OnceLock::new();
    static GENERIC: std::sync::OnceLock<Vec<Regex>> = std::sync::OnceLock::new();
    let direct = compiled(
        &DIRECT,
        &[
            r"\bnvrm:.*gpu has fallen off the bus",
            r"\bnvrm:.*\bxid\b.*\b(?:79|119|120)\b",
            r"\bamdgpu\b.*(?:gpu reset begin|ring\s+\S+\s+timeout|asic reset failed|gpu fault)",
            // not a bare "GPU HANG": i915 logs one for every hang and carries on; a failed reset is the fault
            r"\b(?:i915|xe)\b.*(?:wedged|reset.*failed|failed to reset)",
        ],
    );
    let lower = text.to_lowercase();
    let lines: Vec<&str> = lower.lines().collect();
    if lines.iter().any(|l| direct.iter().any(|p| p.is_match(l))) {
        return true;
    }
    // a kernel oops names its module on other lines: look in the neighbourhood
    let generic = compiled(
        &GENERIC,
        &[
            r"bug:\s*kernel null pointer dereference|watchdog.*(?:hard|soft)\s+lockup|kernel panic|general protection fault",
            // "traps: wineserver[4242] general protection fault ..." is a program that crashed, not the kernel
            r"\btraps:\s+\S.*\[\d+\]\s+general protection fault",
            r"\[nvidia\]|\bnvrm:|\bamdgpu\b|\bi915\b|\bxe\b",
        ],
    );
    let (oops, user_trap, vendor) = (&generic[0], &generic[1], &generic[2]);
    for (i, line) in lines.iter().enumerate() {
        if !oops.is_match(line) || user_trap.is_match(line) {
            continue;
        }
        let (lo, hi) = (i.saturating_sub(40), (i + 41).min(lines.len()));
        if lines[lo..hi].iter().any(|l| vendor.is_match(l)) {
            return true;
        }
    }
    false
}

/// An Intel GPU hang the driver reset its way out of: no fault, but worth a note.
pub fn recovered_gpu_hang_in_text(text: &str) -> bool {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\b(?:i915|xe)\b.*\bgpu hang\b").unwrap());
    text.to_lowercase().lines().any(|l| re.is_match(l))
}

pub const RECOVERED_HANG_NOTE: &str = "The Intel GPU hung and was reset during this boot (\"GPU HANG\" in the kernel log). Its driver recovered, so the game starts. If Minecraft is what hung it and it happens again, update Mesa.";

// ---- the marker

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// the game's wrapper may be running
    Running,
    /// the wrapper returned; the prefix was not yet seen idle
    WrapperReturned,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub version: u32,
    pub engine_rev: String,
    pub phase: Phase,
    /// proves a clear belongs to the launch that armed it
    pub token: String,
    pub boot_id: String,
    pub session: String,
    /// the supervisor that armed it (not the window)
    pub owner_pid: u32,
    pub owner_start: Option<u64>,
    pub created: i64,
    pub wrapper_returned: Option<i64>,
}

/// A proof of an armed launch, kept by the supervisor.
#[derive(Clone, Debug)]
pub struct Armed {
    pub token: String,
}

/// Why a start is refused. `kind` is for the program (and the window's wording), `message` for the player.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalKind {
    /// an earlier game did not come back (this boot or before the last reboot)
    InterruptedLaunch,
    /// a game of ours still seems to be running
    StillActive,
    KernelFaultThisBoot,
    KernelFaultLastBoot,
    /// the X server has no hardware GPU provider
    NoGpuProvider,
    SoftwareFallback,
}

impl Refusal {
    fn new(kind: RefusalKind, message: impl Into<String>) -> Self {
        Refusal { kind, message: message.into() }
    }

    pub fn into_error(self) -> BedrockError {
        BedrockError::new(
            ErrorKind::Runtime,
            format!(
                "Unsafe graphics session: {}. BlockForge did not start Wine, Vulkan or Minecraft.{}",
                self.message,
                match self.kind {
                    RefusalKind::InterruptedLaunch | RefusalKind::StillActive => "",
                    _ =>
                        " Repair or reinstall the graphics driver, make sure the desktop uses the hardware GPU, then reboot.",
                }
            ),
        )
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 65_536 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn sync_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(d) = std::fs::File::open(parent) {
            let _ = d.sync_all();
        }
    }
}

#[cfg(unix)]
fn write_new(path: &Path, bytes: &[u8], exclusive: bool) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).mode(0o600);
    if exclusive {
        o.create_new(true);
    } else {
        o.create(true).truncate(true);
    }
    let mut f = o.open(path)?;
    f.write_all(bytes)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    drop(f);
    sync_dir(path);
    Ok(())
}

/// Replace a file whole: written next to it, flushed, then renamed over it, so a power cut leaves the old or the new.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    write_new(&tmp, bytes, false)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    sync_dir(path);
    Ok(())
}

fn owner_alive(host: &dyn Host, m: &Marker) -> bool {
    match (host.process_start(m.owner_pid), m.owner_start) {
        (None, _) => false,
        (Some(now), Some(then)) => now == then,
        (Some(_), None) => true,
    }
}

pub fn read_marker(paths: &Paths) -> Option<Marker> {
    read_json(&paths.marker)
}

/// What the marker says about the last game, for the next start. `Some` means: do not start.
pub fn interrupted_launch_problem(paths: &Paths, host: &dyn Host) -> Option<Refusal> {
    match std::fs::symlink_metadata(&paths.marker) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            return Some(Refusal::new(
                RefusalKind::InterruptedLaunch,
                "BlockForge can't inspect its graphics safety record; repair the permissions of its data folder, then use \"Acknowledge\" in the Bedrock diagnostics",
            ));
        }
        Ok(_) => {}
    }
    let Some(m) = read_marker(paths) else {
        return Some(Refusal::new(
            RefusalKind::InterruptedLaunch,
            format!(
                "a record of an interrupted Minecraft start exists but is unreadable; once the graphics driver works, remove {}",
                paths.marker.display()
            ),
        ));
    };
    if m.version != STATE_VERSION {
        return Some(Refusal::new(
            RefusalKind::InterruptedLaunch,
            format!(
                "the record of an interrupted Minecraft start has an unknown format; once the driver works, remove {}",
                paths.marker.display()
            ),
        ));
    }
    let boot = host.boot_id();
    let same_boot = !boot.is_empty() && m.boot_id == boot;
    if same_boot && owner_alive(host, &m) {
        return Some(Refusal::new(
            RefusalKind::StillActive,
            "a Minecraft session is still running; close or stop it instead of starting a second one",
        ));
    }
    if same_boot {
        return Some(Refusal::new(
            RefusalKind::InterruptedLaunch,
            "the previous Minecraft session did not end cleanly during this boot; once nothing of it is left running, the next start clears this record by itself. Reboot first if the screen froze or the graphics misbehaved",
        ));
    }
    Some(Refusal::new(
        RefusalKind::InterruptedLaunch,
        "the previous Minecraft session did not end cleanly before the last reboot or power loss (as when the computer is shut down with Minecraft open). If the screen froze or the computer had to be forced off, repair the graphics driver first; otherwise acknowledge the interrupted start once",
    ))
}

/// Durably mark the start of a game immediately before its process is spawned. Fails if a marker exists, unless
/// `allow_unsafe` says to replace it (an explicit override by the player).
pub fn arm(
    paths: &Paths,
    host: &dyn Host,
    engine_rev: &str,
    session: &str,
    allow_unsafe: bool,
) -> Result<Armed, Refusal> {
    let mut token_bytes = [0u8; 16];
    // the token only has to be unguessable by accident, but make it random anyway
    let id = uuid::Uuid::new_v4();
    token_bytes.copy_from_slice(id.as_bytes());
    let token = hex::encode(token_bytes);
    let owner_pid = std::process::id();
    let marker = Marker {
        version: STATE_VERSION,
        engine_rev: engine_rev.to_string(),
        phase: Phase::Running,
        token: token.clone(),
        boot_id: host.boot_id(),
        session: session.to_string(),
        owner_pid,
        owner_start: host.process_start(owner_pid),
        created: host.now(),
        wrapper_returned: None,
    };
    let bytes = serde_json::to_vec(&marker).unwrap_or_default();
    if allow_unsafe {
        // replaced only now, so a start that fails earlier leaves the incident as it was
        match std::fs::symlink_metadata(&paths.marker) {
            Ok(m) if m.is_dir() => {
                return Err(Refusal::new(
                    RefusalKind::InterruptedLaunch,
                    format!("{} is a directory; remove it by hand", paths.marker.display()),
                ));
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&paths.marker);
                sync_dir(&paths.marker);
            }
            Err(_) => {}
        }
    }
    match write_new(&paths.marker, &bytes, true) {
        Ok(()) => Ok(Armed { token }),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(Refusal::new(
            RefusalKind::InterruptedLaunch,
            "a previous Minecraft start is still marked as interrupted",
        )),
        Err(e) => {
            let _ = std::fs::remove_file(&paths.marker);
            Err(Refusal::new(RefusalKind::InterruptedLaunch, format!("couldn't record the start of the game: {e}")))
        }
    }
}

/// Clear the marker this launch armed (and only that one). `true` when it was cleared.
pub fn disarm(paths: &Paths, armed: &Armed) -> bool {
    match read_marker(paths) {
        Some(m) if m.token == armed.token => {
            let ok = std::fs::remove_file(&paths.marker).is_ok();
            sync_dir(&paths.marker);
            ok
        }
        _ => false,
    }
}

/// Persist that the wrapper returned, before the prefix's last helper is checked: from here a crash of the
/// supervisor reads as "the wrapper had finished".
pub fn record_wrapper_exit(paths: &Paths, host: &dyn Host, armed: &Armed) -> bool {
    let Some(mut m) = read_marker(paths) else { return false };
    if m.version != STATE_VERSION || m.phase != Phase::Running || m.token != armed.token {
        return false;
    }
    m.phase = Phase::WrapperReturned;
    m.wrapper_returned = Some(host.now());
    write_atomic(&paths.marker, &serde_json::to_vec(&m).unwrap_or_default()).is_ok()
}

/// Clear an orphan marker of this boot, given proof that the prefix is idle. The supervisor that armed it must be
/// gone (a running one still owns it). A marker of an earlier boot, an unreadable one and a token race stay.
pub fn retire_idle_current_boot_marker(paths: &Paths, host: &dyn Host, prefix_idle: bool) -> Option<String> {
    if !prefix_idle {
        return None;
    }
    let m = read_marker(paths)?;
    let boot = host.boot_id();
    if m.version != STATE_VERSION || boot.is_empty() || m.boot_id != boot {
        return None;
    }
    if m.phase == Phase::Running && owner_alive(host, &m) {
        return None;
    }
    if m.phase == Phase::WrapperReturned && m.wrapper_returned.is_none_or(|t| t < m.created) {
        return None;
    }
    if !disarm(paths, &Armed { token: m.token.clone() }) {
        return None;
    }
    Some(match m.phase {
        Phase::WrapperReturned => "Removed an orphaned graphics record of this boot after confirming the Wine prefix is idle; the kernel and display checks still apply.".to_string(),
        Phase::Running => "Cleared the record of a Minecraft session whose supervisor stopped during this boot while the game ran: nothing of it is left running. The kernel and display checks still apply.".to_string(),
    })
}

// ---- acknowledging a previous-boot incident

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AckStatus {
    pub code: &'static str,
    pub can_acknowledge: bool,
    pub message: String,
    pub marker_present: bool,
    pub previous_boot_fault: bool,
}

#[derive(Serialize, Deserialize)]
struct Ack {
    version: u32,
    boot_id: String,
    acknowledged: i64,
    marker: bool,
    previous_boot_fault: bool,
}

fn previous_fault_acknowledged(paths: &Paths, host: &dyn Host) -> bool {
    let boot = host.boot_id();
    read_json::<Ack>(&paths.ack)
        .is_some_and(|a| !boot.is_empty() && a.version == STATE_VERSION && a.boot_id == boot && a.previous_boot_fault)
}

/// `Some("current")`, `Some("previous")` or `None`; a recovered hang goes on `notes`.
fn kernel_fault_scope(paths: &Paths, host: &dyn Host, notes: Option<&mut Vec<String>>) -> Option<&'static str> {
    let current = host.kernel_log(0);
    if current.as_deref().is_some_and(gpu_fault_in_text) {
        return Some("current");
    }
    if let (Some(notes), Some(cur)) = (notes, current.as_deref()) {
        if recovered_gpu_hang_in_text(cur) {
            notes.push(RECOVERED_HANG_NOTE.to_string());
        }
    }
    if previous_fault_acknowledged(paths, host) {
        return None;
    }
    host.kernel_log(-1).as_deref().filter(|t| gpu_fault_in_text(t)).map(|_| "previous")
}

pub fn acknowledgement_status(paths: &Paths, host: &dyn Host) -> AckStatus {
    let mk = |code, can, message: &str, marker, fault| AckStatus {
        code,
        can_acknowledge: can,
        message: message.to_string(),
        marker_present: marker,
        previous_boot_fault: fault,
    };
    let scope = match std::fs::symlink_metadata(&paths.marker) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "none",
        Err(_) => "unknown",
        Ok(_) => match read_marker(paths) {
            None => "unknown",
            Some(m) => {
                let boot = host.boot_id();
                if boot.is_empty() || m.boot_id.is_empty() {
                    "unknown"
                } else if m.boot_id != boot {
                    "previous"
                } else if owner_alive(host, &m) {
                    "active"
                } else {
                    "current"
                }
            }
        },
    };
    match scope {
        "active" => {
            return mk(
                "active-launch",
                false,
                "The marked Minecraft session is still running; stop it before acknowledging a graphics incident.",
                true,
                false,
            );
        }
        "current" => {
            return mk(
                "current-boot-launch",
                false,
                "The interrupted Minecraft start happened during this boot. The next start clears it by itself once nothing of that session is left running; reboot first if the graphics misbehaved.",
                true,
                false,
            );
        }
        "unknown" => {
            return mk(
                "unreadable-marker",
                false,
                "The interrupted-start record can't prove that it belongs to a previous boot, so it can't be acknowledged safely.",
                true,
                false,
            );
        }
        _ => {}
    }
    let fault = kernel_fault_scope(paths, host, None);
    if fault == Some("current") {
        return mk(
            "current-boot-fault",
            false,
            "The graphics driver reported a fatal fault during this boot. Reboot before acknowledging any earlier incident.",
            scope == "previous",
            false,
        );
    }
    let (marker, previous_fault) = (scope == "previous", fault == Some("previous"));
    if marker || previous_fault {
        if host.boot_id().is_empty() {
            return mk(
                "boot-id-unavailable",
                false,
                "The current boot can't be identified, so an earlier graphics incident can't be acknowledged safely.",
                marker,
                previous_fault,
            );
        }
        let mut what = Vec::new();
        if marker {
            what.push("an interrupted Minecraft start");
        }
        if previous_fault {
            what.push("a fatal graphics-driver event");
        }
        return AckStatus {
            code: "previous-boot-incident",
            can_acknowledge: true,
            message: format!(
                "Graphics incident from before the last reboot available to acknowledge: {}.",
                what.join(" and ")
            ),
            marker_present: marker,
            previous_boot_fault: previous_fault,
        };
    }
    mk("none", false, "No earlier graphics incident is available to acknowledge.", false, false)
}

/// Acknowledge one verified previous-boot incident (never a blank state). The caller holds the prefix and game leases
/// and has seen the prefix idle; eligibility is judged again here, right before the write.
pub fn acknowledge(paths: &Paths, host: &dyn Host) -> Result<AckStatus, BedrockError> {
    let status = acknowledgement_status(paths, host);
    if !status.can_acknowledge {
        return Err(BedrockError::new(ErrorKind::Busy, status.message));
    }
    let ack = Ack {
        version: STATE_VERSION,
        boot_id: host.boot_id(),
        acknowledged: host.now(),
        marker: status.marker_present,
        previous_boot_fault: status.previous_boot_fault,
    };
    write_atomic(&paths.ack, &serde_json::to_vec(&ack).unwrap_or_default())
        .map_err(|e| BedrockError::new(ErrorKind::Internal, format!("couldn't record the acknowledgement: {e}")))?;
    if status.marker_present {
        // leaving the marker is still fail-closed: report it rather than pretend it is clear
        std::fs::remove_file(&paths.marker).map_err(|_| {
            BedrockError::new(ErrorKind::Internal, "the earlier graphics incident was acknowledged, but its record of the interrupted start couldn't be removed")
        })?;
        sync_dir(&paths.marker);
    }
    Ok(status)
}

// ---- the display and kernel checks before a start

#[derive(Clone, Debug, Default)]
pub struct Preflight {
    /// `Some` means: don't start
    pub refusal: Option<Refusal>,
    /// advice that doesn't stop the start
    pub notes: Vec<String>,
    /// what an override went past (the player's own risk)
    pub bypassed: Vec<String>,
}

fn x11_session(env: &Env) -> bool {
    let session = env.text("XDG_SESSION_TYPE").to_lowercase();
    if !session.is_empty() {
        return session == "x11";
    }
    !env.text("DISPLAY").is_empty() && env.text("WAYLAND_DISPLAY").is_empty()
}

/// Whether this is an explicitly identified nested gamescope session, whose X server legitimately has no providers.
pub fn nested_gamescope(env: &Env, host: &dyn Host) -> bool {
    if !env.text("GAMESCOPE_WAYLAND_DISPLAY").is_empty() {
        return true;
    }
    for key in ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"] {
        let v = env.text(key).to_lowercase();
        if v.split(|c: char| matches!(c, ':' | ';' | ',') || c.is_whitespace())
            .any(|p| p == "gamescope" || p.starts_with("gamescope-"))
        {
            return true;
        }
    }
    host.gamescope_atoms(env)
}

/// The graphics checks, from the cheapest to the dearest. `include_interrupted: false` finds what a refusal for the
/// marker was hiding.
pub fn graphics_safety_problem(
    paths: &Paths,
    host: &dyn Host,
    env: &Env,
    notes: &mut Vec<String>,
    include_interrupted: bool,
) -> Option<Refusal> {
    if include_interrupted {
        if let Some(r) = interrupted_launch_problem(paths, host) {
            return Some(r);
        }
    }
    match kernel_fault_scope(paths, host, Some(notes)) {
        Some("current") => {
            return Some(Refusal::new(
                RefusalKind::KernelFaultThisBoot,
                "the graphics driver has already reported a fatal kernel fault during this boot; reboot before starting another GPU process",
            ));
        }
        Some(_) => {
            return Some(Refusal::new(
                RefusalKind::KernelFaultLastBoot,
                "the graphics driver reported a fatal kernel fault before the last reboot; after repairing or updating the driver, acknowledge it",
            ));
        }
        None => {}
    }
    if x11_session(env) {
        match host.x_providers(env) {
            Some(0) => {
                if nested_gamescope(env, host) {
                    return None;
                }
                let mut problem = "the X11 session exposes zero RandR GPU providers and is running on a fallback framebuffer or software renderer".to_string();
                if host.nvidia_with_mesa_glx() {
                    problem.push_str(
                        "; the NVIDIA kernel device is loaded while the GLX alternative points to mesa-diverted",
                    );
                }
                return Some(Refusal::new(RefusalKind::NoGpuProvider, problem));
            }
            Some(_) => {}
            None => {
                if host.xorg_software_fallback() {
                    let mut problem =
                        "Xorg failed to open its DRM device and fell back to FBDEV/software rendering".to_string();
                    if host.nvidia_with_mesa_glx() {
                        problem.push_str(
                            "; the NVIDIA kernel device is loaded while the GLX alternative points to mesa-diverted",
                        );
                    }
                    return Some(Refusal::new(RefusalKind::SoftwareFallback, problem));
                }
                // BOL refuses here; without libXrandr to ask, say so and carry on (the doctor shows it too)
                notes.push(
                    "Couldn't verify the X11 hardware GPU provider (xrandr isn't available); the start goes ahead."
                        .to_string(),
                );
            }
        }
    }
    None
}

/// The whole check before a start. With `allow_unsafe` (the player's explicit override) a refusal is recorded in
/// `bypassed` instead of stopping.
pub fn check(paths: &Paths, host: &dyn Host, env: &Env, allow_unsafe: bool) -> Preflight {
    let mut out = Preflight::default();
    let Some(problem) = graphics_safety_problem(paths, host, env, &mut out.notes, true) else { return out };
    if allow_unsafe {
        out.bypassed.push(problem.message.clone());
        if matches!(problem.kind, RefusalKind::InterruptedLaunch | RefusalKind::StillActive) {
            // the marker hides every later check, and the override skips those too: say what they found
            if let Some(hidden) = graphics_safety_problem(paths, host, env, &mut out.notes, false) {
                out.bypassed.push(hidden.message);
            }
        }
        return out;
    }
    out.refusal = Some(problem);
    out
}

// ---- the graphics card for the game

/// The card chosen for the game and the variables that put the game on it, all the way down to Vulkan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GameGpu {
    /// variables for the game; the player's own still win (see [`GameGpu::apply`])
    pub env: Vec<(String, String)>,
    /// the card's name, for the log and the check against what the game really drew on
    pub name: Option<String>,
    pub pci_ids: Option<(u32, u32)>,
    pub note: Option<String>,
}

/// The card `choice` means: its PRIME/offload variables **and** the Vulkan-level selection (NVIDIA's layer, Mesa's
/// device select), because the game picks its own adapter and `DRI_PRIME` alone only reorders the cards.
pub fn game_gpu(choice: &gpu::Choice, gpus: &[gpu::Gpu]) -> GameGpu {
    let effect = gpu::effect(choice, gpus);
    match gpu::resolve(choice, gpus) {
        Some(chosen) => GameGpu {
            env: gpu::vulkan_env(chosen, gpus),
            name: Some(chosen.name.clone()),
            pci_ids: chosen.pci_ids,
            note: effect.note,
        },
        None => GameGpu { env: effect.env, name: effect.gpu, pci_ids: None, note: effect.note },
    }
}

impl GameGpu {
    /// Put the variables in `env`, except those the player's own environment already sets.
    pub fn apply(&self, env: &mut Vec<(String, String)>, players: &[(String, String)]) {
        for (k, v) in &self.env {
            if players.iter().any(|(pk, _)| pk == k) {
                continue;
            }
            match env.iter_mut().find(|(ek, _)| ek == k) {
                Some(pair) => pair.1 = v.clone(),
                None => env.push((k.clone(), v.clone())),
            }
        }
    }
}

/// Shader caches that survive game and engine updates: one folder per engine revision and card, so a change of card
/// or engine never reads another's pipelines (the driver rejects a mismatched cache, but only after the work).
pub fn shader_cache_env(cache_root: &Path, engine_rev: &str, gpu: Option<&GameGpu>) -> Vec<(String, String)> {
    let card = match gpu.and_then(|g| g.pci_ids) {
        Some((v, d)) => format!("{v:04x}-{d:04x}"),
        None => "default".to_string(),
    };
    let safe = |s: &str| {
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect::<String>()
    };
    let dir = cache_root.join("graphics").join(safe(engine_rev)).join(card);
    vec![
        ("VKD3D_SHADER_CACHE_PATH".to_string(), dir.join("vkd3d").to_string_lossy().into_owned()),
        ("DXVK_SHADER_CACHE_PATH".to_string(), dir.join("dxvk").to_string_lossy().into_owned()),
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::scratch;

    /// A value shared between threads that tests read and change through `&self` (the host trait is `Sync`).
    pub struct Sh<T>(std::sync::Mutex<T>);
    impl<T> Sh<T> {
        pub fn new(v: T) -> Self {
            Sh(std::sync::Mutex::new(v))
        }
        pub fn borrow(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
        pub fn borrow_mut(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }
    impl<T: Copy> Sh<T> {
        pub fn get(&self) -> T {
            *self.0.lock().unwrap()
        }
        pub fn set(&self, v: T) {
            *self.0.lock().unwrap() = v;
        }
    }

    pub struct FakeHost {
        pub boot: Sh<String>,
        pub alive: Sh<Vec<(u32, u64)>>,
        pub log_now: Sh<Option<String>>,
        pub log_prev: Sh<Option<String>>,
        pub providers: Sh<Option<u32>>,
        pub xorg_software: Sh<bool>,
        pub mesa_split: Sh<bool>,
        pub clock: Sh<i64>,
    }

    impl FakeHost {
        pub fn new() -> Self {
            FakeHost {
                boot: Sh::new("boot-a".into()),
                alive: Sh::new(vec![(std::process::id(), 777)]),
                log_now: Sh::new(Some("kernel: quiet\n".into())),
                log_prev: Sh::new(Some("kernel: quiet\n".into())),
                providers: Sh::new(Some(1)),
                xorg_software: Sh::new(false),
                mesa_split: Sh::new(false),
                clock: Sh::new(1000),
            }
        }
        fn reboot(&self) {
            *self.boot.borrow_mut() = "boot-b".into();
            // nobody who was running survives a reboot
            self.alive.borrow_mut().clear();
            *self.log_prev.borrow_mut() = self.log_now.borrow().clone();
            *self.log_now.borrow_mut() = Some("kernel: quiet\n".into());
        }
    }

    impl Host for FakeHost {
        fn boot_id(&self) -> String {
            self.boot.borrow().clone()
        }
        fn process_start(&self, pid: u32) -> Option<u64> {
            self.alive.borrow().iter().find(|(p, _)| *p == pid).map(|(_, s)| *s)
        }
        fn now(&self) -> i64 {
            self.clock.set(self.clock.get() + 1);
            self.clock.get()
        }
        fn kernel_log(&self, boot: i32) -> Option<String> {
            if boot == 0 { self.log_now.borrow().clone() } else { self.log_prev.borrow().clone() }
        }
        fn x_providers(&self, _env: &Env) -> Option<u32> {
            self.providers.get()
        }
        fn xorg_software_fallback(&self) -> bool {
            self.xorg_software.get()
        }
        fn nvidia_with_mesa_glx(&self) -> bool {
            self.mesa_split.get()
        }
    }

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    fn x11() -> Env {
        env(&[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":0")])
    }

    #[test]
    fn kernel_faults_are_told_from_noise() {
        assert!(gpu_fault_in_text("NVRM: Xid (PCI:0000:01:00): 79, GPU has fallen off the bus."));
        assert!(gpu_fault_in_text("NVRM: GPU 0000:01:00.0: GPU has fallen off the bus."));
        assert!(gpu_fault_in_text("amdgpu 0000:06:00.0: amdgpu: GPU reset begin!"));
        assert!(gpu_fault_in_text("i915 0000:00:02.0: [drm] GPU HANG: wedged"));
        // a hang the driver recovered from is no fault
        assert!(!gpu_fault_in_text("i915 0000:00:02.0: [drm] GPU HANG: ecode 9:1:85dffffb, in Xorg [1234]"));
        assert!(recovered_gpu_hang_in_text("i915 0000:00:02.0: [drm] GPU HANG: ecode 9"));
        // an oops needs a graphics line near it
        assert!(gpu_fault_in_text("amdgpu: something\nBUG: kernel NULL pointer dereference, address: 0\nCall Trace:"));
        assert!(!gpu_fault_in_text("BUG: kernel NULL pointer dereference, address: 0\niwlwifi: Call Trace"));
        // a program's own crash is not the kernel's
        assert!(!gpu_fault_in_text("amdgpu: ok\ntraps: wineserver[4242] general protection fault ip:7f in libc.so.6"));
        assert!(!gpu_fault_in_text("amdgpu: routine boot chatter\nusb 1-1: new device"));
    }

    #[test]
    fn xrandr_output_is_read() {
        assert_eq!(parse_xrandr_providers("Providers: number : 2\nProvider 0: id: 0x1b8"), Some(2));
        assert_eq!(parse_xrandr_providers("Providers: number : 0"), Some(0));
        assert_eq!(parse_xrandr_providers("xrandr: Can't open display"), None);
        assert!(xorg_log_says_software("(II) FBDEV(0): x\nDRISWRAST GL provider\nFailed to open DRM device for"));
        assert!(!xorg_log_says_software("FBDEV(0): only during discovery"));
    }

    #[test]
    fn a_clean_session_starts_and_a_marker_follows_the_whole_session() {
        let dir = scratch("gpu-clean");
        let paths = Paths::at(&dir);
        let host = FakeHost::new();
        let pre = check(&paths, &host, &x11(), false);
        assert!(pre.refusal.is_none() && pre.notes.is_empty());
        let armed = arm(&paths, &host, "engine-1", "sess", false).unwrap();
        let m = read_marker(&paths).unwrap();
        assert_eq!((m.phase, m.session.as_str(), m.owner_pid), (Phase::Running, "sess", std::process::id()));
        assert_eq!(m.owner_start, Some(777));
        // while the supervisor lives, another start is told a session is running (not "interrupted")
        assert_eq!(check(&paths, &host, &x11(), false).refusal.unwrap().kind, RefusalKind::StillActive);
        assert!(arm(&paths, &host, "engine-1", "sess2", false).is_err(), "never two markers");
        assert!(record_wrapper_exit(&paths, &host, &armed));
        assert_eq!(read_marker(&paths).unwrap().phase, Phase::WrapperReturned);
        // a stranger's token clears nothing
        assert!(!disarm(&paths, &Armed { token: "0".repeat(32) }));
        assert!(paths.marker.exists());
        assert!(disarm(&paths, &armed));
        assert!(!paths.marker.exists());
        assert!(check(&paths, &host, &x11(), false).refusal.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_abrupt_crash_leaves_a_truthful_marker_for_each_phase() {
        let dir = scratch("gpu-crash");
        let paths = Paths::at(&dir);
        let host = FakeHost::new();
        // phase: running; the supervisor dies (not a clean disarm)
        arm(&paths, &host, "e", "s1", false).unwrap();
        host.alive.borrow_mut().clear();
        let r = check(&paths, &host, &x11(), false).refusal.unwrap();
        assert_eq!(r.kind, RefusalKind::InterruptedLaunch);
        assert!(r.message.contains("during this boot"));
        // it won't be retired while something still uses the prefix ...
        assert!(retire_idle_current_boot_marker(&paths, &host, false).is_none());
        assert!(paths.marker.exists());
        // ... and is, with the proof that nothing is left
        let said = retire_idle_current_boot_marker(&paths, &host, true).unwrap();
        assert!(said.contains("supervisor stopped"));
        assert!(!paths.marker.exists());
        // phase: wrapper returned; the supervisor dies before checking the prefix
        host.alive.borrow_mut().push((std::process::id(), 777));
        let armed = arm(&paths, &host, "e", "s2", false).unwrap();
        assert!(record_wrapper_exit(&paths, &host, &armed));
        host.alive.borrow_mut().clear();
        assert!(retire_idle_current_boot_marker(&paths, &host, true).unwrap().contains("orphaned"));
        // a power loss: after the reboot the marker blocks and says it was the last boot
        host.alive.borrow_mut().push((std::process::id(), 777));
        arm(&paths, &host, "e", "s3", false).unwrap();
        host.reboot();
        let r = check(&paths, &host, &x11(), false).refusal.unwrap();
        assert!(r.message.contains("before the last reboot"), "{}", r.message);
        // a marker of an earlier boot is never retired automatically
        assert!(retire_idle_current_boot_marker(&paths, &host, true).is_none());
        assert!(paths.marker.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_recycled_supervisor_number_does_not_keep_a_marker_alive() {
        let dir = scratch("gpu-recycled");
        let paths = Paths::at(&dir);
        let host = FakeHost::new();
        arm(&paths, &host, "e", "s", false).unwrap();
        // the number is alive, but it is another process now (a different start time)
        host.alive.borrow_mut().clear();
        host.alive.borrow_mut().push((std::process::id(), 99999));
        assert_eq!(check(&paths, &host, &x11(), false).refusal.unwrap().kind, RefusalKind::InterruptedLaunch);
        assert!(retire_idle_current_boot_marker(&paths, &host, true).is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_and_foreign_markers_fail_closed() {
        let dir = scratch("gpu-unreadable");
        let paths = Paths::at(&dir);
        let host = FakeHost::new();
        std::fs::create_dir_all(paths.marker.parent().unwrap()).unwrap();
        std::fs::write(&paths.marker, b"{ torn by a power cut").unwrap();
        let r = check(&paths, &host, &x11(), false).refusal.unwrap();
        assert!(r.message.contains("unreadable"));
        assert!(retire_idle_current_boot_marker(&paths, &host, true).is_none(), "an unreadable record is not retired");
        assert!(!acknowledgement_status(&paths, &host).can_acknowledge);
        let alien = Marker {
            version: 99,
            engine_rev: "e".into(),
            phase: Phase::Running,
            token: "t".into(),
            boot_id: host.boot_id(),
            session: "s".into(),
            owner_pid: 1,
            owner_start: None,
            created: 1,
            wrapper_returned: None,
        };
        std::fs::write(&paths.marker, serde_json::to_vec(&alien).unwrap()).unwrap();
        assert!(check(&paths, &host, &x11(), false).refusal.unwrap().message.contains("unknown format"));
        // the override replaces the marker, once, at arm time, and says what it went past
        let pre = check(&paths, &host, &x11(), true);
        assert!(pre.refusal.is_none() && !pre.bypassed.is_empty());
        assert!(paths.marker.exists(), "checking alone doesn't touch the record");
        assert!(arm(&paths, &host, "e", "s", true).is_ok());
        assert_eq!(read_marker(&paths).unwrap().version, STATE_VERSION);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn kernel_fault_blocks_and_only_a_previous_one_can_be_acknowledged() {
        let dir = scratch("gpu-fault");
        let paths = Paths::at(&dir);
        let host = FakeHost::new();
        *host.log_now.borrow_mut() = Some("NVRM: GPU has fallen off the bus.\n".into());
        let r = check(&paths, &host, &x11(), false).refusal.unwrap();
        assert_eq!(r.kind, RefusalKind::KernelFaultThisBoot);
        assert!(!acknowledgement_status(&paths, &host).can_acknowledge);
        assert!(acknowledge(&paths, &host).is_err());
        host.reboot();
        let r = check(&paths, &host, &x11(), false).refusal.unwrap();
        assert_eq!(r.kind, RefusalKind::KernelFaultLastBoot);
        let st = acknowledgement_status(&paths, &host);
        assert!(st.can_acknowledge && st.previous_boot_fault && !st.marker_present, "{st:?}");
        acknowledge(&paths, &host).unwrap();
        assert!(check(&paths, &host, &x11(), false).refusal.is_none());
        // a fresh fault in the new boot still blocks, acknowledgement or not
        *host.log_now.borrow_mut() = Some("amdgpu 0000:06:00.0: amdgpu: GPU reset begin!\n".into());
        assert_eq!(check(&paths, &host, &x11(), false).refusal.unwrap().kind, RefusalKind::KernelFaultThisBoot);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_previous_boot_marker_is_acknowledged_once_and_removed() {
        let dir = scratch("gpu-ack");
        let paths = Paths::at(&dir);
        let host = FakeHost::new();
        arm(&paths, &host, "e", "s", false).unwrap();
        // still the same boot and alive: not acknowledgeable
        assert_eq!(acknowledgement_status(&paths, &host).code, "active-launch");
        host.alive.borrow_mut().clear();
        assert_eq!(acknowledgement_status(&paths, &host).code, "current-boot-launch");
        host.reboot();
        let st = acknowledgement_status(&paths, &host);
        assert_eq!((st.code, st.can_acknowledge, st.marker_present), ("previous-boot-incident", true, true));
        acknowledge(&paths, &host).unwrap();
        assert!(!paths.marker.exists());
        assert_eq!(acknowledgement_status(&paths, &host).code, "none");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn display_without_a_gpu_is_refused_with_a_reason() {
        let dir = scratch("gpu-display");
        let paths = Paths::at(&dir);
        let host = FakeHost::new();
        host.providers.set(Some(0));
        let r = check(&paths, &host, &x11(), false).refusal.unwrap();
        assert_eq!(r.kind, RefusalKind::NoGpuProvider);
        assert!(r.message.contains("zero RandR"));
        host.mesa_split.set(true);
        assert!(check(&paths, &host, &x11(), false).refusal.unwrap().message.contains("mesa-diverted"));
        // gamescope's own X server has none, legitimately
        let gs = env(&[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":1"), ("XDG_CURRENT_DESKTOP", "gamescope")]);
        assert!(check(&paths, &host, &gs, false).refusal.is_none());
        // Wayland sessions are not asked at all
        let wl = env(&[("XDG_SESSION_TYPE", "wayland"), ("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]);
        assert!(check(&paths, &host, &wl, false).refusal.is_none());
        // xrandr can't be asked: a software fallback in Xorg's log refuses; otherwise a note
        host.providers.set(None);
        host.mesa_split.set(false);
        assert!(check(&paths, &host, &x11(), false).refusal.is_none());
        assert_eq!(check(&paths, &host, &x11(), false).notes.len(), 1);
        host.xorg_software.set(true);
        assert_eq!(check(&paths, &host, &x11(), false).refusal.unwrap().kind, RefusalKind::SoftwareFallback);
        // the structured error carries the advice for a driver fault
        let e = check(&paths, &host, &x11(), false).refusal.unwrap().into_error();
        assert!(matches!(e.kind, ErrorKind::Runtime) && e.message.contains("did not start Wine"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn card(
        id: &str,
        vendor: gpu::Vendor,
        discrete: bool,
        default: bool,
        pci: (u32, u32),
        env: Vec<(&str, &str)>,
    ) -> gpu::Gpu {
        gpu::Gpu {
            id: id.into(),
            name: id.into(),
            vendor,
            discrete,
            default,
            env: env.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            pci_ids: Some(pci),
        }
    }

    #[test]
    fn the_game_gets_the_chosen_card_down_to_vulkan() {
        let nvidia = card(
            "nvidia",
            gpu::Vendor::Nvidia,
            true,
            false,
            (0x10de, 0x1f95),
            vec![("__NV_PRIME_RENDER_OFFLOAD", "1"), ("__GLX_VENDOR_LIBRARY_NAME", "nvidia")],
        );
        let amd = card(
            "pci-0000_06_00_0",
            gpu::Vendor::Amd,
            false,
            true,
            (0x1002, 0x15d8),
            vec![("DRI_PRIME", "pci-0000_06_00_0")],
        );
        let all = vec![amd.clone(), nvidia.clone()];
        let g = game_gpu(&gpu::Choice::Dedicated, &all);
        assert_eq!(g.name.as_deref(), Some("nvidia"));
        let get = |k: &str| g.env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("__NV_PRIME_RENDER_OFFLOAD"), Some("1"));
        assert_eq!(get("__VK_LAYER_NV_optimus"), Some("NVIDIA_only"));
        // the other card: Mesa's device select with the exact ids, NVIDIA's layer told to stand aside
        let g = game_gpu(&gpu::Choice::Integrated, &all);
        let get = |k: &str| g.env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("MESA_VK_DEVICE_SELECT"), Some("1002:15d8!"));
        assert_eq!(get("__VK_LAYER_NV_optimus"), Some("non_NVIDIA_only"));
        assert_eq!(get("DRI_PRIME"), Some("pci-0000_06_00_0"));
        // the player's own environment wins; automatic changes nothing
        let mut env = vec![("MESA_VK_DEVICE_SELECT".to_string(), "10de:1f95".to_string())];
        let players = env.clone();
        g.apply(&mut env, &players);
        assert_eq!(env.iter().find(|(k, _)| k == "MESA_VK_DEVICE_SELECT").unwrap().1, "10de:1f95");
        assert!(env.iter().any(|(k, _)| k == "DRI_PRIME"));
        assert!(game_gpu(&gpu::Choice::Auto, &all).env.is_empty());
        // a card that isn't there leaves the choice to the system, with a note
        let missing = game_gpu(&gpu::Choice::Specific("nope".into()), &all);
        assert!(missing.env.is_empty() && missing.note.is_some());
        // one card only: nothing to choose
        assert!(game_gpu(&gpu::Choice::Dedicated, std::slice::from_ref(&nvidia)).env.is_empty());
    }

    #[test]
    fn shader_caches_are_kept_apart_per_engine_and_card() {
        let nv = GameGpu { pci_ids: Some((0x10de, 0x1f95)), ..GameGpu::default() };
        let a = shader_cache_env(Path::new("/c"), "rev 1/x", Some(&nv));
        assert_eq!(a[0], ("VKD3D_SHADER_CACHE_PATH".into(), "/c/graphics/rev_1_x/10de-1f95/vkd3d".into()));
        assert_eq!(a[1].1, "/c/graphics/rev_1_x/10de-1f95/dxvk");
        assert!(shader_cache_env(Path::new("/c"), "r", None)[0].1.contains("/default/"));
    }
}
