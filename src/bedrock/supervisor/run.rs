//! The session itself: what the supervisor process does from the moment it starts until the last Wine process of
//! the prefix is gone, and how the window starts it.
//!
//! Order of a session (each step can end it with a `Failed` event, and nothing after a failed step is done):
//!
//! 1. read and check the plan; start the control socket; take over the staging folder
//! 2. take the leases (one game at a time, the prefix, the shared engine and build)
//! 3. refuse if Wine processes of an earlier session still use the prefix; retire the crash marker of an earlier
//!    session that is provably over
//! 4. check the graphics session without opening a GPU; arm the crash marker
//! 5. start the wrapper in a group of its own; watch the **prefix**, not the wrapper, to know the game is up
//! 6. wait for the wrapper to return, or for a stop (a request on the socket, or SIGTERM/SIGHUP to us)
//! 7. teardown, always: record that the wrapper returned, wait for the prefix to be idle (ending leftovers politely,
//!    then by force), reap, clear the marker only if the prefix is idle, empty the staging folder, read the logs for
//!    causes, say `Exited`
//!
//! Ported from BedrockOnLinux `bol/launch.py` (`_launch_once`), `bol/prefix.py` and `bol/supervision.py`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};

use super::ipc::{self, Control, Incoming, Reply, Server, StopKind};
use super::lease::Held;
use super::plan::{EVENT_PREFIX, FailureKind, LaunchPlan, SESSION_ARG, SessionEvent, Stage, State, StopOutcome};
use super::procs::{ProcId, ProcTable, RealSignals, Reaper, Scope, Signals, Stage as StopStage, StopReport, Timing};
use super::redact::Redactor;
use super::staging::{self, Owner};
use super::sys;
use crate::bedrock::diagnostics::{self, Finding, LogContext};
use crate::bedrock::gpu_safety::{self, Armed, Env, Host, SystemHost};
use crate::bedrock::{BedrockError, ErrorKind};

/// The longest a game log may grow; a game stuck printing is cut off with a note instead of filling the disk.
const MAX_LOG_BYTES: u64 = 256 * 1024 * 1024;
/// Lines waiting to go to the window; the window not reading drops the oldest lines of *its* copy, never the file's.
const WINDOW_QUEUE: usize = 2048;

static TERM_SIGNAL: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_term(sig: i32) {
    // the first signal counts; a second (systemd sends HUP right after TERM) neither restarts nor cuts short
    let _ = TERM_SIGNAL.compare_exchange(0, sig as usize, Ordering::SeqCst, Ordering::SeqCst);
}

// ---- output to the window: bounded, and never a reason to stop

/// Sends lines and events to the window through the supervisor's standard output.
///
/// Events are few and small and always get through (in order, even if late); game lines go through a bounded queue
/// and, when the reader is slow or gone, are dropped here with a count, written to the log file regardless.
#[derive(Clone)]
pub struct Output {
    lines: mpsc::Sender<String>,
    events: mpsc::UnboundedSender<String>,
    dropped: Arc<AtomicU64>,
}

pub struct OutputTask {
    lines: mpsc::Receiver<String>,
    events: mpsc::UnboundedReceiver<String>,
    dropped: Arc<AtomicU64>,
}

pub fn output(capacity: usize) -> (Output, OutputTask) {
    let (lt, lr) = mpsc::channel(capacity);
    let (et, er) = mpsc::unbounded_channel();
    let dropped = Arc::new(AtomicU64::new(0));
    (Output { lines: lt, events: et, dropped: dropped.clone() }, OutputTask { lines: lr, events: er, dropped })
}

