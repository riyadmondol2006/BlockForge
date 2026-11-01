//! Minecraft: Bedrock Edition in the window: its instances' page, what New instance says about it, its cards in the
//! Library and on Home, and the jobs the window starts on the backend (`crate::bedrock`).
//!
//! Everything shown comes from the backend's snapshot, a copy in memory: no disk, network or process lookup happens
//! while a frame is drawn. The snapshot is read again when the backend says it changed (its events come in as
//! [`Msg::Bedrock`]), and every job runs on the runtime's workers, its answer coming back the same way.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, vec2};

use super::covers::{CoverSpec, CoverTheme};
use super::icons::{self, Icon};
use super::theme::*;
use super::widgets::*;
use super::{App, Msg, Page, Task};
use crate::bedrock::{
    self as br, AccountTarget, AuthPrompt, BedrockError, Edition, ErrorKind, InstanceId, InstanceStatus, Readiness,
    SignInState, Support,
};
use crate::install::Progress;

/// How Bedrock instances are tagged where a loader would be named (cards, filters, covers).
pub const TAG: &str = "Bedrock";

/// The most lines of a game's log kept in the window; the whole log is on disk.
const LOG_LINES: usize = 2000;
/// The lines shown on the page, the newest.
const LOG_SHOWN: usize = 400;

/// What the window started on the backend, one at a time per instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Create,
    Install,
    Play,
    Repair,
    Diagnose,
    Backup,
    Delete,
}

impl Op {
    /// What the instance is doing, for its status line.
    fn doing(self) -> &'static str {
        match self {
            Op::Create => "Being made",
            Op::Install => "Downloading the game",
            Op::Play => "Getting the game ready",
            Op::Repair => "Repairing",
            Op::Diagnose => "Looking for problems",
            Op::Backup => "Backing up the worlds",
            Op::Delete => "Being deleted",
        }
    }
}

/// What a job gave back.
pub enum Done {
    Nothing,
    Created(br::BedrockInstance),
    Session(br::Session),
    Report(br::DiagnosticReport),
    Backup(PathBuf),
}

pub enum BedrockMsg {
    /// The snapshot changed (or events were missed): read it again.
    Changed,
    Event(br::Event),
    Done {
        key: uuid::Uuid,
        op: Op,
        result: br::Result<Done>,
    },
    SignedIn {
        key: String,
        result: br::Result<()>,
    },
    SignedOut {
        result: br::Result<()>,
    },
}

struct Job {
    op: Op,
    task: u64,
    progress: Arc<Progress>,
}

struct Live {
    session: br::SessionId,
    control: br::SessionControl,
}

pub struct BedrockUi {
    pub backend: br::Backend,
    pub snap: Arc<br::Snapshot>,
    /// The instance the Bedrock page shows.
    pub open: Option<InstanceId>,
    jobs: HashMap<uuid::Uuid, Job>,
    /// Sign-ins under way, by target (`store:<id>`, `xbox:<id>`), with their Cancel.
    sign_ins: HashMap<String, br::Cancel>,
    logs: HashMap<uuid::Uuid, Arc<Mutex<VecDeque<String>>>>,
    live: HashMap<uuid::Uuid, Live>,
    errors: HashMap<uuid::Uuid, String>,
    reports: HashMap<uuid::Uuid, br::DiagnosticReport>,
    backups: HashMap<uuid::Uuid, PathBuf>,
    /// Instances whose game didn't close when asked: Stop ends it now.
    force: HashSet<uuid::Uuid>,
    /// The edition New instance makes: Release or Preview.
    pub channel: Edition,
    pub confirm_delete: Option<InstanceId>,
    /// The label of the instance being made, so New instance waits for it.
    pub creating: Option<String>,
}

impl BedrockUi {
    /// Allocation only (see `Backend::new`): the records are read later, on a worker.
    pub fn new(dirs: &crate::paths::Dirs, handle: tokio::runtime::Handle) -> Self {
        Self {
            backend: br::Backend::new(dirs, handle),
            snap: Arc::default(),
            open: None,
            jobs: HashMap::new(),
            sign_ins: HashMap::new(),
            logs: HashMap::new(),
            live: HashMap::new(),
            errors: HashMap::new(),
            reports: HashMap::new(),
            backups: HashMap::new(),
            force: HashSet::new(),
            channel: Edition::Release,
            confirm_delete: None,
            creating: None,
        }
    }

    /// The job an instance is busy with.
    pub fn job(&self, id: &InstanceId) -> Option<Op> {
        self.jobs.get(&id.0).map(|j| j.op)
    }

    pub fn status(&self, id: &InstanceId) -> Option<&InstanceStatus> {
        self.snap.instances.iter().find(|s| &s.instance.id == id)
    }
}

pub fn is_live(r: &Readiness) -> bool {
    matches!(r, Readiness::Starting(_) | Readiness::Running(_) | Readiness::Stopping(_))
}

pub fn edition_name(e: Edition) -> &'static str {
    match e {
        Edition::Release => "Release",
        Edition::Preview => "Preview",
    }
}

/// The edition an instance plays.
pub fn edition_of(i: &br::BedrockInstance) -> Edition {
    match &i.selection {
        br::BuildSelection::Latest(e) => *e,
        br::BuildSelection::Installed(b) => b.edition,
    }
}

/// A package version as players know it: the Store's `1.26.5004.0` is Minecraft 1.26.50.4 (the third number holds the
/// last two). Anything else is shown as it is.
pub fn display_version(v: &str) -> String {
    let parts: Vec<u64> = v.split('.').filter_map(|n| n.parse().ok()).collect();
    match parts[..] {
        [a, b, c, 0] if c >= 100 && v.split('.').count() == 4 => format!("{a}.{b}.{}.{}", c / 100, c % 100),
        _ => v.to_string(),
    }
}

/// "Release · 1.26.50.4", or with no build "Release · not downloaded" ("not installed" on Windows).
pub fn version_line(i: &br::BedrockInstance, support: Support) -> String {
    let e = edition_name(edition_of(i));
    match &i.installed {
        Some(b) => format!("{e} \u{00B7} {}", display_version(&b.version)),
        None if support == Support::Windows => format!("{e} \u{00B7} not installed"),
        None => format!("{e} \u{00B7} not downloaded"),
    }
}

/// How an instance stands, in a few words with the kind of mark that goes with them. A job of the window's says what
/// it does.
pub fn status_of(r: &Readiness, job: Option<Op>, support: Support) -> (Status, String) {
    if let Some(op) = job.filter(|op| *op != Op::Play || !is_live(r)) {
        return (Status::Busy, op.doing().to_string());
    }
    match r {
        Readiness::Unknown => (Status::Busy, "Looking at it".into()),
        Readiness::UnsupportedPlatform => (Status::Warn, "Not available on this computer".into()),
        Readiness::NeedsInstall if support == Support::Windows => {
            (Status::Warn, "Minecraft for Windows isn't installed".into())
        }
        Readiness::NeedsInstall => (Status::Warn, "Not downloaded yet".into()),
        Readiness::NeedsStoreSignIn => (Status::Warn, "Sign in to the Microsoft Store to download and play".into()),
        Readiness::NeedsRepair { reason } => (Status::Error, format!("Needs a repair: {reason}")),
        Readiness::Ready { xbox_online_cached } if support == Support::Linux && !xbox_online_cached => {
            (Status::Good, "Ready to play \u{00B7} sign in to Xbox to play online".into())
        }
        Readiness::Ready { .. } => (Status::Good, "Ready to play".into()),
        Readiness::Starting(_) => (Status::Busy, "Starting".into()),
        Readiness::Running(_) => (Status::Good, "Playing".into()),
        Readiness::Stopping(_) => (Status::Busy, "Closing".into()),
    }
}

/// The one thing to press on an instance now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    Play,
    Download,
    /// Windows: the Store's page for the game.
    GetFromStore,
    SignInStore,
    Repair,
    Stop,
    /// A job or the game is on its way: nothing to press but its Cancel.
    Wait,
    Unavailable,
}

