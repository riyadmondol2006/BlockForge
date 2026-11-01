//! The backend's shared state and the work behind [`super::Backend`]: the snapshot the window reads, the events it
//! hears, the records on disk, and the arbitration of jobs (one install or start per instance, one sign-in per
//! account, one Bedrock game at a time in v1).
//!
//! The parts other packages build (the Store, the engine and runtime, the prefix, Xbox, the supervisor) are reached
//! through [`Parts`]: on Linux that is `super::linux::Linux`, which puts them together in the order each one
//! requires; elsewhere [`Elsewhere`], which has none of them. Tests put fakes there.
//!
//! On Windows the game is native and none of those parts are used: the instance finds Minecraft for Windows with the
//! package API and starts it by activation (see `native`). On other systems every operation says it isn't available.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock, Weak};

use tokio::sync::broadcast;

use super::storage::{self, InstanceRecord, Paths};
use super::{
    AccountIdentity, AccountStatus, AccountTarget, AuthPrompt, BedrockError, BedrockInstance, BuildId, BuildInfo,
    BuildSelection, Cancel, CreateRequest, DiagnosticReport, Edition, ErrorKind, Event, InstanceId, InstanceStatus,
    PlayOptions, ProfileId, Readiness, RefreshPolicy, Result, Session, SessionControl, SessionId, SignInState,
    SignOutMode, Snapshot, StopMode, StoreId, Support,
};
use crate::install::Progress;
use crate::launch::Running;
use crate::paths::Dirs;

/// A piece of work of the parts, boxed so [`Parts`] can be a trait object (and replaced by fakes in tests).
pub(super) type Job<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What every part gets: where things are, the runtime to run blocking work on, and the window's events (for the
/// notices a long job has along the way).
pub(super) struct Ctx {
    pub paths: Paths,
    pub handle: tokio::runtime::Handle,
    pub events: broadcast::Sender<Event>,
}

impl Ctx {
    /// Tell the window something worth knowing that doesn't stop the job.
    pub fn notice(&self, instance: Option<&InstanceId>, text: impl Into<String>) {
        let _ = self.events.send(Event::Notice { instance: instance.cloned(), text: text.into() });
    }
}

/// The way an Xbox sign-in shows its code: the window gets it in the snapshot and as an event. Public user code only;
/// the polling credential never comes here.
#[derive(Clone)]
pub(super) struct Prompts {
    shared: Arc<Shared>,
}

impl Prompts {
    pub fn device_code(&self, profile: &ProfileId, url: &str, user_code: &str, expires_at_unix: i64) {
        self.shared.update(|s| {
            s.prompts.retain(|p| !matches!(p, AuthPrompt::DeviceCode { profile: p, .. } if p == profile));
            s.prompts.push(AuthPrompt::DeviceCode {
                profile: profile.clone(),
                url: url.to_string(),
                user_code: user_code.to_string(),
                expires_at_unix,
            });
        });
        self.shared.send(Event::DeviceCode {
            profile: profile.clone(),
            url: url.to_string(),
            user_code: user_code.to_string(),
            expires_at_unix,
        });
    }
}

/// What Play asks the parts to start.
pub(super) struct LaunchRequest {
    pub instance: BedrockInstance,
    pub build: BuildId,
    pub session: SessionId,
    pub options: PlayOptions,
    /// `logs/<session>`: setup, license, game, runtime and diagnostics logs go here.
    pub log_dir: PathBuf,
    /// The build changed since the last start: the profile's worlds are to be copied first (`saves::before_launch`).
    pub backup_first: bool,
}

/// How a started game is stopped: the supervisor's control socket on Linux, the game's windows on Windows.
pub(super) trait StopGame: Send + Sync {
    fn stop(&self, mode: StopMode) -> Job<'_, ()>;
}

/// A game the parts have started: the supervisor's process (its output is the game's log), how to stop it, and two
/// futures the facade waits on: the game is up (its window exists), and the game is over (with its exit code).
pub(super) struct Launched {
    pub running: Option<Running>,
    pub stop: Arc<dyn StopGame>,
    pub started: Pin<Box<dyn Future<Output = ()> + Send>>,
    pub exited: Pin<Box<dyn Future<Output = Option<i32>> + Send>>,
}

/// What the facade needs from packages A to E (PORT-PLAN.md section 4). Every method runs on a worker; the two
/// `*_state` ones are cheap local checks (no network) made while refreshing.
pub(super) trait Parts: Send + Sync + 'static {
    /// The cached sign-in of a Store slot (a hint: it doesn't prove a valid licence).
    fn store_state(&self, ctx: &Ctx, store: &StoreId) -> SignInState;
    /// The cached Xbox sign-in of a game profile.
    fn xbox_state(&self, ctx: &Ctx, profile: &ProfileId) -> SignInState;
    /// Who a Store slot's account is, from its local record (no network).
    fn store_identity(&self, _ctx: &Ctx, _store: &StoreId) -> AccountIdentity {
        AccountIdentity::default()
    }
    /// Who a game profile's Xbox account is, from its local record (no network).
    fn xbox_identity(&self, _ctx: &Ctx, _profile: &ProfileId) -> AccountIdentity {
        AccountIdentity::default()
    }
    /// The one Microsoft sign-in window: the Store sign-in of `store` (None: it's signed in already), then the
    /// Xbox sign-in of each of `profiles` from the same session. Says, per profile, whether its Xbox sign-in was
    /// linked; the facade falls back to the device code for the others. An error only for the Store part.
    fn microsoft_sign_in<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: Option<&'a StoreId>,
        profiles: &'a [ProfileId],
        login_hint: Option<&'a str>,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, Vec<bool>> {
        let _ = login_hint;
        Box::pin(async move {
            if let Some(store) = store {
                self.store_sign_in(ctx, store, progress, cancel).await?;
            }
            Ok(vec![false; profiles.len()])
        })
    }
    fn store_sign_in<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: &'a StoreId,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, ()>;
    fn store_sign_out<'a>(&'a self, ctx: &'a Ctx, store: &'a StoreId, mode: SignOutMode) -> Job<'a, ()>;
    /// The newest build of `edition` the Store offers this slot (network).
    fn store_latest<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: &'a StoreId,
        edition: Edition,
        cancel: Cancel,
    ) -> Job<'a, BuildInfo>;
    /// Download and install the build `selection` names (or update to it), with the licence checked.
    fn store_install<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: &'a StoreId,
        selection: &'a BuildSelection,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, BuildId>;
    fn xbox_sign_in<'a>(
        &'a self,
        ctx: &'a Ctx,
        profile: &'a ProfileId,
        prompts: Prompts,
        cancel: Cancel,
    ) -> Job<'a, ()>;
    fn xbox_sign_out<'a>(&'a self, ctx: &'a Ctx, profile: &'a ProfileId) -> Job<'a, ()>;
    /// Engine, runtime, prefix, saves, Xbox readiness, staging and the supervisor, in that order.
    fn launch<'a>(
        &'a self,
        ctx: &'a Ctx,
        request: LaunchRequest,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, Launched>;
    /// Revalidate and rebuild while idle, keeping the worlds. Some(build) when the game had to be fetched again and
    /// that gave another build than the instance had.
    fn repair<'a>(
        &'a self,
        ctx: &'a Ctx,
        instance: &'a BedrockInstance,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, Option<BuildId>>;
    /// What is wrong, in plain words (empty when nothing is).
    fn diagnose<'a>(&'a self, ctx: &'a Ctx, instance: &'a BedrockInstance) -> Job<'a, Vec<String>>;
}

/// Where Bedrock isn't played through BlockForge's own backend (Windows runs the Store package itself; other systems
/// have nothing yet): the facade never gets this far there, and if it did, every part says it isn't available.
#[cfg(not(all(target_os = "linux", feature = "bedrock")))]
pub(super) struct Elsewhere;

#[cfg(not(all(target_os = "linux", feature = "bedrock")))]
impl Parts for Elsewhere {
    fn store_state(&self, _: &Ctx, _: &StoreId) -> SignInState {
        SignInState::Unknown
    }
    fn xbox_state(&self, _: &Ctx, _: &ProfileId) -> SignInState {
        SignInState::Unknown
    }
    fn store_sign_in<'a>(&'a self, _: &'a Ctx, _: &'a StoreId, _: Arc<Progress>, _: Cancel) -> Job<'a, ()> {
        Box::pin(async { Err(unavailable()) })
    }
    fn store_sign_out<'a>(&'a self, _: &'a Ctx, _: &'a StoreId, _: SignOutMode) -> Job<'a, ()> {
        Box::pin(async { Err(unavailable()) })
    }
    fn store_latest<'a>(&'a self, _: &'a Ctx, _: &'a StoreId, _: Edition, _: Cancel) -> Job<'a, BuildInfo> {
        Box::pin(async { Err(unavailable()) })
    }
    fn store_install<'a>(
        &'a self,
        _: &'a Ctx,
        _: &'a StoreId,
        _: &'a BuildSelection,
        _: Arc<Progress>,
        _: Cancel,
    ) -> Job<'a, BuildId> {
        Box::pin(async { Err(unavailable()) })
    }
    fn xbox_sign_in<'a>(&'a self, _: &'a Ctx, _: &'a ProfileId, _: Prompts, _: Cancel) -> Job<'a, ()> {
        Box::pin(async { Err(unavailable()) })
    }
    fn xbox_sign_out<'a>(&'a self, _: &'a Ctx, _: &'a ProfileId) -> Job<'a, ()> {
        Box::pin(async { Err(unavailable()) })
    }
    fn launch<'a>(&'a self, _: &'a Ctx, _: LaunchRequest, _: Arc<Progress>, _: Cancel) -> Job<'a, Launched> {
        Box::pin(async { Err(unavailable()) })
    }
    fn repair<'a>(
        &'a self,
        _: &'a Ctx,
        _: &'a BedrockInstance,
        _: Arc<Progress>,
        _: Cancel,
    ) -> Job<'a, Option<BuildId>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn diagnose<'a>(&'a self, _: &'a Ctx, _: &'a BedrockInstance) -> Job<'a, Vec<String>> {
        Box::pin(async { Err(unavailable()) })
    }
}