impl Output {
    pub fn line(&self, line: String) {
        if self.lines.try_send(line).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn event(&self, event: &SessionEvent) {
        let _ = self.events.send(format!("{EVENT_PREFIX}{}", serde_json::to_string(event).unwrap_or_default()));
    }
}

impl OutputTask {
    /// Write until both senders are gone. A writer that fails (the window closed its end) stops writing but the task
    /// keeps emptying the queues, so the supervisor never waits on it.
    pub async fn run<W: AsyncWrite + Unpin>(mut self, mut out: W) {
        let mut alive = true;
        let mut lines_open = true;
        let mut events_open = true;
        while lines_open || events_open {
            let next = tokio::select! {
                biased;
                e = self.events.recv(), if events_open => match e { Some(e) => Some(e), None => { events_open = false; None } },
                l = self.lines.recv(), if lines_open => match l { Some(l) => Some(l), None => { lines_open = false; None } },
            };
            let Some(mut text) = next else { continue };
            if alive {
                let dropped = self.dropped.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    text = format!(
                        "[{dropped} lines were not shown here to keep up; the full log is in the log file]\n{text}"
                    );
                }
                text.push('\n');
                // a window that stops reading would block this write forever: that only stalls this task
                if out.write_all(text.as_bytes()).await.is_err() || out.flush().await.is_err() {
                    alive = false;
                }
            }
        }
    }
}

/// Our standard output as an async writer: its own copy of the descriptor, written by a helper thread of the runtime,
/// so a pipe nobody reads blocks only that thread.
fn stdout_writer() -> tokio::fs::File {
    use std::os::fd::AsFd;
    match std::io::stdout().as_fd().try_clone_to_owned() {
        Ok(fd) => tokio::fs::File::from_std(std::fs::File::from(fd)),
        Err(_) => tokio::fs::File::from_std(std::fs::File::create("/dev/null").expect("/dev/null")),
    }
}

// ---- state and events

struct Hub {
    dir: PathBuf,
    state: std::sync::Mutex<State>,
    tx: broadcast::Sender<SessionEvent>,
    out: Output,
}

impl Hub {
    fn emit(&self, event: SessionEvent, stage: Option<Stage>) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.events.push(event.clone());
            if s.events.len() > 64 {
                // a long session's notices; the first events (start, started, ready) are the ones that matter
                let drop = s.events.len() - 64;
                s.events.drain(1..=drop);
            }
            if let Some(st) = stage {
                s.stage = st;
            }
            let _ = ipc::write_state(&self.dir, &s);
        }
        self.out.event(&event);
        let _ = self.tx.send(event);
    }

    fn snapshot(&self) -> State {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

// ---- what a test (or the real thing) provides

pub struct Deps {
    pub host: Arc<dyn Host>,
    pub signals: Arc<dyn Signals>,
    pub table: ProcTable,
    /// the environment the graphics checks look at: the process's own with the plan's on top
    pub check_env: Env,
}

impl Deps {
    pub fn system(plan: &LaunchPlan) -> Deps {
        let mut env = Env::from_process();
        env.0.extend(plan.env.iter().cloned());
        Deps { host: Arc::new(SystemHost), signals: Arc::new(RealSignals), table: ProcTable::system(), check_env: env }
    }
}

fn timing_of(plan: &LaunchPlan) -> Timing {
    let t = plan.timing();
    Timing {
        term_grace: Duration::from_millis(t.term_grace),
        kill_grace: Duration::from_millis(t.kill_grace),
        poll: Duration::from_millis(t.poll.max(1)),
        idle_timeout: Duration::from_millis(t.idle_timeout),
        idle_confirmations: t.idle_confirmations,
    }
}

fn is_the_image(info: &super::procs::ProcInfo, image: &str) -> bool {
    let want = image.to_lowercase();
    if info.cmdline.iter().any(|a| a.to_lowercase().contains(&want)) {
        return true;
    }
    // a command name is cut at 15 characters
    let comm = info.comm.to_lowercase();
    !comm.is_empty() && want.starts_with(&comm) && comm.len() >= 8
}

/// Blocking work on a thread of its own; its panic is a failure to report, not a crash of the supervisor.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

fn failure_code(kind: FailureKind) -> i32 {
    match kind {
        FailureKind::Busy | FailureKind::PrefixInUse => 75,
        FailureKind::GraphicsUnsafe => 78,
        FailureKind::BadPlan => 64,
        FailureKind::Spawn => 71,
        FailureKind::Internal => 70,
    }
}

fn open_log(path: &Path) -> std::io::Result<tokio::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path).map(tokio::fs::File::from_std)
}

/// Appends redacted lines to a file, up to a size limit.
struct LogFile {
    file: tokio::io::BufWriter<tokio::fs::File>,
    written: u64,
    full: bool,
}

impl LogFile {
    fn new(file: tokio::fs::File) -> Self {
        Self { file: tokio::io::BufWriter::new(file), written: 0, full: false }
    }