impl Primary {
    pub fn label(self) -> &'static str {
        match self {
            Primary::Play => "Play",
            Primary::Download => "Download",
            Primary::GetFromStore => "Get it from the Store",
            Primary::SignInStore => "Sign in to the Store",
            Primary::Repair => "Repair",
            Primary::Stop => "Stop",
            Primary::Wait => "Working",
            Primary::Unavailable => "Not available",
        }
    }
}

pub fn primary_of(r: &Readiness, job: Option<Op>, support: Support) -> Primary {
    if job.is_some() {
        return Primary::Wait;
    }
    match r {
        Readiness::UnsupportedPlatform => Primary::Unavailable,
        Readiness::Unknown | Readiness::Starting(_) | Readiness::Stopping(_) => Primary::Wait,
        Readiness::NeedsInstall if support == Support::Windows => Primary::GetFromStore,
        Readiness::NeedsInstall => Primary::Download,
        Readiness::NeedsStoreSignIn => Primary::SignInStore,
        Readiness::NeedsRepair { .. } => Primary::Repair,
        Readiness::Ready { .. } => Primary::Play,
        Readiness::Running(_) => Primary::Stop,
    }
}

/// The sign-in state of `target` in the snapshot.
pub fn account_state(snap: &br::Snapshot, target: &AccountTarget) -> SignInState {
    snap.accounts
        .iter()
        .find(|a| match (&a.target, target) {
            (AccountTarget::Store(a), AccountTarget::Store(b)) => a == b,
            (AccountTarget::Xbox(a), AccountTarget::Xbox(b)) => a == b,
            _ => false,
        })
        .map(|a| a.state.clone())
        .unwrap_or(SignInState::Unknown)
}

fn target_key(t: &AccountTarget) -> String {
    match t {
        AccountTarget::Store(id) => format!("store:{}", id.0),
        AccountTarget::Xbox(id) => format!("xbox:{}", id.0),
    }
}

/// What a card or the page asked for.
#[derive(Clone, Debug)]
pub enum BAct {
    Open(InstanceId),
    /// The page's main button, whatever it is now.
    Primary(InstanceId),
    Play(InstanceId),
    Stop(InstanceId),
    Download(InstanceId),
    SignIn(AccountTarget),
    SignOut(AccountTarget, br::SignOutMode),
    CancelSignIn(AccountTarget),
    CancelJob(InstanceId),
    Repair(InstanceId),
    Diagnose(InstanceId),
    Worlds(InstanceId),
    Backup(InstanceId),
    Delete(InstanceId),
    Channel(InstanceId, Edition),
    Logs,
    Copy(String),
    Link(String),
    StorePage(Edition),
}

/// A cover for a Bedrock instance: its own landscape from its id, drawn as Minecraft 1.21 would be, with the Bedrock
/// tint.
pub fn cover_spec(id: &InstanceId) -> CoverSpec {
    CoverSpec::new(&format!("bedrock-{}", id.0), "1.21", Some(TAG), CoverTheme::Auto)
}

/// A name for a new instance: `wanted`, with a number after it when a Bedrock instance has it already.
pub fn unique_name(snap: &br::Snapshot, wanted: &str) -> String {
    let taken = |n: &str| snap.instances.iter().any(|s| s.instance.name.eq_ignore_ascii_case(n));
    if !taken(wanted) {
        return wanted.to_string();
    }
    (2..).map(|n| format!("{wanted} ({n})")).find(|n| !taken(n)).unwrap_or_else(|| wanted.to_string())
}

/// The small labels beside Bedrock's name: what it is, and how far along it is here.
pub fn tags(ui: &mut Ui) {
    tag(ui, TAG, loader_color(TAG));
    if br::support() == Support::Linux {
        tag(ui, "Experimental", c_secondary());
    }
}

/// Who BlockForge's Bedrock support comes from, in a paragraph, and the links to both projects. Returns the address
/// pressed.
pub fn credit(ui: &mut Ui) -> Option<String> {
    ui.add(egui::Label::new(RichText::new(br::CREDIT).size(FS_META).color(c_muted())).wrap());
    super::about::links(
        ui,
        &[
            ("BedrockOnLinux (MIT)", br::BOL_REPOSITORY.to_string()),
            ("Xodus (GPL-3.0)", br::XODUS_REPOSITORY.to_string()),
        ],
    )
}

