//! Minecraft: Bedrock Edition, BlockForge's own way: the game fetched from the Microsoft Store under the player's own
//! licence (through the Xodus libraries, GPL-3.0) and played on Linux in a Windows compatibility engine (WineGDK),
//! all orchestrated in Rust. A port of what BedrockOnLinux (MIT, https://github.com/Wyze3306/BedrockOnLinux) does,
//! with credit; no BedrockOnLinux program, no Python. The plan the modules follow: docs in the research notes,
//! `PORT-PLAN.md`.
//!
//! Nothing here touches the disk or the network on the window's thread: [`Backend::snapshot`] is a copy of what is
//! in memory, and everything else runs on the runtime's workers.

// The interface is in place before the parts behind it: these come off once every stub is filled in.
#![allow(unused_variables, unused_imports, dead_code)]
use crate::{gpu, install::Progress, launch::Running, paths::Dirs};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{broadcast, watch};

#[cfg(target_os = "linux")]
mod artifact;
mod diagnostics;
#[cfg(target_os = "linux")]
mod engine;
#[cfg(target_os = "linux")]
mod fixups;
#[cfg(target_os = "linux")]
mod gpu_safety;
#[cfg(target_os = "linux")]
mod pins;
#[cfg(target_os = "linux")]
mod prefix;
#[cfg(target_os = "linux")]
mod proton;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(target_os = "linux")]
mod saves;
mod service;
mod storage;
#[cfg(target_os = "linux")]
mod store;
mod supervisor;
#[cfg(target_os = "linux")]
mod xbox;

pub type Result<T> = std::result::Result<T, BedrockError>;

/// Stable UUID-backed identifiers; validate deserialized input before path use.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub uuid::Uuid);
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct StoreId(pub uuid::Uuid);
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct InstanceId(pub uuid::Uuid);
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub uuid::Uuid);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Edition {
    Release,
    Preview,
}

/// Exact package/version, not a mutable "latest" reference.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BuildId {
    pub edition: Edition,
    pub version: String,
    pub package_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BuildSelection {
    Latest(Edition),
    Installed(BuildId),
}

/// Durable metadata; saves are in this instance's own profile prefix.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BedrockInstance {
    pub id: InstanceId,
    pub name: String,
    pub profile: ProfileId,
    pub store: StoreId,
    pub selection: BuildSelection,
    pub installed: Option<BuildId>,
}
#[derive(Clone, Debug)]
pub struct CreateRequest {
    pub name: String,
    pub selection: BuildSelection,
    /// None allocates a new persistent Store slot; UI may reuse an existing one.
    pub store: Option<StoreId>,
}
#[derive(Clone, Copy, Debug)]
pub enum RefreshPolicy {
    LocalOnly,
    NetworkAllowed,
}
#[derive(Clone, Debug)]
pub enum AccountTarget {
    Store(StoreId),
    Xbox(ProfileId),
}
#[derive(Clone, Debug)]
pub enum SignInState {
    Unknown,
    SignedOut,
    SigningIn,
    /// A cached credential hint; does not assert a currently valid entitlement.
    Linked {
        display_name: Option<String>,
    },
    NeedsSignIn {
        reason: String,
    },
}
#[derive(Clone, Debug)]
pub struct AccountStatus {
    pub target: AccountTarget,
    pub state: SignInState,
}
#[derive(Clone, Debug)]
pub struct BuildInfo {
    pub id: BuildId,
    pub installed: bool,
    pub download_bytes: Option<u64>,
}
#[derive(Clone, Debug)]
pub enum Readiness {
    Unknown,
    UnsupportedPlatform,
    NeedsInstall,
    NeedsStoreSignIn,
    NeedsRepair { reason: String },
    Ready { xbox_online_cached: bool },
    Starting(SessionId),
    Running(SessionId),
    Stopping(SessionId),
}
#[derive(Clone, Debug)]
pub struct InstanceStatus {
    pub instance: BedrockInstance,
    pub readiness: Readiness,
}
/// Public UI prompt data only; never contains polling device_code or DA tokens.
#[derive(Clone, Debug)]
pub enum AuthPrompt {
    DeviceCode { profile: ProfileId, url: String, user_code: String, expires_at_unix: i64 },
    StoreWindow { store: StoreId },
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub instances: Vec<InstanceStatus>,
    pub accounts: Vec<AccountStatus>,
    /// Retained until completion/cancel, so a lagged GUI can recover its code.
    pub auth_prompts: Vec<AuthPrompt>,
    pub builds: Vec<BuildInfo>,
    pub refresh_in_progress: bool,
}
#[derive(Clone, Debug)]
pub struct PlayOptions {
    pub gpu: gpu::Choice,
    pub diagnostics: bool,
    /// V1 default false. Online failures become a visible degraded-mode notice.
    pub require_xbox_online: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum StopMode {
    Graceful,
    Force,
}
#[derive(Clone, Copy, Debug)]
pub enum SignOutMode {
    /// Forget user auth; keep stable Store device to avoid registration churn.
    User,
    /// Explicit separate action; forbidden while any slot/profile session is live.
    UserAndDevice,
}
#[derive(Clone, Debug)]
pub enum ErrorKind {
    UnsupportedPlatform,
    Cancelled,
    Busy,
    SignInRequired,
    NotEntitled,
    LicenseUnavailable,
    Network,
    NoSpace,
    Integrity,
    Prefix,
    Runtime,
    InvalidRequest,
    NotImplemented,
    Internal,
}
#[derive(Clone, Debug)]
pub struct BedrockError {
    pub kind: ErrorKind,
    pub message: String,
    pub log_path: Option<PathBuf>,
}
impl std::fmt::Display for BedrockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for BedrockError {}

impl BedrockError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), log_path: None }
    }

    /// What a part not built yet answers, so nothing can take a stub for a success.
    pub fn not_implemented() -> Self {
        Self::new(ErrorKind::NotImplemented, "Bedrock Edition isn't finished in this build of BlockForge yet")
    }

    pub fn unsupported() -> Self {
        Self::new(ErrorKind::UnsupportedPlatform, "Bedrock Edition plays through BlockForge on Linux only for now")
    }
}