    async fn line(&mut self, line: &str) {
        if self.full {
            return;
        }
        if self.written + line.len() as u64 + 1 > MAX_LOG_BYTES {
            self.full = true;
            let _ = self.file.write_all(b"[log cut off here: it reached its size limit]\n").await;
            return;
        }
        self.written += line.len() as u64 + 1;
        let _ = self.file.write_all(line.as_bytes()).await;
        let _ = self.file.write_all(b"\n").await;
    }

    async fn flush(&mut self) {
        let _ = self.file.flush().await;
    }
}

/// Everything about one stop that is under way.
struct Stopping {
    forced: bool,
    by_signal: bool,
    waiters: Vec<tokio::sync::oneshot::Sender<Reply>>,
    tasks: Vec<tokio::task::JoinHandle<StopReport>>,
    report: StopReport,
}

/// Run one session to its end. Returns the process exit code.
pub async fn run(plan: LaunchPlan, secrets: Vec<String>, deps: Deps) -> i32 {
    let dir = plan.session_dir.clone();
    let (out, out_task) = output(WINDOW_QUEUE);
    let writer = tokio::spawn(out_task.run(stdout_writer()));
    let (events_tx, _) = broadcast::channel(128);
    let hub = Arc::new(Hub {
        dir: dir.clone(),
        state: std::sync::Mutex::new(State {
            session: plan.session.clone(),
            supervisor_pid: std::process::id(),
            supervisor_start: staging::process_start(std::process::id()),
            stage: Stage::Starting,
            events: Vec::new(),
        }),
        tx: events_tx.clone(),
        out: out.clone(),
    });
    let code = session(&plan, secrets, deps, &hub, &out, events_tx).await;
    drop(hub);
    drop(out);
    // let the window read the last events if it is still there, but never wait for one that is not
    let _ = tokio::time::timeout(Duration::from_secs(3), writer).await;
    code
}

async fn fail(hub: &Hub, kind: FailureKind, message: impl Into<String>) -> i32 {
    let message = message.into();
    hub.emit(SessionEvent::Failed { kind, message }, Some(Stage::Failed));
    failure_code(kind)
}

async fn session(
    plan: &LaunchPlan,
    secrets: Vec<String>,
    deps: Deps,
    hub: &Arc<Hub>,
    out: &Output,
    events_tx: broadcast::Sender<SessionEvent>,
) -> i32 {
    if let Err(why) = plan.validate() {
        return fail(hub, FailureKind::BadPlan, format!("The session plan was refused: {why}")).await;
    }
    let timing = timing_of(plan);
    let t_ms = plan.timing();
    let _ = std::fs::create_dir_all(&plan.log_dir);
    // our own record: from now on this process, not the window that made the folder, owns the session
    let owner = Owner::this_process(&plan.session);
    let _ = std::fs::write(plan.session_dir.join("owner.json"), serde_json::to_vec(&owner).unwrap_or_default());
    let _ = ipc::write_state(&plan.session_dir, &hub.snapshot());
    sys::become_subreaper();
    sys::on_signal(sys::SIGTERM, on_term);
    sys::on_signal(sys::SIGHUP, on_term);

    // the control socket answers from here on; requests wait in the queue until the loop reads them
    let (stop_tx, mut stop_rx) = mpsc::channel::<Incoming>(16);
    let server = match Server::bind(&plan.session_dir) {
        Ok(s) => s,
        Err(e) => return fail(hub, FailureKind::Internal, format!("Couldn't open the control socket: {e}")).await,
    };
    let snap_hub = hub.clone();
    let serve = tokio::spawn(server.serve(stop_tx, Arc::new(move || snap_hub.snapshot()), events_tx));

    let finish = |code: i32| async move { code };
    let code = {
        let r = session_inner(plan, secrets, &deps, hub, out, &mut stop_rx, timing, t_ms).await;
        finish(r).await
    };
    // answer whoever is still waiting on a stop
    stop_rx.close();
    while let Ok(inc) = stop_rx.try_recv() {
        let _ = inc.reply.send(match hub.snapshot().stage {
            Stage::Finished => Reply::Stopped { outcome: StopOutcome { seen: 0, killed: 0, prefix_idle: true } },
            _ => Reply::Error { message: "the session is over".into() },
        });
    }
    // the last events reach connected followers before the socket goes
    tokio::time::sleep(Duration::from_millis(50)).await;
    serve.abort();
    code
}