impl App {
    /// Start listening to the backend and read its records, both on the runtime: the window doesn't wait.
    pub(super) fn bedrock_start(&mut self) {
        if self.shot.is_some() {
            return;
        }
        let backend = self.bedrock.backend.clone();
        let n = self.notifier();
        let mut events = backend.subscribe();
        self.rt.spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match events.recv().await {
                    Ok(br::Event::SnapshotChanged) => n.send_soon(Msg::Bedrock(BedrockMsg::Changed)),
                    Ok(e) => n.send(Msg::Bedrock(BedrockMsg::Event(e))),
                    // events are hints: what was missed is in the snapshot
                    Err(RecvError::Lagged(_)) => n.send(Msg::Bedrock(BedrockMsg::Changed)),
                    Err(RecvError::Closed) => break,
                }
            }
        });
        self.rt.spawn(async move {
            if let Err(e) = backend.refresh(br::RefreshPolicy::LocalOnly, br::Cancel::default()).await {
                crate::logs::warn(format!("Bedrock: {e}"));
            }
        });
    }

    pub(super) fn bedrock_msg(&mut self, msg: BedrockMsg) {
        match msg {
            BedrockMsg::Changed => {
                // the screenshot tour shows a snapshot of its own
                if self.shot.is_none() {
                    self.bedrock.snap = self.bedrock.backend.snapshot();
                }
            }
            BedrockMsg::Event(e) => self.bedrock_event(e),
            BedrockMsg::Done { key, op, result } => self.bedrock_done(key, op, result),
            BedrockMsg::SignedIn { key, result } => {
                self.bedrock.sign_ins.remove(&key);
                match result {
                    Ok(()) => self.toast("Signed in"),
                    Err(e) if matches!(e.kind, ErrorKind::Cancelled) => {}
                    Err(e) => self.toast_err(format!("Sign-in didn't finish: {e}")),
                }
            }
            BedrockMsg::SignedOut { result } => match result {
                Ok(()) => self.toast("Signed out"),
                Err(e) => self.toast_err(format!("Couldn't sign out: {e}")),
            },
        }
    }

    fn bedrock_event(&mut self, e: br::Event) {
        match e {
            br::Event::DeviceCode { user_code, url, .. } => {
                // as the Java sign-in does: the code on the clipboard and the page open, so pasting it is all
                self.ctx.copy_text(user_code);
                self.open_link(&url);
            }
            br::Event::Notice { text, .. } => self.toast(text),
            br::Event::SessionExited { session, code } => {
                let ended: Vec<uuid::Uuid> =
                    self.bedrock.live.iter().filter(|(_, l)| l.session == session).map(|(k, _)| *k).collect();
                for k in ended {
                    self.bedrock.live.remove(&k);
                    self.bedrock.force.remove(&k);
                }
                match code {
                    Some(0) | None => {}
                    Some(c) => self.toast_err(format!("Minecraft: Bedrock Edition closed with code {c}")),
                }
            }
            br::Event::SnapshotChanged | br::Event::StoreWindowOpened(_) | br::Event::SessionStarted(_) => {}
        }
        if self.shot.is_none() {
            self.bedrock.snap = self.bedrock.backend.snapshot();
        }
    }

    fn bedrock_done(&mut self, key: uuid::Uuid, op: Op, result: br::Result<Done>) {
        if let Some(job) = self.bedrock.jobs.remove(&key) {
            self.tasks.retain(|t| t.id != job.task);
        }
        if op == Op::Create {
            self.bedrock.creating = None;
        }
        let done = match result {
            Ok(d) => d,
            Err(e) => {
                if !matches!(e.kind, ErrorKind::Cancelled) {
                    let text = explain(&e);
                    self.bedrock.errors.insert(key, text.clone());
                    // an error of a page not open is said where the player is
                    if self.page != Page::Bedrock || self.bedrock.open.as_ref().is_none_or(|o| o.0 != key) {
                        self.toast_err(text);
                    }
                }
                return;
            }
        };
        self.bedrock.errors.remove(&key);
        match done {
            Done::Nothing => match op {
                Op::Install => self.toast("Bedrock Edition is downloaded"),
                Op::Repair => self.toast("Repaired"),
                Op::Delete => {
                    if self.bedrock.open.as_ref().is_some_and(|o| o.0 == key) {
                        self.bedrock.open = None;
                        if self.page == Page::Bedrock {
                            self.page = Page::Library;
                        }
                    }
                }
                _ => {}
            },
            Done::Created(inst) => {
                self.toast(format!("{} is ready to set up", inst.name));
                self.bedrock.open = Some(inst.id);
                self.page = Page::Bedrock;
            }
            Done::Session(session) => {
                let buf = self.bedrock.logs.entry(key).or_default().clone();
                buf.lock().unwrap_or_else(|e| e.into_inner()).clear();
                if let Some(mut running) = session.running {
                    let ctx = self.ctx.clone();
                    self.rt.spawn(async move {
                        while let Some(line) = running.lines.recv().await {
                            {
                                let mut b = buf.lock().unwrap_or_else(|e| e.into_inner());
                                b.push_back(line);
                                while b.len() > LOG_LINES {
                                    b.pop_front();
                                }
                            }
                            ctx.request_repaint_after(std::time::Duration::from_millis(150));
                        }
                        let _ = running.child.wait().await;
                    });
                }
                self.bedrock.live.insert(key, Live { session: session.id, control: session.control });
            }
            Done::Report(report) => {
                if report.issues.is_empty() {
                    self.toast("No problems found");
                }
                self.bedrock.reports.insert(key, report);
            }
            Done::Backup(path) => {
                self.toast("The worlds are backed up");
                self.bedrock.backups.insert(key, path);
            }
        }
        if self.shot.is_none() {
            self.bedrock.snap = self.bedrock.backend.snapshot();
        }
    }

    /// Run `work` on the backend with a row in the sidebar, once per `key` at a time. Its Cancel there reaches the
    /// backend's cancel.
    fn bedrock_run<F, Fut>(&mut self, key: uuid::Uuid, op: Op, label: String, work: F)
    where
        F: FnOnce(br::Backend, Arc<Progress>, br::Cancel) -> Fut + Send + 'static,
        Fut: Future<Output = br::Result<Done>> + Send + 'static,
    {
        if self.bedrock.jobs.contains_key(&key) {
            return;
        }
        let task = self.next_task;
        self.next_task += 1;
        let progress = Progress::new();
        self.tasks.push(Task { id: task, label, progress: progress.clone() });
        self.bedrock.jobs.insert(key, Job { op, task, progress: progress.clone() });
        self.bedrock.errors.remove(&key);
        let (backend, n) = (self.bedrock.backend.clone(), self.notifier());
        self.rt.spawn(async move {
            let cancel = br::Cancel::default();
            let watch = {
                let (p, c) = (progress.clone(), cancel.clone());
                tokio::spawn(async move {
                    while !p.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                    c.cancel();
                })
            };
            let result = work(backend, progress, cancel).await;
            watch.abort();
            n.send(Msg::Bedrock(BedrockMsg::Done { key, op, result }));
        });
    }

    /// Make a new Bedrock instance called `wanted` playing `edition`, signing in to the Store with the slot the other
    /// instances use (one Microsoft account owns the game), and open its page once it is there.
    pub(super) fn bedrock_create(&mut self, wanted: &str, edition: Edition) {
        let name = unique_name(&self.bedrock.snap, wanted);
        let store = self.bedrock.snap.accounts.iter().find_map(|a| match &a.target {
            AccountTarget::Store(id) => Some(id.clone()),
            AccountTarget::Xbox(_) => None,
        });
        let label = format!("Creating {name}");
        self.bedrock.creating = Some(label.clone());
        let request = br::CreateRequest { name, selection: br::BuildSelection::Latest(edition), store };
        self.bedrock_run(uuid::Uuid::new_v4(), Op::Create, label, move |b, _, _| async move {
            b.create_instance(request).await.map(Done::Created)
        });
    }

    pub(super) fn bedrock_apply(&mut self, act: BAct) {
        let support = br::support();
        let name_of = |this: &Self, id: &InstanceId| {
            this.bedrock.status(id).map(|s| s.instance.name.clone()).unwrap_or_else(|| TAG.to_string())
        };
        match act {
            BAct::Open(id) => {
                self.bedrock.open = Some(id);
                self.page = Page::Bedrock;
            }
            BAct::Primary(id) => {
                let Some(st) = self.bedrock.status(&id) else { return };
                let next = match primary_of(&st.readiness, self.bedrock.job(&id), support) {
                    Primary::Play => Some(BAct::Play(id.clone())),
                    Primary::Download => Some(BAct::Download(id.clone())),
                    Primary::GetFromStore => Some(BAct::StorePage(edition_of(&st.instance))),
                    Primary::SignInStore => Some(BAct::SignIn(AccountTarget::Store(st.instance.store.clone()))),
                    Primary::Repair => Some(BAct::Repair(id.clone())),
                    Primary::Stop => Some(BAct::Stop(id.clone())),
                    Primary::Wait | Primary::Unavailable => None,
                };
                if let Some(a) = next {
                    self.bedrock_apply(a);
                }
            }
            BAct::Play(id) => {
                let Some(st) = self.bedrock.status(&id) else { return };
                if support == Support::Windows && matches!(st.readiness, Readiness::NeedsInstall) {
                    self.bedrock_apply(BAct::StorePage(edition_of(&st.instance)));
                    return;
                }
                // Linux: a game not downloaded yet is downloaded first, in the same job
                let download = matches!(st.readiness, Readiness::NeedsInstall);
                let gpu = crate::gpu::Choice::parse(&self.settings.gpu);
                let options = br::PlayOptions { gpu, diagnostics: false, require_xbox_online: false };
                let label = format!("Starting {}", name_of(self, &id));
                self.bedrock.force.remove(&id.0);
                self.bedrock.reports.remove(&id.0);
                self.bedrock_run(id.0, Op::Play, label, move |b, p, c| async move {
                    if download {
                        b.install(id.clone(), p.clone(), c.clone()).await?;
                    }
                    b.play(id, options, p, c).await.map(Done::Session)
                });
            }
            BAct::Stop(id) => {
                let Some(live) = self.bedrock.live.get(&id.0) else {
                    self.toast_err("This game wasn't started from this window. Close it in the game.");
                    return;
                };
                let control = live.control.clone();
                let force = self.bedrock.force.contains(&id.0);
                let n = self.notifier();
                let key = id.0;
                self.rt.spawn(async move {
                    let mode = if force { br::StopMode::Force } else { br::StopMode::Graceful };
                    let result = control.stop(mode).await;
                    n.send(Msg::Bedrock(BedrockMsg::Done {
                        key,
                        op: Op::Play,
                        result: result.map(|()| Done::Nothing).map_err(|e| BedrockError {
                            message: format!("{} Press Stop again to end it.", e.message),
                            ..e
                        }),
                    }));
                });
                // a second Stop ends the game at once
                self.bedrock.force.insert(id.0);
            }
            BAct::Download(id) => {
                let label = format!("Downloading {}", name_of(self, &id));
                self.bedrock_run(id.0, Op::Install, label, move |b, p, c| async move {
                    b.install(id, p, c).await.map(|_| Done::Nothing)
                });
            }
            BAct::SignIn(target) => {
                let key = target_key(&target);
                if self.bedrock.sign_ins.contains_key(&key) {
                    return;
                }
                let cancel = br::Cancel::default();
                self.bedrock.sign_ins.insert(key.clone(), cancel.clone());
                let (b, n) = (self.bedrock.backend.clone(), self.notifier());
                self.rt.spawn(async move {
                    let result = b.sign_in(target, Progress::new(), cancel).await;
                    n.send(Msg::Bedrock(BedrockMsg::SignedIn { key, result }));
                });
            }
            BAct::CancelSignIn(target) => {
                if let Some(c) = self.bedrock.sign_ins.get(&target_key(&target)) {
                    c.cancel();
                }
            }
            BAct::SignOut(target, mode) => {
                let (b, n) = (self.bedrock.backend.clone(), self.notifier());
                self.rt.spawn(async move {
                    let result = b.sign_out(target, mode).await;
                    n.send(Msg::Bedrock(BedrockMsg::SignedOut { result }));
                });
            }
            BAct::CancelJob(id) => {
                if let Some(j) = self.bedrock.jobs.get(&id.0) {
                    j.progress.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            BAct::Repair(id) => {
                let label = format!("Repairing {}", name_of(self, &id));
                self.bedrock_run(id.0, Op::Repair, label, move |b, p, c| async move {
                    b.repair(id, p, c).await.map(|()| Done::Nothing)
                });
            }
            BAct::Diagnose(id) => {
                let label = format!("Checking {}", name_of(self, &id));
                self.bedrock_run(id.0, Op::Diagnose, label, move |b, _, _| async move {
                    b.diagnose(id).await.map(Done::Report)
                });
            }
            BAct::Backup(id) => {
                let label = format!("Backing up {}", name_of(self, &id));
                self.bedrock_run(id.0, Op::Backup, label, move |b, _, _| async move {
                    b.back_up_worlds(id).await.map(Done::Backup)
                });
            }
            BAct::Worlds(id) => {
                let Some(path) = self.bedrock.backend.worlds_folder(&id) else { return };
                // the folder appears the first time the game starts; until then its parent is opened
                let (n, ask) = (self.notifier(), path.clone());
                self.rt.spawn_blocking(move || {
                    let open = if ask.is_dir() {
                        ask
                    } else {
                        ask.ancestors().find(|p| p.is_dir()).map(PathBuf::from).unwrap_or(ask)
                    };
                    if let Err(e) = open::that_detached(&open) {
                        n.send(Msg::Toast(format!("Couldn't open {}: {e}", open.display()), true));
                    }
                });
            }
            BAct::Delete(id) => {
                let label = format!("Deleting {}", name_of(self, &id));
                self.bedrock_run(id.0, Op::Delete, label, move |b, _, _| async move {
                    b.delete_instance(id).await.map(|()| Done::Nothing)
                });
            }
            BAct::Channel(id, edition) => {
                let n = self.notifier();
                let b = self.bedrock.backend.clone();
                self.rt.spawn(async move {
                    if let Err(e) = b.select_build(id, br::BuildSelection::Latest(edition)).await {
                        n.send(Msg::Toast(e.message, true));
                    }
                });
            }
            BAct::Logs => {
                let dir = self.dirs.base.join("bedrock").join("logs");
                let n = self.notifier();
                self.rt.spawn_blocking(move || {
                    let _ = std::fs::create_dir_all(&dir);
                    if let Err(e) = open::that_detached(&dir) {
                        n.send(Msg::Toast(format!("Couldn't open {}: {e}", dir.display()), true));
                    }
                });
            }
            BAct::Copy(text) => {
                self.ctx.copy_text(text);
                self.toast("Copied the code");
            }
            BAct::Link(url) => self.open_link(&url),
            BAct::StorePage(e) => self.open_link(&br::store_page(e)),
        }
    }

    /// The page of one Bedrock instance: its banner with the button for what is next, then how it stands, its
    /// sign-ins, its worlds, its log and where it comes from.
    pub(super) fn bedrock_ui(&mut self, ui: &mut Ui) {
        let snap = self.bedrock.snap.clone();
        let Some(st) = self.bedrock.open.as_ref().and_then(|id| snap.instances.iter().find(|s| &s.instance.id == id))
        else {
            self.page = Page::Library;
            return;
        };
        let mut act = None;
        let room = ui.available_width();
        let column = (room - ui.spacing().scroll.allocated_width()).min(super::library::FORM_COLUMN);
        ui.scope(|ui| {
            ui.set_max_width(column);
            if ui.ctx().content_rect().height() >= 640.0 {
                ui.horizontal(|ui| {
                    ui.set_height(32.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if back_link(ui, "Library").clicked() {
                        self.page = Page::Library;
                    }
                    ui.label(RichText::new("\u{203A}").size(FS_BODY).color(c_faint()));
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(RichText::new(&st.instance.name).font(super::fonts::medium(13.0))).truncate(),
                    );
                });
                ui.add_space(6.0 - ui.spacing().item_spacing.y);
            }
            self.bedrock_banner(ui, st, &mut act);
        });
        ui.add_space(16.0);
        ScrollArea::vertical().id_salt("bedrock-page").auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(column);
            ui.spacing_mut().item_spacing.y = 0.0;
            self.bedrock_body(ui, &snap, st, &mut act);
            ui.add_space(16.0);
        });
        if let Some(a) = act {
            self.bedrock_apply(a);
        }
    }

    fn bedrock_banner(&mut self, ui: &mut Ui, st: &InstanceStatus, act: &mut Option<BAct>) {
        let support = br::support();
        let id = &st.instance.id;
        let job = self.bedrock.job(id);
        let h = if ui.ctx().content_rect().height() < 640.0 { 132.0 } else { 168.0 };
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let radius = R_CARD + 4;
        let spec = cover_spec(id).with_fade_left(true);
        let (tw, th) = super::workbench::cover_size(rect.size());
        let tex = self.covers.texture(&self.ctx, &spec, tw, th);
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            p.add(card_shadow().as_shape(rect, CornerRadius::same(radius)));
            egui::Image::from_texture((tex.id(), vec2(tw as f32, th as f32)))
                .uv(super::workbench::cover_uv((tw, th), rect.size()))
                .corner_radius(radius)
                .paint_at(ui, rect);
            shade_under(p, rect, radius, 150);
            p.rect_stroke(
                rect,
                CornerRadius::same(radius),
                Stroke::new(1.0, Color32::from_white_alpha(22)),
                egui::StrokeKind::Inside,
            );
        }
        let pad = 22.0;
        let inner = rect.shrink2(vec2(pad, 18.0));
        // the buttons at the top right, the name along the bottom
        let mut right = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(inner.min, egui::pos2(inner.right(), inner.top() + 40.0)))
                .layout(Layout::right_to_left(Align::Center)),
        );
        right.spacing_mut().item_spacing.x = 8.0;
        let kebab = cover_kebab(&mut right);
        egui::Popup::menu(&kebab).gap(6.0).show(|ui| {
            super::instance_menu::menu_items(ui);
            ui.set_min_width(210.0);
            let idle = job.is_none() && !is_live(&st.readiness);
            if support == Support::Linux
                && ui.add_enabled(idle, egui::Button::new("Repair")).on_hover_text(REPAIR_HINT).clicked()
            {
                *act = Some(BAct::Repair(id.clone()));
            }
            if ui.add_enabled(job.is_none(), egui::Button::new("Diagnose")).clicked() {
                *act = Some(BAct::Diagnose(id.clone()));
            }
            if ui.button("Open worlds folder").clicked() {
                *act = Some(BAct::Worlds(id.clone()));
            }
            if ui.add_enabled(idle, egui::Button::new("Back up worlds")).clicked() {
                *act = Some(BAct::Backup(id.clone()));
            }
            ui.separator();
            if ui.add_enabled(idle, egui::Button::new("Delete\u{2026}")).clicked() {
                self.bedrock.confirm_delete = Some(id.clone());
            }
        });
        let primary = primary_of(&st.readiness, job, support);
        let label = match (primary, &st.readiness) {
            (Primary::Wait, _) if job.is_some() => "Cancel",
            (Primary::Wait, Readiness::Starting(_)) => "Starting",
            (Primary::Wait, Readiness::Stopping(_)) => "Closing",
            (Primary::Wait, _) => "Loading",
            (Primary::Stop, _) if self.bedrock.force.contains(&id.0) => "End now",
            (p, _) => p.label(),
        };
        let resp = match primary {
            Primary::Play => play_button_sized(&mut right, vec2(120.0, 40.0)),
            Primary::Stop => danger_sized(&mut right, Icon::Stop, label, vec2(110.0, 40.0)),
            Primary::Wait if job.is_some() => secondary(&mut right, label),
            Primary::Wait | Primary::Unavailable => primary_sized(&mut right, label, vec2(110.0, 40.0), false),
            _ => primary_sized(&mut right, label, vec2(0.0, 40.0), true),
        };
        if resp.clicked() {
            *act = Some(if primary == Primary::Wait { BAct::CancelJob(id.clone()) } else { BAct::Primary(id.clone()) });
        }
        let name_h = 56.0;
        let mut name = ui.new_child(
            UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(inner.left(), inner.bottom() - name_h),
                    egui::pos2(inner.right(), inner.bottom()),
                ))
                .layout(Layout::top_down(Align::Min)),
        );
        name.set_clip_rect(rect.intersect(ui.clip_rect()));
        name.spacing_mut().item_spacing.y = 4.0;
        name.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let text = RichText::new(&st.instance.name).font(super::fonts::semibold(FS_TITLE)).color(on_cover());
            ui.add(egui::Label::new(text).truncate());
        });
        name.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            tag_on_cover(ui, TAG, loader_color_on_cover());
            let line = format!("Minecraft: Bedrock Edition \u{00B7} {}", version_line(&st.instance, support));
            let text = RichText::new(line).font(super::fonts::mono(12.5)).color(Color32::from_white_alpha(215));
            ui.add(egui::Label::new(text).truncate());
        });
    }

    fn bedrock_body(&mut self, ui: &mut Ui, snap: &br::Snapshot, st: &InstanceStatus, act: &mut Option<BAct>) {
        const GAP: f32 = 14.0;
        let support = br::support();
        let id = st.instance.id.clone();
        if support == Support::Unavailable {
            notice(ui, NoticeKind::Warn, br::UNAVAILABLE, None);
            ui.add_space(GAP);
        }
        if let Some(e) = self.bedrock.errors.get(&id.0).cloned() {
            if notice(ui, NoticeKind::Error, &e, Some("Dismiss")).is_some_and(|r| r.clicked()) {
                self.bedrock.errors.remove(&id.0);
            }
            ui.add_space(GAP);
        }
        if let Some(job) = self.bedrock.jobs.get(&id.0) {
            let (op, progress) = (job.op, job.progress.clone());
            card(ui, |ui| job_card(ui, op, &progress, act, &id));
            ui.add_space(GAP);
        }
        // a sign-in waiting for the player comes first: it is what is to be done now
        for prompt in &snap.auth_prompts {
            match prompt {
                AuthPrompt::DeviceCode { profile, url, user_code, expires_at_unix }
                    if *profile == st.instance.profile =>
                {
                    card(ui, |ui| device_code_card(ui, url, user_code, *expires_at_unix, profile, act));
                    ui.add_space(GAP);
                }
                AuthPrompt::StoreWindow { store } if *store == st.instance.store => {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            spinner_sized(ui, 18.0);
                            ui.label(RichText::new("Finish signing in to the Microsoft Store").semibold());
                        });
                        ui.add_space(4.0);
                        muted(
                            ui,
                            "A Microsoft sign-in window is open. Sign in with the account that owns Minecraft for \
                             Windows; BlockForge never sees your password.",
                        );
                        ui.add_space(8.0);
                        if secondary(ui, "Cancel").clicked() {
                            *act = Some(BAct::CancelSignIn(AccountTarget::Store(store.clone())));
                        }
                    });
                    ui.add_space(GAP);
                }
                _ => {}
            }
        }
        let wide = ui.available_width() >= 760.0;
        let status = |ui: &mut Ui, this: &mut Self, act: &mut Option<BAct>| this.bedrock_status_card(ui, snap, st, act);
        let accounts =
            |ui: &mut Ui, this: &mut Self, act: &mut Option<BAct>| this.bedrock_accounts_card(ui, snap, st, act);
        if wide {
            let half = ((ui.available_width() - GAP) / 2.0).floor();
            let mut state = (&mut *self, &mut *act);
            super::library::two_columns(
                ui,
                half,
                GAP,
                &mut state,
                |ui, (this, act)| status(ui, this, act),
                |ui, (this, act)| accounts(ui, this, act),
            );
        } else {
            status(ui, self, act);
            ui.add_space(GAP);
            accounts(ui, self, act);
        }
        ui.add_space(GAP);
        if let Some(report) = self.bedrock.reports.get(&id.0).cloned() {
            card(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(RichText::new("What Diagnose found").size(FS_SECTION).semibold());
                if report.issues.is_empty() {
                    status_line(ui, Status::Good, "No problems found");
                }
                for issue in &report.issues {
                    status_line(ui, Status::Warn, issue);
                }
            });
            ui.add_space(GAP);
        }
        self.bedrock_worlds_card(ui, st, act);
        ui.add_space(GAP);
        self.bedrock_log_card(ui, &id, act);
        ui.add_space(GAP);
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("About Bedrock Edition here").size(FS_SECTION).semibold());
            });
            let line = match support {
                Support::Windows => {
                    "Minecraft for Windows from the Microsoft Store, played as it is: BlockForge finds it, starts it, \
                     and backs up its worlds. Windows signs you in."
                }
                _ => {
                    "Your own copy of Minecraft for Windows, downloaded from the Microsoft Store with your licence \
                     and played in a Windows compatibility engine, in a game profile of its own. Java mods, loaders \
                     and Java settings are not for it."
                }
            };
            ui.add(egui::Label::new(RichText::new(line).color(c_muted())).wrap());
            ui.add_space(4.0);
            if let Some(url) = credit(ui) {
                *act = Some(BAct::Link(url));
            }
        });
    }

    fn bedrock_status_card(&mut self, ui: &mut Ui, snap: &br::Snapshot, st: &InstanceStatus, act: &mut Option<BAct>) {
        let support = br::support();
        let id = &st.instance.id;
        let edition = edition_of(&st.instance);
        let job = self.bedrock.job(id);
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(RichText::new("Status").size(FS_SECTION).semibold());
            let (kind, text) = status_of(&st.readiness, job, support);
            status_line(ui, kind, &text);
            let newest = snap.builds.iter().rfind(|b| b.id.edition == edition && !b.installed);
            let installed = st.instance.installed.as_ref().map(|b| display_version(&b.version));
            let mut cells = vec![
                ("Edition", edition_name(edition).to_string()),
                ("Installed", installed.clone().unwrap_or_else(|| "\u{2014}".into())),
            ];
            if support == Support::Linux {
                let size = newest.and_then(|b| b.download_bytes).map(human_size).unwrap_or_else(|| "~2 GB".into());
                cells.push(("Download", size));
            }
            facts(ui, &cells);
            if let Some(b) = newest.filter(|_| installed.is_some() && support == Support::Linux) {
                let text = format!("Version {} is out", display_version(&b.id.version));
                if notice(ui, NoticeKind::Info, &text, Some("Update")).is_some_and(|r| r.clicked()) {
                    *act = Some(BAct::Download(id.clone()));
                }
            }
            // Release or Preview, changed while the game is closed; each keeps its own worlds
            let idle = job.is_none() && !is_live(&st.readiness);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new("Plays").color(c_muted()));
                ui.add_enabled_ui(idle && support != Support::Unavailable, |ui| {
                    let on = if edition == Edition::Release { 0 } else { 1 };
                    if let Some(i) = segmented(ui, &["Release", "Preview"], on).filter(|i| *i != on) {
                        let e = if i == 0 { Edition::Release } else { Edition::Preview };
                        *act = Some(BAct::Channel(id.clone(), e));
                    }
                });
            });
            small_muted(ui, "Preview tries what comes next. It keeps its own worlds, apart from Release's.");
        });
    }

    fn bedrock_accounts_card(&mut self, ui: &mut Ui, snap: &br::Snapshot, st: &InstanceStatus, act: &mut Option<BAct>) {
        let support = br::support();
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if support == Support::Windows {
                ui.label(RichText::new("Minecraft for Windows").size(FS_SECTION).semibold());
                let edition = edition_of(&st.instance);
                match &st.instance.installed {
                    Some(b) => {
                        status_line(ui, Status::Good, &format!("Installed: {}", display_version(&b.version)));
                    }
                    None => {
                        status_line(ui, Status::Warn, "Not installed for this Windows user");
                        if primary(ui, "Get it from the Microsoft Store").clicked() {
                            *act = Some(BAct::StorePage(edition));
                        }
                    }
                }
                muted(
                    ui,
                    "The Microsoft Store installs and updates it. In the game, sign in with Xbox to play with friends, \
                     on Realms and on servers.",
                );
                return;
            }
            ui.label(RichText::new("Sign-ins").size(FS_SECTION).semibold());
            muted(ui, "Two, and they can be different accounts.");
            ui.add_space(2.0);
            let store = AccountTarget::Store(st.instance.store.clone());
            let xbox = AccountTarget::Xbox(st.instance.profile.clone());
            for (target, title, why) in [
                (store, "Microsoft Store", "The account that owns Minecraft for Windows, to download and start it"),
                (xbox, "Xbox", "Your Xbox profile, to play online: friends, Realms, servers, Marketplace"),
            ] {
                rule(ui);
                let state = account_state(snap, &target);
                let pending = self.bedrock.sign_ins.contains_key(&target_key(&target));
                sign_in_row(ui, &target, title, why, &state, pending, act);
            }
        });
    }

    fn bedrock_worlds_card(&mut self, ui: &mut Ui, st: &InstanceStatus, act: &mut Option<BAct>) {
        let id = &st.instance.id;
        let folder = self.bedrock.backend.worlds_folder(id);
        let idle = self.bedrock.job(id).is_none() && !is_live(&st.readiness);
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let mut head = super::instance_menu::SplitRow::new(ui, egui::Id::unique("bedrock-worlds-head"), 34.0);
            head.left(ui, |ui| {
                ui.label(RichText::new("Worlds").size(FS_SECTION).semibold());
            });
            head.right(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if secondary_enabled(ui, "Back up", idle)
                    .on_hover_text("Copy every world, while the game is closed")
                    .clicked()
                {
                    *act = Some(BAct::Backup(id.clone()));
                }
                if secondary(ui, "Open folder").clicked() {
                    *act = Some(BAct::Worlds(id.clone()));
                }
            });
            head.finish(ui);
            if let Some(path) = &folder {
                path_line(ui, &path.to_string_lossy());
            }
            let note = if br::support() == Support::Windows {
                "Each player who signs in has a folder of their own, and Shared is for playing signed out."
            } else {
                "In this instance's own game profile: other Bedrock instances and other launchers never see them."
            };
            small_muted(ui, note);
            if let Some(b) = self.bedrock.backups.get(&id.0) {
                status_line(ui, Status::Good, &format!("Backed up to {}", b.display()));
            }
        });
    }

    fn bedrock_log_card(&mut self, ui: &mut Ui, id: &InstanceId, act: &mut Option<BAct>) {
        let lines: Vec<String> = self
            .bedrock
            .logs
            .get(&id.0)
            .map(|b| {
                let b = b.lock().unwrap_or_else(|e| e.into_inner());
                b.iter().skip(b.len().saturating_sub(LOG_SHOWN)).cloned().collect()
            })
            .unwrap_or_default();
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let mut head = super::instance_menu::SplitRow::new(ui, egui::Id::unique("bedrock-log-head"), 34.0);
            head.left(ui, |ui| {
                ui.label(RichText::new("Log").size(FS_SECTION).semibold());
            });
            head.right(ui, |ui| {
                if !lines.is_empty() && ghost(ui, "Copy").clicked() {
                    ui.ctx().copy_text(lines.join("\n"));
                }
                if ghost(ui, "Open logs folder").clicked() {
                    *act = Some(BAct::Logs);
                }
            });
            head.finish(ui);
            if lines.is_empty() {
                small_muted(
                    ui,
                    "The game's log shows here while it plays. Every start's full log is kept in the logs folder.",
                );
                return;
            }
            super::library::log_panel(ui, |ui| {
                ScrollArea::vertical().id_salt("bedrock-log").max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for text in &lines {
                        let line = crate::launch::LogLine { level: level_of(text), text: text.to_string() };
                        super::library::log_row(ui, &line, true);
                    }
                });
            });
        });
    }

    /// "Delete <name>?" for a Bedrock instance.
    pub(super) fn bedrock_confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(id) = self.bedrock.confirm_delete.clone() else { return };
        let Some(st) = self.bedrock.status(&id).cloned() else {
            self.bedrock.confirm_delete = None;
            return;
        };
        let live = is_live(&st.readiness);
        let (mut close, mut delete) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("bedrock-confirm-delete")).frame(super::modal_frame(ctx)).show(
            ctx,
            |ui| {
                ui.set_width(super::services::modal_width(ctx, 440.0));
                ui.label(RichText::new(format!("Delete \"{}\"?", st.instance.name)).size(FS_SECTION).semibold());
                ui.add_space(4.0);
                if live {
                    ui.label(RichText::new("Its game is running. Close it first, then delete it.").color(c_warn()));
                } else if br::support() == Support::Windows {
                    muted(
                        ui,
                        "The instance and its world backups are removed. Minecraft for Windows and its worlds stay as \
                         they are.",
                    );
                } else {
                    muted(
                        ui,
                        "Its game profile is removed with its worlds and settings. This can't be undone. The downloaded \
                         game and your Store sign-in stay for other instances.",
                    );
                }
                ui.add_space(20.0);
                match super::services::confirm_buttons(ui, "bedrock-confirm-delete", "Cancel", "Delete", true, !live) {
                    Some(true) => delete = true,
                    Some(false) => close = true,
                    None => {}
                }
            },
        );
        if modal.should_close() {
            close = true;
        }
        if delete && !live {
            self.bedrock_apply(BAct::Delete(id));
            close = true;
        }
        if close {
            self.bedrock.confirm_delete = None;
        }
    }

    /// For the screenshot tour: two Bedrock instances in memory (nothing is written), one ready to play with both
    /// sign-ins, one waiting for its Xbox code, and a game log.
    pub(super) fn shot_bedrock(&mut self, state: &str) {
        let id = |n: u128| uuid::Uuid::from_u128(n);
        let store = br::StoreId(id(0xb5));
        let first = br::BedrockInstance {
            id: InstanceId(id(0xb1)),
            name: "Bedrock survival".into(),
            profile: br::ProfileId(id(0xa1)),
            store: store.clone(),
            selection: br::BuildSelection::Latest(Edition::Release),
            installed: Some(br::BuildId {
                edition: Edition::Release,
                version: "1.26.5004.0".into(),
                package_id: "Microsoft.MinecraftUWP_1.26.5004.0_x64__8wekyb3d8bbwe".into(),
            }),
        };
        let second = br::BedrockInstance {
            id: InstanceId(id(0xb2)),
            name: "Preview".into(),
            profile: br::ProfileId(id(0xa2)),
            store: store.clone(),
            selection: br::BuildSelection::Latest(Edition::Preview),
            installed: None,
        };
        let (ready, xbox) = match state {
            "signin" => (Readiness::NeedsStoreSignIn, SignInState::SigningIn),
            _ => (
                Readiness::Ready { xbox_online_cached: true },
                SignInState::Linked { display_name: Some("Steve".into()) },
            ),
        };
        let store_state = if state == "signin" {
            SignInState::NeedsSignIn { reason: "Sign in again: the Store asked for it".into() }
        } else {
            SignInState::Linked { display_name: Some("owner@outlook.com".into()) }
        };
        let mut prompts = Vec::new();
        if state == "signin" {
            prompts.push(AuthPrompt::DeviceCode {
                profile: first.profile.clone(),
                url: "https://www.microsoft.com/link".into(),
                user_code: "BX7K-QW2P".into(),
                expires_at_unix: chrono::Utc::now().timestamp() + 14 * 60,
            });
        }
        self.bedrock.snap = Arc::new(br::Snapshot {
            instances: vec![
                InstanceStatus { instance: first.clone(), readiness: ready },
                InstanceStatus { instance: second.clone(), readiness: Readiness::NeedsInstall },
            ],
            accounts: vec![
                br::AccountStatus { target: AccountTarget::Store(store), state: store_state },
                br::AccountStatus { target: AccountTarget::Xbox(first.profile.clone()), state: xbox },
                br::AccountStatus {
                    target: AccountTarget::Xbox(second.profile.clone()),
                    state: SignInState::SignedOut,
                },
            ],
            auth_prompts: prompts,
            builds: vec![br::BuildInfo {
                id: br::BuildId { edition: Edition::Release, version: "1.26.6005.0".into(), package_id: "p".into() },
                installed: false,
                download_bytes: Some(2_090_000_000),
            }],
            refresh_in_progress: false,
        });
        let log: VecDeque<String> = [
            "[BlockForge] Engine native21 (WineGDK), checked",
            "[BlockForge] GPU: NVIDIA GeForce GTX 1650 Mobile (vkd3d-proton)",
            "[BlockForge] Profile prefix ready; worlds backed up before the new build",
            "[supervisor] game window is up after 9.4 s",
            "[game] Minecraft: Bedrock Edition 1.26.50, signed in as Steve",
            "[game] warn: Realms: this account has no subscription",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        self.bedrock.logs.insert(first.id.0, Arc::new(Mutex::new(log)));
        self.bedrock.jobs.clear();
        self.bedrock.reports.clear();
        self.bedrock.errors.clear();
        self.bedrock.backups.clear();
        self.bedrock.open = Some(first.id.clone());
        if state == "download" {
            // the Preview instance downloading its game
            let progress = Progress::new();
            *progress.stage.lock().unwrap_or_else(|e| e.into_inner()) =
                "Downloading Minecraft Preview 1.26.7022.0".into();
            progress.bytes_total.store(2_090_000_000, std::sync::atomic::Ordering::Relaxed);
            progress.bytes_done.store(640_000_000, std::sync::atomic::Ordering::Relaxed);
            self.bedrock.jobs.insert(second.id.0, Job { op: Op::Install, task: u64::MAX - 1, progress });
            self.bedrock.open = Some(second.id);
        } else if state != "signin" {
            self.bedrock.backups.insert(
                first.id.0,
                PathBuf::from("~/.local/share/blockforge/bedrock/profiles/a1/backups/release/2026-11-01_18-20-04"),
            );
        }
    }

    /// For the screenshot tour's other pages: no Bedrock instance, as on a computer that has none.
    pub(super) fn shot_no_bedrock(&mut self) {
        self.bedrock.snap = Arc::default();
        self.bedrock.open = None;
        self.bedrock.jobs.clear();
    }
}

