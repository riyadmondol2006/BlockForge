//! The Convert page: pick a Bedrock world, choose the Java version to write, watch it run, read what happened.
//!
//! The engine is behind `crate::convert::backend::Backend`; this file is the page's state and its link to the window
//! (background threads, files dropped on the window, the entries on the Tools page and an instance's overview), and
//! `convert_view.rs` draws it. Nothing here exists until the page is first opened.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use egui::Ui;

use super::{App, Msg, Page};
use crate::convert::backend::{
    self, Backend, ConvertJob, Detected, DimensionSelection, Edition, Event, Summary, Target,
};
use crate::instances::Instance;
use crate::tr;

/// How long a run can go on before stopping it asks "are you sure".
pub(super) const CONFIRM_STOP_AFTER: Duration = Duration::from_secs(5);
/// Warnings kept to show; a run can say the same thing for every column.
const MAX_WARNINGS: usize = 500;
/// Rows of the table of blocks without an equivalent.
pub(super) const MAX_UNMAPPED: usize = 20;
/// The repository the footer links to.
pub(super) const CHUNKER_URL: &str = "https://github.com/HiveGamesOSS/Chunker";

/// What the background work tells the page.
pub enum ConvertMsg {
    /// A world was looked at (`seq` says which look this is the answer to).
    Detected { seq: u64, result: Result<Detected, String> },
    /// A file chooser closed.
    Picked { pick: Pick, path: Option<PathBuf> },
    /// A running conversion said something.
    Event { run: u64, event: Event },
}

/// What a file chooser was opened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    SourceFolder,
    SourceFile,
    OutputParent,
}

/// What the page asks the window to do.
pub(super) enum Act {
    Pick(Pick),
    Start,
    Open(PathBuf),
    Copy(String),
    Link(&'static str),
    BackToTools,
    BackToInstance(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Screen {
    Setup,
    Running,
    Done,
    Failed,
}

/// The instance a converted world is added to: its `saves` folder and the Minecraft version it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Dest {
    /// The instance's folder name, to go back to it.
    pub folder: String,
    pub name: String,
    pub saves: PathBuf,
    pub mc_version: String,
}

impl Dest {
    pub(super) fn of(inst: &Instance) -> Dest {
        Dest {
            folder: inst.folder.clone(),
            name: inst.name().to_string(),
            saves: inst.root.join("saves"),
            mc_version: inst.mc_version().to_string(),
        }
    }
}

/// What is known about the chosen world.
pub(super) enum Look {
    Nothing,
    Busy,
    Found(Detected),
    Unknown(String),
}

/// Whether the output folder can be written to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutputCheck {
    Ok,
    Empty,
    IsSource,
    InsideSource,
    IsFile,
    NotEmpty,
}

/// A conversion in progress.
pub(super) struct RunView {
    pub id: u64,
    pub started: Instant,
    pub cancel: Arc<AtomicBool>,
    pub stage: String,
    pub progress: f32,
    pub warnings: Vec<String>,
    /// Warnings past [`MAX_WARNINGS`], counted but not kept.
    pub more_warnings: usize,
    /// Asking whether to stop.
    pub confirm: bool,
    /// Stop was asked for and the engine has not answered yet.
    pub stopping: bool,
    pub job: ConvertJob,
}

/// How a run ended, for the window (a toast when the page is not showing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Nothing,
    Done,
    Failed,
    Cancelled,
}

pub struct ConvertState {
    pub(super) backend: Arc<dyn Backend>,
    pub(super) screen: Screen,
    pub(super) source: Option<PathBuf>,
    pub(super) look: Look,
    /// Numbers the looks at a world, so the answer to an old one is ignored.
    seq: u64,
    pub(super) picking: bool,
    /// The instance the world is added to; `None` for the standalone flow of the Tools page.
    pub(super) dest: Option<Dest>,
    pub(super) versions: Vec<String>,
    pub(super) version: String,
    pub(super) dims: Vec<(String, bool)>,
    pub(super) output: String,
    /// The player typed or chose the output, so a new world does not replace it.
    pub(super) out_edited: bool,
    out_check: Option<(String, Option<PathBuf>, OutputCheck)>,
    pub(super) run: Option<RunView>,
    run_counter: u64,
    pub(super) summary: Option<Summary>,
    pub(super) error: Option<String>,
    /// What the failed run was, for the details.
    pub(super) last_job: Option<ConvertJob>,
    pub(super) last_warnings: Vec<String>,
}

impl ConvertState {
    pub(super) fn new(backend: Arc<dyn Backend>) -> ConvertState {
        let versions = backend.target_versions(Edition::Java);
        let version = pick_version(&versions, None);
        ConvertState {
            backend,
            screen: Screen::Setup,
            source: None,
            look: Look::Nothing,
            seq: 0,
            picking: false,
            dest: None,
            versions,
            version,
            dims: Vec::new(),
            output: String::new(),
            out_edited: false,
            out_check: None,
            run: None,
            run_counter: 0,
            summary: None,
            error: None,
            last_job: None,
            last_warnings: Vec::new(),
        }
    }

    /// Back to the first screen, forgetting the world (the choices for the target stay as they were).
    pub(super) fn start_over(&mut self) {
        self.screen = Screen::Setup;
        self.source = None;
        self.look = Look::Nothing;
        self.seq += 1;
        self.out_edited = false;
        self.output.clear();
        self.out_check = None;
        self.summary = None;
        self.error = None;
        self.last_job = None;
        self.last_warnings.clear();
    }

    /// Adds the world to an instance (or, with `None`, back to the standalone flow): starts over, and the version
    /// to write is the instance's own when it can be written, else the newest.
    pub(super) fn set_dest(&mut self, dest: Option<Dest>) {
        self.start_over();
        self.version = pick_version(&self.versions, dest.as_ref().map(|d| d.mc_version.as_str()));
        self.dest = dest;
    }