#[allow(clippy::too_many_arguments)]
async fn session_inner(
    plan: &LaunchPlan,
    secrets: Vec<String>,
    deps: &Deps,
    hub: &Arc<Hub>,
    out: &Output,
    stop_rx: &mut mpsc::Receiver<Incoming>,
    timing: Timing,
    t_ms: super::plan::TimingMs,
) -> i32 {
    let paths = gpu_safety::Paths::at(&plan.bedrock_root);
    let scope_prefix = Scope::prefix_only(&plan.prefix);

    // 2. leases
    let lease_plan = plan.leases.clone();
    let wait = t_ms.lease_wait();
    let held: Held = match blocking(move || lease_plan.take(wait, &|| TERM_SIGNAL.load(Ordering::SeqCst) != 0)).await {
        Some(Ok(h)) => h,
        Some(Err(super::lease::LeaseError::Busy(p))) => {
            let what = if p.ends_with("game.lease") {
                "Another Bedrock game is already running; v1 plays one at a time.".to_string()
            } else {
                format!("{} is in use by another BlockForge job or session.", p.display())
            };
            return fail(hub, FailureKind::Busy, what).await;
        }
        Some(Err(e)) => return fail(hub, FailureKind::Busy, e.to_string()).await,
        None => return fail(hub, FailureKind::Internal, "taking the leases failed").await,
    };

    // 3. nothing of an earlier session may be using the prefix; an earlier session's marker is retired if it is over
    let table = deps.table.clone();
    let early = table.in_scope(&scope_prefix);
    if !early.is_empty() {
        return fail(
            hub,
            FailureKind::PrefixInUse,
            format!("{} Wine process(es) of an earlier session still use this profile's prefix. Stop them from the session list before starting the game again.", early.len()),
        )
        .await;
    }
    if let Some(said) = gpu_safety::retire_idle_current_boot_marker(&paths, deps.host.as_ref(), true) {
        hub.emit(SessionEvent::Notice { text: said }, None);
    }

    // 4. graphics checks and the marker
    let (host, env, allow) = (deps.host.clone(), deps.check_env.clone(), plan.allow_unsafe_gpu);
    let p2 = paths.clone();
    let pre = match blocking(move || gpu_safety::check(&p2, host.as_ref(), &env, allow)).await {
        Some(p) => p,
        None => return fail(hub, FailureKind::Internal, "the graphics check failed to run").await,
    };
    if let Some(refusal) = pre.refusal {
        return fail(hub, FailureKind::GraphicsUnsafe, refusal.into_error().message).await;
    }
    for n in pre.notes {
        hub.emit(SessionEvent::Notice { text: n }, None);
    }
    for b in pre.bypassed {
        hub.emit(SessionEvent::Notice { text: format!("The graphics safety check was bypassed at the player's request: {b}. This start is at their own risk.") }, None);
    }
    let staging = plan.staging_dir.as_deref().map(|d| adopt_staging(d, &plan.session));
    let armed: Armed =
        match gpu_safety::arm(&paths, deps.host.as_ref(), &plan.engine_rev, &plan.session, plan.allow_unsafe_gpu) {
            Ok(a) => a,
            Err(r) => return fail(hub, FailureKind::GraphicsUnsafe, r.into_error().message).await,
        };
    hub.emit(SessionEvent::Starting, Some(Stage::Starting));

    // 5. the wrapper, in a group of its own, holding our leases if we are killed
    held.inherit_into_children();
    let mut env = plan.env.clone();
    if !env.iter().any(|(k, _)| k == "WINEPREFIX") {
        env.push(("WINEPREFIX".into(), plan.prefix.to_string_lossy().into_owned()));
    }
    if TERM_SIGNAL.load(Ordering::SeqCst) != 0 {
        gpu_safety::disarm(&paths, &armed);
        return fail(hub, FailureKind::Internal, "The supervisor was asked to stop before the game started.").await;
    }
    let mut running = match crate::launch::spawn_process_without(
        &plan.program,
        &plan.args,
        &plan.cwd,
        &env,
        &plan.env_remove,
        None,
        None,
    ) {
        Ok(r) => r,
        Err(e) => {
            // nothing started: the record must not claim a game ran
            if !gpu_safety::disarm(&paths, &armed) {
                hub.emit(SessionEvent::Notice { text: "The game could not be started and its graphics safety record could not be cleared; inspect it in the diagnostics.".into() }, None);
            }
            return fail(hub, FailureKind::Spawn, format!("Couldn't start the game's wrapper: {e}")).await;
        }
    };
    let wrapper_pid = running.child.id().unwrap_or(0);
    hub.emit(SessionEvent::Started { pid: wrapper_pid }, Some(Stage::Running));

    // 6. wait
    let mut redactor = Redactor::new(&secrets);
    let mut log = match open_log(&plan.log_dir.join("game.log")) {
        Ok(f) => LogFile::new(f),
        Err(_) => LogFile::new(tokio::fs::File::from_std(std::fs::File::open("/dev/null").expect("/dev/null"))),
    };
    let scope = Scope { prefix: &plan.prefix, root: Some(std::process::id()) };
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut ready = false;
    let mut returned: Option<Option<i32>> = None;
    let mut lines_open = true;
    let mut stopping: Option<Stopping> = None;
    let mut seen: std::collections::HashSet<ProcId> = Default::default();
    let mut scan_every = 0u32;
    let mut signalled_at: Option<tokio::time::Instant> = None;

    macro_rules! begin_stop {
        ($force:expr, $by_signal:expr, $waiter:expr) => {{
            let force: bool = $force;
            let st = stopping.get_or_insert_with(|| Stopping {
                forced: false,
                by_signal: false,
                waiters: vec![],
                tasks: vec![],
                report: StopReport::default(),
            });
            if let Some(w) = $waiter {
                st.waiters.push(w);
            }
            let first = st.tasks.is_empty();
            // a second request upgrades a polite stop to a forced one; a repeat of the same does nothing
            if first || (force && !st.forced) {
                if first {
                    hub.emit(
                        SessionEvent::Stopping { mode: if force { "force" } else { "graceful" }.into() },
                        Some(Stage::Stopping),
                    );
                }
                st.forced |= force;
                st.by_signal |= $by_signal;
                let (table, signals, prefix) = (deps.table.clone(), deps.signals.clone(), plan.prefix.clone());
                let me = std::process::id();
                st.tasks.push(tokio::task::spawn_blocking(move || {
                    let reaper = Reaper {
                        table: &table,
                        signals: signals.as_ref(),
                        scope: Scope { prefix: &prefix, root: Some(me) },
                    };
                    reaper.stop(timing, force, |_: StopStage| {}, std::thread::sleep)
                }));
            }
        }};
    }

    loop {
        let all_stops_done = stopping.as_ref().is_none_or(|s| s.tasks.iter().all(|t| t.is_finished()));
        tokio::select! {
            biased;
            inc = stop_rx.recv() => {
                if let Some(inc) = inc {
                    begin_stop!(inc.kind == StopKind::Force, false, Some(inc.reply));
                }
            }
            status = running.child.wait(), if returned.is_none() => {
                returned = Some(status.ok().and_then(|s| crate::launch::exit_code(&s)));
            }
            line = running.lines.recv(), if lines_open => match line {
                Some(l) => {
                    let l = redactor.apply(&l);
                    log.line(&l).await;
                    out.line(l);
                }
                None => lines_open = false,
            },
            _ = tick.tick() => {
                log.flush().await;
                if let Some(sig) = std::num::NonZeroUsize::new(TERM_SIGNAL.load(Ordering::SeqCst)) {
                    if signalled_at.is_none() {
                        signalled_at = Some(tokio::time::Instant::now());
                        hub.emit(SessionEvent::Notice { text: format!("The supervisor was asked to stop (signal {sig}) while the game runs; closing the game and clearing up before it ends.") }, None);
                        begin_stop!(false, true, None::<tokio::sync::oneshot::Sender<Reply>>);
                    }
                }
                // a signal's stop is bounded: past the grace everything is ended by force
                if let (Some(at), Some(st)) = (signalled_at, stopping.as_ref()) {
                    if !st.forced && at.elapsed() >= t_ms.signal_grace() {
                        begin_stop!(true, true, None::<tokio::sync::oneshot::Sender<Reply>>);
                    }
                }
                if !ready && returned.is_none() {
                    scan_every += 1;
                    // looking through /proc is not free: twice a second at most, and not at all once the game is up
                    if scan_every % 2 == 0 {
                        let found = deps.table.in_scope(&scope);
                        seen.extend(found.iter().map(|i| i.id));
                        if found.iter().any(|i| is_the_image(i, &plan.ready_image)) {
                            ready = true;
                            if let Some(s) = &staging {
                                // the loader unlinks what it opened: anything still here was never needed
                                s.release_files();
                            }
                            hub.emit(SessionEvent::Ready, Some(Stage::Ready));
                        }
                    }
                }
            }
        }
        if returned.is_some() && (!lines_open || all_stops_done) {
            // the wrapper is gone; whatever it printed last may still be in the pipe
            while lines_open {
                match tokio::time::timeout(Duration::from_secs(2), running.lines.recv()).await {
                    Ok(Some(l)) => {
                        let l = redactor.apply(&l);
                        log.line(&l).await;
                        out.line(l);
                    }
                    _ => lines_open = false,
                }
            }
            break;
        }
    }
    log.flush().await;

    // 7. teardown
    let game_returned = returned.is_some();
    let mut wrapper_code = returned.flatten();
    if game_returned && !gpu_safety::record_wrapper_exit(&paths, deps.host.as_ref(), &armed) {
        hub.emit(SessionEvent::Notice { text: "The game returned, but its graphics safety record could not note that phase. A failed teardown will need an explicit acknowledgement.".into() }, None);
    }
    let (table2, signals2, prefix2) = (deps.table.clone(), deps.signals.clone(), plan.prefix.clone());
    let me = std::process::id();
    let (leftover, report) = blocking(move || {
        let reaper =
            Reaper { table: &table2, signals: signals2.as_ref(), scope: Scope { prefix: &prefix2, root: Some(me) } };
        // the wrapper is gone: the prefix usually follows within moments (wineserver lingers a few seconds)
        let mut left = reaper.wait_idle(timing, std::thread::sleep);
        let mut report = StopReport::default();
        if !left.is_empty() {
            report = reaper.stop(timing, false, |_| {}, std::thread::sleep);
            left = reaper.wait_idle(timing, std::thread::sleep);
        }
        (left, report)
    })
    .await
    .unwrap_or((vec![], StopReport::default()));
    // a wrapper that never returned (we ended its prefix's processes, or only the wrapper was stubborn): ours to end
    if !game_returned {
        let _ = running.child.start_kill();
        if let Ok(Ok(st)) = tokio::time::timeout(Duration::from_secs(5), running.child.wait()).await {
            wrapper_code = crate::launch::exit_code(&st);
        }
    }
    // orphans the subreaper adopted: collect the ones that ended so none lingers as a zombie
    let mut stop_seen = report.seen;
    for id in &seen {
        sys::reap(id.pid);
    }
    if let Some(st) = stopping.as_mut() {
        for t in st.tasks.drain(..) {
            if let Ok(r) = t.await {
                st.report.seen = st.report.seen.max(r.seen);
                st.report.killed += r.killed;
            }
        }
        stop_seen = stop_seen.max(st.report.seen);
    }
    let prefix_idle = leftover.is_empty();
    // the record is cleared only when the wrapper is over and nothing of the prefix is left, as BedrockOnLinux does
    let wrapper_over = game_returned || running.child.try_wait().ok().flatten().is_some();
    let marker_cleared = prefix_idle && wrapper_over && gpu_safety::disarm(&paths, &armed);
    if !prefix_idle {
        hub.emit(
            SessionEvent::Notice { text: format!("{} Wine process(es) of the prefix could not be ended. The graphics safety record stays; stop them and check the driver before starting again.", leftover.len()) },
            None,
        );
    } else if !marker_cleared {
        hub.emit(SessionEvent::Notice { text: "The game ended, but its graphics safety record could not be cleared; acknowledge it in the diagnostics after checking the driver.".into() }, None);
    }
    // leftovers of the decrypted image: the staging folder goes with the session
    if let Some(s) = &staging {
        s.release_files();
    }
    drop(staging);
    let log_dir = plan.log_dir.clone();
    let ctx = LogContext {
        expected_gpu: plan.expected_gpu.clone(),
        virtual_gpu: diagnostics::host_has_virtio_gpu(),
        env_overrides: vec![],
    };
    let findings: Vec<Finding> = blocking(move || diagnostics::diagnose_dir(&log_dir, &ctx)).await.unwrap_or_default();
    let fatal = findings.iter().any(|f| f.is_fatal());
    let outcome = StopOutcome {
        seen: stop_seen,
        killed: stopping.as_ref().map(|s| s.report.killed).unwrap_or(report.killed),
        prefix_idle,
    };
    if let Some(st) = stopping.take() {
        for w in st.waiters {
            let _ = w.send(Reply::Stopped { outcome: outcome.clone() });
        }
    }
    drop(held);
    hub.emit(SessionEvent::Exited { code: wrapper_code, prefix_idle, marker_cleared, findings }, Some(Stage::Finished));
    let signal = TERM_SIGNAL.load(Ordering::SeqCst);
    if signal != 0 {
        return 128 + signal as i32;
    }
    if fatal && wrapper_code.unwrap_or(0) == 0 {
        // the game "returned" but never opened: not a success
        return 1;
    }
    wrapper_code.unwrap_or(0)
}