const REPAIR_HINT: &str = "Checks the engine, the runtime and the game profile again, keeping your worlds";

/// The level of a line of Bedrock's log, from its words.
fn level_of(line: &str) -> crate::launch::Level {
    let l = line.to_ascii_lowercase();
    if l.contains("error") || l.contains("fatal") {
        crate::launch::Level::Error
    } else if l.contains("warn") {
        crate::launch::Level::Warn
    } else {
        crate::launch::Level::Info
    }
}

/// Text on a cover: white in both looks (the cover is shaded under it).
fn on_cover() -> Color32 {
    Color32::from_white_alpha(250)
}

fn loader_color_on_cover() -> Color32 {
    // the dark look's tone reads on the shaded cover in either look
    Color32::from_rgb(0x9B, 0xD3, 0x6A)
}

/// A tag on a cover: tinted glass rather than the page's tint.
fn tag_on_cover(ui: &mut Ui, text: &str, color: Color32) {
    let g = ui.painter().layout_no_wrap(text.to_string(), super::fonts::medium(11.5), color);
    let (rect, _) = ui.allocate_exact_size(vec2(g.size().x + 14.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(R_TAG), Color32::from_black_alpha(110));
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(R_TAG),
        Stroke::new(1.0, color.gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
}

/// The round menu button on a cover.
fn cover_kebab(ui: &mut Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click());
    let t = super::motion::hover(ui, resp.id, resp.hovered());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(R_CONTROL + 2), Color32::from_black_alpha(90 + (60.0 * t) as u8));
    p.rect_stroke(
        rect,
        CornerRadius::same(R_CONTROL + 2),
        Stroke::new(1.0, Color32::from_white_alpha(40)),
        egui::StrokeKind::Inside,
    );
    icons::paint(p, Icon::Kebab, rect.center(), 16.0, Color32::from_white_alpha(235));
    focus_ring(ui, &resp, rect, R_CONTROL + 2);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "More"));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("More")
}