    /// Where the converted world goes by default.
    fn suggested_output(&self, world_name: &str) -> Option<PathBuf> {
        match (&self.dest, &self.source) {
            (Some(dest), _) => Some(free_name(&dest.saves.join(safe_name(world_name)))),
            (None, Some(src)) => Some(default_output(src, world_name)),
            (None, None) => None,
        }
    }

    /// A world was chosen: forget the last one and number the look at this one. Returns its number.
    fn begin_look(&mut self, path: PathBuf) -> u64 {
        self.start_over();
        self.source = Some(path);
        self.look = Look::Busy;
        self.seq
    }

    /// The answer of a look at a world.
    fn apply_detected(&mut self, seq: u64, result: Result<Detected, String>) {
        if seq != self.seq || !matches!(self.look, Look::Busy) {
            return;
        }
        match result {
            // only Bedrock worlds are converted; a Java one is told so and there is no way on
            Ok(d) if d.edition != Edition::Bedrock => self.look = Look::Unknown(backend::JAVA_WORLD.to_string()),
            Ok(d) => {
                self.dims = d.dimensions.iter().map(|n| (n.clone(), true)).collect();
                if !self.out_edited {
                    if let Some(out) = self.suggested_output(&d.name) {
                        self.output = out.to_string_lossy().to_string();
                        self.out_check = None;
                    }
                }
                self.look = Look::Found(d);
            }
            Err(e) => self.look = Look::Unknown(e),
        }
    }

    pub(super) fn found(&self) -> Option<&Detected> {
        match &self.look {
            Look::Found(d) => Some(d),
            _ => None,
        }
    }

    /// Whether the output can be written to; looked at again only when the text or the world changed.
    pub(super) fn output_check(&mut self) -> OutputCheck {
        let key = (self.output.clone(), self.source.clone());
        if let Some((text, src, check)) = &self.out_check {
            if *text == key.0 && *src == key.1 {
                return *check;
            }
        }
        let check = check_output(self.source.as_deref(), &self.output);
        self.out_check = Some((key.0, key.1, check));
        check
    }

    /// The output text was changed by the player.
    pub(super) fn output_edited(&mut self) {
        self.out_edited = true;
        self.out_check = None;
    }

    /// Why a conversion cannot start now, or `None` when it can.
    pub(super) fn blocked(&mut self) -> Option<String> {
        if self.found().is_none() {
            return Some(tr!("Choose a world first."));
        }
        if self.versions.is_empty() {
            return Some(tr!("There is no version to convert to yet."));
        }
        if !self.dims.iter().any(|(_, on)| *on) {
            return Some(tr!("Choose at least one dimension."));
        }
        match self.output_check() {
            OutputCheck::Ok => None,
            OutputCheck::NotEmpty => Some(tr!("Choose a folder that is empty or does not exist yet.")),
            _ => Some(tr!("Choose where to save the converted world.")),
        }
    }

    /// The job the choices describe.
    pub(super) fn job(&mut self) -> Option<ConvertJob> {
        if self.blocked().is_some() {
            return None;
        }
        let selection = if self.dims.iter().all(|(_, on)| *on) {
            DimensionSelection::All
        } else {
            DimensionSelection::Only(self.dims.iter().filter(|(_, on)| *on).map(|(n, _)| n.clone()).collect())
        };
        Some(ConvertJob {
            input: self.source.clone()?,
            output: PathBuf::from(self.output.trim()),
            target: Target { edition: Edition::Java, version: self.version.clone() },
            dimensions: selection,
            // nothing is copied between different editions, so there is nothing to keep
            keep_unknown_nbt: false,
        })
    }

    /// The world is being converted.
    fn begin_run(&mut self, job: ConvertJob) -> (u64, Arc<AtomicBool>) {
        self.run_counter += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.run = Some(RunView {
            id: self.run_counter,
            started: Instant::now(),
            cancel: cancel.clone(),
            stage: tr!("Starting"),
            progress: 0.0,
            warnings: Vec::new(),
            more_warnings: 0,
            confirm: false,
            stopping: false,
            job: job.clone(),
        });
        self.last_job = Some(job);
        self.screen = Screen::Running;
        (self.run_counter, cancel)
    }

    /// Stop was pressed: asks first when the run has gone on for a while.
    pub(super) fn ask_stop(&mut self) {
        let Some(run) = self.run.as_mut() else { return };
        if run.started.elapsed() > CONFIRM_STOP_AFTER {
            run.confirm = true;
        } else {
            Self::stop_now(run);
        }
    }

    pub(super) fn stop_now(run: &mut RunView) {
        run.confirm = false;
        run.stopping = true;
        run.cancel.store(true, Ordering::Relaxed);
    }

