//! The packages of PORT-PLAN.md put together on Linux: [`Linux`] is what the facade (`service.rs`) calls through
//! `Parts`. Each step keeps the order and the locks its package asks for:
//!
//! Play: engine and runtime (B) → in parallel, the Xbox chain (D) and the prefix work: the build's lease and its
//! game-folder fixups (A, C), the compatibility setup (B), the RNG stub before wineboot (C), headless wineboot and
//! `wineserver -w` through the runtime (B), the rest of the prefix setup (C) and the saves (C) → the refresh token into
//! the stopped prefix (C, D) → a fresh content licence and the decrypted images in RAM (A) → the Wine plan (B) → the
//! session plan with the GPU choice and the image map (E) → the Xbox account checked once more (D) → the supervisor
//! (E), which takes over the leases, the staged images and the game.
//!
//! Everything here runs on the runtime's workers; file work goes to blocking workers.
use super::{
    AccountIdentity, BedrockError, BedrockInstance, BuildId, BuildInfo, BuildSelection, Cancel, Edition, ErrorKind,
    Event, InstanceId, ProfileId, Result, SignInState, SignOutMode, StopMode, StoreId, diagnostics,
    engine::{self, Check, Engine},
    fixups, gpu_safety, helpers, payloads, prefix, proton,
    runtime::{self, InnerPlan, Program, Runtime},
    saves,
    service::{Ctx, Job, LaunchRequest, Launched, Parts, Prompts, StopGame},
    storage::Paths,
    store,
    supervisor::{self, Control, FileMap, LaunchPlan, SessionEvent, StagingDir, lease::LeaseSet},
    xbox::{self, Xbox},
};
use crate::install::Progress;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// The game's executable, the image the supervisor waits for.
const GAME_EXE: &str = "Minecraft.Windows.exe";
/// Recorded in a build's receipt for the files the game-folder fixups changed; a new revision runs them again.
const FIXUPS_REVISION: &str = "bf-fixups-1";
/// Beside a build's `content/`: the fixups revision that build has had.
const FIXUPS_STAMP: &str = ".bf-fixups";
/// How long wineboot and `wineserver -w` may take: BedrockOnLinux allows 300 s for Wine, plus up to 1500 s for the
/// runtime's first setup.
const SETUP_TIMEOUT: Duration = Duration::from_secs(1800);
/// How long the supervisor may take to get from its start to starting the game (it waits for leases and checks the
/// graphics state first).
const START_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) struct Linux;

// ---- paths of one profile

struct Profile {
    id: ProfileId,
    dir: PathBuf,
}

impl Profile {
    fn of(paths: &Paths, id: &ProfileId) -> Profile {
        Profile { id: id.clone(), dir: paths.profile(id) }
    }
    fn compat(&self) -> PathBuf {
        self.dir.join("compat")
    }
    fn prefix(&self) -> PathBuf {
        self.compat().join("pfx")
    }
    /// The runtime's per-profile state (pressure-vessel's variable dir, GStreamer registry, XDG dirs).
    fn state(&self) -> PathBuf {
        self.compat().join("state")
    }
    /// The prefix's launch lock: setup, repair, sign-out and play of this prefix exclude each other. The supervisor
    /// takes the same one for the whole session.
    fn launch_lease(&self) -> PathBuf {
        self.dir.join("locks/launch.lease")
    }
    fn backups(&self) -> PathBuf {
        self.dir.join("backups")
    }
    /// The session the profile last played, for Diagnose.
    fn last_session(&self) -> PathBuf {
        self.dir.join("last-session")
    }
    fn saves(&self) -> saves::Saves {
        saves::Saves::new(self.prefix(), self.backups())
    }
}

fn err(kind: ErrorKind, message: impl Into<String>) -> BedrockError {
    BedrockError::new(kind, message)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| err(ErrorKind::Internal, format!("A Bedrock job stopped unexpectedly: {e}")))?
}

fn client() -> reqwest::Client {
    crate::http::client().clone()
}

/// The Store adapter's context. Only a sign-in needs the helper window; the rest work without it.
fn store_ctx(ctx: &Ctx, sign_in: bool) -> Result<store::Context> {
    let auth_helper = if sign_in { helpers::auth_helper()? } else { helpers::auth_helper().unwrap_or_default() };
    Ok(store::Context { root: ctx.paths.root().to_path_buf(), client: client(), auth_helper })
}

/// Take the prefix's launch lock now, or say a game or another job has it.
fn take_launch_lease(profile: &Profile) -> Result<supervisor::lease::Lease> {
    use supervisor::lease::{Kind, Lease, LeaseError};
    Lease::try_take(&profile.launch_lease(), Kind::Exclusive).map_err(|e| match e {
        LeaseError::Busy(_) => {
            err(ErrorKind::Busy, "This instance's game or another job is using its Wine prefix. Close the game first.")
        }
        other => err(ErrorKind::Internal, other.to_string()),
    })
}

/// One check or install of the engine and runtime at a time in this process; a second is told to wait (B).
static ASSETS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn engine_and_runtime(
    ctx: &Ctx,
    progress: &Arc<Progress>,
    cancel: &Cancel,
    check: Check,
) -> Result<(Engine, Runtime)> {
    let _one = ASSETS.try_lock().map_err(|_| {
        err(ErrorKind::Busy, "The Bedrock engine is being checked or installed by another job. Wait for it to finish.")
    })?;
    let client = client();
    let base = ctx.paths.root();
    let engine = engine::ensure(base, &client, progress, cancel, check).await?;
    let runtime = runtime::ensure(base, &client, progress, cancel, check).await?;
    Ok((engine, runtime))
}

/// What a Wine process in the container gets from the player's session: who and where they are, their display,
/// audio and session bus. Only names the runtime helper allows.
fn session_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in [
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "DISPLAY",
        "XAUTHORITY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "PULSE_SERVER",
        "PIPEWIRE_REMOTE",
    ] {
        if let Ok(v) = std::env::var(key)
            && !v.is_empty()
        {
            env.insert(key.to_string(), v);
        }
    }
    env
}

/// The plan for one Wine program of this profile in the runtime: wineboot and `wineserver -w` for setup (headless),
/// the game for Play.
#[allow(clippy::too_many_arguments)]
fn inner_plan(
    engine: &Engine,
    profile: &Profile,
    helper: &Path,
    plan_path: PathBuf,
    program: Program,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: PathBuf,
    mounts: Vec<PathBuf>,
) -> InnerPlan {
    InnerPlan {
        schema: 1,
        engine: engine.root().to_path_buf(),
        prefix: profile.prefix(),
        headless: program != Program::Wine,
        program,
        args,
        env,
        cwd,
        ntsync: false,
        nvapi: false,
        helper: helper.to_path_buf(),
        plan_path,
        state: profile.state(),
        mounts,
        app_id: profile.id.0.to_string(),
    }
}

/// Run a headless Wine setup program of this profile through the runtime and wait for it, its output appended to
/// `log`. The plan file is deleted when it ends. Returns whether it succeeded and the log's text (for the RNG-abort
/// check); an error is something that kept it from running at all.
#[allow(clippy::too_many_arguments)]
async fn run_setup(
    engine: &Engine,
    runtime: &Runtime,
    profile: &Profile,
    helper: &Path,
    program: Program,
    log: &Path,
    cancel: &Cancel,
) -> Result<(bool, String)> {
    let plan_dir = profile.compat().join("plans");
    let plan_path = plan_dir.join(format!("{}.json", uuid::Uuid::new_v4().simple()));
    let plan = inner_plan(
        engine,
        profile,
        helper,
        plan_path.clone(),
        program,
        vec![],
        session_env(),
        profile.compat(),
        vec![],
    );
    let (r, e, p, log_path) = (runtime.clone(), engine.clone(), plan.clone(), log.to_path_buf());
    let (spec, out) = blocking(move || {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(p.plan_path.parent().expect("plan folder"))
            .map_err(|e| err(ErrorKind::Prefix, format!("Couldn't make the setup plan folder: {e}")))?;
        runtime::write_plan(&r, &e, &p)?;
        let spec = runtime::command(&r, &e, &p)?;
        if let Some(dir) = log_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| err(ErrorKind::Prefix, format!("Couldn't open the setup log: {e}")))?;
        Ok((spec, out))
    })
    .await?;
    let start = std::time::Instant::now();
    let result = async {
        let mut cmd = tokio::process::Command::from(spec.command());
        let err_out = out.try_clone().map_err(|e| err(ErrorKind::Prefix, e.to_string()))?;
        cmd.stdin(std::process::Stdio::null()).stdout(out).stderr(err_out).kill_on_drop(true);
        let mut child =
            cmd.spawn().map_err(|e| err(ErrorKind::Runtime, format!("Couldn't start the Steam Linux Runtime: {e}")))?;
        let status = tokio::select! {
            s = tokio::time::timeout(SETUP_TIMEOUT, child.wait()) => match s {
                Ok(s) => s.map_err(|e| err(ErrorKind::Runtime, e.to_string()))?,
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(err(ErrorKind::Prefix, "Setting up the Wine prefix took too long and was stopped."));
                }
            },
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(err(ErrorKind::Cancelled, "Cancelled."));
            }
        };
        Ok(status)
    }
    .await;
    let _ = tokio::fs::remove_file(&plan_path).await;
    let status = result?;
    let text = diagnostics::read_edges(log, 256 * 1024).unwrap_or_default();
    crate::logs::info(format!("Bedrock: {program:?} ended with {status} after {:.1} s", start.elapsed().as_secs_f64()));
    Ok((status.success(), text))
}