/// A shade rising from the bottom of `rect`, so the words there read on a bright sky.
fn shade_under(p: &egui::Painter, rect: egui::Rect, radius: u8, alpha: u8) {
    let top = rect.top() + rect.height() * 0.25;
    let mut mesh = egui::Mesh::default();
    let clear = Color32::from_black_alpha(0);
    let dark = Color32::from_black_alpha(alpha);
    let i = mesh.vertices.len() as u32;
    mesh.colored_vertex(egui::pos2(rect.left(), top), clear);
    mesh.colored_vertex(egui::pos2(rect.right(), top), clear);
    mesh.colored_vertex(egui::pos2(rect.right(), rect.bottom() - radius as f32), dark);
    mesh.colored_vertex(egui::pos2(rect.left(), rect.bottom() - radius as f32), dark);
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
    p.add(mesh);
    let foot = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - radius as f32), rect.right_bottom());
    p.rect_filled(foot, CornerRadius { nw: 0, ne: 0, sw: radius, se: radius }, dark);
}

/// A job under way: what it does, how far it got, and its Cancel.
fn job_card(ui: &mut Ui, op: Op, progress: &Progress, act: &mut Option<BAct>, id: &InstanceId) {
    use std::sync::atomic::Ordering::Relaxed;
    ui.spacing_mut().item_spacing.y = 8.0;
    let stage = progress.stage.lock().map(|s| s.clone()).unwrap_or_default();
    ui.horizontal(|ui| {
        spinner_sized(ui, 18.0);
        let text = if stage.is_empty() { op.doing().to_string() } else { stage };
        ui.add(egui::Label::new(RichText::new(text).semibold()).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if secondary(ui, "Cancel").clicked() {
                *act = Some(BAct::CancelJob(id.clone()));
            }
        });
    });
    let (done, total) = (progress.bytes_done.load(Relaxed), progress.bytes_total.load(Relaxed));
    let fraction = (total > 0).then(|| (done as f64 / total as f64).clamp(0.0, 1.0) as f32);
    progress_bar(ui, fraction, 6.0);
    if total > 0 {
        let speed = progress.speed();
        let mut line = format!("{} of {}", human_size(done), human_size(total));
        if speed > 0.0 {
            line.push_str(&format!(" \u{00B7} {}/s", human_size(speed as u64)));
        }
        ui.label(RichText::new(line).font(super::fonts::mono(12.0)).color(c_muted()));
    }
}