    /// Something a run said.
    fn on_event(&mut self, run_id: u64, event: Event) -> Outcome {
        let Some(run) = self.run.as_mut().filter(|r| r.id == run_id) else { return Outcome::Nothing };
        match event {
            Event::Stage(s) => run.stage = s,
            Event::Progress(p) => run.progress = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 },
            Event::Warning(w) => {
                if run.warnings.len() < MAX_WARNINGS {
                    run.warnings.push(w);
                } else {
                    run.more_warnings += 1;
                }
            }
            Event::Done(mut summary) => {
                summary.unmapped_blocks.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                self.run = None;
                self.summary = Some(summary);
                self.screen = Screen::Done;
                return Outcome::Done;
            }
            Event::Failed(message) => {
                self.last_warnings = self.run.take().map(|r| r.warnings).unwrap_or_default();
                self.error = Some(message);
                self.screen = Screen::Failed;
                return Outcome::Failed;
            }
            Event::Cancelled => {
                self.run = None;
                self.screen = Screen::Setup;
                return Outcome::Cancelled;
            }
        }
        Outcome::Nothing
    }

    /// Everything a bug report needs, as text.
    pub(super) fn details(&self) -> String {
        let mut out = format!("BlockForge {} ({})\n", crate::VERSION, std::env::consts::OS);
        if let Some(job) = &self.last_job {
            out += &format!(
                "Source: {}\nOutput: {}\nTarget: {} {}\nDimensions: {:?}\n",
                job.input.display(),
                job.output.display(),
                job.target.edition.label(),
                job.target.version,
                job.dimensions
            );
        }
        if let Some(dest) = &self.dest {
            out += &format!("Instance: {} (Minecraft {})\n", dest.name, dest.mc_version);
        }
        if let Some(d) = self.found() {
            out += &format!("World: {} ({} {}, {} bytes)\n", d.name, d.edition.label(), d.version, d.size_bytes);
        }
        out += &format!("Error: {}\n", self.error.as_deref().unwrap_or("none"));
        if !self.last_warnings.is_empty() {
            out += "Warnings:\n";
            for w in self.last_warnings.iter().take(50) {
                out += &format!("  {w}\n");
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------------------
// Pure helpers

/// The version to write: `wanted` (an instance's Minecraft version) when the list has it, else the newest.
pub(super) fn pick_version(versions: &[String], wanted: Option<&str>) -> String {
    wanted
        .and_then(|w| versions.iter().find(|v| v.as_str() == w))
        .or_else(|| versions.first())
        .cloned()
        .unwrap_or_default()
}

/// A world's name made safe to use as a folder name.
pub(super) fn safe_name(name: &str) -> String {
    let cleaned: String =
        name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() { tr!("World") } else { cleaned }
}

/// Where the converted world goes by default: next to the source, called "<name> (converted)".
pub(super) fn default_output(source: &Path, world_name: &str) -> PathBuf {
    let name = if world_name.trim().is_empty() {
        source.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    } else {
        world_name.to_string()
    };
    let folder = format!("{} ({})", safe_name(&name), tr!("converted"));
    source.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.join(&folder)).unwrap_or_else(|| folder.into())
}

/// `path` with links and `..` resolved as far as the file system knows them.
fn normalized(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    match (path.parent().and_then(|p| p.canonicalize().ok()), path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

/// Whether a converted world can be written to `output` without touching `source` or what is already there.
pub(super) fn check_output(source: Option<&Path>, output: &str) -> OutputCheck {
    let text = output.trim();
    if text.is_empty() {
        return OutputCheck::Empty;
    }
    let out = Path::new(text);
    if let Some(src) = source {
        let (o, s) = (normalized(out), normalized(src));
        if o == s {
            return OutputCheck::IsSource;
        }
        if src.is_dir() && o.starts_with(&s) {
            return OutputCheck::InsideSource;
        }
    }
    if out.is_file() {
        return OutputCheck::IsFile;
    }
    match std::fs::read_dir(out).map(|mut rd| rd.next().is_some()) {
        Ok(true) => OutputCheck::NotEmpty,
        _ => OutputCheck::Ok,
    }
}

/// `path`, or `path 2`, `path 3` ... the first of them that does not exist.
pub(super) fn free_name(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let base = path.to_string_lossy().to_string();
    (2..).map(|n| PathBuf::from(format!("{base} {n}"))).find(|p| !p.exists()).unwrap_or_else(|| path.to_path_buf())
}

/// "42 s", "1 min 05 s", "2 h 03 min".
pub(super) fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min {:02} s", s / 60, s % 60),
        _ => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
    }
}

/// What the dimension called `id` is named for the player.
pub(super) fn dimension_label(id: &str) -> String {
    match id {
        "overworld" => tr!("Overworld"),
        "nether" => tr!("The Nether"),
        "end" => tr!("The End"),
        other => other.to_string(),
    }
}

/// A number the way the language writes it ("1,234,567").
pub(super) fn number(n: u64) -> String {
    crate::i18n::format_number(n.min(i64::MAX as u64) as i64, crate::i18n::selected_locale().language)
}

// ---------------------------------------------------------------------------------------------------------
// The window's side

impl App {
    /// Opens the page, making its state the first time. With `into`, the world will be added to that instance; without
    /// it, the page is the standalone converter. A running conversion is never interrupted.
    pub(super) fn open_convert(&mut self, into: Option<&Instance>) {
        self.page = Page::Convert;
        self.convert_open();
        let Some(st) = self.convert.as_mut() else { return };
        let dest = into.map(Dest::of);
        if st.screen == Screen::Running {
            if dest != st.dest {
                self.toast(tr!("A conversion is already running. It has to finish first."));
            }
        } else if dest.is_some() || st.dest.is_some() {
            st.set_dest(dest);
        }
    }

    fn convert_open(&mut self) {
        if self.convert.is_none() {
            self.convert = Some(Box::new(ConvertState::new(backend::backend())));
        }
    }

    pub(super) fn convert_ui(&mut self, ui: &mut Ui) {
        self.convert_open();
        let Some(mut st) = self.convert.take() else { return };
        let acts = st.show(ui);
        self.convert = Some(st);
        for a in acts {
            self.convert_act(a);
        }
    }

    /// A file or folder was dropped on the window: the page takes it when it is waiting for a world.
    pub(super) fn convert_handle_drop(&mut self, path: &Path) -> bool {
        if self.page != Page::Convert || self.convert.as_ref().is_none_or(|s| s.screen != Screen::Setup) {
            return false;
        }
        self.convert_look(path.to_path_buf());
        true
    }

    /// Starts looking at a world on a background thread.
    fn convert_look(&mut self, path: PathBuf) {
        let Some(st) = self.convert.as_mut() else { return };
        let seq = st.begin_look(path.clone());
        let backend = st.backend.clone();
        let n = self.notifier();
        self.rt.spawn_blocking(move || {
            let look = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| backend.detect(&path)));
            let result = look.unwrap_or_else(|_| Err(tr!("The converter stopped unexpectedly.")));
            n.send(Msg::Convert(ConvertMsg::Detected { seq, result }));
        });
    }

    pub(super) fn convert_msg(&mut self, m: ConvertMsg) {
        match m {
            ConvertMsg::Detected { seq, result } => {
                if let Some(st) = self.convert.as_mut() {
                    st.apply_detected(seq, result);
                }
            }
            ConvertMsg::Picked { pick, path } => {
                let Some(st) = self.convert.as_mut() else { return };
                st.picking = false;
                let Some(path) = path else { return };
                match pick {
                    Pick::SourceFolder | Pick::SourceFile => self.convert_look(path),
                    Pick::OutputParent => {
                        let name = st
                            .found()
                            .map(|d| d.name.clone())
                            .or_else(|| {
                                st.source.as_ref().and_then(|s| s.file_stem()).map(|s| s.to_string_lossy().to_string())
                            })
                            .unwrap_or_default();
                        let folder = format!("{} ({})", safe_name(&name), tr!("converted"));
                        st.output = path.join(folder).to_string_lossy().to_string();
                        st.output_edited();
                    }
                }
            }
            ConvertMsg::Event { run, event } => {
                let Some(st) = self.convert.as_mut() else { return };
                let outcome = st.on_event(run, event);
                let elsewhere = self.page != Page::Convert;
                match outcome {
                    Outcome::Done if elsewhere => {
                        self.toast(tr!("Your world was converted. Open the Convert page to see where it is."))
                    }
                    Outcome::Failed if elsewhere => {
                        self.toast_err(tr!("The world could not be converted. Open the Convert page for details."))
                    }
                    Outcome::Cancelled => self.toast(tr!("Conversion stopped.")),
                    _ => {}
                }
            }
        }
    }

    fn convert_act(&mut self, act: Act) {
        match act {
            Act::Pick(pick) => self.convert_pick(pick),
            Act::Start => self.convert_start(),
            Act::Open(path) => {
                // a packed world is a file: show the folder it is in
                let target = if path.is_file() { path.parent().map(Path::to_path_buf).unwrap_or(path) } else { path };
                self.open_path(&target);
            }
            Act::Copy(text) => {
                self.ctx.copy_text(text);
                self.toast(tr!("Copied"));
            }
            Act::Link(url) => self.open_link(url),
            Act::BackToTools => self.page = Page::Tools,
            Act::BackToInstance(folder) => self.open_instance(&folder),
        }
    }

    /// Opens a file chooser on a background thread.
    fn convert_pick(&mut self, pick: Pick) {
        let Some(st) = self.convert.as_mut() else { return };
        if st.picking {
            return;
        }
        st.picking = true;
        let start = match pick {
            Pick::OutputParent => Path::new(st.output.trim()).parent().map(Path::to_path_buf),
            _ => st.source.as_ref().and_then(|s| s.parent()).map(Path::to_path_buf),
        };
        let n = self.notifier();
        self.rt.spawn_blocking(move || {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = start.filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            let path = match pick {
                Pick::SourceFolder => dialog.set_title(tr!("Choose a world folder")).pick_folder(),
                Pick::SourceFile => dialog
                    .set_title(tr!("Choose a packed world"))
                    .add_filter(tr!("Packed worlds"), &["mcworld", "zip"])
                    .pick_file(),
                Pick::OutputParent => dialog.set_title(tr!("Save the converted world in")).pick_folder(),
            };
            n.send(Msg::Convert(ConvertMsg::Picked { pick, path }));
        });
    }

    /// Runs the conversion on its own thread; a second one forwards what it says to the window.
    fn convert_start(&mut self) {
        let Some(st) = self.convert.as_mut() else { return };
        let Some(job) = st.job() else { return };
        let backend = st.backend.clone();
        let (run, cancel) = st.begin_run(job.clone());
        let (tx, rx) = channel::<Event>();
        let spawned = std::thread::Builder::new().name("convert".into()).spawn(move || backend.run(job, tx, cancel));
        let n = self.notifier();
        if spawned.is_err() {
            n.send(Msg::Convert(ConvertMsg::Event {
                run,
                event: Event::Failed(tr!("The converter could not start.")),
            }));
            return;
        }
        let relay = std::thread::Builder::new().name("convert-events".into()).spawn(move || {
            let mut last_progress = Instant::now() - Duration::from_secs(1);
            let mut ended = false;
            for event in rx {
                let end = matches!(event, Event::Done(_) | Event::Failed(_) | Event::Cancelled);
                // a flood of progress reports would only make the window redraw for nothing
                if matches!(event, Event::Progress(_)) {
                    if last_progress.elapsed() < Duration::from_millis(80) {
                        continue;
                    }
                    last_progress = Instant::now();
                }
                n.send(Msg::Convert(ConvertMsg::Event { run, event }));
                if end {
                    ended = true;
                    break;
                }
            }
            if !ended {
                let event = Event::Failed(tr!("The converter stopped unexpectedly."));
                n.send(Msg::Convert(ConvertMsg::Event { run, event }));
            }
        });
        if relay.is_err() {
            if let Some(st) = self.convert.as_mut() {
                st.on_event(run, Event::Failed(tr!("The converter could not start.")));
            }
        }
    }

    /// The window is closing: a running conversion is asked to stop.
    pub(super) fn convert_on_exit(&mut self) {
        if let Some(run) = self.convert.as_ref().and_then(|s| s.run.as_ref()) {
            run.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The "Add a Bedrock world" button of an instance: opens the converter, set to save into the instance.
    pub(super) fn add_bedrock_world_button(&mut self, ui: &mut Ui, inst: &Instance) {
        let hint = tr!("Convert a Bedrock world to Java Edition and add it to this instance");
        if super::widgets::secondary(ui, &tr!("Add a Bedrock world")).on_hover_text(hint).clicked() {
            self.open_convert(Some(inst));
        }
    }

    /// The item of the side menu the page belongs to: the Library for an instance's world, else Tools.
    pub(super) fn convert_nav_page(&self) -> Page {
        if self.convert.as_ref().is_some_and(|s| s.dest.is_some()) { Page::Library } else { Page::Tools }
    }

    /// The entry on the Tools page. Returns whether its button was pressed.
    pub(super) fn convert_tool_card(&mut self, ui: &mut Ui) -> bool {
        super::tools::card_frame(
            ui,
            &tr!("Convert a Bedrock world"),
            &tr!("Turn a Bedrock Edition world into a Java Edition world"),
            &tr!("Open"),
            true,
            None,
        )
    }
}

// ---------------------------------------------------------------------------------------------------------
// Screenshots

impl App {
    /// For the screenshot tour: the page in a made-up state.
    pub(super) fn shot_convert(&mut self, variant: &str) {
        self.page = Page::Convert;
        self.convert_open();
        let Some(st) = self.convert.as_mut() else { return };
        st.fake(variant);
    }
}

impl ConvertState {
    fn fake(&mut self, variant: &str) {
        let source = PathBuf::from("/home/player/Downloads/Mace, Spear, Sword PvP Map! V4.mcworld");
        let detected = Detected {
            edition: Edition::Bedrock,
            version: "1.26.51".into(),
            name: "Mace, Spear, Sword PvP Map! V4".into(),
            dimensions: vec!["overworld".into(), "nether".into(), "end".into()],
            size_bytes: 48_300_000,
        };
        self.screen = Screen::Setup;
        self.run = None;
        self.summary = None;
        self.error = None;
        self.versions = vec!["1.21.4".into(), "1.21.1".into(), "1.20.6".into(), "1.20.4".into()];
        self.version = "1.21.4".into();
        self.dest = None;
        self.dims = detected.dimensions.iter().map(|n| (n.clone(), n != "end")).collect();
        self.output = default_output(&source, &detected.name).to_string_lossy().to_string();
        self.out_edited = true;
        self.source = Some(source.clone());
        let name = detected.name.clone();
        self.look = Look::Found(detected);
        if variant.starts_with("convert-instance") {
            let saves = PathBuf::from("/home/player/.local/share/BlockForge/instances/Survival/saves");
            self.output = saves.join(safe_name(&name)).to_string_lossy().to_string();
            self.dest =
                Some(Dest { folder: "Survival".into(), name: "Survival".into(), saves, mc_version: "1.21.4".into() });
        }
        let job = ConvertJob {
            input: source,
            output: PathBuf::from(&self.output),
            target: Target { edition: Edition::Java, version: "1.21.4".into() },
            dimensions: DimensionSelection::All,
            keep_unknown_nbt: false,
        };
        match variant {
            "convert-instance-done" => {
                self.screen = Screen::Done;
                self.summary = Some(Summary {
                    columns: 1_284,
                    unmapped_blocks: vec![("minecraft:trial_spawner".into(), 214)],
                    warnings: Vec::new(),
                    elapsed: Duration::from_secs(73),
                    output: PathBuf::from(&self.output),
                });
            }
            "convert-empty" => {
                self.source = None;
                self.look = Look::Nothing;
            }
            "convert-unknown" => {
                self.look = Look::Unknown(backend::JAVA_WORLD.to_string());
            }
            "convert-conflict" => {
                // a folder that exists and has files in it
                self.output = std::env::temp_dir().to_string_lossy().to_string();
                self.out_check = None;
            }
            "convert-stop" => {
                self.screen = Screen::Running;
                self.run = Some(RunView {
                    id: 1,
                    started: Instant::now() - Duration::from_secs(42),
                    cancel: Arc::new(AtomicBool::new(false)),
                    stage: "Converting the Overworld".into(),
                    progress: 0.46,
                    warnings: Vec::new(),
                    more_warnings: 0,
                    confirm: true,
                    stopping: false,
                    job,
                });
            }
            "convert-progress" => {
                self.screen = Screen::Running;
                self.run = Some(RunView {
                    id: 1,
                    started: Instant::now() - Duration::from_secs(42),
                    cancel: Arc::new(AtomicBool::new(false)),
                    stage: "Converting the Overworld".into(),
                    progress: 0.46,
                    warnings: vec!["Skipped a banner pattern that is not known".into(); 3],
                    more_warnings: 0,
                    confirm: false,
                    stopping: false,
                    job,
                });
            }
            "convert-done" => {
                self.screen = Screen::Done;
                let blocks = [
                    ("minecraft:trial_spawner", 214),
                    ("minecraft:heavy_core", 96),
                    ("minecraft:vault", 41),
                    ("minecraft:crafter", 12),
                    ("minecraft:decorated_pot", 5),
                ];
                self.summary = Some(Summary {
                    columns: 1_284,
                    unmapped_blocks: blocks.iter().map(|(n, c)| (n.to_string(), *c)).collect(),
                    warnings: vec![
                        "Bossbar settings of a command block were left out".into(),
                        "A written book had more pages than Java Edition allows and was cut".into(),
                        "Two signs had text in a language style Java Edition does not use".into(),
                    ],
                    elapsed: Duration::from_secs(73),
                    output: PathBuf::from(&self.output),
                });
            }
            "convert-error" => {
                self.screen = Screen::Failed;
                self.last_job = Some(job);
                self.error = Some(
                    "The world could not be read: the folder \"db\" has no CURRENT file, so it is not a Bedrock world."
                        .into(),
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::backend::NotReady;

    fn state() -> ConvertState {
        ConvertState::new(Arc::new(NotReady))
    }

    /// A backend with a Bedrock world and Java versions, for the flows.
    struct Fake;
    impl Backend for Fake {
        fn detect(&self, _p: &Path) -> Result<Detected, String> {
            Ok(Detected {
                edition: Edition::Bedrock,
                version: "1.21.0".into(),
                name: "Home".into(),
                dimensions: vec!["overworld".into(), "nether".into()],
                size_bytes: 10,
            })
        }
        fn target_versions(&self, e: Edition) -> Vec<String> {
            match e {
                Edition::Java => vec!["1.21.4".into(), "1.20.6".into()],
                Edition::Bedrock => Vec::new(),
            }
        }
        fn run(&self, _j: ConvertJob, _e: std::sync::mpsc::Sender<Event>, _c: Arc<AtomicBool>) {}
    }

    /// A backend that finds a Java world, which the page must not go on with.
    struct JavaFake;
    impl Backend for JavaFake {
        fn detect(&self, _p: &Path) -> Result<Detected, String> {
            Ok(Detected {
                edition: Edition::Java,
                version: "1.21.4".into(),
                name: "Home".into(),
                dimensions: vec!["overworld".into()],
                size_bytes: 10,
            })
        }
        fn target_versions(&self, e: Edition) -> Vec<String> {
            Fake.target_versions(e)
        }
        fn run(&self, _j: ConvertJob, _e: std::sync::mpsc::Sender<Event>, _c: Arc<AtomicBool>) {}
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bf-convert-gui-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn the_output_goes_next_to_the_world() {
        let out = default_output(Path::new("/saves/My World"), "My World");
        assert_eq!(out, Path::new("/saves/My World (converted)"));
        // an archive keeps its file's folder; a name with slashes does not escape it
        let out = default_output(Path::new("/dl/map.mcworld"), "a/b: c");
        assert_eq!(out, Path::new("/dl/a_b_ c (converted)"));
        // no name at all: the file's name
        assert_eq!(default_output(Path::new("/dl/map.mcworld"), " "), Path::new("/dl/map (converted)"));
        assert_eq!(default_output(Path::new("map"), "map"), Path::new("map (converted)"));
        assert_eq!(safe_name("..."), "World");
    }

    #[test]
    fn the_source_is_never_the_output() {
        let dir = tmp("source");
        let world = dir.join("World");
        std::fs::create_dir_all(world.join("region")).unwrap();
        let w = world.to_string_lossy().to_string();
        assert_eq!(check_output(Some(&world), &w), OutputCheck::IsSource);
        assert_eq!(check_output(Some(&world), &format!("{w}/")), OutputCheck::IsSource);
        assert_eq!(check_output(Some(&world), &format!("{w}/region/x")), OutputCheck::InsideSource);
        assert_eq!(check_output(Some(&world), &format!("{w}/../World")), OutputCheck::IsSource);
        assert_eq!(check_output(Some(&world), ""), OutputCheck::Empty);
        assert_eq!(check_output(Some(&world), "   "), OutputCheck::Empty);
        assert_eq!(check_output(Some(&world), &dir.join("New").to_string_lossy()), OutputCheck::Ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_output_must_be_empty() {
        let dir = tmp("exists");
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let text = out.to_string_lossy().to_string();
        assert_eq!(check_output(None, &text), OutputCheck::Ok, "an empty folder is fine");
        std::fs::write(out.join("level.dat"), b"x").unwrap();
        assert_eq!(check_output(None, &text), OutputCheck::NotEmpty);
        assert_eq!(check_output(None, &out.join("level.dat").to_string_lossy()), OutputCheck::IsFile);
        assert_eq!(free_name(&out), dir.join("out 2"));
        std::fs::create_dir_all(dir.join("out 2")).unwrap();
        assert_eq!(free_name(&out), dir.join("out 3"));
        assert_eq!(free_name(&dir.join("nothing")), dir.join("nothing"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_detected_world_sets_up_the_choices() {
        let mut st = ConvertState::new(Arc::new(Fake));
        let seq = st.begin_look(PathBuf::from("/saves/Home"));
        assert!(matches!(st.look, Look::Busy));
        // the answer to an older look is ignored
        st.apply_detected(seq - 1, Err("old".into()));
        assert!(matches!(st.look, Look::Busy));
        st.apply_detected(seq, Ok(Fake.detect(Path::new("/")).unwrap()));
        assert_eq!(st.version, "1.21.4", "the newest");
        assert_eq!(st.dims.len(), 2);
        assert!(st.dims.iter().all(|(_, on)| *on));
        assert_eq!(Path::new(&st.output), Path::new("/saves/Home (converted)"));
        // the output the player chose stays when another world is looked at
        st.output = "/elsewhere/x".into();
        st.output_edited();
        let seq = st.begin_look(PathBuf::from("/saves/Other"));
        assert!(st.output.is_empty(), "a new world starts without an output");
        st.apply_detected(seq, Ok(Fake.detect(Path::new("/")).unwrap()));
        assert_eq!(Path::new(&st.output), Path::new("/saves/Home (converted)"));
        // nothing to write to: no Java versions at all
        let mut none = state();
        assert!(none.versions.is_empty() && none.version.is_empty());
        assert!(none.blocked().is_some());
    }

    #[test]
    fn the_version_follows_the_instance() {
        let list: Vec<String> = ["1.21.4", "1.20.6", "1.13.2"].iter().map(|v| v.to_string()).collect();
        assert_eq!(pick_version(&list, None), "1.21.4", "the newest");
        assert_eq!(pick_version(&list, Some("1.20.6")), "1.20.6");
        assert_eq!(pick_version(&list, Some("1.13.2")), "1.13.2");
        assert_eq!(pick_version(&list, Some("1.12.2")), "1.21.4", "older than 1.13: the newest");
        assert_eq!(pick_version(&list, Some("26.3")), "1.21.4", "not in the list: the newest");
        assert_eq!(pick_version(&list, Some("")), "1.21.4");
        assert_eq!(pick_version(&[], Some("1.21.4")), "");

        let dest = |mc: &str| Dest {
            folder: "Survival".into(),
            name: "Survival".into(),
            saves: "/inst/saves".into(),
            mc_version: mc.into(),
        };
        let mut st = ConvertState::new(Arc::new(Fake));
        assert_eq!(st.version, "1.21.4");
        st.set_dest(Some(dest("1.20.6")));
        assert_eq!(st.version, "1.20.6", "the instance's version");
        st.set_dest(Some(dest("1.12.2")));
        assert_eq!(st.version, "1.21.4", "an old instance gets the newest");
        st.set_dest(Some(dest("1.20.6")));
        st.set_dest(None);
        assert_eq!(st.version, "1.21.4", "the Tools page flow is back to the newest");
        assert!(st.dest.is_none());
    }

    #[test]
    fn a_world_for_an_instance_goes_to_its_saves_with_a_free_name() {
        let dir = tmp("dest");
        let saves = dir.join("saves");
        let dest = Dest {
            folder: "Survival".into(),
            name: "Survival".into(),
            saves: saves.clone(),
            mc_version: "1.20.6".into(),
        };
        let mut st = ConvertState::new(Arc::new(Fake));
        st.set_dest(Some(dest));
        // no saves folder yet: the world's name, as it is
        let seq = st.begin_look(dir.join("dl").join("Home.mcworld"));
        st.apply_detected(seq, Ok(Fake.detect(Path::new("/")).unwrap()));
        assert_eq!(Path::new(&st.output), saves.join("Home"));
        let job = st.job().expect("a free place in the saves");
        assert_eq!(job.output, saves.join("Home"));
        assert_eq!(job.target, Target { edition: Edition::Java, version: "1.20.6".into() });
        assert!(!job.keep_unknown_nbt);
        // a world of that name is there: the next one is called "Home 2", "Home 3"
        std::fs::create_dir_all(saves.join("Home")).unwrap();
        std::fs::write(saves.join("Home").join("level.dat"), b"x").unwrap();
        let seq = st.begin_look(dir.join("dl").join("Home.mcworld"));
        st.apply_detected(seq, Ok(Fake.detect(Path::new("/")).unwrap()));
        assert_eq!(Path::new(&st.output), saves.join("Home 2"));
        assert!(st.job().is_some());
        std::fs::create_dir_all(saves.join("Home 2")).unwrap();
        let seq = st.begin_look(dir.join("dl").join("Home.mcworld"));
        st.apply_detected(seq, Ok(Fake.detect(Path::new("/")).unwrap()));
        assert_eq!(Path::new(&st.output), saves.join("Home 3"));
        // typing the name of a world that is there is refused as before
        st.output = saves.join("Home").to_string_lossy().to_string();
        st.output_edited();
        assert_eq!(st.output_check(), OutputCheck::NotEmpty);
        assert!(st.job().is_none());
        // starting over keeps the instance; the Tools page flow puts the output next to the source again
        st.start_over();
        assert!(st.dest.is_some());
        st.set_dest(None);
        let seq = st.begin_look(dir.join("dl").join("Home.mcworld"));
        st.apply_detected(seq, Ok(Fake.detect(Path::new("/")).unwrap()));
        assert_eq!(Path::new(&st.output), dir.join("dl").join("Home (converted)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_java_world_is_refused() {
        let mut st = ConvertState::new(Arc::new(JavaFake));
        let seq = st.begin_look(PathBuf::from("/saves/Home"));
        st.apply_detected(seq, JavaFake.detect(Path::new("/")));
        assert!(st.found().is_none());
        match &st.look {
            Look::Unknown(why) => assert_eq!(why, backend::JAVA_WORLD),
            _ => panic!("a Java world must not be accepted"),
        }
        assert!(st.dims.is_empty() && st.output.is_empty());
        assert!(st.blocked().is_some());
        assert!(st.job().is_none(), "no way to start");
        // the engine says the same for what it detects as Java
        assert!(backend::JAVA_WORLD.contains("Bedrock worlds to Java Edition"));
        // a Bedrock world chosen afterwards works again
        st.backend = Arc::new(Fake);
        let seq = st.begin_look(PathBuf::from("/saves/Other"));
        st.apply_detected(seq, Fake.detect(Path::new("/")));
        assert!(st.found().is_some());
    }

    #[test]
    fn a_dropped_file_that_is_no_world_is_refused() {
        let dir = tmp("drop");
        let file = dir.join("notes.txt");
        std::fs::write(&file, b"hello").unwrap();
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let mut st = ConvertState::new(Arc::new(backend::Engine));
        for path in [file, empty, dir.join("missing")] {
            let seq = st.begin_look(path.clone());
            let result = st.backend.detect(&path);
            assert!(result.is_err(), "{path:?}");
            st.apply_detected(seq, result);
            assert!(matches!(st.look, Look::Unknown(_)), "{path:?}");
            assert_eq!(st.source.as_deref(), Some(path.as_path()));
            assert!(st.blocked().is_some() && st.job().is_none());
        }
        // the engine's own answer reaches the player as it is
        let seq = st.begin_look(dir.join("notes.txt"));
        st.apply_detected(seq, Err("This does not look like a Minecraft world".into()));
        assert!(matches!(&st.look, Look::Unknown(w) if w.contains("does not look like")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_job_follows_the_choices() {
        let dir = tmp("job");
        let mut st = ConvertState::new(Arc::new(Fake));
        assert!(st.job().is_none(), "no world yet");
        let seq = st.begin_look(dir.join("Home"));
        st.apply_detected(seq, Ok(Fake.detect(Path::new("/")).unwrap()));
        let job = st.job().expect("a world and an output that is free");
        assert_eq!(job.dimensions, DimensionSelection::All);
        assert_eq!(job.target, Target { edition: Edition::Java, version: "1.21.4".into() });
        assert!(!job.keep_unknown_nbt);
        st.dims[1].1 = false;
        let job = st.job().unwrap();
        assert_eq!(job.dimensions, DimensionSelection::Only(vec!["overworld".into()]));
        assert!(!job.keep_unknown_nbt, "nothing is copied between editions");
        st.dims[0].1 = false;
        assert!(st.job().is_none(), "no dimension at all");
        assert!(st.blocked().unwrap().contains("dimension"));
        st.dims[0].1 = true;
        // into the world itself: refused
        st.output = dir.join("Home").to_string_lossy().to_string();
        st.output_edited();
        assert!(st.job().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_reports_and_ends_once() {
        let mut st = state();
        let job = ConvertJob {
            input: "/a".into(),
            output: "/b".into(),
            target: Target { edition: Edition::Java, version: "1.21.4".into() },
            dimensions: DimensionSelection::All,
            keep_unknown_nbt: false,
        };
        let (id, cancel) = st.begin_run(job);
        assert_eq!(st.screen, Screen::Running);
        // what an older run says is ignored
        assert_eq!(st.on_event(id + 1, Event::Failed("old".into())), Outcome::Nothing);
        st.on_event(id, Event::Stage("Reading".into()));
        st.on_event(id, Event::Progress(7.0));
        st.on_event(id, Event::Warning("w".into()));
        let run = st.run.as_ref().unwrap();
        assert_eq!((run.stage.as_str(), run.progress, run.warnings.len()), ("Reading", 1.0, 1));
        st.on_event(id, Event::Progress(f32::NAN));
        assert_eq!(st.run.as_ref().unwrap().progress, 0.0);
        // stopping right away needs no question; the engine is told through the flag
        st.ask_stop();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(st.run.as_ref().unwrap().stopping);
        assert_eq!(st.on_event(id, Event::Cancelled), Outcome::Cancelled);
        assert_eq!(st.screen, Screen::Setup);
        assert!(st.run.is_none());
    }

    #[test]
    fn stopping_a_long_run_asks_first() {
        let mut st = state();
        let job = ConvertJob {
            input: "/a".into(),
            output: "/b".into(),
            target: Target { edition: Edition::Java, version: "1.21.4".into() },
            dimensions: DimensionSelection::All,
            keep_unknown_nbt: false,
        };
        let (_, cancel) = st.begin_run(job);
        st.run.as_mut().unwrap().started = Instant::now() - Duration::from_secs(6);
        st.ask_stop();
        assert!(st.run.as_ref().unwrap().confirm);
        assert!(!cancel.load(Ordering::Relaxed), "not until it is confirmed");
        ConvertState::stop_now(st.run.as_mut().unwrap());
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn a_summary_lists_the_commonest_blocks_first() {
        let mut st = state();
        let job = ConvertJob {
            input: "/a".into(),
            output: "/b".into(),
            target: Target { edition: Edition::Java, version: "1.21.4".into() },
            dimensions: DimensionSelection::All,
            keep_unknown_nbt: false,
        };
        let (id, _) = st.begin_run(job);
        let summary = Summary {
            columns: 5,
            unmapped_blocks: vec![("b".into(), 1), ("a".into(), 9), ("c".into(), 9)],
            warnings: Vec::new(),
            elapsed: Duration::from_secs(1),
            output: "/b".into(),
        };
        assert_eq!(st.on_event(id, Event::Done(summary)), Outcome::Done);
        let names: Vec<_> = st.summary.as_ref().unwrap().unmapped_blocks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a", "c", "b"]);
        assert_eq!(st.screen, Screen::Done);
    }

    #[test]
    fn a_failure_keeps_what_it_said() {
        let mut st = state();
        let job = ConvertJob {
            input: "/a".into(),
            output: "/b".into(),
            target: Target { edition: Edition::Java, version: "1.21.4".into() },
            dimensions: DimensionSelection::All,
            keep_unknown_nbt: false,
        };
        let (id, _) = st.begin_run(job);
        st.on_event(id, Event::Warning("odd".into()));
        assert_eq!(st.on_event(id, Event::Failed("it broke".into())), Outcome::Failed);
        assert_eq!(st.screen, Screen::Failed);
        let text = st.details();
        assert!(text.contains("it broke") && text.contains("odd") && text.contains("/a") && text.contains("1.21.4"));
    }

    #[test]
    fn times_and_numbers_read_well() {
        assert_eq!(format_elapsed(Duration::from_secs(7)), "7 s");
        assert_eq!(format_elapsed(Duration::from_secs(65)), "1 min 05 s");
        assert_eq!(format_elapsed(Duration::from_secs(7380)), "2 h 03 min");
        assert_eq!(number(1_234_567), "1,234,567");
        assert_eq!(dimension_label("nether"), "The Nether");
        assert_eq!(dimension_label("twilight"), "twilight");
    }
}