/// Take over a staging folder the window made: record ourselves as its owner so the sweep of dead sessions judges it
/// by this process. A folder that is not a session folder of this session is left alone and not used.
fn adopt_staging(dir: &Path, session: &str) -> StagingDirGuard {
    StagingDirGuard::adopt(dir, session)
}

/// A staging folder of ours that is removed when the session ends (see [`StagingDir`]; this one already existed).
struct StagingDirGuard {
    dir: Option<PathBuf>,
}

impl StagingDirGuard {
    fn adopt(dir: &Path, session: &str) -> Self {
        // only a folder named for this session, containing our owner record, is taken
        let ok = dir.file_name().is_some_and(|n| n == session)
            && staging::read_owner(dir).is_some_and(|o| o.session == session);
        if !ok {
            return StagingDirGuard { dir: None };
        }
        staging::write_owner(dir, &Owner::this_process(session));
        StagingDirGuard { dir: Some(dir.to_path_buf()) }
    }

    fn release_files(&self) -> usize {
        self.dir.as_deref().map(staging::release_files_in).unwrap_or(0)
    }
}

impl Drop for StagingDirGuard {
    fn drop(&mut self) {
        if let Some(d) = self.dir.take() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

// ---- the handler `main` calls

/// If `args` ask for a session (`--bedrock-session <folder>`), run it and return the process's exit code. Reads the
/// plan from the folder and the secrets from standard input. Does nothing and returns `None` otherwise.
pub fn session_main(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some(SESSION_ARG) {
        return None;
    }
    let Some(dir) = args.get(1).map(PathBuf::from) else {
        eprintln!("{SESSION_ARG} needs the session folder");
        return Some(64);
    };
    let plan = match ipc::read_plan(&dir) {
        Ok(p) => p,
        Err(why) => {
            eprintln!("bedrock session: {why}");
            return Some(64);
        }
    };
    // standard input is the one place secrets travel: {"secrets":["..."]}
    #[derive(serde::Deserialize, Default)]
    struct Secrets {
        #[serde(default)]
        secrets: Vec<String>,
    }
    let mut input = String::new();
    {
        use std::io::Read;
        let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    }
    let secrets = serde_json::from_str::<Secrets>(&input).unwrap_or_default().secrets;
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("bedrock session: {e}");
            return Some(70);
        }
    };
    let deps = Deps::system(&plan);
    Some(rt.block_on(run(plan, secrets, deps)))
}