/// The Xbox sign-in's code, large, with the page to type it at.
fn device_code_card(
    ui: &mut Ui,
    url: &str,
    code: &str,
    expires_at_unix: i64,
    profile: &br::ProfileId,
    act: &mut Option<BAct>,
) {
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(RichText::new("Sign in to Xbox").size(FS_SECTION).semibold());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        egui::Frame::new()
            .fill(c_accent_soft())
            .stroke(Stroke::new(1.0, c_accent().gamma_multiply(0.6)))
            .corner_radius(CornerRadius::same(R_CONTROL))
            .inner_margin(egui::Margin::symmetric(18, 8))
            .show(ui, |ui| {
                ui.label(RichText::new(code).size(FS_TITLE).semibold().monospace());
            });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if secondary(ui, "Copy code").clicked() {
                    *act = Some(BAct::Copy(code.to_string()));
                }
                if secondary(ui, "Open the page").clicked() {
                    *act = Some(BAct::Link(url.to_string()));
                }
                if ghost(ui, "Cancel").clicked() {
                    *act = Some(BAct::CancelSignIn(AccountTarget::Xbox(profile.clone())));
                }
            });
        });
    });
    let left = (expires_at_unix - chrono::Utc::now().timestamp()).max(0) / 60;
    ui.horizontal_top(|ui| {
        spinner_sized(ui, 18.0);
        let text = format!("Type the code at {url} and finish there. It works for {left} more minutes.");
        ui.add(egui::Label::new(RichText::new(text).color(c_muted())).wrap());
    });
}

