//! The Rust supervisor that owns a running game: Wine, its leases and its teardown (package E of PORT-PLAN.md).
//!
//! Ported from BedrockOnLinux `bol/launch.py` (the launch, its wait loop and its teardown), `bol/prefix.py` (locks,
//! prefix processes), `bol/supervision.py` (stop signals) and `bol/xodus.py` (the decrypted image's staging), with
//! credit. The shape, in one picture:
//!
//! ```text
//!  window (egui)                                  supervisor (this module's `session_main`)
//!  ─────────────                                  ─────────────────────────────────────────
//!  spawn(plan, secrets) ──launch::spawn_process──▶ blockforge --bedrock-session <dir>
//!       │  plan.json (0600), secrets on stdin            │ leases · graphics checks · marker · staging
//!       │                                                │ spawns Wine (the wrapper) in its own group
//!       ◀── event lines among the log lines ─────────────┤ watches the prefix, not the wrapper
//!       ◀── state.json (atomic) ─────────────────────────┤ answers Stop on control.sock
//!  Control::stop(mode) ──control.sock──────────────────▶ polite request → bounded escalation → reap →
//!                                                          wineserver idle check → marker cleared
//! ```
//!
//! The window may close and a new one attach to the same session with [`Control::attach`]: the supervisor is its own
//! process, in its own process group, and does not care that nobody reads its output.
//!
//! **Integrator**: call [`session_main`] first thing in `main`, before any window or runtime is made:
//!
//! ```ignore
//! let args: Vec<String> = std::env::args().skip(1).collect();
//! if let Some(code) = blockforge::bedrock::run_session_if_requested(&args) { std::process::exit(code); }
//! ```
//!
//! (`run_session_if_requested` is a one-line wrapper in `bedrock/mod.rs` around `supervisor::session_main`.)

pub mod lease;
pub mod plan;

#[cfg(target_os = "linux")]
mod ipc;
#[cfg(target_os = "linux")]
mod procs;
#[cfg(target_os = "linux")]
mod redact;
#[cfg(target_os = "linux")]
mod run;
#[cfg(target_os = "linux")]
mod staging;
#[cfg(target_os = "linux")]
mod sys;

pub use plan::{
    EVENT_PREFIX, FailureKind, LaunchPlan, SESSION_ARG, SessionEvent, Stage, State, StopOutcome, TimingMs,
    parse_event_line,
};

#[cfg(target_os = "linux")]
pub use ipc::Control;
#[cfg(target_os = "linux")]
pub use run::{Spawned, cleanup_stale_sessions, session_main, spawn, spawn_with};
#[cfg(target_os = "linux")]
pub use staging::{FileMap, StagingDir, boot_id, process_start, sweep_stale};

/// Free and total bytes of the file system `path` is on.
#[cfg(target_os = "linux")]
pub fn free_space(path: &std::path::Path) -> Option<(u64, u64)> {
    sys::space(path)
}

/// Where decrypted game files may be staged on this computer (RAM only), if anywhere.
#[cfg(target_os = "linux")]
pub fn staging_system_root() -> Option<std::path::PathBuf> {
    staging::system_root().map(|c| c.root)
}

/// The window's handle on a session where there are no sessions (every platform but Linux).
#[cfg(not(target_os = "linux"))]
#[derive(Clone, Debug)]
pub struct Control;

#[cfg(not(target_os = "linux"))]
impl Control {
    pub async fn stop(&self, _mode: super::StopMode) -> Result<StopOutcome, super::BedrockError> {
        Err(super::BedrockError::unsupported())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