// ---- the window's side

/// A session that was started.
pub struct Spawned {
    /// the supervisor process: its lines are the game's log with event lines among them ([`super::parse_event_line`])
    pub running: crate::launch::Running,
    pub control: Control,
}

/// Make the session folder and start the supervisor for `plan` with `exe` (the BlockForge executable). Must be called
/// from within the tokio runtime. `secrets` are values to hide wherever they show up in the game's output. Returns
/// as soon as the supervisor process exists: use [`Control::wait_started`] to learn whether it got as far as the game.
pub fn spawn(exe: &Path, plan: &LaunchPlan, secrets: &[String]) -> Result<Spawned, BedrockError> {
    let args = vec![SESSION_ARG.to_string(), plan.session_dir.to_string_lossy().into_owned()];
    spawn_with(exe, &args, &[], plan, secrets)
}

/// [`spawn`] with the supervisor's command line and environment given (a test starts a copy of its own binary that way).
pub fn spawn_with(
    exe: &Path,
    args: &[String],
    env: &[(String, String)],
    plan: &LaunchPlan,
    secrets: &[String],
) -> Result<Spawned, BedrockError> {
    use std::os::unix::fs::DirBuilderExt;
    plan.validate()
        .map_err(|e| BedrockError::new(ErrorKind::InvalidRequest, format!("The session plan was refused: {e}")))?;
    let io = |what: &str, e: std::io::Error| BedrockError::new(ErrorKind::Internal, format!("{what}: {e}"));
    for d in [&plan.session_dir, &plan.log_dir] {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(d)
            .map_err(|e| io("making the session folders", e))?;
    }
    ipc::write_plan(&plan.session_dir, plan).map_err(|e| io("writing the session plan", e))?;
    // until the supervisor takes over, this process owns the folder
    let owner = Owner::this_process(&plan.session);
    std::fs::write(plan.session_dir.join("owner.json"), serde_json::to_vec(&owner).unwrap_or_default())
        .map_err(|e| io("writing the session owner", e))?;
    let payload = serde_json::to_vec(&serde_json::json!({ "secrets": secrets })).unwrap_or_default();
    let running =
        crate::launch::spawn_process(exe, args, &plan.session_dir, env, Some(payload), None).map_err(|e| {
            BedrockError::new(ErrorKind::Runtime, format!("Couldn't start the Bedrock session supervisor: {e}"))
        })?;
    Ok(Spawned { running, control: Control::attach(&plan.session_dir) })
}