/// One sign-in of the page: what it is, how it stands and what it is for, with the button that changes it at the
/// right.
fn sign_in_row(
    ui: &mut Ui,
    target: &AccountTarget,
    title: &str,
    why: &str,
    state: &SignInState,
    pending: bool,
    act: &mut Option<BAct>,
) {
    let (kind, text) = match state {
        SignInState::Linked { display_name: Some(n) } => (Status::Good, format!("Signed in as {n}")),
        SignInState::Linked { display_name: None } => (Status::Good, "Signed in".to_string()),
        SignInState::SigningIn => (Status::Busy, "Signing in".to_string()),
        SignInState::NeedsSignIn { reason } => (Status::Warn, reason.clone()),
        SignInState::SignedOut => (Status::Warn, "Not signed in".to_string()),
        SignInState::Unknown => (Status::Warn, "Not checked yet".to_string()),
    };
    const BUTTON_W: f32 = 104.0;
    ui.horizontal_top(|ui| {
        let words = (ui.available_width() - BUTTON_W - 12.0).max(120.0);
        ui.allocate_ui_with_layout(vec2(words, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(RichText::new(title).semibold());
            status_line(ui, kind, &text);
            ui.add(egui::Label::new(RichText::new(why).size(FS_META).color(c_muted())).wrap());
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let busy = pending || matches!(state, SignInState::SigningIn);
            if busy {
                if secondary(ui, "Cancel").clicked() {
                    *act = Some(BAct::CancelSignIn(target.clone()));
                }
            } else if matches!(state, SignInState::Linked { .. }) {
                if ghost(ui, "Sign out").clicked() {
                    *act = Some(BAct::SignOut(target.clone(), br::SignOutMode::User));
                }
            } else if primary(ui, "Sign in").clicked() {
                *act = Some(BAct::SignIn(target.clone()));
            }
        });
    });
}