fn setup_failed(program: Program, log: &Path) -> BedrockError {
    BedrockError {
        kind: ErrorKind::Prefix,
        message: format!(
            "Setting up the Wine prefix failed ({program:?} didn't finish). The setup log has the details."
        ),
        log_path: Some(log.to_path_buf()),
    }
}

/// The prefix of `profile`, ready for the game: Proton's compatibility setup, the RNG stub before wineboot,
/// wineboot and `wineserver -w` when the prefix is new or the engine changed, then the rest of the setup (system DLLs,
/// registry, GameInput from `game_dir`). The launch lease must be held. Returns notes for the player.
#[allow(clippy::too_many_arguments)]
async fn set_up_prefix(
    engine: &Engine,
    runtime: &Runtime,
    profile: &Profile,
    helper: &Path,
    sources: &fixups::DllSources,
    game_dir: &Path,
    log: &Path,
    progress: &Arc<Progress>,
    cancel: &Cancel,
) -> Result<Vec<String>> {
    progress.stage("Preparing the Wine prefix");
    let (e, pfx, stub) = (engine.clone(), profile.prefix(), payloads::cryptbase(sources));
    let builtin = engine.root().join("files/lib/wine/x86_64-windows/cryptbase.dll");
    let needs_boot = blocking(move || {
        prefix::require_prefix_idle(&pfx, "set up the Wine prefix")?;
        proton::prepare(&e, &pfx, false)?;
        // after Proton's pass, which may have put the engine's own cryptbase there: wineboot needs the real stub
        prefix::seed_bootstrap_cryptbase(&pfx, Some(&stub), &builtin)?;
        Ok(!prefix::prefix_ready(&pfx) || prefix::engine_is_stale(&pfx, e.revision()))
    })
    .await?;
    if needs_boot {
        progress.stage("Setting up Windows in the Wine prefix (the first time takes a minute or two)");
        let mut booted = run_setup(engine, runtime, profile, helper, Program::Wineboot, log, cancel).await?;
        // one reseed and retry when wineboot aborted for want of the RNG stub (C, B)
        if !booted.0 && prefix::log_shows_rng_abort(&booted.1) {
            run_setup(engine, runtime, profile, helper, Program::WineserverWait, log, cancel).await?;
            let (pfx, stub) = (profile.prefix(), payloads::cryptbase(sources));
            let builtin = engine.root().join("files/lib/wine/x86_64-windows/cryptbase.dll");
            blocking(move || Ok(prefix::seed_bootstrap_cryptbase(&pfx, Some(&stub), &builtin)?)).await?;
            booted = run_setup(engine, runtime, profile, helper, Program::Wineboot, log, cancel).await?;
        }
        // the prefix must be idle before its hives are edited offline, whatever wineboot did
        let (waited, _) = run_setup(engine, runtime, profile, helper, Program::WineserverWait, log, cancel).await?;
        if !booted.0 {
            return Err(setup_failed(Program::Wineboot, log));
        }
        if !waited {
            return Err(setup_failed(Program::WineserverWait, log));
        }
    }
    progress.stage("Finishing the Wine prefix");
    let inputs = prefix::SetupInputs {
        prefix: profile.prefix(),
        engine_root: engine.root().to_path_buf(),
        engine_rev: engine.revision().to_string(),
        game_dir: game_dir.to_path_buf(),
        registry: prefix::GameRegistry::default(),
        cryptbase: Some(payloads::cryptbase(sources)),
    };
    let c = cancel.clone();
    let report = blocking(move || prefix::finish_setup(&inputs, &move || c.is_cancelled())).await?;
    crate::logs::info(format!(
        "Bedrock: prefix set up ({} system DLL(s) refreshed, registry {}, GameInput {:?})",
        report.dlls_refreshed,
        if report.registry_changed { "updated" } else { "unchanged" },
        report.gameinput
    ));
    Ok(report.notes)
}

/// The game-folder fixups (C) on an idle build whose lease is held exclusively: the GDK DLLs, the OpenSSL XCurl set,
/// the CA bundle, the libHttpClient provider gate and the HBUI patches. The changed files are recorded in the build's
/// receipt (A), so its checks and updates keep them. Runs once per build generation and fixups revision.
async fn fix_build(build: &mut store::InstalledBuild, sources: &fixups::DllSources) -> Result<Vec<String>> {
    let content = build.content.clone();
    let stamp =
        content.parent().map(|p| p.join(FIXUPS_STAMP)).ok_or_else(|| err(ErrorKind::Integrity, "Build root"))?;
    let s = stamp.clone();
    if blocking(move || Ok(std::fs::read_to_string(&s).is_ok_and(|v| v.trim() == FIXUPS_REVISION))).await? {
        return Ok(Vec::new());
    }
    let src = sources.clone();
    let c = content.clone();
    let report = blocking(move || Ok(fixups::install_game_dlls(&c, &src)?)).await?;
    build
        .record_patches(FIXUPS_REVISION, |name| {
            let lower = name.to_ascii_lowercase();
            (!name.contains('/') && lower.ends_with(".dll")) || lower.starts_with("data/gui/dist/hbui/")
        })
        .await?;
    let mut notes = Vec::new();
    if let Some(fixups::GateStatus::AnchorNotFound | fixups::GateStatus::Misaligned) = report.lhc_gate {
        notes.push(
            "This Minecraft build's libHttpClient changed, so it couldn't be switched to the OpenSSL web client: \
             signing in inside the game may loop. Report the Minecraft version."
                .to_string(),
        );
    }
    if let Some(problem) = report.hbui.as_ref().and_then(|r| r.status().diagnostic().1) {
        notes.push(problem);
    }
    blocking(move || {
        prefix::write_atomic(&stamp, format!("{FIXUPS_REVISION}\n").as_bytes(), 0o600).map_err(BedrockError::from)
    })
    .await?;
    Ok(notes)
}

/// The patch recorded for the game's program when the build ships it in the clear and its stack is raised on disk.
const STACK_PATCH: &str = "bf-stack-reserve-16m";

/// A program the build ships in the clear runs from disk, so its stack is raised there, as BOL does for a plain
/// program (bol/fixups.py `bump_stack_reserve`), and the change is recorded in the build's receipt (A) so its checks
/// and updates keep it. An encrypted program gets the same in its staged copy instead. On every start: the edit reads
/// 1 KiB when it is already there, and the receipt (which hashes the program) is written only the first time.
async fn raise_plain_stack(build: &mut store::InstalledBuild) -> Result<Vec<String>> {
    if build.images_to_decrypt().await.iter().any(|n| n.eq_ignore_ascii_case(GAME_EXE)) {
        return Ok(Vec::new());
    }
    let path = build.content.join(GAME_EXE);
    let raised = blocking(move || Ok(fixups::raise_stack_reserve_path(&path, fixups::GAME_STACK_RESERVE))).await?;
    match raised {
        Err(e) => Ok(vec![format!("The game's stack couldn't be raised ({e}); it may crash in its settings.")]),
        Ok(None) if build.patch_revision(GAME_EXE) == Some(STACK_PATCH) => Ok(Vec::new()),
        Ok(old) => {
            if let Some(old) = old {
                crate::logs::info(format!(
                    "Bedrock: the game's stack raised from {} KiB to {} KiB",
                    old / 1024,
                    fixups::GAME_STACK_RESERVE / 1024
                ));
            }
            build.record_patch(GAME_EXE, STACK_PATCH).await?;
            Ok(Vec::new())
        }
    }
}

/// `Z:\abs\path\content\<relative with backslashes>`: `relative` under `content` as Wine's `Z:` drive names it.
fn dos_name(content: &Path, relative: &str) -> String {
    format!("Z:{}\\{}", content.to_string_lossy().replace('/', "\\"), relative.replace('/', "\\"))
}

/// `\??\Z:\abs\path\content\<relative with backslashes>`: the NT name Wine's loader asks for an image by.
fn nt_name(content: &Path, relative: &str) -> String {
    format!("\\??\\Z:{}\\{}", content.to_string_lossy().replace('/', "\\"), relative.replace('/', "\\"))
}

/// Whether the engine's wineserver can use ntsync and the kernel offers it (a few reads, cached per process).
fn ntsync(engine: &Engine) -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        Path::new("/dev/ntsync").exists() && diagnostics::engine_supports_ntsync(engine.root()) == Some(true)
    })
}

/// The supervisor's control, as the facade stops a game with it.
struct SupervisorStop(Control);

impl StopGame for SupervisorStop {
    fn stop(&self, mode: StopMode) -> Job<'_, ()> {
        Box::pin(async move { self.0.stop(mode).await.map(|_| ()) })
    }
}

/// The session's folder until its supervisor owns it: removed when the start goes no further (it has no owner record
/// yet, so the start-up sweep would never judge it).
struct SessionFolder(Option<PathBuf>);