/// This system's parts.
fn platform_parts() -> Arc<dyn Parts> {
    #[cfg(all(target_os = "linux", feature = "bedrock"))]
    {
        Arc::new(super::linux::Linux)
    }
    #[cfg(not(all(target_os = "linux", feature = "bedrock")))]
    {
        Arc::new(Elsewhere)
    }
}

/// What the window can be told the platform offers.
pub fn support() -> Support {
    if cfg!(target_os = "linux") {
        Support::Linux
    } else if cfg!(windows) {
        Support::Windows
    } else {
        Support::Unavailable
    }
}

/// Why Bedrock can't be played here, said where it would be offered.
pub const UNAVAILABLE: &str = "Not on macOS yet: Minecraft for Windows needs a Windows compatibility engine for \
                               Apple Silicon that BlockForge doesn't have yet.";

fn unavailable() -> BedrockError {
    BedrockError::new(ErrorKind::UnsupportedPlatform, UNAVAILABLE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LivePhase {
    Starting,
    Running,
    Stopping,
}

#[derive(Clone, Debug)]
struct Live {
    session: SessionId,
    phase: LivePhase,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum JobKey {
    Instance(uuid::Uuid),
    Store(uuid::Uuid),
    Xbox(uuid::Uuid),
    /// One Bedrock game at a time on this computer in v1 (PORT-PLAN.md section 4).
    Game,
    Refresh,
}

impl JobKey {
    fn busy(&self) -> &'static str {
        match self {
            JobKey::Instance(_) => "This instance is busy with another job. Wait for it to finish.",
            JobKey::Store(_) => "This Microsoft Store sign-in is busy with another job. Wait for it to finish.",
            JobKey::Xbox(_) => "An Xbox sign-in for this instance is already open.",
            JobKey::Game => "A Bedrock game is already starting or running. Close it first: one plays at a time.",
            JobKey::Refresh => "Bedrock is already being looked at again.",
        }
    }
}

#[derive(Default)]
struct State {
    loaded: bool,
    read_only: Option<String>,
    instances: Vec<InstanceRecord>,
    stores: Vec<StoreId>,
    store_states: HashMap<StoreId, SignInState>,
    xbox_states: HashMap<ProfileId, SignInState>,
    store_ids: HashMap<StoreId, AccountIdentity>,
    xbox_ids: HashMap<ProfileId, AccountIdentity>,
    prompts: Vec<AuthPrompt>,
    latest: HashMap<Edition, BuildInfo>,
    /// Minecraft for Windows as the package API found it (Windows only).
    native: HashMap<Edition, super::native::Package>,
    live: HashMap<InstanceId, Live>,
    repair: HashMap<InstanceId, String>,
    jobs: HashSet<JobKey>,
    refreshing: bool,
}

struct Shared {
    support: Support,
    snapshot: RwLock<Arc<Snapshot>>,
    events: broadcast::Sender<Event>,
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, e: Event) {
        // no receiver yet is fine: the snapshot holds the state
        let _ = self.events.send(e);
    }

    /// Change the state, then publish a new snapshot of it.
    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let (r, snap) = {
            let mut s = self.lock();
            let r = change(&mut s);
            (r, snapshot_of(self.support, &s))
        };
        *self.snapshot.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(snap);
        self.send(Event::SnapshotChanged);
        r
    }

    fn notice(&self, instance: Option<InstanceId>, text: impl Into<String>) {
        self.send(Event::Notice { instance, text: text.into() });
    }
}

/// Holds job keys until dropped.
struct Guard {
    shared: Arc<Shared>,
    keys: Vec<JobKey>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut s = self.shared.lock();
        for k in &self.keys {
            s.jobs.remove(k);
        }
    }
}

/// Where a profile's game keeps its players' folders inside its Wine prefix (BedrockOnLinux bol/content.py).
fn prefix_users(paths: &Paths, profile: &ProfileId, edition: Edition) -> PathBuf {
    paths
        .profile(profile)
        .join("compat/pfx/drive_c/users/steamuser/AppData/Roaming")
        .join(super::native::data_folder(edition))
        .join("Users")
}

/// What an instance can do now, from what is known in memory.
fn readiness(support: Support, s: &State, rec: &InstanceRecord) -> Readiness {
    if support == Support::Unavailable {
        return Readiness::UnsupportedPlatform;
    }
    if let Some(live) = s.live.get(&rec.id) {
        return match live.phase {
            LivePhase::Starting => Readiness::Starting(live.session.clone()),
            LivePhase::Running => Readiness::Running(live.session.clone()),
            LivePhase::Stopping => Readiness::Stopping(live.session.clone()),
        };
    }
    if support == Support::Windows {
        return if s.native.contains_key(&rec.edition()) {
            Readiness::Ready { xbox_online_cached: false }
        } else {
            Readiness::NeedsInstall
        };
    }
    if let Some(reason) = s.repair.get(&rec.id) {
        return Readiness::NeedsRepair { reason: reason.clone() };
    }
    let store = s.store_states.get(&rec.store).cloned().unwrap_or(SignInState::Unknown);
    if matches!(store, SignInState::SignedOut | SignInState::NeedsSignIn { .. } | SignInState::SigningIn) {
        return Readiness::NeedsStoreSignIn;
    }
    if !installed_matches(rec) {
        return Readiness::NeedsInstall;
    }
    let xbox = s.xbox_states.get(&rec.profile);
    Readiness::Ready { xbox_online_cached: matches!(xbox, Some(SignInState::Linked { .. })) }
}

/// Whether the build installed for an instance is the one it asks for.
fn installed_matches(rec: &InstanceRecord) -> bool {
    match (&rec.selection, &rec.installed) {
        (_, None) => false,
        (BuildSelection::Latest(e), Some(b)) => b.edition == *e,
        (BuildSelection::Installed(want), Some(b)) => want == b,
    }
}

fn snapshot_of(support: Support, s: &State) -> Snapshot {
    let instances = s
        .instances
        .iter()
        .map(|rec| {
            let mut instance = rec.public();
            if support == Support::Windows {
                // on Windows the installed build is the package the system has, found when refreshing
                instance.installed = s.native.get(&rec.edition()).map(|p| p.build_id());
            }
            InstanceStatus { instance, readiness: readiness(support, s, rec) }
        })
        .collect();
    let mut accounts: Vec<AccountStatus> = s
        .stores
        .iter()
        .map(|id| AccountStatus {
            target: AccountTarget::Store(id.clone()),
            state: s.store_states.get(id).cloned().unwrap_or(SignInState::Unknown),
            identity: s.store_ids.get(id).cloned().unwrap_or_default(),
        })
        .collect();
    accounts.extend(s.instances.iter().map(|rec| AccountStatus {
        target: AccountTarget::Xbox(rec.profile.clone()),
        state: s.xbox_states.get(&rec.profile).cloned().unwrap_or(SignInState::Unknown),
        identity: s.xbox_ids.get(&rec.profile).cloned().unwrap_or_default(),
    }));
    let mut builds: Vec<BuildInfo> = Vec::new();
    let mut add = |info: BuildInfo| {
        if let Some(b) = builds.iter_mut().find(|b| b.id == info.id) {
            b.installed |= info.installed;
            b.download_bytes = b.download_bytes.or(info.download_bytes);
        } else {
            builds.push(info);
        }
    };
    if support == Support::Windows {
        for p in s.native.values() {
            add(BuildInfo { id: p.build_id(), installed: true, download_bytes: None });
        }
    } else {
        for rec in &s.instances {
            if let Some(b) = &rec.installed {
                add(BuildInfo { id: b.clone(), installed: true, download_bytes: None });
            }
        }
        for info in s.latest.values() {
            add(info.clone());
        }
    }
    Snapshot { instances, accounts, auth_prompts: s.prompts.clone(), builds, refresh_in_progress: s.refreshing }
}

pub struct Service {
    pub(super) root: PathBuf,
    pub(super) handle: tokio::runtime::Handle,
    shared: Arc<Shared>,
    parts: Arc<dyn Parts>,
}

impl Service {
    /// Only allocates: no folder is made or read here.
    pub fn new(dirs: &Dirs, handle: tokio::runtime::Handle) -> Self {
        Self::with_parts(dirs.base.join("bedrock"), handle, support(), platform_parts())
    }

    pub(super) fn with_parts(
        root: PathBuf,
        handle: tokio::runtime::Handle,
        support: Support,
        parts: Arc<dyn Parts>,
    ) -> Self {
        Self {
            root,
            handle,
            shared: Arc::new(Shared {
                support,
                snapshot: RwLock::new(Arc::new(Snapshot::default())),
                events: broadcast::channel(256).0,
                state: Mutex::new(State::default()),
            }),
            parts,
        }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.shared.snapshot.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    fn ctx(&self) -> Ctx {
        Ctx { paths: Paths::at(&self.root), handle: self.handle.clone(), events: self.shared.events.clone() }
    }

    fn support(&self) -> Support {
        self.shared.support
    }

    /// Fails on a system where Bedrock isn't available.
    fn available(&self) -> Result<()> {
        if self.support() == Support::Unavailable { Err(unavailable()) } else { Ok(()) }
    }

    /// Take job keys, or say what holds one of them.
    fn begin(&self, keys: Vec<JobKey>) -> Result<Guard> {
        let mut s = self.shared.lock();
        if let Some(taken) = keys.iter().find(|k| s.jobs.contains(k)) {
            return Err(BedrockError::new(ErrorKind::Busy, taken.busy()));
        }
        s.jobs.extend(keys.iter().cloned());
        Ok(Guard { shared: self.shared.clone(), keys })
    }

    /// Run blocking work (the disk, the system's package API) on a worker.
    async fn blocking<T: Send + 'static>(&self, f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
        self.handle
            .spawn_blocking(f)
            .await
            .map_err(|e| BedrockError::new(ErrorKind::Internal, format!("A Bedrock job stopped unexpectedly: {e}")))?
    }

    fn record(&self, id: &InstanceId) -> Result<InstanceRecord> {
        self.shared
            .lock()
            .instances
            .iter()
            .find(|r| &r.id == id)
            .cloned()
            .ok_or_else(|| BedrockError::new(ErrorKind::InvalidRequest, "That Bedrock instance is gone"))
    }