/// watch retains state changes; cancellation wakes pending auth/network waits.
#[derive(Clone)]
pub struct Cancel(Arc<watch::Sender<bool>>);
impl Default for Cancel {
    fn default() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }
}
impl Cancel {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        let _ = rx.wait_for(|v| *v).await;
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    SnapshotChanged,
    /// Public user code only. Never the device_code polling credential.
    DeviceCode {
        profile: ProfileId,
        url: String,
        user_code: String,
        expires_at_unix: i64,
    },
    StoreWindowOpened(StoreId),
    SessionStarted(SessionId),
    SessionExited {
        session: SessionId,
        code: Option<i32>,
    },
    Notice {
        instance: Option<InstanceId>,
        text: String,
    },
}

/// Allocation-only facade; methods schedule work away from the UI thread.
#[derive(Clone)]
pub struct Backend {
    inner: Arc<service::Service>,
}
impl Backend {
    /// Uses Dirs.base/bedrock. No disk probes, directories or network here.
    pub fn new(dirs: &Dirs, handle: tokio::runtime::Handle) -> Self {
        Self { inner: Arc::new(service::Service::new(dirs, handle)) }
    }
    /// Memory-only clone. Never scans or checks installed applications.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.snapshot()
    }
    /// Events are hints: recover from broadcast lag by rereading snapshot.
    /// Auth/result delivery remains guaranteed by operation future/snapshot.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.subscribe()
    }
    /// Background reload/detection; network only when explicitly requested.
    pub async fn refresh(&self, policy: RefreshPolicy, cancel: Cancel) -> Result<Arc<Snapshot>> {
        Err(BedrockError::not_implemented())
    }
    /// Creates metadata and an empty profile, not a Wine or GPU process.
    pub async fn create_instance(&self, request: CreateRequest) -> Result<BedrockInstance> {
        Err(BedrockError::not_implemented())
    }
    /// Changes this idle instance's selection; preserves prefix and snapshots
    /// same-edition user data before the new build is first launched.
    pub async fn select_build(&self, instance: InstanceId, selection: BuildSelection) -> Result<()> {
        Err(BedrockError::not_implemented())
    }
    /// Signs into only the named Store slot or Xbox game profile.
    pub async fn sign_in(&self, target: AccountTarget, progress: Arc<Progress>, cancel: Cancel) -> Result<()> {
        Err(BedrockError::not_implemented())
    }
    /// Idle-only. Rotates epoch and clears durable hive/cache copies safely.
    /// UserAndDevice is valid only for a Store target; otherwise InvalidRequest.
    pub async fn sign_out(&self, target: AccountTarget, mode: SignOutMode) -> Result<()> {
        Err(BedrockError::not_implemented())
    }
    /// Resolves selection, validates entitlement and atomically installs assets.
    pub async fn install(&self, instance: InstanceId, progress: Arc<Progress>, cancel: Cancel) -> Result<BuildId> {
        Err(BedrockError::not_implemented())
    }
    /// Prepares cached assets/prefix/auth, obtains launch license, spawns supervisor.
    /// Missing install is NeedsInstall; caller chains install then play explicitly.
    pub async fn play(
        &self,
        instance: InstanceId,
        options: PlayOptions,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Result<Session> {
        Err(BedrockError::not_implemented())
    }
    /// Revalidates owned assets/prefix while idle; preserves all user data.
    pub async fn repair(&self, instance: InstanceId, progress: Arc<Progress>, cancel: Cancel) -> Result<()> {
        Err(BedrockError::not_implemented())
    }
    /// Does file/process/sysfs inspection on workers; no automatic game relaunch.
    pub async fn diagnose(&self, instance: InstanceId) -> Result<DiagnosticReport> {
        Err(BedrockError::not_implemented())
    }
}

/// Running refers to the Rust supervisor, which owns Wine, leases and teardown.
/// GUI may drain lines/wait child, but must use control.stop for ordinary Stop.
pub struct Session {
    pub id: SessionId,
    pub running: Running,
    pub control: SessionControl,
    pub log_dir: PathBuf,
}
#[derive(Clone)]
pub struct SessionControl {
    inner: Arc<supervisor::Control>,
}
impl SessionControl {
    /// Graceful requests prefix-scoped TERM, wait, then bounded escalation.
    /// Success means teardown completed and prefix idleness was checked.
    pub async fn stop(&self, mode: StopMode) -> Result<()> {
        Err(BedrockError::not_implemented())
    }
}
#[derive(Clone, Debug)]
pub struct DiagnosticReport {
    pub issues: Vec<String>,
    pub log_dir: PathBuf,
}
