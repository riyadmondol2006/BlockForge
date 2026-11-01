//! Minecraft: Bedrock Edition, BlockForge's own way: the game fetched from the Microsoft Store under the player's own
//! licence (through the Xodus libraries, GPL-3.0) and played on Linux in a Windows compatibility engine (WineGDK),
//! all orchestrated in Rust. A port of what BedrockOnLinux (MIT, https://github.com/Wyze3306/BedrockOnLinux) does,
//! with credit; no BedrockOnLinux program, no Python. The plan the modules follow: docs in the research notes,
//! `PORT-PLAN.md`.
//!
//! Nothing here touches the disk or the network on the window's thread: [`Backend::snapshot`] is a copy of what is
//! in memory, and everything else runs on the runtime's workers.

// Where BlockForge's own backend isn't built (Windows and macOS, which play the game natively or not at all, and a
// Linux build without the `bedrock` feature), the facade's plumbing for it has nothing to call.
#![cfg_attr(not(all(target_os = "linux", feature = "bedrock")), allow(dead_code, unused_imports))]
use crate::{gpu, install::Progress, launch::Running, paths::Dirs};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, watch};

#[cfg(target_os = "linux")]
mod artifact;
#[cfg(target_os = "linux")]
mod diagnostics;
#[cfg(target_os = "linux")]
mod engine;
#[cfg(target_os = "linux")]
mod fixups;
#[cfg(target_os = "linux")]
mod gpu_safety;
#[cfg(target_os = "linux")]
mod helpers;
#[cfg(all(target_os = "linux", feature = "bedrock"))]
mod linux;
mod native;
#[cfg(target_os = "linux")]
mod payloads;
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
#[cfg(all(target_os = "linux", feature = "bedrock"))]
mod store;
#[cfg(target_os = "linux")]
mod supervisor;
#[cfg(target_os = "linux")]
mod xbox;

pub type Result<T> = std::result::Result<T, BedrockError>;

pub use native::store_page;
pub use service::{UNAVAILABLE, support};

/// Whether a line of a session's output is one of its supervisor's event lines rather than the game's log (the
/// window leaves those out of the log it shows; the snapshot and the events carry what they say).
pub fn is_session_event(line: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        supervisor::parse_event_line(line).is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = line;
        false
    }
}

/// The game session's supervisor runs in a BlockForge process of its own: `blockforge --bedrock-session <folder>`.
/// `main` calls this before anything else (no window, no runtime): with those arguments it runs the session and
/// returns the process's exit code; otherwise it does nothing and returns None.
pub fn run_session_if_requested(args: &[String]) -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        supervisor::session_main(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        None
    }
}

/// How Bedrock is played on this system: through BlockForge's own backend (Linux), as the native Store package the
/// system runs (Windows), or not yet (macOS and the rest; [`UNAVAILABLE`] says why).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Support {
    Linux,
    Windows,
    Unavailable,
}

/// What BlockForge's Bedrock support follows and uses, shown wherever Bedrock is offered.
pub const CREDIT: &str = "BlockForge plays Bedrock Edition the way BedrockOnLinux does (by Wyze3306 and its \
                          contributors, MIT licence), ported to Rust, and downloads your own copy from the Microsoft \
                          Store with the Xodus libraries (GPL-3.0). Neither is affiliated with Mojang or Microsoft.";