impl Drop for SessionFolder {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// What lives as long as the session: the engine and runtime leases (the supervisor holds its own too), and what is
/// cleaned up after it.
struct Held {
    _engine: super::artifact::Lease,
    _runtime: super::artifact::Lease,
    wine_plan: PathBuf,
}

/// Follow a started session to its end: Ready (the game's window) starts the Xbox presence and resolves `started`;
/// the supervisor's notices reach the window; when it is over the presence is taken down, the Wine plan deleted, a
/// cut-off `options.txt` put back, and `exited` gets the game's code.
#[allow(clippy::too_many_arguments)]
async fn follow(
    control: Control,
    instance: InstanceId,
    ctx_events: tokio::sync::broadcast::Sender<Event>,
    xbox: Xbox,
    readiness: xbox::OnlineReadiness,
    saves: saves::Saves,
    edition: Edition,
    held: Held,
    started: tokio::sync::oneshot::Sender<()>,
    exited: tokio::sync::oneshot::Sender<Option<i32>>,
) {
    let notice = |text: String| {
        let _ = ctx_events.send(Event::Notice { instance: Some(instance.clone()), text });
    };
    let mut started = Some(started);
    let since = std::time::SystemTime::now();
    let mut presence: Option<xbox::PresenceSession> = None;
    let mut code: Option<Option<i32>> = None;
    if let Ok(mut events) = control.subscribe().await {
        while let Some(event) = events.recv().await {
            match event {
                SessionEvent::Ready => {
                    if let Some(tx) = started.take() {
                        let _ = tx.send(());
                    }
                    let p = xbox::start_presence(&xbox, true, &readiness).await;
                    if readiness.is_online() && !p.is_active() {
                        notice(xbox::PRESENCE_UNAVAILABLE.to_string());
                    }
                    presence = Some(p);
                }
                SessionEvent::Notice { text } => notice(text),
                SessionEvent::Exited { code: c, findings, prefix_idle, .. } => {
                    for f in findings {
                        notice(f.text);
                    }
                    if !prefix_idle {
                        notice("Some Wine processes of the game were still running after it closed.".into());
                    }
                    code = Some(c);
                    break;
                }
                SessionEvent::Failed { message, .. } => {
                    notice(message);
                    code = Some(None);
                    break;
                }
                _ => {}
            }
        }
    }
    // the event stream ended (or never came): wait for the supervisor itself, which may have died without a word
    loop {
        let c = control.clone();
        let alive = tokio::task::spawn_blocking(move || c.supervisor_alive()).await.unwrap_or(false);
        if !alive {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if code.is_none() {
        let c = control.clone();
        code = tokio::task::spawn_blocking(move || {
            c.state().and_then(|s| {
                s.events.iter().rev().find_map(|e| match e {
                    SessionEvent::Exited { code, .. } => Some(*code),
                    _ => None,
                })
            })
        })
        .await
        .ok()
        .flatten();
    }
    if let Some(p) = presence {
        // published but never accepted by Xbox Live: friends saw the player offline all along
        if p.is_active() && !p.announced() {
            notice(xbox::PRESENCE_UNAVAILABLE.to_string());
        }
        p.stop().await;
    }
    let plan = held.wine_plan.clone();
    let signed_in = readiness.is_online();
    let ran_signed_out = tokio::task::spawn_blocking(move || {
        let _ = std::fs::remove_file(&plan);
        saves.restore_truncated_options(None);
        signed_in && saves.ran_signed_out(edition, since)
    })
    .await
    .unwrap_or(false);
    if ran_signed_out {
        notice(
            "The game played signed out this time, so the worlds of your Xbox account didn't show (they are still \
             there). Sign in inside the game, or sign in to Xbox again from BlockForge."
                .into(),
        );
    }
    drop(held);
    let _ = exited.send(code.flatten());
}

/// Start-up, on a worker: what earlier runs' sessions left (their folders, their staged images), now that their
/// supervisors are gone. Sessions that still run are kept.
pub(super) fn clean_up_after_earlier_sessions(base: &Path) {
    let ram = supervisor::staging_system_root();
    let report = supervisor::cleanup_stale_sessions(base, ram.as_deref());
    if !report.removed.is_empty() {
        crate::logs::info(format!("Bedrock: removed {} folder(s) of earlier sessions", report.removed.len()));
    }
}

/// The game's graphics card and the variables that put it there, for the Wine plan (allowed names only) and the
/// supervisor's checks.
fn gpu_env(choice: &crate::gpu::Choice, gpus: &[crate::gpu::Gpu]) -> (gpu_safety::GameGpu, Vec<(String, String)>) {
    let game = gpu_safety::game_gpu(choice, gpus);
    // what the player exported themselves wins
    let players: Vec<(String, String)> = std::env::vars().filter(|(k, _)| bedrock_runtime::allowed_env(k)).collect();
    let mut env = Vec::new();
    game.apply(&mut env, &players);
    env.retain(|(k, _)| {
        let allowed = bedrock_runtime::allowed_env(k);
        if !allowed {
            crate::logs::warn(format!("Bedrock: the GPU variable {k} can't be passed into the game's container"));
        }
        allowed
    });
    (game, env)
}

impl Linux {
    async fn launch_inner(
        &self,
        ctx: &Ctx,
        req: LaunchRequest,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Result<Launched> {
        let base = ctx.paths.root().to_path_buf();
        let instance = req.instance.clone();
        let profile = Profile::of(&ctx.paths, &instance.profile);
        let session = req.session.clone();
        let log_dir = req.log_dir.clone();
        let setup_log = log_dir.join("setup.log");
        {
            let (dir, last, s) = (log_dir.clone(), profile.last_session(), session.0.to_string());
            blocking(move || {
                std::fs::create_dir_all(&dir).map_err(|e| err(ErrorKind::Internal, e.to_string()))?;
                let _ = prefix::write_atomic(&last, s.as_bytes(), 0o600);
                Ok(())
            })
            .await?;
        }

        // a Store slot that isn't signed in can't license the game: say so before downloads and minutes of prefix
        // work (A); the licence itself is only proven at staging
        let (root, store) = (base.clone(), instance.store.clone());
        match blocking(move || Ok(store::cached_state(&root, &store))).await? {
            SignInState::Linked { .. } => {}
            SignInState::NeedsSignIn { reason } => return Err(err(ErrorKind::SignInRequired, reason)),
            _ => {
                return Err(err(
                    ErrorKind::SignInRequired,
                    "Sign in to the Microsoft Store account that owns the game first.",
                ));
            }
        }

        // 1. engine, runtime (B), the game-folder payloads and the runtime helper
        let (engine, runtime) = engine_and_runtime(ctx, &progress, &cancel, Check::Quick).await?;
        let held_engine = engine.lease()?;
        let held_runtime = runtime.lease()?;
        let sources = payloads::ensure(&base, &client(), &progress, &cancel).await?;
        let b = base.clone();
        let helper = blocking(move || helpers::runtime_helper(&b)).await?;
        let sctx = store_ctx(ctx, false)?;
        let xbox_account = Xbox::for_profile(&profile.dir);

        // 2. the Xbox chain (D) alongside the prefix work (A, B, C)
        let prefix_work = async {
            let lease = blocking({
                let profile = Profile::of(&ctx.paths, &instance.profile);
                move || take_launch_lease(&profile)
            })
            .await?;
            progress.stage("Opening the installed game");
            let mut build = store::install(
                &sctx,
                instance.store.clone(),
                BuildSelection::Installed(req.build.clone()),
                progress.clone(),
                cancel.clone(),
            )
            .await?;
            let mut notes = fix_build(&mut build, &sources).await?;
            notes.extend(raise_plain_stack(&mut build).await?);
            notes.extend(
                set_up_prefix(
                    &engine,
                    &runtime,
                    &profile,
                    &helper,
                    &sources,
                    &build.content,
                    &setup_log,
                    &progress,
                    &cancel,
                )
                .await?,
            );
            progress.stage("Checking your worlds");
            let (saves, edition, version, backup_first) =
                (profile.saves(), req.build.edition, req.build.version.clone(), req.backup_first);
            let saved = blocking(move || {
                let policy = saves::LaunchPolicy { seed_preview_from_release: true, allow_downgrade: false };
                let mut out = saves.before_launch(edition, &version, policy)?;
                if backup_first && out.backed_up.is_none() {
                    out.backed_up =
                        saves.back_up(edition, "build-change", Some(&version), out.previous.as_deref(), &[])?;
                }
                saves.snapshot_options();
                Ok(out)
            })
            .await?;
            if saved.recovered_interrupted_swap {
                notes.push("An earlier restore of your worlds had been cut off; it was finished first.".into());
            }
            if let Some(path) = saved.backed_up {
                notes.push(format!("Your worlds were backed up before this build opens them: {}", path.display()));
            }
            Ok::<_, BedrockError>((lease, build, notes))
        };
        let (prepared, prefix_done) = tokio::join!(xbox::prepare(&xbox_account, &cancel), prefix_work);
        let (launch_lease, build, notes) = prefix_done?;
        for n in notes {
            ctx.notice(Some(&instance.id), n);
        }
        let prepared = match prepared {
            Ok(p) => Some(p),
            Err(e) if matches!(e.kind, ErrorKind::Cancelled | ErrorKind::SignInRequired) => return Err(e),
            Err(e) => {
                ctx.notice(Some(&instance.id), format!("Xbox Live couldn't be prepared ({e}); playing offline."));
                None
            }
        };
        if let Some(p) = &prepared {
            for stage in &p.stages {
                crate::logs::info(format!("Bedrock: Xbox {stage}"));
            }
        }
        let readiness = prepared
            .as_ref()
            .map(|p| p.readiness.clone())
            .unwrap_or_else(|| xbox::OnlineReadiness::Offline(xbox::Offline::not_linked()));

        // 3. the refresh token into the stopped prefix, which the launch lease still keeps stopped (C, D)
        let token = prepared.as_ref().and_then(|p| p.registry_refresh_token.clone());
        let pfx = profile.prefix();
        blocking(move || {
            match &token {
                Some(t) => prefix::set_refresh_token(&pfx, t.expose())?,
                None => prefix::remove_refresh_token(&pfx)?,
            };
            Ok(())
        })
        .await?;
        if let Some(text) = readiness.notice() {
            if req.options.require_xbox_online {
                return Err(err(ErrorKind::SignInRequired, text));
            }
            ctx.notice(Some(&instance.id), text);
        }

        // 4. the images the build keeps encrypted on disk, decrypted with a fresh licence into RAM, in the folder the
        // supervisor will own (A, E). Images the build ships in the clear (this one's own program, for one) run from
        // disk like BOL runs a plain program: no licence, no copy, no map.
        let mut map = FileMap::new();
        let mut staging = None;
        if !build.images_to_decrypt().await.is_empty() {
            progress.stage("Getting the game's licence and decrypting it");
            let ram = supervisor::staging_system_root()
                .ok_or_else(|| err(ErrorKind::Runtime, supervisor::StageError::NoRam.to_string()))?;
            let (need, sid) = (build.decrypted_bytes().await?, session.0.to_string());
            let dir = blocking(move || {
                StagingDir::create(&ram, &sid, need).map_err(|e| err(ErrorKind::NoSpace, e.to_string()))
            })
            .await?;
            let staged = store::stage_into(&sctx, instance.store.clone(), &build, dir.path(), cancel.clone()).await?;
            for image in &staged.images {
                if image.relative.eq_ignore_ascii_case(GAME_EXE) {
                    // a 16 MiB stack, as the plaintext path gets (C)
                    if let Err(e) = fixups::raise_stack_reserve(&image.file, fixups::GAME_STACK_RESERVE) {
                        ctx.notice(
                            Some(&instance.id),
                            format!("The game's stack couldn't be raised ({e}); it may crash in its settings."),
                        );
                    }
                }
                map.add(&image.path, &nt_name(&build.content, &image.relative))
                    .map_err(|e| err(ErrorKind::Integrity, e.to_string()))?;
            }
            staging = Some(dir);
        }
        let staging_path = staging.as_ref().map(|s| s.path().to_path_buf());

        // 5. the Wine plan (B): the session, the graphics card (E), the online payload (D) and the image map (A)
        progress.stage("Starting the game");
        let gpus = blocking(|| Ok(crate::gpu::detect())).await?;
        let (game_gpu, gpu_vars) = gpu_env(&req.options.gpu, &gpus);
        let mut env = session_env();
        for (k, v) in gpu_vars {
            env.insert(k, v);
        }
        let shader_env = gpu_safety::shader_cache_env(&base.join("cache"), engine.revision(), Some(&game_gpu));
        let shader_root = base.join("cache/graphics");
        for (k, v) in &shader_env {
            env.insert(k.clone(), v.clone());
        }
        if let Some((k, v)) = readiness.wine_env() {
            env.insert(k.to_string(), v);
        }
        if !map.is_empty() {
            env.insert("WINE_DLL_FILE_MAP".into(), map.value());
        }
        // X11 through XWayland by default; Vulkan's present-wait path misbehaves there under a Wayland session
        if env.contains_key("WAYLAND_DISPLAY") {
            env.insert("WINE_DISABLE_VULKAN_OPWR".into(), "1".into());
        }
        if req.options.diagnostics {
            env.insert("WINEDEBUG".into(), "+seh,+loaddll,err+all,warn+module".into());
            env.insert("VKD3D_DEBUG".into(), "warn".into());
        }
        let session_dir = base.join("sessions").join(session.0.to_string());
        let wine_plan = session_dir.join("wine-plan.json");
        // The program is named by its DOS path: Wine's launcher resolves that like any path (the file is there on
        // disk, encrypted or not) and, when the map names a staged image for it, the loader opens that instead (and
        // unlinks it). Handed the NT name itself, this engine's launcher couldn't find the program and passed it to
        // start.exe ("ShellExecuteEx failed: Invalid name").
        let exe = dos_name(&build.content, GAME_EXE);
        let mounts = staging_path.iter().cloned().chain([xbox_account.dir().to_path_buf(), shader_root.clone()]);
        let mut plan = inner_plan(
            &engine,
            &profile,
            &helper,
            wine_plan.clone(),
            Program::Wine,
            vec![exe],
            env,
            build.content.clone(),
            mounts.collect(),
        );
        plan.ntsync = blocking({
            let e = engine.clone();
            move || Ok(ntsync(&e))
        })
        .await?;
        let (r, e, p, dirs) = (runtime.clone(), engine.clone(), plan.clone(), shader_env.clone());
        let sd = session_dir.clone();
        let mut folder = SessionFolder(Some(session_dir.clone()));
        let spec = blocking(move || {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&sd)
                .map_err(|e| err(ErrorKind::Internal, format!("Couldn't make the session folder: {e}")))?;
            for (_, d) in &dirs {
                let _ = std::fs::create_dir_all(d);
            }
            runtime::write_plan(&r, &e, &p)?;
            runtime::command(&r, &e, &p)
        })
        .await?;
        // pressure-vessel gets exactly the outer environment B chose: everything else the supervisor inherits from
        // BlockForge is removed, as B's own command() clears it
        let mut env_remove = spec.env_removals.clone();
        env_remove.extend(
            std::env::vars_os().filter_map(|(k, _)| k.into_string().ok()).filter(|k| !spec.env.contains_key(k)),
        );
        env_remove.sort();
        env_remove.dedup();
        let launch = LaunchPlan {
            session: session.0.to_string(),
            session_dir: session_dir.clone(),
            log_dir: log_dir.clone(),
            bedrock_root: base.clone(),
            prefix: profile.prefix(),
            program: spec.program.clone(),
            args: spec.argv.clone(),
            cwd: spec.cwd.clone(),
            env: spec.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            env_remove,
            ready_image: GAME_EXE.into(),
            leases: LeaseSet {
                game: Some(base.join("locks/game.lease")),
                prefix: Some(profile.launch_lease()),
                assets: vec![
                    engine::dir(&base).join(format!(".{}.lock", engine.revision())),
                    runtime::dir(&base).join(format!(".{}.lock", runtime.revision())),
                    base.join("locks").join(format!("build-{}", req.build.package_id)),
                ],
            },
            staging_dir: None,
            engine_rev: engine.revision().to_string(),
            allow_unsafe_gpu: false,
            expected_gpu: game_gpu.name.clone(),
            timing: None,
        };

        // 6. the Xbox account is still the one the payload and the token belong to (D)
        if let Some(p) = &prepared
            && p.registry_refresh_token.is_some()
            && let Some(epoch) = &p.epoch
            && !xbox::epoch_is_current(&xbox_account, epoch).await?
        {
            return Err(err(
                ErrorKind::SignInRequired,
                "This instance was signed out of Xbox while the game was getting ready. Press Play again.",
            ));
        }

        // 7. hand everything to the supervisor (E): the staged folder, and (by letting go of ours) the leases
        let mut launch = launch;
        launch.staging_dir = staging.map(StagingDir::hand_over);
        drop(build);
        drop(launch_lease);
        let secrets: Vec<String> = prepared
            .as_ref()
            .and_then(|p| p.registry_refresh_token.as_ref())
            .map(|t| vec![t.expose().to_string()])
            .unwrap_or_default();
        // the session runs as `blockforge --bedrock-session`; BF_BEDROCK_SESSION_EXE names the program for a run
        // that isn't BlockForge itself (the real-world tests, which are the test program)
        let exe = match std::env::var_os("BF_BEDROCK_SESSION_EXE").filter(|v| !v.is_empty()) {
            Some(exe) => PathBuf::from(exe),
            None => std::env::current_exe()
                .map_err(|e| err(ErrorKind::Runtime, format!("Couldn't find the BlockForge program: {e}")))?,
        };
        let spawned = match supervisor::spawn(&exe, &launch, &secrets) {
            Ok(s) => s,
            Err(e) => {
                if let Some(dir) = &launch.staging_dir {
                    let _ = tokio::fs::remove_dir_all(dir).await;
                }
                return Err(e);
            }
        };
        // from here the supervisor owns the session's folder (and the sweep of dead sessions judges it)
        folder.0 = None;
        if let Err(e) = spawned.control.wait_started(START_TIMEOUT).await {
            let _ = tokio::fs::remove_file(&wine_plan).await;
            // a refusal of the graphics check is not damage for Repair to fix
            let graphics = spawned.control.state().is_some_and(|s| {
                s.events
                    .iter()
                    .any(|e| matches!(e, SessionEvent::Failed { kind: supervisor::FailureKind::GraphicsUnsafe, .. }))
            });
            if !graphics {
                return Err(e);
            }
            let b = base.clone();
            let can_acknowledge = blocking(move || {
                Ok(gpu_safety::acknowledgement_status(&gpu_safety::Paths::at(&b), &gpu_safety::SystemHost)
                    .can_acknowledge)
            })
            .await
            .unwrap_or(false);
            let hint = if can_acknowledge {
                " Once you've checked the graphics driver, Repair acknowledges the earlier incident."
            } else {
                ""
            };
            return Err(BedrockError { kind: ErrorKind::InvalidRequest, message: format!("{}{hint}", e.message), ..e });
        }
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (exited_tx, exited_rx) = tokio::sync::oneshot::channel();
        let held = Held { _engine: held_engine, _runtime: held_runtime, wine_plan };
        ctx.handle.spawn(follow(
            spawned.control.clone(),
            instance.id.clone(),
            ctx.events.clone(),
            xbox_account,
            readiness,
            profile.saves(),
            req.build.edition,
            held,
            started_tx,
            exited_tx,
        ));
        Ok(Launched {
            running: Some(spawned.running),
            stop: Arc::new(SupervisorStop(spawned.control)),
            started: Box::pin(async move {
                // a session that ends before its window appears never counts as started
                if started_rx.await.is_err() {
                    std::future::pending::<()>().await;
                }
            }),
            exited: Box::pin(async move { exited_rx.await.ok().flatten() }),
        })
    }

    async fn repair_inner(
        &self,
        ctx: &Ctx,
        instance: &BedrockInstance,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Result<Option<BuildId>> {
        let base = ctx.paths.root().to_path_buf();
        let profile = Profile::of(&ctx.paths, &instance.profile);
        let log = ctx.paths.logs().join(format!("repair-{}", uuid::Uuid::new_v4())).join("setup.log");
        // every file of the engine and runtime against their pins (B)
        let (engine, runtime) = engine_and_runtime(ctx, &progress, &cancel, Check::Full).await?;
        let _engine = engine.lease()?;
        let _runtime = runtime.lease()?;
        let sources = payloads::ensure(&base, &client(), &progress, &cancel).await?;
        let b = base.clone();
        let helper = blocking(move || helpers::runtime_helper(&b)).await?;
        let lease = {
            let profile = Profile::of(&ctx.paths, &instance.profile);
            blocking(move || take_launch_lease(&profile)).await?
        };
        // an earlier graphics incident that blocks every start: Repair is the player's word that it was looked into
        // (no game may run meanwhile: the single-game lease is held for it, and this prefix's)
        let (b, pfx) = (base.clone(), profile.prefix());
        let acknowledged = blocking(move || {
            use supervisor::lease::{Kind, Lease};
            let paths = gpu_safety::Paths::at(&b);
            let status = gpu_safety::acknowledgement_status(&paths, &gpu_safety::SystemHost);
            if !status.can_acknowledge {
                return Ok(None);
            }
            let _game = Lease::try_take(&b.join("locks/game.lease"), Kind::Exclusive)
                .map_err(|_| err(ErrorKind::Busy, "A Bedrock game is running. Close it first."))?;
            prefix::require_prefix_idle(&pfx, "acknowledge the graphics incident")?;
            gpu_safety::acknowledge(&paths, &gpu_safety::SystemHost).map(|s| Some(s.message))
        })
        .await?;
        if let Some(message) = acknowledged {
            ctx.notice(Some(&instance.id), format!("Acknowledged the earlier graphics incident: {message}"));
        }
        // the installed build: every file against its receipt (A); fetched again when damaged and the Store still
        // offers it
        let mut fetched = None;
        let mut build = match &instance.installed {
            Some(id) => {
                let sctx = store_ctx(ctx, false)?;
                progress.stage("Checking every file of the installed game");
                let build = store::install(
                    &sctx,
                    instance.store.clone(),
                    BuildSelection::Installed(id.clone()),
                    progress.clone(),
                    cancel.clone(),
                )
                .await?;
                let damaged = build.damaged_files(&cancel).await?;
                if damaged.is_empty() {
                    Some(build)
                } else {
                    drop(build);
                    let BuildSelection::Latest(edition) = instance.selection else {
                        return Err(err(
                            ErrorKind::Integrity,
                            format!(
                                "{} file(s) of Minecraft {} are damaged, and the Store only offers its newest build. \
                                 Choose the newest build and repair again; your worlds are kept.",
                                damaged.len(),
                                id.version
                            ),
                        ));
                    };
                    progress.stage(format!("Downloading {} damaged file(s) again", damaged.len()));
                    let again = store::install(
                        &sctx,
                        instance.store.clone(),
                        BuildSelection::Latest(edition),
                        progress.clone(),
                        cancel.clone(),
                    )
                    .await?;
                    if &again.id != id {
                        fetched = Some(again.id.clone());
                    }
                    clean_previous_generations(&again).await;
                    Some(again)
                }
            }
            None => None,
        };
        // the fixups again, from scratch
        if let Some(b) = build.as_mut() {
            let stamp = b.content.parent().map(|p| p.join(FIXUPS_STAMP));
            if let Some(stamp) = stamp {
                let _ = tokio::fs::remove_file(stamp).await;
            }
            for n in fix_build(b, &sources).await? {
                ctx.notice(Some(&instance.id), n);
            }
        }
        // Wine's part of the prefix rebuilt around the game's own folders (worlds, settings), then set up again (B, C)
        progress.stage("Rebuilding the Wine prefix (your worlds are kept)");
        let (pfx, compat) = (profile.prefix(), profile.compat());
        let kept = blocking(move || {
            proton::invalidate(&pfx)?;
            Ok(prefix::reset_prefix_keeping_game_data(&compat)?)
        })
        .await?;
        crate::logs::info(format!("Bedrock: prefix repair kept {} game folder(s)", kept.len()));
        let empty;
        let game_dir = match &build {
            Some(b) => b.content.clone(),
            None => {
                // no game yet: GameInput waits for the first Play
                empty = profile.compat().join("no-game");
                let e = empty.clone();
                blocking(move || std::fs::create_dir_all(&e).map_err(|e| err(ErrorKind::Prefix, e.to_string())))
                    .await?;
                empty.clone()
            }
        };
        let notes =
            set_up_prefix(&engine, &runtime, &profile, &helper, &sources, &game_dir, &log, &progress, &cancel).await?;
        if build.is_some() {
            for n in notes {
                ctx.notice(Some(&instance.id), n);
            }
        }
        drop(build);
        drop(lease);
        Ok(fetched)
    }

    async fn diagnose_inner(&self, ctx: &Ctx, instance: &BedrockInstance) -> Result<Vec<String>> {
        let base = ctx.paths.root().to_path_buf();
        let profile = Profile::of(&ctx.paths, &instance.profile);
        let logs = ctx.paths.logs();
        let build = instance.installed.as_ref().and_then(|b| ctx.paths.build(b).ok());
        // the questions behind "my friends can't see or join me", asked of Xbox Live when the account is linked
        let account = Xbox::for_profile(&profile.dir);
        let mut online = Vec::new();
        if matches!(xbox::cached_state(&account), SignInState::Linked { .. }) {
            if !xbox::online_cached(&account).await {
                online.push("Xbox: no valid online sign-in is cached; the next Play renews it.".to_string());
            } else {
                let social = xbox::social_snapshot(&account).await;
                match (social.error, social.state) {
                    (Some(e), _) => online.push(format!("Xbox Live couldn't be asked about your friends: {e}")),
                    (None, state) => crate::logs::info(format!(
                        "Bedrock: Xbox presence {}, {} friend(s), {} in a multiplayer session",
                        state.as_deref().unwrap_or("unknown"),
                        social.friends.unwrap_or(0),
                        social.in_session.unwrap_or(0)
                    )),
                }
            }
        }
        let mut issues = blocking(move || {
            let gpus = crate::gpu::detect();
            let engine = engine::open(&engine::dir(&base)).ok().flatten();
            let facts = diagnostics::HostFacts::probe(&base, engine.as_ref().map(|e| e.root()), &gpus);
            let paths = gpu_safety::Paths::at(&base);
            let pre = gpu_safety::check(&paths, &gpu_safety::SystemHost, &gpu_safety::Env::from_process(), false);
            let last = std::fs::read_to_string(profile.last_session())
                .ok()
                .and_then(|s| uuid::Uuid::parse_str(s.trim()).ok())
                .map(|id| logs.join(id.to_string()));
            let log_dir = last.clone().unwrap_or_else(|| logs.join("diagnostics"));
            let ctx = diagnostics::LogContext {
                expected_gpu: None,
                virtual_gpu: diagnostics::host_has_virtio_gpu(),
                env_overrides: Vec::new(),
            };
            let findings = last.as_deref().map(|d| diagnostics::diagnose_dir(d, &ctx)).unwrap_or_default();
            let report = diagnostics::build_report(&diagnostics::ReportInput {
                facts,
                graphics_refusal: pre.refusal.map(|r| r.into_error().message),
                graphics_notes: pre.notes,
                findings,
                log_dir: log_dir.clone(),
            });
            let _ = diagnostics::write_report(&log_dir, &report);
            let mut issues = report.into_public().issues;
            if engine.is_none() {
                issues.push("The Bedrock engine isn't installed yet: Play downloads and installs it.".into());
            }
            for (what, found) in
                [("Store sign-in", helpers::auth_helper()), ("runtime", helpers::runtime_helper(&base))]
            {
                if let Err(e) = found {
                    issues.push(format!("{what} helper: {e}"));
                }
            }
            if let Some(root) = build {
                let content = root.join("content");
                if let (_, Some(problem)) = fixups::hbui_inspect(&content).status().diagnostic() {
                    issues.push(problem);
                }
            }
            let ack = gpu_safety::acknowledgement_status(&paths, &gpu_safety::SystemHost);
            if ack.can_acknowledge {
                issues.push(format!("{} Repair acknowledges it once you've checked the graphics driver.", ack.message));
            }
            Ok(issues)
        })
        .await?;
        issues.extend(online);
        Ok(issues)
    }
}

/// Old generations of a build left by an update or a repair (`.previous-*` beside it). Only while this build's lease
/// is held, so no session can be using them.
async fn clean_previous_generations(build: &store::InstalledBuild) {
    let Some(version_dir) = build.content.parent().and_then(Path::parent).map(Path::to_path_buf) else { return };
    let _ = tokio::task::spawn_blocking(move || {
        for entry in std::fs::read_dir(&version_dir).into_iter().flatten().flatten() {
            let old = entry.file_name().to_string_lossy().starts_with(".previous-");
            if old && entry.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    })
    .await;
}

impl Parts for Linux {
    fn store_state(&self, ctx: &Ctx, store: &StoreId) -> SignInState {
        store::cached_state(ctx.paths.root(), store)
    }

    fn xbox_state(&self, ctx: &Ctx, profile: &ProfileId) -> SignInState {
        xbox::cached_state(&Xbox::for_profile(&ctx.paths.profile(profile)))
    }

    fn store_identity(&self, ctx: &Ctx, store: &StoreId) -> AccountIdentity {
        match store::cached_state(ctx.paths.root(), store) {
            SignInState::Linked { display_name: Some(name) } => {
                AccountIdentity { email: Some(name), ..Default::default() }
            }
            _ => AccountIdentity::default(),
        }
    }

    fn xbox_identity(&self, ctx: &Ctx, profile: &ProfileId) -> AccountIdentity {
        xbox::cached_identity(&Xbox::for_profile(&ctx.paths.profile(profile)))
    }

    fn store_sign_in<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: &'a StoreId,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, ()> {
        Box::pin(async move {
            let window = store::Window { login_hint: None, xbox_codes: 0 };
            store::sign_in_with(&store_ctx(ctx, true)?, Some(store.clone()), window, progress, cancel).await.map(|_| ())
        })
    }

    fn microsoft_sign_in<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: Option<&'a StoreId>,
        profiles: &'a [ProfileId],
        login_hint: Option<&'a str>,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, Vec<bool>> {
        Box::pin(async move {
            // one window: the Store's sign-in (A), then Xbox's authorisation in the same session, one code per profile
            let window = store::Window { login_hint: login_hint.map(str::to_string), xbox_codes: profiles.len() };
            let steps =
                store::sign_in_with(&store_ctx(ctx, true)?, store.cloned(), window, progress.clone(), cancel.clone())
                    .await?;
            let mut linked = vec![false; profiles.len()];
            for ((profile, step), linked) in profiles.iter().zip(steps).zip(linked.iter_mut()) {
                let account = Xbox::for_profile(&ctx.paths.profile(profile));
                // each code becomes the profile's refresh token exactly as a device code's would (D)
                *linked = match step {
                    store::XboxStep::Code(code) => {
                        progress.stage("Linking Xbox Live");
                        match xbox::sign_in_with_code(&account, &code, &cancel).await {
                            Ok(_) => true,
                            Err(e) => {
                                crate::logs::info(format!(
                                    "Bedrock: the sign-in window's Xbox code was not taken ({e}); the device code \
                                     follows"
                                ));
                                false
                            }
                        }
                    }
                    store::XboxStep::Unavailable(why) => {
                        crate::logs::info(format!(
                            "Bedrock: the sign-in window gave no Xbox code ({why}); the device code follows"
                        ));
                        false
                    }
                };
                if *linked {
                    // Xbox Live's chain now, so the window can tell who signed in and Play finds a fresh payload; an
                    // offline answer is no failure of the sign-in
                    progress.stage("Checking Xbox Live");
                    if let Err(e) = xbox::prepare(&account, &cancel).await {
                        crate::logs::info(format!("Bedrock: Xbox Live wasn't checked after the sign-in: {e}"));
                    }
                }
            }
            Ok(linked)
        })
    }

    fn store_sign_out<'a>(&'a self, ctx: &'a Ctx, store: &'a StoreId, mode: SignOutMode) -> Job<'a, ()> {
        // the facade has checked no game of this slot is starting, running or stopping (A)
        Box::pin(async move { store::sign_out(&store_ctx(ctx, false)?, store.clone(), mode).await })
    }

    fn store_latest<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: &'a StoreId,
        edition: Edition,
        cancel: Cancel,
    ) -> Job<'a, BuildInfo> {
        Box::pin(async move {
            let (id, bytes) = store::latest(&store_ctx(ctx, false)?, store.clone(), edition, cancel).await?;
            let installed = ctx.paths.build(&id).is_ok_and(|p| p.join("install.json").is_file());
            Ok(BuildInfo { id, installed, download_bytes: Some(bytes) })
        })
    }

    fn store_install<'a>(
        &'a self,
        ctx: &'a Ctx,
        store: &'a StoreId,
        selection: &'a BuildSelection,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, BuildId> {
        Box::pin(async move {
            let build =
                store::install(&store_ctx(ctx, false)?, store.clone(), selection.clone(), progress, cancel).await?;
            crate::logs::info(format!(
                "Bedrock: Minecraft {} installed ({} file(s) downloaded, the rest were already there)",
                build.id.version, build.downloaded_files
            ));
            clean_previous_generations(&build).await;
            Ok(build.id.clone())
        })
    }

    fn xbox_sign_in<'a>(
        &'a self,
        ctx: &'a Ctx,
        profile: &'a ProfileId,
        prompts: Prompts,
        cancel: Cancel,
    ) -> Job<'a, ()> {
        Box::pin(async move {
            let account = Xbox::for_profile(&ctx.paths.profile(profile));
            let p = profile.clone();
            xbox::sign_in_with(
                &account,
                move |code| prompts.device_code(&p, &code.url, &code.user_code, code.expires_at_unix),
                &cancel,
            )
            .await
            .map(|_| ())
        })
    }

    fn xbox_sign_out<'a>(&'a self, ctx: &'a Ctx, profile: &'a ProfileId) -> Job<'a, ()> {
        Box::pin(async move {
            // the copy in the prefix's registry goes first, while the prefix is held stopped (C), then the account (D)
            let p = Profile::of(&ctx.paths, profile);
            blocking(move || {
                let _lease = take_launch_lease(&p)?;
                prefix::remove_refresh_token(&p.prefix())?;
                Ok(())
            })
            .await?;
            xbox::sign_out(&Xbox::for_profile(&ctx.paths.profile(profile))).await
        })
    }

    fn launch<'a>(
        &'a self,
        ctx: &'a Ctx,
        request: LaunchRequest,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, Launched> {
        Box::pin(self.launch_inner(ctx, request, progress, cancel))
    }

    fn repair<'a>(
        &'a self,
        ctx: &'a Ctx,
        instance: &'a BedrockInstance,
        progress: Arc<Progress>,
        cancel: Cancel,
    ) -> Job<'a, Option<BuildId>> {
        Box::pin(self.repair_inner(ctx, instance, progress, cancel))
    }

    fn diagnose<'a>(&'a self, ctx: &'a Ctx, instance: &'a BedrockInstance) -> Job<'a, Vec<String>> {
        Box::pin(self.diagnose_inner(ctx, instance))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bedrock::artifact;

    #[test]
    fn images_are_named_the_way_the_loader_asks_for_them() {
        let content = Path::new("/home/p/.local/share/blockforge/bedrock/builds/release/1.26.60.5/x/content");
        assert_eq!(
            nt_name(content, "Minecraft.Windows.exe"),
            "\\??\\Z:\\home\\p\\.local\\share\\blockforge\\bedrock\\builds\\release\\1.26.60.5\\x\\content\\Minecraft.Windows.exe"
        );
        assert!(nt_name(content, "sub/dir/a.dll").ends_with("\\content\\sub\\dir\\a.dll"));
    }

    #[test]
    fn setup_plans_are_headless_and_play_is_not() {
        let base = Path::new("/owned/bedrock");
        let engine = Engine::synthetic(base.join("engine/wow64-archs-native21"));
        let profile = Profile::of(&Paths::at(base), &ProfileId(uuid::Uuid::nil()));
        let plan = |program| {
            inner_plan(
                &engine,
                &profile,
                Path::new("/owned/bedrock/helpers/x/bedrock-runtime"),
                base.join("sessions/s/wine-plan.json"),
                program,
                vec![],
                BTreeMap::from([("HOME".into(), "/home/p".into())]),
                profile.compat(),
                vec![],
            )
        };
        assert!(plan(Program::Wineboot).headless && plan(Program::WineserverWait).headless);
        let game = plan(Program::Wine);
        assert!(!game.headless);
        assert_eq!(game.app_id, "00000000-0000-0000-0000-000000000000");
        assert_eq!(game.prefix, base.join("profiles/00000000-0000-0000-0000-000000000000/compat/pfx"));
        bedrock_runtime::validate(&game).unwrap();
    }

    #[test]
    fn the_session_environment_holds_only_what_the_runtime_allows() {
        assert!(session_env().keys().all(|k| bedrock_runtime::allowed_env(k)));
        let (_, gpu) = gpu_env(&crate::gpu::Choice::Auto, &[]);
        assert!(gpu.iter().all(|(k, _)| bedrock_runtime::allowed_env(k)), "{gpu:?}");
    }

    /// The real check of the one Microsoft sign-in, which needs the player: the helper window (BF_BEDROCK_AUTH_HELPER)
    /// opens the Store's hosted sign-in, prefilled with BF_BEDROCK_LOGIN_HINT if set; after it, the same window goes
    /// to Xbox's authorisation page in the same session; the code is exchanged for the Xbox refresh token and the
    /// Xbox Live chain is run. Then, with the Store signed in already, an Xbox-only window (the "Link Xbox Live"
    /// button) is checked the same way. Prints redacted results only: which step gave a code or why not, signed-in
    /// states, whether the chain went online and a gamertag came. Everything is kept under BF_BEDROCK_ONE_LOGIN_DIR
    /// (by default `~/.cache/bf-bedrock-onelogin-real`), never in BlockForge's data folder. See
    /// src/bedrock/store/INTEGRATION.md, "One Microsoft sign-in".
    #[test]
    #[ignore = "needs a person to sign in with a real Microsoft account; needs BF_BEDROCK_AUTH_HELPER"]
    fn real_one_microsoft_sign_in() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let root = std::env::var_os("BF_BEDROCK_ONE_LOGIN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cache/bf-bedrock-onelogin-real"));
        let helper = PathBuf::from(std::env::var_os("BF_BEDROCK_AUTH_HELPER").expect("BF_BEDROCK_AUTH_HELPER"));
        let hint = std::env::var("BF_BEDROCK_LOGIN_HINT").ok().filter(|h| !h.is_empty());
        let ctx = store::Context { root: root.join("bedrock"), client: client(), auth_helper: helper };
        // fixed ids, so a rerun keeps the Store device registered the first time
        let id = StoreId(uuid::Uuid::from_u128(0x0e1091));
        let profile = root.join("bedrock/profiles/one-login");
        let rt = crate::testutil::runtime();
        rt.block_on(async {
            let account = Xbox::for_profile(&profile);
            xbox::sign_out(&account).await.expect("clear the scratch Xbox sign-in");
            let report = |label: &str, steps: Vec<store::XboxStep>| -> Option<xbox::Secret> {
                let mut code = None;
                for step in steps {
                    match step {
                        store::XboxStep::Code(c) => {
                            eprintln!("{label}: the window gave an Xbox code ({} chars)", c.expose().len());
                            code = Some(c);
                        }
                        store::XboxStep::Unavailable(why) => eprintln!("{label}: no Xbox code ({why})"),
                    }
                }
                code
            };
            let chain = |label: &'static str| {
                let account = account.clone();
                async move {
                    let p = xbox::prepare(&account, &Cancel::default()).await.expect("prepare");
                    match &p.readiness {
                        xbox::OnlineReadiness::Online { gamertag, .. } => {
                            eprintln!(
                                "{label}: ONLINE, gamertag {}",
                                if gamertag.is_some() { "present" } else { "missing" }
                            )
                        }
                        xbox::OnlineReadiness::Offline(o) => eprintln!("{label}: OFFLINE: {}", o.notice()),
                    }
                    let who = xbox::cached_identity(&account);
                    eprintln!("{label}: xuid {}", if who.xuid.is_some() { "present" } else { "missing" });
                }
            };

            eprintln!("\n==> Sign in in the window that opens (Store, then Xbox in the same window).\n");
            let window = store::Window { login_hint: hint.clone(), xbox_codes: 1 };
            let steps = store::sign_in_with(&ctx, Some(id.clone()), window, Progress::new(), Cancel::default())
                .await
                .expect("the Store part of the one sign-in");
            let store_state = store::cached_state(&ctx.root, &id);
            eprintln!(
                "store: {}",
                if matches!(store_state, SignInState::Linked { .. }) { "signed in" } else { "NOT signed in" }
            );
            match report("one window", steps) {
                Some(code) => {
                    let got = xbox::sign_in_with_code(&account, &code, &Cancel::default()).await;
                    eprintln!("one window: code exchange {}", if got.is_ok() { "accepted" } else { "REFUSED" });
                    if got.is_ok() {
                        chain("one window").await;
                    }
                }
                None => eprintln!("one window: BlockForge would fall back to the device code here"),
            }

            // the "Link Xbox Live" path: the Store is signed in, the window links Xbox only
            xbox::sign_out(&account).await.unwrap();
            eprintln!("\n==> A second window opens for Xbox only: sign in (or allow) there.\n");
            let window = store::Window { login_hint: hint, xbox_codes: 1 };
            let steps = store::sign_in_with(&ctx, None, window, Progress::new(), Cancel::default()).await.unwrap();
            if let Some(code) = report("xbox-only window", steps) {
                let got = xbox::sign_in_with_code(&account, &code, &Cancel::default()).await;
                eprintln!("xbox-only window: code exchange {}", if got.is_ok() { "accepted" } else { "REFUSED" });
                if got.is_ok() {
                    chain("xbox-only window").await;
                }
            }
        });
    }

    /// The real local check, without any sign-in, on a scratch data root (`BLOCKFORGE_DIR`, by default
    /// `~/.cache/bf-bedrock-integ`): a Bedrock instance made through the Backend API; the pinned engine and runtime
    /// installed by `ensure` from the research archives (hard-linked into `bedrock/cache/artifacts`, where `ensure`
    /// looks before downloading; the archives themselves are kept); the game-folder payloads; the prefix set up as
    /// Play does (Proton's pass, the RNG stub, headless wineboot and `wineserver -w` in the runtime, then the
    /// registry; GameInput skipped as there is no game); and the Play chain stopping at the Store with
    /// NeedsStoreSignIn / SignInRequired. Needs the static runtime helper in BF_BEDROCK_RUNTIME_HELPER.
    #[test]
    #[ignore = "real engine + SteamRT + wineboot on a scratch root, ~6 GiB; needs BF_BEDROCK_RUNTIME_HELPER"]
    fn real_play_chain_without_sign_in() {
        use crate::bedrock::{
            AccountTarget, Backend, CreateRequest, PlayOptions, Readiness, RefreshPolicy, SessionId, pins,
        };
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let root = std::env::var_os("BLOCKFORGE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cache/bf-bedrock-integ"));
        let dirs = crate::paths::Dirs::at(&root);
        let base = dirs.base.join("bedrock");
        let research = home.join(".cache/bedrock-research");
        let cache = base.join("cache/artifacts");
        std::fs::create_dir_all(&cache).unwrap();
        for (pin, archive) in
            [(pins::ENGINE, "engine-native21.tar.gz"), (pins::RUNTIME, "steamrt3-3.0.20261021.266025.tar.xz")]
        {
            let name = format!("{}.{}", pin.revision, if pin.xz { "tar.xz" } else { "tar.gz" });
            let installed = base.join(if pin.xz { "runtime/steamrt3" } else { "engine" }).join(pin.revision);
            if !installed.is_dir() && !cache.join(&name).exists() {
                std::fs::hard_link(research.join(archive), cache.join(&name)).unwrap();
            }
        }
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let started = std::time::Instant::now();
            let backend = Backend::new(&dirs, tokio::runtime::Handle::current());
            backend.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap();
            let snap = backend.snapshot();
            let made = match snap.instances.iter().find(|i| i.instance.name == "Integration check") {
                Some(i) => i.instance.clone(),
                None => backend
                    .create_instance(CreateRequest {
                        name: "Integration check".into(),
                        selection: BuildSelection::Latest(Edition::Release),
                        store: None,
                    })
                    .await
                    .unwrap(),
            };
            println!("instance {} profile {} store {}", made.id.0, made.profile.0, made.store.0);
            let options = PlayOptions { gpu: crate::gpu::Choice::Auto, diagnostics: false, require_xbox_online: false };

            // the facade: a signed-out Store slot asks for the sign-in, and Play says so
            let status = backend.snapshot().instances.iter().find(|i| i.instance.id == made.id).cloned().unwrap();
            println!("readiness: {:?}", status.readiness);
            assert!(matches!(status.readiness, Readiness::NeedsStoreSignIn));
            let Err(e) = backend.play(made.id.clone(), options.clone(), Progress::new(), Cancel::default()).await
            else {
                panic!("Play can't start without the Store")
            };
            println!("Backend::play: {:?}: {}", e.kind, e.message);
            assert!(matches!(e.kind, ErrorKind::SignInRequired));
            let e = backend.install(made.id.clone(), Progress::new(), Cancel::default()).await.unwrap_err();
            println!("Backend::install: {:?}: {}", e.kind, e.message);
            assert!(matches!(e.kind, ErrorKind::SignInRequired));

            // the parts, as Play runs them, up to the Store
            let (events, _) = tokio::sync::broadcast::channel(16);
            let ctx = Ctx { paths: Paths::at(&base), handle: tokio::runtime::Handle::current(), events };
            let progress = Progress::new();
            let cancel = Cancel::default();
            let t = std::time::Instant::now();
            let (engine, runtime) = engine_and_runtime(&ctx, &progress, &cancel, Check::Quick).await.unwrap();
            println!(
                "engine {} and runtime {} ready in {:.1} s",
                engine.revision(),
                runtime.revision(),
                t.elapsed().as_secs_f64()
            );
            let t = std::time::Instant::now();
            let sources = payloads::ensure(&base, &client(), &progress, &cancel).await.unwrap();
            println!("payloads ready in {:.1} s: {sources:?}", t.elapsed().as_secs_f64());
            let helper = helpers::runtime_helper(&base).unwrap();
            println!("runtime helper: {}", helper.display());
            let profile = Profile::of(&ctx.paths, &made.profile);
            let lease = take_launch_lease(&profile).unwrap();
            let no_game = profile.compat().join("no-game");
            std::fs::create_dir_all(&no_game).unwrap();
            let log = ctx.paths.logs().join("integration-check").join("setup.log");
            let t = std::time::Instant::now();
            let notes =
                set_up_prefix(&engine, &runtime, &profile, &helper, &sources, &no_game, &log, &progress, &cancel)
                    .await
                    .unwrap();
            println!("prefix set up in {:.1} s; notes: {notes:?}", t.elapsed().as_secs_f64());
            let pfx = profile.prefix();
            assert!(prefix::prefix_ready(&pfx));
            assert_eq!(prefix::read_engine_rev(&pfx).as_deref(), Some(engine.revision()));
            assert!(prefix::prefix_processes(&pfx).unwrap().is_empty(), "nothing of the prefix may still run");
            let system = std::fs::read_to_string(pfx.join("system.reg")).unwrap();
            assert!(system.contains("\"ConsoleMode\"=dword:00000008"), "the game's registry values are in");
            assert_eq!(
                artifact::sha256(&pfx.join("drive_c/windows/system32/cryptbase.dll")).unwrap(),
                artifact::sha256(&payloads::cryptbase(&sources)).unwrap(),
                "the native RNG stub is in system32"
            );
            // a second pass is warm: no wineboot
            let t = std::time::Instant::now();
            set_up_prefix(&engine, &runtime, &profile, &helper, &sources, &no_game, &log, &progress, &cancel)
                .await
                .unwrap();
            println!("warm prefix check in {:.2} s", t.elapsed().as_secs_f64());
            drop(lease);

            // Play's chain itself: it stops at the Store, cleanly
            let request = LaunchRequest {
                instance: made.clone(),
                build: BuildId {
                    edition: Edition::Release,
                    version: "1.0.0.0".into(),
                    package_id: uuid::Uuid::nil().to_string(),
                },
                session: SessionId(uuid::Uuid::new_v4()),
                options,
                log_dir: ctx.paths.logs().join("integration-check"),
                backup_first: false,
            };
            let Err(e) = Linux.launch(&ctx, request, Progress::new(), Cancel::default()).await else {
                panic!("the chain can't get past the Store")
            };
            println!("Play chain: {:?}: {}", e.kind, e.message);
            assert!(matches!(e.kind, ErrorKind::SignInRequired));
            let e = Linux.store_latest(&ctx, &made.store, Edition::Release, Cancel::default()).await.unwrap_err();
            println!("Store latest: {:?}: {}", e.kind, e.message);
            assert!(matches!(e.kind, ErrorKind::SignInRequired));
            let e = Linux
                .store_install(
                    &ctx,
                    &made.store,
                    &BuildSelection::Latest(Edition::Release),
                    Progress::new(),
                    Cancel::default(),
                )
                .await
                .unwrap_err();
            println!("Store install: {:?}: {}", e.kind, e.message);
            assert!(matches!(e.kind, ErrorKind::SignInRequired));
            // and the sign-outs of a never-signed-in instance are harmless
            backend
                .sign_out(AccountTarget::Xbox(made.profile.clone()), crate::bedrock::SignOutMode::User)
                .await
                .unwrap();
            let report = backend.diagnose(made.id.clone()).await.unwrap();
            println!("diagnose: {:#?}", report.issues);
            println!("whole check: {:.1} s", started.elapsed().as_secs_f64());
        });
    }

    #[test]
    fn a_held_prefix_refuses_a_second_job() {
        let root = crate::testutil::scratch("bedrock-launch-lease");
        let profile = Profile::of(&Paths::at(&root), &ProfileId(uuid::Uuid::new_v4()));
        let held = take_launch_lease(&profile).unwrap();
        assert!(matches!(take_launch_lease(&profile).unwrap_err().kind, ErrorKind::Busy));
        drop(held);
        take_launch_lease(&profile).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Play and Stop, for real, with the game installed in BLOCKFORGE_DIR (see `real_install_with_saved_sign_in`):
    /// the first instance is started, left running for BF_PLAY_SECONDS (default 60), then stopped as the Stop
    /// button does, and what is left behind is checked. Needs the built program and helpers:
    /// `BF_BEDROCK_SESSION_EXE=target/debug/blockforge BF_BEDROCK_RUNTIME_HELPER=target/debug/bedrock-runtime
    /// BLOCKFORGE_DIR=<root> cargo test --locked --lib real_play_and_stop -- --ignored --nocapture`
    #[test]
    #[ignore = "starts Minecraft for real from BLOCKFORGE_DIR; needs a display"]
    fn real_play_and_stop() {
        use crate::bedrock::{Backend, PlayOptions, Readiness, RefreshPolicy, StopMode};
        let Some(root) = std::env::var_os("BLOCKFORGE_DIR").map(PathBuf::from) else {
            eprintln!("skipped: set BLOCKFORGE_DIR to a data folder with Bedrock installed");
            return;
        };
        let seconds: u64 = std::env::var("BF_PLAY_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
        let gpu = crate::gpu::Choice::parse(&std::env::var("BF_GPU").unwrap_or_default());
        let dirs = crate::paths::Dirs::at(&root);
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let backend = Backend::new(&dirs, tokio::runtime::Handle::current());
            backend.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap();
            let status = backend.snapshot().instances.first().cloned().expect("a Bedrock instance");
            let id = status.instance.id.clone();
            println!("{}: {:?}", status.instance.name, status.readiness);
            let progress = Progress::new();
            let watch = {
                let p = progress.clone();
                tokio::spawn(async move {
                    let mut last = String::new();
                    loop {
                        let now = p.stage_text();
                        if now != last {
                            println!("  {now}");
                            last = now;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                })
            };
            let started = std::time::Instant::now();
            let diagnostics = std::env::var_os("BF_DIAG").is_some();
            let options = PlayOptions { gpu, diagnostics, require_xbox_online: false };
            let session = match backend.play(id.clone(), options, progress, Cancel::default()).await {
                Ok(s) => s,
                Err(e) => {
                    watch.abort();
                    panic!("play failed: {:?}: {}", e.kind, e.message);
                }
            };
            watch.abort();
            println!("started in {:.0} s; logs in {}", started.elapsed().as_secs_f32(), session.log_dir.display());
            let until = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
            while std::time::Instant::now() < until {
                let r = backend.snapshot().instances.iter().find(|i| i.instance.id == id).map(|i| i.readiness.clone());
                if !matches!(r, Some(Readiness::Starting(_) | Readiness::Running(_))) {
                    println!("the game ended by itself: {r:?}");
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            let stopping = std::time::Instant::now();
            let stopped = session.control.stop(StopMode::Graceful).await;
            println!(
                "stop: {:?} in {:.1} s",
                stopped.as_ref().map_err(|e| &e.message),
                stopping.elapsed().as_secs_f32()
            );
            stopped.unwrap();
        });
    }

    /// The download, for real, with a Store sign-in already saved in BLOCKFORGE_DIR (made in the window): every
    /// instance there is installed and the result printed. Run with the window closed (they share the locks):
    /// `BLOCKFORGE_DIR=<root> cargo test --locked --lib real_install_with_saved_sign_in -- --ignored --nocapture`
    #[test]
    #[ignore = "downloads Minecraft for real (~2 GB) with a sign-in saved in BLOCKFORGE_DIR"]
    fn real_install_with_saved_sign_in() {
        use crate::bedrock::{Backend, RefreshPolicy};
        let Some(root) = std::env::var_os("BLOCKFORGE_DIR").map(PathBuf::from) else {
            eprintln!("skipped: set BLOCKFORGE_DIR to a data folder with a Bedrock instance signed in to the Store");
            return;
        };
        let dirs = crate::paths::Dirs::at(&root);
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let backend = Backend::new(&dirs, tokio::runtime::Handle::current());
            backend.refresh(RefreshPolicy::LocalOnly, Cancel::default()).await.unwrap();
            for status in backend.snapshot().instances.iter() {
                let id = status.instance.id.clone();
                println!("{}: {:?}", status.instance.name, status.readiness);
                let progress = Progress::new();
                let watch = {
                    let p = progress.clone();
                    tokio::spawn(async move {
                        let mut last = String::new();
                        loop {
                            let now = format!("{} {:.0}%", p.stage_text(), p.fraction() * 100.0);
                            if now != last {
                                println!("  {now}");
                                last = now;
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }
                    })
                };
                let started = std::time::Instant::now();
                let result = backend.install(id, progress, Cancel::default()).await;
                watch.abort();
                match result {
                    Ok(build) => println!("installed {build:?} in {:.0} s", started.elapsed().as_secs_f32()),
                    Err(e) => println!("install failed: {:?}: {}", e.kind, e.message),
                }
            }
        });
    }
}