    fn writable(&self) -> Result<()> {
        match &self.shared.lock().read_only {
            Some(why) => Err(BedrockError::new(ErrorKind::InvalidRequest, why.clone())),
            None => Ok(()),
        }
    }

    /// Refuse to change an instance whose game is starting, running or stopping.
    fn idle(&self, id: &InstanceId) -> Result<()> {
        if self.shared.lock().live.contains_key(id) {
            return Err(BedrockError::new(ErrorKind::Busy, "Close the game first."));
        }
        Ok(())
    }

    /// The other BlockForge processes are kept off this instance while the returned lock is held.
    async fn lock_instance(&self, id: &InstanceId) -> Result<storage::FileLock> {
        let (paths, name) = (Paths::at(&self.root), format!("instance-{}", id.0));
        self.blocking(move || storage::try_lock(&paths, &name)).await
    }

    async fn save(&self, rec: InstanceRecord) -> Result<()> {
        let paths = Paths::at(&self.root);
        self.blocking(move || storage::save(&paths, &rec)).await
    }

    /// Read the records the first time they are needed.
    async fn ensure_loaded(&self) -> Result<()> {
        if self.shared.lock().loaded {
            return Ok(());
        }
        self.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.map(|_| ())
    }

    pub async fn refresh(&self, policy: RefreshPolicy, cancel: Cancel) -> Result<Arc<Snapshot>> {
        let Ok(_guard) = self.begin(vec![JobKey::Refresh]) else {
            // one is under way: what it finds is published when it is done
            return Ok(self.snapshot());
        };
        self.shared.update(|s| s.refreshing = true);
        let result = self.refresh_inner(policy, cancel).await;
        self.shared.update(|s| s.refreshing = false);
        result.map(|()| self.snapshot())
    }

    async fn refresh_inner(&self, policy: RefreshPolicy, cancel: Cancel) -> Result<()> {
        let support = self.support();
        let paths = Paths::at(&self.root);
        let parts = self.parts.clone();
        let handle = self.handle.clone();
        let events = self.shared.events.clone();
        let first = !self.shared.lock().loaded;
        let (loaded, store_states, xbox_states, native) = self
            .blocking(move || {
                // what sessions of an earlier run left behind (their folders, their staged images), once per run
                #[cfg(all(target_os = "linux", feature = "bedrock"))]
                if first && support == Support::Linux {
                    super::linux::clean_up_after_earlier_sessions(paths.root());
                }
                #[cfg(not(all(target_os = "linux", feature = "bedrock")))]
                let _ = first;
                let loaded = storage::load_all(&paths);
                let ctx = Ctx { paths, handle, events };
                let (mut stores, mut xbox) = (HashMap::new(), HashMap::new());
                if support == Support::Linux {
                    for id in &loaded.stores {
                        stores.insert(id.clone(), (parts.store_state(&ctx, id), parts.store_identity(&ctx, id)));
                    }
                    for rec in &loaded.instances {
                        let profile = &rec.profile;
                        xbox.insert(
                            profile.clone(),
                            (parts.xbox_state(&ctx, profile), parts.xbox_identity(&ctx, profile)),
                        );
                    }
                }
                #[allow(unused_mut)]
                let mut native = HashMap::new();
                #[cfg(windows)]
                {
                    if support == Support::Windows {
                        for e in [Edition::Release, Edition::Preview] {
                            if let Some(p) = super::native::detect(e) {
                                native.insert(e, p);
                            }
                        }
                    }
                }
                Ok((loaded, stores, xbox, native))
            })
            .await?;
        for p in &loaded.problems {
            crate::logs::warn(format!("Bedrock: {p}"));
            self.shared.notice(None, p.clone());
        }
        let editions: Vec<(StoreId, Edition)> = self.shared.update(|s| {
            s.loaded = true;
            s.read_only = loaded.read_only;
            s.instances = loaded.instances;
            s.stores = loaded.stores;
            // a sign-in under way keeps showing as one
            for (id, (st, who)) in store_states {
                if !matches!(s.store_states.get(&id), Some(SignInState::SigningIn)) {
                    s.store_states.insert(id.clone(), st);
                    s.store_ids.insert(id, who);
                }
            }
            for (id, (st, who)) in xbox_states {
                if !matches!(s.xbox_states.get(&id), Some(SignInState::SigningIn)) {
                    s.xbox_states.insert(id.clone(), st);
                    s.xbox_ids.insert(id, who);
                }
            }
            s.native = native;
            let mut wanted: Vec<(StoreId, Edition)> = Vec::new();
            for rec in &s.instances {
                // only a signed-in Store slot can ask the Store
                let signed_in = matches!(s.store_states.get(&rec.store), Some(SignInState::Linked { .. }));
                if signed_in
                    && matches!(rec.selection, BuildSelection::Latest(_))
                    && !wanted.iter().any(|(_, e)| *e == rec.edition())
                {
                    wanted.push((rec.store.clone(), rec.edition()));
                }
            }
            wanted
        });
        if matches!(policy, RefreshPolicy::NetworkAllowed) && support == Support::Linux {
            let ctx = self.ctx();
            for (store, edition) in editions {
                match self.parts.store_latest(&ctx, &store, edition, cancel.clone()).await {
                    Ok(info) => self.shared.update(|s| {
                        s.latest.insert(edition, info);
                    }),
                    Err(e) if matches!(e.kind, ErrorKind::Cancelled) => return Err(e),
                    // what the Store said is kept for the window; the instances still play what they have
                    Err(e) => self.shared.notice(None, format!("Couldn't look for a newer Bedrock build: {e}")),
                }
            }
        }
        Ok(())
    }

    pub async fn create_instance(&self, request: CreateRequest) -> Result<BedrockInstance> {
        self.available()?;
        self.ensure_loaded().await?;
        self.writable()?;
        let name = storage::instance_name(&request.name)?;
        storage::check_selection(&request.selection)?;
        let (store, new_store) = match request.store {
            Some(id) => {
                if !self.shared.lock().stores.contains(&id) {
                    return Err(BedrockError::new(ErrorKind::InvalidRequest, "That Store sign-in is gone"));
                }
                (id, false)
            }
            None => (StoreId(uuid::Uuid::new_v4()), true),
        };
        let rec = InstanceRecord {
            schema: storage::SCHEMA,
            id: InstanceId(uuid::Uuid::new_v4()),
            name,
            profile: ProfileId(uuid::Uuid::new_v4()),
            store,
            selection: request.selection,
            installed: None,
            created: storage::now(),
            last_played: None,
            backup_before_play: false,
        };
        let (paths, made) = (Paths::at(&self.root), rec.clone());
        self.blocking(move || storage::create(&paths, &made, new_store)).await?;
        let public = rec.public();
        self.shared.update(|s| {
            if new_store {
                s.stores.push(rec.store.clone());
                s.store_states.insert(rec.store.clone(), SignInState::SignedOut);
            }
            s.xbox_states.insert(rec.profile.clone(), SignInState::SignedOut);
            s.instances.push(rec);
        });
        Ok(public)
    }

    pub async fn rename_instance(&self, id: &InstanceId, name: &str) -> Result<()> {
        self.writable()?;
        let name = storage::instance_name(name)?;
        let _guard = self.begin(vec![JobKey::Instance(id.0)])?;
        let mut rec = self.record(id)?;
        rec.name = name;
        self.save(rec.clone()).await?;
        self.replace(rec);
        Ok(())
    }

    /// Put a changed record into the state and publish.
    fn replace(&self, rec: InstanceRecord) {
        self.shared.update(|s| {
            if let Some(r) = s.instances.iter_mut().find(|r| r.id == rec.id) {
                *r = rec;
            }
        });
    }

    pub async fn delete_instance(&self, id: &InstanceId) -> Result<()> {
        self.writable()?;
        let _guard = self.begin(vec![JobKey::Instance(id.0)])?;
        self.idle(id)?;
        let rec = self.record(id)?;
        let _lock = self.lock_instance(id).await?;
        let paths = Paths::at(&self.root);
        let gone = rec.clone();
        self.blocking(move || storage::remove(&paths, &gone)).await?;
        self.shared.update(|s| {
            s.instances.retain(|r| r.id != rec.id);
            s.xbox_states.remove(&rec.profile);
            s.xbox_ids.remove(&rec.profile);
            s.repair.remove(&rec.id);
            s.prompts.retain(|p| !matches!(p, AuthPrompt::DeviceCode { profile, .. } if *profile == rec.profile));
        });
        Ok(())
    }

    pub async fn select_build(&self, id: InstanceId, selection: BuildSelection) -> Result<()> {
        self.available()?;
        self.writable()?;
        storage::check_selection(&selection)?;
        let _guard = self.begin(vec![JobKey::Instance(id.0)])?;
        self.idle(&id)?;
        let mut rec = self.record(&id)?;
        let before = rec.edition();
        let installed = rec.installed.clone();
        rec.selection = selection;
        // the next start copies the worlds first when the build it plays is another one
        if installed.is_some() && (before != rec.edition() || !installed_matches(&rec)) {
            rec.backup_before_play = true;
        }
        self.save(rec.clone()).await?;
        self.replace(rec);
        Ok(())
    }