pub const BOL_REPOSITORY: &str = "https://github.com/Wyze3306/BedrockOnLinux";
pub const XODUS_REPOSITORY: &str = "https://github.com/xodus-gaming/xodus";

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
    /// Who it is, as far as is known on this computer (empty when signed out or not known yet).
    pub identity: AccountIdentity,
}
/// Who a Microsoft account is, from what its sign-ins left on this computer: the Store's sign-in name, the Xbox
/// account's XUID, gamertag and user hash. For telling whether a Bedrock sign-in is the same person as one of
/// BlockForge's own Microsoft accounts (their sign-ins can't be shared: they are different Microsoft apps).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountIdentity {
    /// The sign-in name: an e-mail address (or phone number).
    pub email: Option<String>,
    pub gamertag: Option<String>,
    pub xuid: Option<String>,
    /// Xbox Live's user hash (`uhs`).
    pub user_hash: Option<String>,
}
impl AccountIdentity {
    pub fn is_empty(&self) -> bool {
        self == &AccountIdentity::default()
    }
    /// Fill in what `self` doesn't know from `other` (the Store's name and the Xbox account's IDs, say).
    pub fn merged(mut self, other: &AccountIdentity) -> AccountIdentity {
        for (mine, theirs) in [
            (&mut self.email, &other.email),
            (&mut self.gamertag, &other.gamertag),
            (&mut self.xuid, &other.xuid),
            (&mut self.user_hash, &other.user_hash),
        ] {
            if mine.is_none() {
                mine.clone_from(theirs);
            }
        }
        self
    }
    /// Whether both are the same person: `Some(false)` only on proof (different XUIDs, or different sign-in
    /// names), `Some(true)` on any shared ID, `None` when nothing can be compared. A user hash or a gamertag can
    /// only confirm: either may change, so a difference proves nothing.
    pub fn same_person(&self, other: &AccountIdentity) -> Option<bool> {
        fn both<'a>(a: &'a Option<String>, b: &'a Option<String>) -> Option<(&'a str, &'a str)> {
            Some((a.as_deref().filter(|s| !s.is_empty())?, b.as_deref().filter(|s| !s.is_empty())?))
        }
        if let Some((a, b)) = both(&self.xuid, &other.xuid) {
            return Some(a == b);
        }
        if let Some((a, b)) = both(&self.email, &other.email) {
            if a.eq_ignore_ascii_case(b) {
                return Some(true);
            }
            // A phone number on one side and an address on the other prove nothing.
            if a.contains('@') && b.contains('@') {
                return Some(false);
            }
        }
        if both(&self.user_hash, &other.user_hash).is_some_and(|(a, b)| a == b)
            || both(&self.gamertag, &other.gamertag).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some(true);
        }
        None
    }
    /// The name to show for it: the gamertag, else the sign-in name.
    pub fn name(&self) -> Option<&str> {
        self.gamertag.as_deref().or(self.email.as_deref()).filter(|s| !s.is_empty())
    }
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
    /// How to stop a game that was already running when BlockForge opened (one a closed window left behind), once
    /// the first refresh has found it. None for any other instance.
    pub fn attached_control(&self, id: &InstanceId) -> Option<SessionControl> {
        self.inner.attached_control(id)
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
        self.inner.refresh(policy, cancel).await
    }
    /// Creates metadata and an empty profile, not a Wine or GPU process.
    pub async fn create_instance(&self, request: CreateRequest) -> Result<BedrockInstance> {
        self.inner.create_instance(request).await
    }
    /// Idle-only. The name only; the profile and its worlds stay where they are.
    pub async fn rename_instance(&self, instance: InstanceId, name: String) -> Result<()> {
        self.inner.rename_instance(&instance, &name).await
    }
    /// Idle-only. Removes the instance and its profile (its Wine prefix with the worlds in it on Linux; on Windows
    /// the game's own worlds folder is left alone). The Store slot stays for other instances.
    pub async fn delete_instance(&self, instance: InstanceId) -> Result<()> {
        self.inner.delete_instance(&instance).await
    }
    /// Changes this idle instance's selection; preserves prefix and snapshots
    /// same-edition user data before the new build is first launched.
    pub async fn select_build(&self, instance: InstanceId, selection: BuildSelection) -> Result<()> {
        self.inner.select_build(instance, selection).await
    }
    /// The one Microsoft sign-in for a Store target: one window signs in to the Store slot and then, in the same
    /// session, links Xbox for every game profile using that slot (falling back to the device code, as
    /// `AccountTarget::Xbox` does, for any it couldn't); a Store slot signed in already only links Xbox. Both
    /// states are in the snapshot after. An error only when the Store part fails. `AccountTarget::Xbox` signs in
    /// only that profile, by device code.
    pub async fn sign_in(&self, target: AccountTarget, progress: Arc<Progress>, cancel: Cancel) -> Result<()> {
        self.inner.sign_in(target, None, progress, cancel).await
    }
    /// [`Backend::sign_in`] with the sign-in name of the BlockForge account the player plays with prefilled in the
    /// window (ignored unless it is an e-mail address, and for `AccountTarget::Xbox`).
    pub async fn sign_in_as(
        &self,
        target: AccountTarget,
        login_hint: Option<String>,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Result<()> {
        self.inner.sign_in(target, login_hint, progress, cancel).await
    }
    /// Idle-only. Rotates epoch and clears durable hive/cache copies safely.
    /// UserAndDevice is valid only for a Store target; otherwise InvalidRequest.
    /// A Store target signs the whole Microsoft account out: each of its profiles' Xbox sign-in (the registry copy
    /// first, then the account), then the Store user; the Store device stays unless UserAndDevice.
    pub async fn sign_out(&self, target: AccountTarget, mode: SignOutMode) -> Result<()> {
        self.inner.sign_out(target, mode).await
    }
    /// Resolves selection, validates entitlement and atomically installs assets.
    pub async fn install(&self, instance: InstanceId, progress: Arc<Progress>, cancel: Cancel) -> Result<BuildId> {
        self.inner.install(instance, progress, cancel).await
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
        self.inner.play(instance, options, progress, cancel).await
    }
    /// Revalidates owned assets/prefix while idle; preserves all user data.
    pub async fn repair(&self, instance: InstanceId, progress: Arc<Progress>, cancel: Cancel) -> Result<()> {
        self.inner.repair(instance, progress, cancel).await
    }
    /// Does file/process/sysfs inspection on workers; no automatic game relaunch.
    pub async fn diagnose(&self, instance: InstanceId) -> Result<DiagnosticReport> {
        self.inner.diagnose(instance).await
    }
    /// Where the instance's worlds are kept (each player's folder, and Shared). Computed from memory, not checked.
    pub fn worlds_folder(&self, instance: &InstanceId) -> Option<PathBuf> {
        self.inner.worlds_folder(instance)
    }
    /// Idle-only. Copies the worlds into the profile's backups and returns the folder of the copy.
    pub async fn back_up_worlds(&self, instance: InstanceId) -> Result<PathBuf> {
        self.inner.back_up_worlds(instance).await
    }
}

/// Running refers to the Rust supervisor, which owns Wine, leases and teardown.
/// GUI may drain lines/wait child, but must use control.stop for ordinary Stop.
pub struct Session {
    pub id: SessionId,
    /// The supervisor's process, whose output is the game's log. None where the system runs the game itself (a
    /// Windows package): its state comes in the snapshot and the events.
    pub running: Option<Running>,
    pub control: SessionControl,
    pub log_dir: PathBuf,
}
#[derive(Clone)]
pub struct SessionControl {
    inner: Arc<service::Control>,
}
impl SessionControl {
    /// Graceful requests prefix-scoped TERM, wait, then bounded escalation.
    /// Success means teardown completed and prefix idleness was checked.
    pub async fn stop(&self, mode: StopMode) -> Result<()> {
        self.inner.stop(mode).await
    }
}
#[derive(Clone, Debug)]
pub struct DiagnosticReport {
    pub issues: Vec<String>,
    pub log_dir: PathBuf,
}