/// What a cleanup of old sessions did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupReport {
    pub removed: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

/// Remove what dead BlockForge sessions left: their folder under `<bedrock>/sessions` and, with the same id, their
/// staging folder under `ram_root` (see [`staging::system_root`]). Only sessions whose owner is gone are touched; one
/// whose prefix still has Wine processes of ours is kept (a game that outlived its supervisor).
pub fn cleanup_stale_sessions(bedrock_root: &Path, ram_root: Option<&Path>) -> CleanupReport {
    let mut report = CleanupReport::default();
    let table = ProcTable::system();
    if let Ok(rd) = std::fs::read_dir(bedrock_root.join("sessions")) {
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let real_dir = e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false);
            if !real_dir || uuid::Uuid::parse_str(&name).is_err() {
                continue;
            }
            let owner: Option<Owner> =
                std::fs::read(path.join("owner.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
            // the supervisor's record in state.json is the freshest word on who owns the session
            let state = ipc::read_state(&path);
            let alive = match (&state, &owner) {
                (Some(s), _) if s.supervisor_start.is_some() => {
                    staging::process_start(s.supervisor_pid) == s.supervisor_start
                }
                (_, Some(o)) => o.is_alive(),
                // no record at all: not something we can show is ours
                _ => {
                    report.kept.push(path);
                    continue;
                }
            };
            if alive {
                report.kept.push(path);
                continue;
            }
            if let Ok(plan) = ipc::read_plan(&path) {
                if !table.in_prefix(&plan.prefix).is_empty() {
                    report.kept.push(path);
                    continue;
                }
            }
            if std::fs::remove_dir_all(&path).is_ok() {
                if let Some(ram) = ram_root {
                    let staged = ram.join(&name);
                    // only if it is a session folder of ours whose owner is gone too
                    if staging::read_owner(&staged).is_some_and(|o| o.session == name && !o.is_alive()) {
                        let _ = std::fs::remove_dir_all(staged);
                    }
                }
                report.removed.push(path);
            } else {
                report.kept.push(path);
            }
        }
    }
    // staging folders whose session folder is long gone, by the sweep of dead owners
    if let Some(ram) = ram_root {
        for gone in staging::sweep_stale(ram).removed {
            report.removed.push(gone);
        }
    }
    report
}