/// What an error from the backend means for the player.
pub fn explain(e: &BedrockError) -> String {
    match e.kind {
        ErrorKind::NotImplemented => {
            "This part of Bedrock Edition isn't finished in this build of BlockForge yet.".to_string()
        }
        ErrorKind::Network => format!("{} Check the connection and try again.", e.message),
        ErrorKind::NoSpace => format!("{} Free some disk space and try again.", e.message),
        _ => e.message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> br::SessionId {
        br::SessionId(uuid::Uuid::nil())
    }

    #[test]
    fn the_main_button_follows_what_the_instance_needs() {
        let linux = Support::Linux;
        assert_eq!(primary_of(&Readiness::NeedsStoreSignIn, None, linux), Primary::SignInStore);
        assert_eq!(primary_of(&Readiness::NeedsInstall, None, linux), Primary::Download);
        assert_eq!(primary_of(&Readiness::NeedsInstall, None, Support::Windows), Primary::GetFromStore);
        assert_eq!(primary_of(&Readiness::Ready { xbox_online_cached: false }, None, linux), Primary::Play);
        assert_eq!(primary_of(&Readiness::Running(session()), None, linux), Primary::Stop);
        assert_eq!(primary_of(&Readiness::Starting(session()), None, linux), Primary::Wait);
        assert_eq!(primary_of(&Readiness::NeedsRepair { reason: "x".into() }, None, linux), Primary::Repair);
        assert_eq!(primary_of(&Readiness::UnsupportedPlatform, None, linux), Primary::Unavailable);
        // a job of the window's comes first: nothing is started twice
        assert_eq!(primary_of(&Readiness::Ready { xbox_online_cached: true }, Some(Op::Install), linux), Primary::Wait);
    }

    #[test]
    fn the_status_says_what_is_going_on_in_words() {
        let linux = Support::Linux;
        let (s, t) = status_of(&Readiness::Ready { xbox_online_cached: false }, None, linux);
        assert_eq!(s, Status::Good);
        assert!(t.contains("Xbox"), "{t}");
        let (_, t) = status_of(&Readiness::Ready { xbox_online_cached: false }, None, Support::Windows);
        assert_eq!(t, "Ready to play");
        let (s, t) = status_of(&Readiness::NeedsInstall, Some(Op::Install), linux);
        assert_eq!((s, t.as_str()), (Status::Busy, "Downloading the game"));
        // Play's job ends once the game is starting: the game's own state shows then
        let (_, t) = status_of(&Readiness::Running(session()), Some(Op::Play), linux);
        assert_eq!(t, "Playing");
        let (s, _) = status_of(&Readiness::NeedsRepair { reason: "prefix".into() }, None, linux);
        assert_eq!(s, Status::Error);
    }

    #[test]
    fn a_name_that_is_taken_gets_a_number() {
        let inst = |name: &str| InstanceStatus {
            instance: br::BedrockInstance {
                id: InstanceId(uuid::Uuid::new_v4()),
                name: name.into(),
                profile: br::ProfileId(uuid::Uuid::new_v4()),
                store: br::StoreId(uuid::Uuid::new_v4()),
                selection: br::BuildSelection::Latest(Edition::Release),
                installed: None,
            },
            readiness: Readiness::NeedsInstall,
        };
        let snap = br::Snapshot { instances: vec![inst("Bedrock"), inst("bedrock (2)")], ..Default::default() };
        assert_eq!(unique_name(&snap, "Bedrock"), "Bedrock (3)");
        assert_eq!(unique_name(&snap, "Other"), "Other");
        assert_eq!(version_line(&snap.instances[0].instance, Support::Linux), "Release \u{00B7} not downloaded");
    }

    #[test]
    fn store_versions_read_as_minecraft_versions() {
        assert_eq!(display_version("1.26.5004.0"), "1.26.50.4");
        assert_eq!(display_version("1.21.12001.0"), "1.21.120.1");
        assert_eq!(display_version("1.26.60.5"), "1.26.60.5");
        assert_eq!(display_version("1.26.5004.1"), "1.26.5004.1");
        assert_eq!(display_version("beta"), "beta");
    }

    #[test]
    fn log_lines_get_their_level_from_their_words() {
        assert!(matches!(level_of("[game] warn: Realms"), crate::launch::Level::Warn));
        assert!(matches!(level_of("ERROR: device lost"), crate::launch::Level::Error));
        assert!(matches!(level_of("[supervisor] up"), crate::launch::Level::Info));
    }
}