    pub async fn sign_in(
        &self,
        target: AccountTarget,
        login_hint: Option<String>,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Result<()> {
        self.available()?;
        if self.support() == Support::Windows {
            return Err(BedrockError::new(
                ErrorKind::InvalidRequest,
                "On Windows, Minecraft and the Microsoft Store sign in with your Windows account themselves.",
            ));
        }
        self.ensure_loaded().await?;
        let ctx = self.ctx();
        match &target {
            AccountTarget::Store(store) => self.microsoft_sign_in(&ctx, store, login_hint, progress, cancel).await,
            AccountTarget::Xbox(profile) => {
                if !self.shared.lock().instances.iter().any(|r| &r.profile == profile) {
                    return Err(BedrockError::new(ErrorKind::InvalidRequest, "That Bedrock instance is gone"));
                }
                let _guard = self.begin(vec![JobKey::Xbox(profile.0)])?;
                let before = self.shared.update(|s| {
                    s.xbox_states.insert(profile.clone(), SignInState::SigningIn).unwrap_or(SignInState::SignedOut)
                });
                let prompts = Prompts { shared: self.shared.clone() };
                let result = self.parts.xbox_sign_in(&ctx, profile, prompts, cancel).await;
                let after = self.after_sign_in(&result, before, || self.parts.xbox_state(&ctx, profile));
                let who = self.parts.xbox_identity(&ctx, profile);
                self.shared.update(|s| {
                    s.prompts.retain(|p| !matches!(p, AuthPrompt::DeviceCode { profile: id, .. } if id == profile));
                    s.xbox_states.insert(profile.clone(), after);
                    s.xbox_ids.insert(profile.clone(), who);
                });
                result
            }
        }
    }

    /// The one Microsoft sign-in of a Store slot: one window for the Store and the Xbox sign-in of every game
    /// profile on the slot that hasn't got one (a slot signed in already only links those), then the device code
    /// for any profile the window couldn't link. Only the Store part can fail the whole; an Xbox profile that
    /// couldn't be linked is told in its state and a notice, and plays offline meanwhile.
    async fn microsoft_sign_in(
        &self,
        ctx: &Ctx,
        store: &StoreId,
        login_hint: Option<String>,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Result<()> {
        let (store_too, profiles) = {
            let s = self.shared.lock();
            if !s.stores.contains(store) {
                return Err(BedrockError::new(ErrorKind::InvalidRequest, "That Store sign-in is gone"));
            }
            let mut profiles: Vec<ProfileId> = Vec::new();
            for rec in s.instances.iter().filter(|r| &r.store == store) {
                let linked = matches!(s.xbox_states.get(&rec.profile), Some(SignInState::Linked { .. }));
                // a profile with its own (device-code) sign-in under way is left to it
                if !linked && !s.jobs.contains(&JobKey::Xbox(rec.profile.0)) && !profiles.contains(&rec.profile) {
                    profiles.push(rec.profile.clone());
                }
            }
            let store_linked = matches!(s.store_states.get(store), Some(SignInState::Linked { .. }));
            // signed in to both already: this is a fresh Store sign-in, as asked
            (!store_linked || profiles.is_empty(), profiles)
        };
        let store_guard = self.begin(vec![JobKey::Store(store.0)])?;
        let _xbox_guard = self.begin(profiles.iter().map(|p| JobKey::Xbox(p.0)).collect())?;
        let (store_before, xbox_before) = self.shared.update(|s| {
            s.prompts.push(AuthPrompt::StoreWindow { store: store.clone() });
            let store_before = if store_too {
                s.store_states.insert(store.clone(), SignInState::SigningIn).unwrap_or(SignInState::SignedOut)
            } else {
                s.store_states.get(store).cloned().unwrap_or(SignInState::Unknown)
            };
            let xbox_before: Vec<SignInState> = profiles
                .iter()
                .map(|p| s.xbox_states.insert(p.clone(), SignInState::SigningIn).unwrap_or(SignInState::SignedOut))
                .collect();
            (store_before, xbox_before)
        });
        self.shared.send(Event::StoreWindowOpened(store.clone()));
        let hint = login_hint.as_deref().filter(|h| h.contains('@'));
        let window = self
            .parts
            .microsoft_sign_in(ctx, store_too.then_some(store), &profiles, hint, progress.clone(), cancel.clone())
            .await;
        let linked = match window {
            Ok(linked) => linked,
            Err(e) => {
                let after = self.after_sign_in(&Err(e.clone()), store_before, || SignInState::Unknown);
                self.shared.update(|s| {
                    s.prompts.retain(|p| !matches!(p, AuthPrompt::StoreWindow { store: id } if id == store));
                    if store_too {
                        s.store_states.insert(store.clone(), after);
                    }
                    for (p, before) in profiles.iter().zip(&xbox_before) {
                        s.xbox_states.insert(p.clone(), before.clone());
                    }
                });
                return Err(e);
            }
        };
        let store_after = self.after_sign_in(&Ok(()), store_before, || self.parts.store_state(ctx, store));
        let store_who = self.parts.store_identity(ctx, store);
        // what each linked profile's record says now (a few small reads, before the state is locked)
        let now: Vec<Option<(SignInState, AccountIdentity)>> = profiles
            .iter()
            .enumerate()
            .map(|(i, p)| {
                linked.get(i).copied().unwrap_or(false).then(|| {
                    let state = self.after_sign_in(&Ok(()), SignInState::SignedOut, || self.parts.xbox_state(ctx, p));
                    (state, self.parts.xbox_identity(ctx, p))
                })
            })
            .collect();
        let mut waiting = Vec::new();
        self.shared.update(|s| {
            s.prompts.retain(|p| !matches!(p, AuthPrompt::StoreWindow { store: id } if id == store));
            s.store_states.insert(store.clone(), store_after);
            s.store_ids.insert(store.clone(), store_who);
            for ((p, before), now) in profiles.iter().zip(&xbox_before).zip(now) {
                match now {
                    Some((state, who)) => {
                        s.xbox_states.insert(p.clone(), state);
                        s.xbox_ids.insert(p.clone(), who);
                    }
                    None if cancel.is_cancelled() => {
                        s.xbox_states.insert(p.clone(), before.clone());
                    }
                    None => waiting.push(p.clone()),
                }
            }
        });
        // the Store is done: installs may go ahead while a device code waits for the player
        drop(store_guard);
        for profile in waiting {
            // the window couldn't link this profile (no session, a refusal, the window closed): the device code
            let prompts = Prompts { shared: self.shared.clone() };
            let result = self.parts.xbox_sign_in(ctx, &profile, prompts, cancel.clone()).await;
            let after = self.after_sign_in(&result, SignInState::SignedOut, || self.parts.xbox_state(ctx, &profile));
            let who = self.parts.xbox_identity(ctx, &profile);
            self.shared.update(|s| {
                s.prompts.retain(|p| !matches!(p, AuthPrompt::DeviceCode { profile: id, .. } if *id == profile));
                s.xbox_states.insert(profile.clone(), after);
                s.xbox_ids.insert(profile.clone(), who);
            });
            match result {
                Ok(()) => {}
                Err(e) if matches!(e.kind, ErrorKind::Cancelled) => break,
                Err(e) => self.shared.notice(
                    None,
                    format!(
                        "Xbox Live wasn't linked: {e} Single-player and LAN still work; sign in again from the \
                         instance page to play online."
                    ),
                ),
            }
        }
        Ok(())
    }

    /// The sign-in state after a sign-in ended: what the part says now when it worked, the one before when it was
    /// cancelled, and the reason when it failed.
    fn after_sign_in(
        &self,
        result: &Result<()>,
        before: SignInState,
        now: impl FnOnce() -> SignInState,
    ) -> SignInState {
        match result {
            Ok(()) => match now() {
                SignInState::Unknown | SignInState::SignedOut | SignInState::SigningIn => {
                    SignInState::Linked { display_name: None }
                }
                other => other,
            },
            Err(e) if matches!(e.kind, ErrorKind::Cancelled) => match before {
                SignInState::SigningIn => SignInState::SignedOut,
                other => other,
            },
            Err(e) => SignInState::NeedsSignIn { reason: e.message.clone() },
        }
    }

    pub async fn sign_out(&self, target: AccountTarget, mode: SignOutMode) -> Result<()> {
        self.available()?;
        if self.support() == Support::Windows {
            return Err(BedrockError::new(
                ErrorKind::InvalidRequest,
                "On Windows, sign out in Minecraft or the Store.",
            ));
        }
        self.ensure_loaded().await?;
        let ctx = self.ctx();
        match &target {
            AccountTarget::Store(store) => {
                let _guard = self.begin(vec![JobKey::Store(store.0)])?;
                // idle only: no game of an instance signing in with this slot may be live
                let (live, profiles) = {
                    let s = self.shared.lock();
                    let live = s.instances.iter().any(|r| &r.store == store && s.live.contains_key(&r.id))
                        || s.instances.iter().any(|r| &r.store == store && s.jobs.contains(&JobKey::Instance(r.id.0)));
                    // the one Microsoft account signs out of everything it signed in to: each profile's Xbox
                    let mut profiles: Vec<ProfileId> = Vec::new();
                    for rec in s.instances.iter().filter(|r| &r.store == store) {
                        let out = matches!(s.xbox_states.get(&rec.profile), Some(SignInState::SignedOut));
                        if !out && !profiles.contains(&rec.profile) {
                            profiles.push(rec.profile.clone());
                        }
                    }
                    (live, profiles)
                };
                if live {
                    return Err(BedrockError::new(ErrorKind::Busy, "Close the game and wait for its jobs first."));
                }
                let _xbox_guard = self.begin(profiles.iter().map(|p| JobKey::Xbox(p.0)).collect())?;
                // Xbox first (each one's registry copy, then its account), and the Store user only when they all
                // went: a failure leaves the Store signed in, so what the window shows stays true
                for profile in &profiles {
                    self.parts.xbox_sign_out(&ctx, profile).await?;
                    self.shared.update(|s| {
                        s.xbox_states.insert(profile.clone(), SignInState::SignedOut);
                        s.xbox_ids.remove(profile);
                    });
                }
                self.parts.store_sign_out(&ctx, store, mode).await?;
                self.shared.update(|s| {
                    s.store_states.insert(store.clone(), SignInState::SignedOut);
                    s.store_ids.remove(store);
                });
            }
            AccountTarget::Xbox(profile) => {
                if matches!(mode, SignOutMode::UserAndDevice) {
                    return Err(BedrockError::new(
                        ErrorKind::InvalidRequest,
                        "Only a Store sign-in has a device to forget",
                    ));
                }
                let _guard = self.begin(vec![JobKey::Xbox(profile.0)])?;
                let live = {
                    let s = self.shared.lock();
                    s.instances.iter().any(|r| &r.profile == profile && s.live.contains_key(&r.id))
                };
                if live {
                    return Err(BedrockError::new(ErrorKind::Busy, "Close the game first."));
                }
                self.parts.xbox_sign_out(&ctx, profile).await?;
                self.shared.update(|s| {
                    s.xbox_states.insert(profile.clone(), SignInState::SignedOut);
                    s.xbox_ids.remove(profile);
                });
            }
        }
        Ok(())
    }

    pub async fn install(&self, id: InstanceId, progress: Arc<Progress>, cancel: Cancel) -> Result<BuildId> {
        self.available()?;
        if self.support() == Support::Windows {
            return Err(BedrockError::new(
                ErrorKind::InvalidRequest,
                "On Windows, Minecraft is installed from the Microsoft Store. BlockForge opens its page.",
            ));
        }
        self.ensure_loaded().await?;
        self.writable()?;
        let rec = self.record(&id)?;
        let _guard = self.begin(vec![JobKey::Instance(id.0), JobKey::Store(rec.store.0)])?;
        self.idle(&id)?;
        let _lock = self.lock_instance(&id).await?;
        let ctx = self.ctx();
        let build = self.parts.store_install(&ctx, &rec.store, &rec.selection, progress, cancel).await?;
        storage::check_build(&build)?;
        // the record is read again: a rename while downloading is kept
        let mut rec = self.record(&id)?;
        if rec.installed.as_ref().is_some_and(|b| b != &build) {
            rec.backup_before_play = true;
        }
        rec.installed = Some(build.clone());
        self.save(rec.clone()).await?;
        self.shared.update(|s| {
            s.repair.remove(&id);
            if let Some(r) = s.instances.iter_mut().find(|r| r.id == id) {
                *r = rec;
            }
            if let Some(l) = s.latest.get_mut(&build.edition).filter(|l| l.id == build) {
                l.installed = true;
            }
        });
        Ok(build)
    }

    pub async fn play(
        &self,
        id: InstanceId,
        options: PlayOptions,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Result<Session> {
        self.available()?;
        self.ensure_loaded().await?;
        let rec = self.record(&id)?;
        let guard = self.begin(vec![JobKey::Instance(id.0), JobKey::Game])?;
        match readiness(self.support(), &self.shared.lock(), &rec) {
            Readiness::NeedsInstall => {
                let text = if self.support() == Support::Windows {
                    "Minecraft for Windows isn't installed. Get it from the Microsoft Store first."
                } else {
                    "Install the game first."
                };
                return Err(BedrockError::new(ErrorKind::InvalidRequest, text));
            }
            Readiness::NeedsStoreSignIn => {
                return Err(BedrockError::new(
                    ErrorKind::SignInRequired,
                    "Sign in to the Microsoft Store account that owns the game first.",
                ));
            }
            Readiness::NeedsRepair { reason } => {
                return Err(BedrockError::new(ErrorKind::Integrity, format!("Repair the instance first: {reason}")));
            }
            Readiness::Starting(_) | Readiness::Running(_) | Readiness::Stopping(_) => {
                return Err(BedrockError::new(ErrorKind::Busy, "The game is already running."));
            }
            Readiness::UnsupportedPlatform => return Err(unavailable()),
            Readiness::Unknown | Readiness::Ready { .. } => {}
        }
        let build = rec.installed.clone();
        if self.support() == Support::Linux && build.is_none() {
            return Err(BedrockError::new(ErrorKind::InvalidRequest, "Install the game first."));
        }
        let lock = self.lock_instance(&id).await?;
        let session = SessionId(uuid::Uuid::new_v4());
        let log_dir = Paths::at(&self.root).session_logs(&session);
        self.shared.update(|s| {
            s.live.insert(id.clone(), Live { session: session.clone(), phase: LivePhase::Starting });
        });
        let launched = match self.support() {
            Support::Windows => self.launch_native(&rec).await,
            _ => {
                let request = LaunchRequest {
                    instance: rec.public(),
                    build: build.unwrap_or_else(|| BuildId {
                        edition: rec.edition(),
                        version: String::new(),
                        package_id: String::new(),
                    }),
                    session: session.clone(),
                    options,
                    log_dir: log_dir.clone(),
                    backup_first: rec.backup_before_play,
                };
                self.parts.launch(&self.ctx(), request, progress, cancel).await
            }
        };
        let launched = match launched {
            Ok(l) => l,
            Err(e) => {
                self.shared.update(|s| {
                    s.live.remove(&id);
                    // a damaged build, prefix or runtime is for Repair; a sign-in or the network is not
                    if matches!(e.kind, ErrorKind::Integrity | ErrorKind::Prefix | ErrorKind::Runtime) {
                        s.repair.insert(id.clone(), e.message.clone());
                    }
                });
                return Err(e);
            }
        };
        if rec.backup_before_play {
            let mut r = self.record(&id)?;
            r.backup_before_play = false;
            let _ = self.save(r.clone()).await;
            self.replace(r);
        }
        let control = SessionControl {
            inner: Arc::new(Control {
                shared: Arc::downgrade(&self.shared),
                instance: id.clone(),
                session: session.clone(),
                stop: launched.stop,
            }),
        };
        self.watch(id, session.clone(), launched.started, launched.exited, guard, lock);
        Ok(Session { id: session, running: launched.running, control, log_dir })
    }

    /// Follow a started game: Running once it is up, and when it is over the instance is free again, its last play
    /// is saved, and the window is told. The job keys and the lock are held until then.
    fn watch(
        &self,
        id: InstanceId,
        session: SessionId,
        started: Pin<Box<dyn Future<Output = ()> + Send>>,
        exited: Pin<Box<dyn Future<Output = Option<i32>> + Send>>,
        guard: Guard,
        lock: storage::FileLock,
    ) {
        let shared = self.shared.clone();
        let paths = Paths::at(&self.root);
        self.handle.spawn(async move {
            let held = (guard, lock);
            let mut exited = exited;
            let code = tokio::select! {
                _ = started => {
                    shared.update(|s| {
                        if let Some(l) = s.live.get_mut(&id).filter(|l| l.phase == LivePhase::Starting) {
                            l.phase = LivePhase::Running;
                        }
                    });
                    shared.send(Event::SessionStarted(session.clone()));
                    exited.await
                }
                code = &mut exited => code,
            };
            let rec = shared.update(|s| {
                s.live.remove(&id);
                let rec = s.instances.iter_mut().find(|r| r.id == id)?;
                rec.last_played = Some(storage::now());
                Some(rec.clone())
            });
            if let Some(rec) = rec {
                let _ = tokio::task::spawn_blocking(move || storage::save(&paths, &rec)).await;
            }
            // free before it is said, so a Play answering the event isn't refused
            drop(held);
            shared.send(Event::SessionExited { session, code });
        });
    }

    /// Windows: start Minecraft for Windows by activating its package.
    async fn launch_native(&self, rec: &InstanceRecord) -> Result<Launched> {
        #[cfg(windows)]
        {
            let edition = rec.edition();
            let Some(pkg) = self.shared.lock().native.get(&edition).cloned() else {
                return Err(BedrockError::new(ErrorKind::InvalidRequest, "Minecraft for Windows isn't installed"));
            };
            let install = pkg.path.clone();
            let pid = self.blocking(move || super::native::activate(edition, &install)).await?;
            let install = pkg.path.clone();
            let handle = self.handle.clone();
            let exited = Box::pin(async move {
                handle.spawn_blocking(move || super::native::wait(pid, &install)).await.ok().flatten()
            });
            Ok(Launched {
                running: None,
                stop: Arc::new(NativeStop { install: pkg.path, handle: self.handle.clone() }),
                started: Box::pin(async {}),
                exited,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = rec;
            Err(BedrockError::not_implemented())
        }
    }

    pub async fn repair(&self, id: InstanceId, progress: Arc<Progress>, cancel: Cancel) -> Result<()> {
        self.available()?;
        if self.support() == Support::Windows {
            return Err(BedrockError::new(
                ErrorKind::InvalidRequest,
                "On Windows, repair Minecraft in Settings, Apps, Installed apps, Minecraft for Windows, Advanced \
                 options.",
            ));
        }
        self.ensure_loaded().await?;
        let rec = self.record(&id)?;
        let _guard = self.begin(vec![JobKey::Instance(id.0)])?;
        self.idle(&id)?;
        let _lock = self.lock_instance(&id).await?;
        let fetched = self.parts.repair(&self.ctx(), &rec.public(), progress, cancel).await?;
        if let Some(build) = fetched {
            // the Store only had another build to fetch: the instance plays that one now
            storage::check_build(&build)?;
            let mut rec = self.record(&id)?;
            if rec.installed.as_ref().is_some_and(|b| b != &build) {
                rec.backup_before_play = true;
            }
            rec.installed = Some(build);
            self.save(rec.clone()).await?;
            self.replace(rec);
        }
        self.shared.update(|s| {
            s.repair.remove(&id);
        });
        Ok(())
    }

    pub async fn diagnose(&self, id: InstanceId) -> Result<DiagnosticReport> {
        self.available()?;
        self.ensure_loaded().await?;
        let rec = self.record(&id)?;
        let log_dir = Paths::at(&self.root).logs();
        let issues = match self.support() {
            Support::Windows => {
                let users = super::native::users_folder(rec.edition());
                let found = self.shared.lock().native.get(&rec.edition()).cloned();
                self.blocking(move || {
                    let mut issues = Vec::new();
                    match found {
                        None => issues.push(
                            "Minecraft for Windows isn't installed for this Windows user. Get it from the Microsoft \
                             Store."
                                .to_string(),
                        ),
                        Some(p) if !p.path.is_dir() => issues.push(format!(
                            "Minecraft {} is registered, but its folder {} isn't there. Repair it in Settings, Apps.",
                            p.version,
                            p.path.display()
                        )),
                        Some(_) => {}
                    }
                    if users.as_ref().is_none_or(|u| !u.is_dir()) {
                        issues.push("No worlds folder yet: it is made the first time the game starts.".into());
                    }
                    Ok(issues)
                })
                .await?
            }
            _ => self.parts.diagnose(&self.ctx(), &rec.public()).await?,
        };
        Ok(DiagnosticReport { issues, log_dir })
    }

    /// Where the instance's worlds are: the game's own folder on Windows, the profile's prefix on Linux. Computed, not
    /// looked at.
    pub fn worlds_folder(&self, id: &InstanceId) -> Option<PathBuf> {
        let rec = self.record(id).ok()?;
        match self.support() {
            Support::Windows => super::native::users_folder(rec.edition()),
            Support::Linux => Some(prefix_users(&Paths::at(&self.root), &rec.profile, rec.edition())),
            Support::Unavailable => None,
        }
    }

    /// Copy the instance's worlds (every player's, and Shared) into the profile's backups, while the game is closed.
    pub async fn back_up_worlds(&self, id: InstanceId) -> Result<PathBuf> {
        self.available()?;
        let rec = self.record(&id)?;
        let _guard = self.begin(vec![JobKey::Instance(id.0)])?;
        self.idle(&id)?;
        let from = self
            .worlds_folder(&id)
            .ok_or_else(|| BedrockError::new(ErrorKind::InvalidRequest, "This instance has no worlds folder"))?;
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let to = Paths::at(&self.root).world_backups(&rec.profile, rec.edition()).join(stamp);
        #[cfg(windows)]
        let native = self.shared.lock().native.get(&rec.edition()).cloned();
        self.blocking(move || {
            #[cfg(windows)]
            {
                if native.is_some_and(|p| super::native::running(&p.path)) {
                    return Err(BedrockError::new(ErrorKind::Busy, "Close Minecraft first, so its worlds are saved."));
                }
            }
            if !from.is_dir() {
                return Err(BedrockError::new(
                    ErrorKind::InvalidRequest,
                    "There are no worlds yet: they are made when the game first starts.",
                ));
            }
            match storage::copy_tree(&from, &to) {
                Ok(_) => Ok(to),
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&to);
                    Err(e)
                }
            }
        })
        .await
    }
}

/// A game's control, as [`SessionControl`] holds it.
pub(super) struct Control {
    shared: Weak<Shared>,
    instance: InstanceId,
    session: SessionId,
    stop: Arc<dyn StopGame>,
}

impl Control {
    pub async fn stop(&self, mode: StopMode) -> Result<()> {
        let mark = |phase: LivePhase| {
            if let Some(shared) = self.shared.upgrade() {
                shared.update(|s| {
                    if let Some(l) = s.live.get_mut(&self.instance).filter(|l| l.session == self.session) {
                        l.phase = phase;
                    }
                });
            }
        };
        mark(LivePhase::Stopping);
        let result = self.stop.stop(mode).await;
        if result.is_err() {
            // still running: Stop again (or Force) is offered
            mark(LivePhase::Running);
        }
        result
    }
}

#[cfg(windows)]
struct NativeStop {
    install: PathBuf,
    handle: tokio::runtime::Handle,
}

#[cfg(windows)]
impl StopGame for NativeStop {
    fn stop(&self, mode: StopMode) -> Job<'_, ()> {
        let install = self.install.clone();
        let handle = self.handle.clone();
        Box::pin(async move {
            handle.spawn_blocking(move || super::native::stop(&install, mode)).await.map_err(|e| {
                BedrockError::new(ErrorKind::Internal, format!("Stopping the game stopped unexpectedly: {e}"))
            })?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    /// Parts that succeed (or fail as told), counting what was asked of them.
    #[derive(Default)]
    struct Fake {
        launches: AtomicUsize,
        installs: AtomicUsize,
        store_outs: Mutex<Vec<SignOutMode>>,
        fail_launch: Mutex<Option<ErrorKind>>,
        /// The ends of the started and exited futures of the last launch.
        ends: Mutex<Option<(oneshot::Sender<()>, oneshot::Sender<Option<i32>>)>>,
        stop_calls: AtomicUsize,
        /// The sign-in window gives no Xbox code (so the device code follows).
        window_refuses: std::sync::atomic::AtomicBool,
        /// The device code is entered at once (otherwise it waits for a cancel).
        device_code_ok: std::sync::atomic::AtomicBool,
        xbox_linked: Mutex<HashSet<ProfileId>>,
        /// What was asked of the sign-ins, in order.
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    struct FakeStop(Arc<Fake>);

    impl StopGame for FakeStop {
        fn stop(&self, _mode: StopMode) -> Job<'_, ()> {
            self.0.stop_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    struct FakeParts(Arc<Fake>);

    impl Parts for FakeParts {
        fn store_state(&self, _: &Ctx, _: &StoreId) -> SignInState {
            SignInState::Linked { display_name: Some("Owner".into()) }
        }
        fn xbox_state(&self, _: &Ctx, p: &ProfileId) -> SignInState {
            if self.0.xbox_linked.lock().unwrap().contains(p) {
                SignInState::Linked { display_name: Some("FakeTag".into()) }
            } else {
                SignInState::SignedOut
            }
        }
        fn store_identity(&self, _: &Ctx, _: &StoreId) -> AccountIdentity {
            AccountIdentity { email: Some("owner@outlook.com".into()), ..Default::default() }
        }
        fn xbox_identity(&self, _: &Ctx, p: &ProfileId) -> AccountIdentity {
            if !self.0.xbox_linked.lock().unwrap().contains(p) {
                return AccountIdentity::default();
            }
            AccountIdentity {
                gamertag: Some("FakeTag".into()),
                xuid: Some("2535400000000001".into()),
                ..Default::default()
            }
        }
        fn microsoft_sign_in<'a>(
            &'a self,
            _: &'a Ctx,
            store: Option<&'a StoreId>,
            profiles: &'a [ProfileId],
            hint: Option<&'a str>,
            _: Arc<Progress>,
            c: Cancel,
        ) -> Job<'a, Vec<bool>> {
            self.0.calls.lock().unwrap().push(format!(
                "window store={} xbox={} hint={hint:?}",
                store.is_some(),
                profiles.len()
            ));
            Box::pin(async move {
                if c.is_cancelled() {
                    return Err(BedrockError::new(ErrorKind::Cancelled, "cancelled"));
                }
                let links = !self.0.window_refuses.load(Ordering::SeqCst);
                if links {
                    self.0.xbox_linked.lock().unwrap().extend(profiles.iter().cloned());
                }
                Ok(vec![links; profiles.len()])
            })
        }
        fn store_sign_in<'a>(&'a self, _: &'a Ctx, _: &'a StoreId, _: Arc<Progress>, c: Cancel) -> Job<'a, ()> {
            Box::pin(async move {
                if c.is_cancelled() { Err(BedrockError::new(ErrorKind::Cancelled, "cancelled")) } else { Ok(()) }
            })
        }
        fn store_sign_out<'a>(&'a self, _: &'a Ctx, _: &'a StoreId, mode: SignOutMode) -> Job<'a, ()> {
            self.0.store_outs.lock().unwrap().push(mode);
            self.0.calls.lock().unwrap().push("store-out".into());
            Box::pin(async { Ok(()) })
        }
        fn store_latest<'a>(&'a self, _: &'a Ctx, _: &'a StoreId, e: Edition, _: Cancel) -> Job<'a, BuildInfo> {
            Box::pin(async move {
                Ok(BuildInfo {
                    id: BuildId { edition: e, version: "1.26.60.5".into(), package_id: "pkg".into() },
                    installed: false,
                    download_bytes: Some(2_090_000_000),
                })
            })
        }
        fn store_install<'a>(
            &'a self,
            _: &'a Ctx,
            _: &'a StoreId,
            sel: &'a BuildSelection,
            _: Arc<Progress>,
            _: Cancel,
        ) -> Job<'a, BuildId> {
            self.0.installs.fetch_add(1, Ordering::SeqCst);
            let edition = match sel {
                BuildSelection::Latest(e) => *e,
                BuildSelection::Installed(b) => b.edition,
            };
            Box::pin(async move { Ok(BuildId { edition, version: "1.26.60.5".into(), package_id: "pkg".into() }) })
        }
        fn xbox_sign_in<'a>(&'a self, _: &'a Ctx, p: &'a ProfileId, prompts: Prompts, c: Cancel) -> Job<'a, ()> {
            self.0.calls.lock().unwrap().push("device-code".into());
            Box::pin(async move {
                prompts.device_code(p, "https://www.microsoft.com/link", "ABCD-1234", 4_000_000_000);
                if self.0.device_code_ok.load(Ordering::SeqCst) {
                    self.0.xbox_linked.lock().unwrap().insert(p.clone());
                    return Ok(());
                }
                c.cancelled().await;
                Err(BedrockError::new(ErrorKind::Cancelled, "cancelled"))
            })
        }
        fn xbox_sign_out<'a>(&'a self, _: &'a Ctx, p: &'a ProfileId) -> Job<'a, ()> {
            self.0.xbox_linked.lock().unwrap().remove(p);
            self.0.calls.lock().unwrap().push(format!("xbox-out {}", p.0));
            Box::pin(async { Ok(()) })
        }
        fn launch<'a>(&'a self, _: &'a Ctx, _: LaunchRequest, _: Arc<Progress>, _: Cancel) -> Job<'a, Launched> {
            self.0.launches.fetch_add(1, Ordering::SeqCst);
            let fail = self.0.fail_launch.lock().unwrap().take();
            let fake = self.0.clone();
            Box::pin(async move {
                if let Some(kind) = fail {
                    return Err(BedrockError::new(kind, "the prefix is damaged"));
                }
                // the game takes a moment to start, so a second Play arrives while it is starting
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                let (up_tx, up_rx) = oneshot::channel();
                let (end_tx, end_rx) = oneshot::channel();
                *fake.ends.lock().unwrap() = Some((up_tx, end_tx));
                Ok(Launched {
                    running: None,
                    stop: Arc::new(FakeStop(fake.clone())),
                    started: Box::pin(async move {
                        let _ = up_rx.await;
                    }),
                    exited: Box::pin(async move { end_rx.await.ok().flatten() }),
                })
            })
        }
        fn repair<'a>(
            &'a self,
            _: &'a Ctx,
            _: &'a BedrockInstance,
            _: Arc<Progress>,
            _: Cancel,
        ) -> Job<'a, Option<BuildId>> {
            Box::pin(async { Ok(None) })
        }
        fn diagnose<'a>(&'a self, _: &'a Ctx, _: &'a BedrockInstance) -> Job<'a, Vec<String>> {
            Box::pin(async { Ok(vec!["Vulkan: no device".into()]) })
        }
    }

    fn service(tag: &str, fake: &Arc<Fake>) -> (Arc<Service>, PathBuf) {
        let root = crate::testutil::scratch(tag);
        let svc = Service::with_parts(
            root.join("bedrock"),
            tokio::runtime::Handle::current(),
            Support::Linux,
            Arc::new(FakeParts(fake.clone())),
        );
        (Arc::new(svc), root)
    }

    fn create(name: &str, store: Option<StoreId>) -> CreateRequest {
        CreateRequest { name: name.into(), selection: BuildSelection::Latest(Edition::Release), store }
    }

    fn options() -> PlayOptions {
        PlayOptions { gpu: crate::gpu::Choice::Auto, diagnostics: false, require_xbox_online: false }
    }

    fn readiness_of(svc: &Service, id: &InstanceId) -> Readiness {
        svc.snapshot().instances.iter().find(|i| &i.instance.id == id).unwrap().readiness.clone()
    }

    #[test]
    fn making_the_backend_reads_nothing_and_its_snapshot_is_empty() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let root = crate::testutil::scratch("bedrock-new");
        let dirs = Dirs::at(root.join("data"));
        let backend = super::super::Backend::new(&dirs, rt.handle().clone());
        let snap = backend.snapshot();
        assert!(snap.instances.is_empty() && snap.accounts.is_empty() && !snap.refresh_in_progress);
        assert!(!root.join("data/bedrock").exists(), "no folder is made until something is asked for");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn create_allocates_a_profile_and_shares_a_store_slot_when_asked() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-create", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        let b = svc.create_instance(create("Second", Some(a.store.clone()))).await.unwrap();
        let c = svc.create_instance(create("Third", None)).await.unwrap();
        assert_eq!(a.store, b.store);
        assert_ne!(a.store, c.store);
        assert!(a.profile != b.profile && b.profile != c.profile);
        let paths = Paths::at(root.join("bedrock"));
        assert!(paths.profile(&a.profile).is_dir() && paths.profile(&b.profile).is_dir());
        let snap = svc.snapshot();
        assert_eq!(snap.instances.len(), 3);
        // two Store slots and three Xbox profiles
        let stores = snap.accounts.iter().filter(|a| matches!(a.target, AccountTarget::Store(_))).count();
        assert_eq!((stores, snap.accounts.len()), (2, 5));
        // a slot that doesn't exist can't be reused
        assert!(svc.create_instance(create("x", Some(StoreId(uuid::Uuid::new_v4())))).await.is_err());
        // a second backend on the same folder reads them back
        let again = Service::with_parts(
            root.join("bedrock"),
            tokio::runtime::Handle::current(),
            Support::Linux,
            Arc::new(FakeParts(fake.clone())),
        );
        let snap = again.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap();
        assert_eq!(snap.instances.len(), 3);
        assert!(matches!(readiness_of(&again, &a.id), Readiness::NeedsInstall));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn play_runs_once_at_a_time_and_follows_the_game_to_its_end() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-play", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        let b = svc.create_instance(create("Second", Some(a.store.clone()))).await.unwrap();
        svc.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap();
        // no build yet: Play says so and starts nothing
        let e = svc.play(a.id.clone(), options(), Progress::new(), Cancel::default()).await.err().unwrap();
        assert!(matches!(e.kind, ErrorKind::InvalidRequest));
        svc.install(a.id.clone(), Progress::new(), Cancel::default()).await.unwrap();
        svc.install(b.id.clone(), Progress::new(), Cancel::default()).await.unwrap();
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::Ready { xbox_online_cached: false }));
        let mut events = svc.subscribe();
        // two Plays at once: one starts, the other is refused, and the other instance can't start a second game
        let (first, second) = tokio::join!(
            svc.play(a.id.clone(), options(), Progress::new(), Cancel::default()),
            svc.play(a.id.clone(), options(), Progress::new(), Cancel::default()),
        );
        let session = match (first, second) {
            (Ok(s), Err(e)) | (Err(e), Ok(s)) => {
                assert!(matches!(e.kind, ErrorKind::Busy));
                s
            }
            _ => panic!("exactly one Play starts"),
        };
        assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
        let other = svc.play(b.id.clone(), options(), Progress::new(), Cancel::default()).await.err().unwrap();
        assert!(matches!(other.kind, ErrorKind::Busy));
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::Starting(_)));
        // no change to an instance while its game is live
        assert!(svc.delete_instance(&a.id).await.is_err());
        assert!(svc.select_build(a.id.clone(), BuildSelection::Latest(Edition::Preview)).await.is_err());
        assert!(svc.sign_out(AccountTarget::Store(a.store.clone()), SignOutMode::User).await.is_err());
        let (up, end) = fake.ends.lock().unwrap().take().unwrap();
        up.send(()).unwrap();
        loop {
            if let Event::SessionStarted(id) = events.recv().await.unwrap() {
                assert_eq!(id, session.id);
                break;
            }
        }
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::Running(_)));
        session.control.stop(StopMode::Graceful).await.unwrap();
        assert_eq!(fake.stop_calls.load(Ordering::SeqCst), 1);
        end.send(Some(0)).unwrap();
        loop {
            if let Event::SessionExited { session: id, code } = events.recv().await.unwrap() {
                assert_eq!((id, code), (session.id.clone(), Some(0)));
                break;
            }
        }
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::Ready { .. }));
        // the game is over: the other instance can play now
        assert!(svc.play(b.id.clone(), options(), Progress::new(), Cancel::default()).await.is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_damaged_prefix_asks_for_repair_and_repair_clears_it() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-repair", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        svc.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap();
        svc.install(a.id.clone(), Progress::new(), Cancel::default()).await.unwrap();
        *fake.fail_launch.lock().unwrap() = Some(ErrorKind::Prefix);
        assert!(svc.play(a.id.clone(), options(), Progress::new(), Cancel::default()).await.is_err());
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::NeedsRepair { .. }));
        assert!(svc.play(a.id.clone(), options(), Progress::new(), Cancel::default()).await.is_err());
        svc.repair(a.id.clone(), Progress::new(), Cancel::default()).await.unwrap();
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::Ready { .. }));
        let report = svc.diagnose(a.id.clone()).await.unwrap();
        assert_eq!(report.issues, vec!["Vulkan: no device".to_string()]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn the_device_code_stays_in_the_snapshot_until_the_sign_in_ends() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-xbox", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        let mut events = svc.subscribe();
        let cancel = Cancel::default();
        let job = {
            let (svc, cancel, profile) = (svc.clone(), cancel.clone(), a.profile.clone());
            tokio::spawn(async move { svc.sign_in(AccountTarget::Xbox(profile), None, Progress::new(), cancel).await })
        };
        loop {
            if let Event::DeviceCode { user_code, .. } = events.recv().await.unwrap() {
                assert_eq!(user_code, "ABCD-1234");
                break;
            }
        }
        // a window that missed the event still finds the code
        let snap = svc.snapshot();
        assert!(
            matches!(&snap.auth_prompts[..], [AuthPrompt::DeviceCode { user_code, .. }] if user_code == "ABCD-1234")
        );
        // a second sign-in for the same profile is refused while the first is open
        let again = svc.sign_in(AccountTarget::Xbox(a.profile.clone()), None, Progress::new(), Cancel::default()).await;
        assert!(matches!(again.unwrap_err().kind, ErrorKind::Busy));
        cancel.cancel();
        let e = job.await.unwrap().unwrap_err();
        assert!(matches!(e.kind, ErrorKind::Cancelled));
        let snap = svc.snapshot();
        assert!(snap.auth_prompts.is_empty());
        let xbox = snap.accounts.iter().find(|s| matches!(&s.target, AccountTarget::Xbox(p) if *p == a.profile));
        assert!(matches!(xbox.unwrap().state, SignInState::SignedOut));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn store_sign_in_and_out_follow_the_slot() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-store", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        // a new slot starts signed out: Play and Install wait for the Store
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::NeedsStoreSignIn));
        svc.sign_in(AccountTarget::Store(a.store.clone()), None, Progress::new(), Cancel::default()).await.unwrap();
        let state = |svc: &Service| {
            svc.snapshot()
                .accounts
                .iter()
                .find(|s| matches!(&s.target, AccountTarget::Store(id) if *id == a.store))
                .map(|s| s.state.clone())
                .unwrap()
        };
        assert!(matches!(state(&svc), SignInState::Linked { display_name: Some(_) }));
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::NeedsInstall));
        // only a Store sign-in has a device to forget
        let e = svc.sign_out(AccountTarget::Xbox(a.profile.clone()), SignOutMode::UserAndDevice).await.unwrap_err();
        assert!(matches!(e.kind, ErrorKind::InvalidRequest));
        svc.sign_out(AccountTarget::Store(a.store.clone()), SignOutMode::User).await.unwrap();
        assert!(matches!(state(&svc), SignInState::SignedOut));
        assert!(matches!(fake.store_outs.lock().unwrap()[..], [SignOutMode::User]));
        let _ = std::fs::remove_dir_all(root);
    }

    fn account<'a>(snap: &'a Snapshot, target: &AccountTarget) -> &'a AccountStatus {
        snap.accounts
            .iter()
            .find(|s| match (&s.target, target) {
                (AccountTarget::Store(a), AccountTarget::Store(b)) => a == b,
                (AccountTarget::Xbox(a), AccountTarget::Xbox(b)) => a == b,
                _ => false,
            })
            .unwrap()
    }

    #[tokio::test]
    async fn one_sign_in_links_the_store_and_every_profile_on_its_slot() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-one", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        let b = svc.create_instance(create("Second", Some(a.store.clone()))).await.unwrap();
        let other = svc.create_instance(create("Other", None)).await.unwrap();
        let target = AccountTarget::Store(a.store.clone());
        svc.sign_in(target.clone(), Some("owner@outlook.com".into()), Progress::new(), Cancel::default())
            .await
            .unwrap();
        // one window, both profiles of the slot, the BlockForge account's name prefilled; no device code
        assert_eq!(fake.calls(), ["window store=true xbox=2 hint=Some(\"owner@outlook.com\")"]);
        let snap = svc.snapshot();
        assert!(snap.auth_prompts.is_empty());
        assert!(matches!(account(&snap, &target).state, SignInState::Linked { .. }));
        assert_eq!(account(&snap, &target).identity.email.as_deref(), Some("owner@outlook.com"));
        for p in [&a.profile, &b.profile] {
            let x = account(&snap, &AccountTarget::Xbox(p.clone()));
            assert!(matches!(&x.state, SignInState::Linked { display_name: Some(n) } if n == "FakeTag"));
            assert_eq!(x.identity.xuid.as_deref(), Some("2535400000000001"));
        }
        // the other slot's profile is left alone, and Play waits only for this one sign-in (then the download)
        assert!(matches!(account(&snap, &AccountTarget::Xbox(other.profile.clone())).state, SignInState::SignedOut));
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::NeedsInstall));
        assert!(matches!(readiness_of(&svc, &other.id), Readiness::NeedsStoreSignIn));

        // with the Store signed in, a profile without Xbox gets a window for Xbox only
        svc.sign_out(AccountTarget::Xbox(b.profile.clone()), SignOutMode::User).await.unwrap();
        svc.sign_in(target.clone(), Some("not an address".into()), Progress::new(), Cancel::default()).await.unwrap();
        assert_eq!(fake.calls().last().unwrap(), "window store=false xbox=1 hint=None");
        assert!(matches!(
            account(&svc.snapshot(), &AccountTarget::Xbox(b.profile.clone())).state,
            SignInState::Linked { .. }
        ));

        // signing the account out signs out each profile's Xbox first, then the Store user (the device stays)
        fake.calls.lock().unwrap().clear();
        svc.sign_out(target.clone(), SignOutMode::User).await.unwrap();
        let calls = fake.calls();
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert!(calls[..2].iter().all(|c| c.starts_with("xbox-out ")) && calls[2] == "store-out", "{calls:?}");
        let snap = svc.snapshot();
        for t in [target, AccountTarget::Xbox(a.profile.clone()), AccountTarget::Xbox(b.profile.clone())] {
            assert!(matches!(account(&snap, &t).state, SignInState::SignedOut));
            assert!(account(&snap, &t).identity.is_empty());
        }
        assert!(matches!(fake.store_outs.lock().unwrap()[..], [SignOutMode::User]));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn without_a_code_from_the_window_the_device_code_follows_and_nothing_fails() {
        let fake = Arc::new(Fake::default());
        fake.window_refuses.store(true, Ordering::SeqCst);
        fake.device_code_ok.store(true, Ordering::SeqCst);
        let (svc, root) = service("bedrock-fallback", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        let mut events = svc.subscribe();
        svc.sign_in(AccountTarget::Store(a.store.clone()), None, Progress::new(), Cancel::default()).await.unwrap();
        assert_eq!(fake.calls(), ["window store=true xbox=1 hint=None", "device-code"]);
        let mut saw_code = false;
        while let Ok(e) = events.try_recv() {
            saw_code |= matches!(e, Event::DeviceCode { ref user_code, .. } if user_code == "ABCD-1234");
            assert!(!matches!(e, Event::Notice { .. }), "a fallback that worked is no news");
        }
        assert!(saw_code);
        let snap = svc.snapshot();
        assert!(snap.auth_prompts.is_empty());
        assert!(matches!(account(&snap, &AccountTarget::Xbox(a.profile.clone())).state, SignInState::Linked { .. }));

        // the player cancels the device code: the Store sign-in still stands, Xbox stays signed out
        fake.device_code_ok.store(false, Ordering::SeqCst);
        let b = svc.create_instance(create("Second", Some(a.store.clone()))).await.unwrap();
        let cancel = Cancel::default();
        let job = {
            let (svc, cancel, store) = (svc.clone(), cancel.clone(), a.store.clone());
            tokio::spawn(async move { svc.sign_in(AccountTarget::Store(store), None, Progress::new(), cancel).await })
        };
        while !svc.snapshot().auth_prompts.iter().any(|p| matches!(p, AuthPrompt::DeviceCode { .. })) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        // installs aren't held up by a device code waiting for the player
        svc.install(a.id.clone(), Progress::new(), Cancel::default()).await.unwrap();
        cancel.cancel();
        job.await.unwrap().unwrap();
        let snap = svc.snapshot();
        assert!(snap.auth_prompts.is_empty());
        assert!(matches!(account(&snap, &AccountTarget::Store(a.store.clone())).state, SignInState::Linked { .. }));
        assert!(matches!(account(&snap, &AccountTarget::Xbox(b.profile.clone())).state, SignInState::SignedOut));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn identities_match_on_proof_and_differ_only_on_proof() {
        let id = |email: Option<&str>, xuid: Option<&str>, uhs: Option<&str>, gt: Option<&str>| AccountIdentity {
            email: email.map(Into::into),
            xuid: xuid.map(Into::into),
            user_hash: uhs.map(Into::into),
            gamertag: gt.map(Into::into),
        };
        let bedrock = id(Some("Owner@Outlook.com"), Some("1"), Some("h1"), Some("Steve"));
        assert_eq!(bedrock.same_person(&id(None, Some("1"), None, None)), Some(true));
        assert_eq!(
            bedrock.same_person(&id(Some("owner@outlook.com"), Some("2"), None, None)),
            Some(false),
            "XUID wins"
        );
        assert_eq!(bedrock.same_person(&id(Some("owner@outlook.com"), None, None, None)), Some(true));
        assert_eq!(bedrock.same_person(&id(Some("someone@else.com"), None, None, None)), Some(false));
        assert_eq!(bedrock.same_person(&id(Some("+15550100"), None, None, None)), None, "a phone and an address");
        assert_eq!(bedrock.same_person(&id(None, None, Some("h1"), None)), Some(true));
        assert_eq!(
            bedrock.same_person(&id(None, None, Some("h2"), Some("Alex"))),
            None,
            "hashes and tags only confirm"
        );
        assert_eq!(bedrock.same_person(&id(None, None, None, Some("steve"))), Some(true));
        assert_eq!(bedrock.same_person(&AccountIdentity::default()), None);
        let merged = id(Some("a@b.c"), None, None, None).merged(&id(Some("x@y.z"), Some("9"), None, Some("T")));
        assert_eq!(merged, id(Some("a@b.c"), Some("9"), None, Some("T")));
        assert_eq!(merged.name(), Some("T"));
    }

    #[tokio::test]
    async fn changing_the_build_backs_up_first_and_renames_and_deletes_are_kept() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-select", &fake);
        let a = svc.create_instance(create("First", None)).await.unwrap();
        svc.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap();
        svc.install(a.id.clone(), Progress::new(), Cancel::default()).await.unwrap();
        svc.select_build(a.id.clone(), BuildSelection::Latest(Edition::Preview)).await.unwrap();
        assert!(svc.record(&a.id).unwrap().backup_before_play);
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::NeedsInstall));
        let bad = BuildId { edition: Edition::Release, version: "../x".into(), package_id: "p".into() };
        assert!(svc.select_build(a.id.clone(), BuildSelection::Installed(bad)).await.is_err());
        svc.rename_instance(&a.id, "  Renamed ").await.unwrap();
        assert_eq!(svc.snapshot().instances[0].instance.name, "Renamed");
        svc.delete_instance(&a.id).await.unwrap();
        assert!(svc.snapshot().instances.is_empty());
        let again = Service::with_parts(
            root.join("bedrock"),
            tokio::runtime::Handle::current(),
            Support::Linux,
            Arc::new(FakeParts(fake.clone())),
        );
        assert!(again.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap().instances.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_network_refresh_lists_the_newest_build() {
        let fake = Arc::new(Fake::default());
        let (svc, root) = service("bedrock-latest", &fake);
        svc.create_instance(create("First", None)).await.unwrap();
        let snap = svc.refresh(RefreshPolicy::NetworkAllowed, Cancel::default()).await.unwrap();
        assert_eq!(snap.builds.len(), 1);
        assert_eq!(snap.builds[0].download_bytes, Some(2_090_000_000));
        assert!(!snap.builds[0].installed && !snap.refresh_in_progress);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(all(target_os = "linux", feature = "bedrock"))]
    #[tokio::test]
    async fn the_real_parts_ask_for_the_store_sign_in_before_anything_else() {
        let root = crate::testutil::scratch("bedrock-real-parts");
        let svc = Service::with_parts(
            root.join("bedrock"),
            tokio::runtime::Handle::current(),
            Support::Linux,
            Arc::new(crate::bedrock::linux::Linux),
        );
        let a = svc.create_instance(create("First", None)).await.unwrap();
        // a new slot is signed out: Install and Play say so, without the network
        let e = svc.install(a.id.clone(), Progress::new(), Cancel::default()).await.unwrap_err();
        assert!(matches!(e.kind, ErrorKind::SignInRequired), "{e}");
        let Err(e) = svc.play(a.id.clone(), options(), Progress::new(), Cancel::default()).await else {
            panic!("Play must ask for the Store sign-in")
        };
        assert!(matches!(e.kind, ErrorKind::SignInRequired), "{e}");
        assert!(matches!(readiness_of(&svc, &a.id), Readiness::NeedsStoreSignIn));
        // an Xbox sign-out of a profile that never signed in is quietly done
        svc.sign_out(AccountTarget::Xbox(a.profile.clone()), SignOutMode::User).await.unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn elsewhere_everything_says_it_is_not_available() {
        let root = crate::testutil::scratch("bedrock-mac");
        let svc = Service::with_parts(
            root.join("bedrock"),
            tokio::runtime::Handle::current(),
            Support::Unavailable,
            Arc::new(FakeParts(Arc::new(Fake::default()))),
        );
        let e = svc.create_instance(create("First", None)).await.unwrap_err();
        assert!(matches!(e.kind, ErrorKind::UnsupportedPlatform));
        assert!(!root.join("bedrock").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
