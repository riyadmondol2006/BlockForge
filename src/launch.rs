//! Turning an instance and an account into a running game: everything between "the player pressed Play" and "the game
//! has closed". It follows the Java edition's `Instance.launch`, `prepareForLaunch` and `MCLauncher` step by step:
//! checking the files and the Java, unpacking natives, choosing the game's memory, writing the first-start graphics
//! settings, building the command line (with the class cache, an argument file for Windows, the wrapper command and
//! Quick Play), running the pre-launch and post-exit commands, and cleaning up afterwards.
//!
//! The pieces live in `launch/`: `memory` (how much memory and what kind of computer), `javapick` (which Java),
//! `command` (the command line), `jvmargs`, `classcache`, `gameoptions`, `lwjgl`, `legacy`, `commands`, `prompts`
//! (questions and warnings as data), `warmer` (reading files ahead), `fractureiser` (the mod scan), and the helpers:
//! `boost` (fast start), `lite` (the smoothness helper) and `lowend` (Low-end mode).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::accounts::{self, Account};
use crate::config::Settings;
use crate::install::{self, Progress};
use crate::instances::Instance;
use crate::java;
use crate::model::{Features, VersionJson};
use crate::msauth;
use crate::paths::Dirs;
use crate::platform;

pub mod boost;
pub mod classcache;
pub mod command;
pub mod commands;
pub mod fractureiser;
pub mod gamelog;
pub mod gameoptions;
pub mod javapick;
pub mod jvmargs;
pub mod legacy;
pub mod lite;
pub mod lowend;
pub mod lwjgl;
pub mod memory;
pub mod prompts;
pub mod settings;
pub mod warmer;
pub mod workers;

use prompts::{LaunchProblem, Prompt};
use settings::{LaunchSettings, QuickPlay};

/// What to do once the game is up, beyond the instance's own Quick Play setting. These win over it for this one start
/// (the Java edition's "Continue" button on the home page opens one world just this once).
#[derive(Default, Clone, Debug)]
pub struct LaunchOptions {
    /// Join this server on start (`host` or `host:port`).
    pub server: Option<String>,
    /// Open this world on start.
    pub world: Option<String>,
}

/// More things a start can be told. `prepare` uses the defaults.
#[derive(Default, Clone, Debug)]
pub struct LaunchExtras {
    /// Join this Realm on start.
    pub realm: Option<String>,
    /// Play under this name instead of the account's (the Java edition's offline launch asks for one).
    pub offline_name: Option<String>,
    /// The screen's size, for "maximise Minecraft"; found by asking the system when not given.
    pub screen_size: Option<(u32, u32)>,
}

pub struct Prepared {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    natives_dir: PathBuf,
    lwjgl_natives_dir: Option<PathBuf>,
    custom_jar: Option<PathBuf>,
    secrets: Vec<String>,
    /// Hides the sign-in token in the game's output (old versions print it as their "Session ID"), so it never
    /// reaches the log view, the console, a copy or an upload.
    censor: gamelog::LineCensor,
    /// What was decided on the way, for the log.
    pub notes: Vec<String>,
    /// The same worries as data a window can show (a toast, a dialog).
    pub warnings: Vec<Prompt>,
    /// How long each step of getting ready took.
    pub timings: Vec<(&'static str, Duration)>,
    /// The memory this start was given and why.
    pub memory: memory::MemoryDecision,
    /// The Java that runs the game.
    pub java: javapick::JavaPick,
    /// The game is started through the legacy wrapper (Minecraft 1.5.2 and older).
    pub legacy: bool,
    /// The game was started with the fast-start helper (see `boost`), so a quick failure may be its doing.
    pub fast_start: bool,
    /// The game was started with the smoothness helper (see `lite`), the same way.
    pub lite: bool,
    /// The game's world-generation threads get a lower priority while it runs (see `workers`).
    pub lower_workers: bool,
    /// The commands to run before and after, if commands are switched on.
    pub pre_launch: Option<String>,
    pub post_exit: Option<String>,
    instance: Instance,
    launch_settings: LaunchSettings,
    class_cache: Option<classcache::Pending>,
    pre_done: AtomicBool,
    post_done: AtomicBool,
    cleaned: AtomicBool,
    /// While the game is being started and runs, the file warmer keeps off the disk.
    _busy: warmer::BusyGuard,
    /// While the game is being started and runs, another game of the instance leaves its settings file alone.
    _in_use: lowend::InUse,
}

impl Prepared {
    /// The command line with the player's token and id hidden, safe to show or save.
    pub fn display(&self) -> String {
        let mut parts = vec![quote_argument(&self.program.display().to_string())];
        parts.extend(self.args.iter().map(|a| quote_argument(a)));
        let mut s = parts.join(" ");
        for secret in self.secrets.iter().filter(|s| s.len() > 3) {
            s = s.replace(secret.as_str(), "REDACTED");
        }
        s
    }

    /// "1234 ms (files 1200 ms, natives 12 ms)", the Java edition's one line on how long getting ready took, leaving out
    /// steps that took no time worth mentioning.
    pub fn timings_summary(&self) -> String {
        let total: Duration = self.timings.iter().map(|(_, d)| *d).sum();
        let parts: Vec<String> = self
            .timings
            .iter()
            .filter(|(_, d)| d.as_millis() >= 1)
            .map(|(n, d)| format!("{n} {} ms", d.as_millis()))
            .collect();
        if parts.is_empty() {
            format!("{} ms", total.as_millis())
        } else {
            format!("{} ms ({})", total.as_millis(), parts.join(", "))
        }
    }

    /// Runs the pre-launch command, if the player has commands switched on and one set. `spawn` does this itself when it
    /// has not been done, so calling this first (off the window's thread) is optional. A command that fails stops the
    /// start.
    pub async fn run_pre_launch(&self) -> Result<()> {
        let Some(cmd) = self.pre_launch.as_deref() else { return Ok(()) };
        if self.pre_done.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        match commands::run(&self.instance, cmd, &self.launch_settings, &self.java).await {
            Ok(out) => {
                tracing_log(&out.lines);
                Ok(())
            }
            Err(e) => Err(LaunchProblem { prompt: prompts::command_failed(Some(&e.message)) }.into()),
        }
    }

    /// Runs the post-exit command. Returns its failure as text (the start already happened, so this is only reported).
    pub async fn run_post_exit(&self) -> Option<String> {
        let cmd = self.post_exit.as_deref()?;
        if self.post_done.swap(true, Ordering::SeqCst) {
            return None;
        }
        commands::run(&self.instance, cmd, &self.launch_settings, &self.java).await.err().map(|e| e.message)
    }

    /// The game has closed (or never started): delete the temporary natives, start building the class cache, and run the
    /// post-exit command if that wasn't done yet.
    pub fn cleanup(&self) {
        if self.cleaned.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = std::fs::remove_dir_all(&self.natives_dir);
        if let Some(d) = &self.lwjgl_natives_dir {
            let _ = std::fs::remove_dir_all(d);
        }
        if let Some(j) = &self.custom_jar {
            let _ = std::fs::remove_file(j);
        }
        // if this start wrote down its classes, make the class cache now that the game has closed
        classcache::game_exited(self.class_cache.clone());
        if let Some(cmd) = self.post_exit.as_deref() {
            if !self.post_done.swap(true, Ordering::SeqCst) {
                let _ = commands::run_blocking(&self.instance, cmd, &self.launch_settings, &self.java);
            }
        }
    }
}

/// Command output goes nowhere by itself; kept as a hook so a window could show it.
fn tracing_log(_lines: &[String]) {}

/// Only one renewal of a Microsoft login runs at a time: Microsoft hands out a new refresh token each time, so two
/// renewals at once could leave the saved one useless.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Renew the Microsoft login if it ran out, and save the new tokens.
pub async fn fresh_account(dirs: &Dirs, account: &Account) -> Result<Account> {
    refresh_if_expiring(dirs, account, 120).await
}

/// Renew the login now if its token runs out within `seconds`, and save the new tokens. The renewal takes several
/// seconds (five requests to Microsoft, Xbox and Minecraft in a row), so the window does it ahead of time and
/// pressing Play finds a fresh token waiting.
pub async fn refresh_if_expiring(dirs: &Dirs, account: &Account, seconds: i64) -> Result<Account> {
    if !account.needs_refresh_within(seconds) {
        return Ok(account.clone());
    }
    let _only_one = REFRESH_LOCK.lock().await;
    // a renewal that finished while this one waited has saved newer tokens already
    let current = accounts::load(dirs).into_iter().find(|a| a.uuid == account.uuid).unwrap_or_else(|| account.clone());
    if !current.needs_refresh_within(seconds) {
        return Ok(current);
    }
    let mut renewed = current.clone();
    if let Err(e) = msauth::refresh(&mut renewed).await {
        // what the attempt learnt is kept: a rotated refresh token, or that Microsoft wants a new sign-in
        let learnt = renewed.must_login != current.must_login
            || renewed.oauth_token.as_ref().map(|o| &o.refresh_token)
                != current.oauth_token.as_ref().map(|o| &o.refresh_token);
        if learnt {
            save_account(dirs, &renewed)?;
        }
        // with no connection the game still starts for singleplayer with the saved sign-in, the way the official
        // launcher does; servers will turn it away until it is renewed
        if msauth::is_offline_error(&e) && !renewed.must_login {
            crate::logs::warn(format!(
                "Couldn't renew the sign-in of {} (no connection); starting with the saved one, so only singleplayer will work",
                renewed.name()
            ));
            return Ok(renewed);
        }
        return Err(e);
    }
    save_account(dirs, &renewed)?;
    Ok(renewed)
}

/// Put `account` in place of the saved account with its id (or add it), keeping every other account as it is on
/// disk.
fn save_account(dirs: &Dirs, account: &Account) -> Result<()> {
    let mut all = accounts::load(dirs);
    match all.iter_mut().find(|a| a.uuid == account.uuid) {
        Some(slot) => *slot = account.clone(),
        None => all.push(account.clone()),
    }
    accounts::save(dirs, &all)
}

/// Which account an instance is played with: the one it names (`launcher.account`), else the one chosen in the launcher
/// (`lastAccount`), else the first. None when there are no accounts at all.
pub fn account_for(dirs: &Dirs, settings: &Settings, instance: &Instance) -> Option<Account> {
    let all = accounts::load(dirs);
    let ls = LaunchSettings::resolve(settings, instance);
    if let Some(name) = &ls.account {
        if let Some(a) =
            all.iter().find(|a| a.name().eq_ignore_ascii_case(name) || a.username.eq_ignore_ascii_case(name))
        {
            return Some(a.clone());
        }
    }
    selected_account(settings, &all).cloned()
}

/// Whether games of this instance start with the fast-start helper: the instance's own switch if it has one,
/// otherwise the launcher-wide setting (see `boost`).
pub fn fast_start_enabled(settings: &Settings, instance: &Instance) -> bool {
    boost::enabled(settings, instance)
}

/// Whether games of this instance start with the smoothness helper: the instance's own switch if it has one,
/// otherwise the launcher-wide setting (see `lite`).
pub fn lite_enabled(settings: &Settings, instance: &Instance) -> bool {
    lite::enabled(settings, instance)
}

/// The Low-end mode of this instance: its own choice if it has one, otherwise the launcher-wide setting (see
/// `lowend`).
pub fn low_end_mode(settings: &Settings, instance: &Instance) -> lowend::Mode {
    lowend::mode(settings, instance)
}

/// The account chosen in the launcher.
pub fn selected_account<'a>(settings: &Settings, all: &'a [Account]) -> Option<&'a Account> {
    settings.last_account.as_ref().and_then(|id| all.iter().find(|a| &a.uuid == id)).or(all.first())
}

/// Split a line of Java options the way a shell would, so `-Dname="a b"` stays one argument.
pub fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// One argument as it has to be written so [`split_args`] (or a shell) reads it back as one argument: in quotes when it
/// is empty or has spaces or quotes in it. Fabric's `-DFabricMcEmu= net.minecraft.client.main.Main ` is one such.
pub fn quote_argument(argument: &str) -> String {
    let plain = !argument.is_empty() && !argument.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'');
    if plain {
        argument.to_string()
    } else if argument.contains('"') {
        format!("'{argument}'")
    } else {
        format!("\"{argument}\"")
    }
}

pub fn executable_in_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// Everything between "the player pressed Play" and "start the process": make sure the files are there, unpack
/// natives, and build the command line. Uses the instance's own Quick Play setting plus whatever `options` says.
pub async fn prepare(
    dirs: &Dirs,
    settings: &Settings,
    instance: &Instance,
    account: &Account,
    progress: &Arc<Progress>,
    options: &LaunchOptions,
) -> Result<Prepared> {
    prepare_ext(dirs, settings, instance, account, progress, options, &LaunchExtras::default()).await
}

/// `prepare` with the extras (a Realm, an offline player name, the screen size).
pub async fn prepare_ext(
    dirs: &Dirs,
    settings: &Settings,
    instance: &Instance,
    account: &Account,
    progress: &Arc<Progress>,
    options: &LaunchOptions,
    extras: &LaunchExtras,
) -> Result<Prepared> {
    // reading game files in the background would only get in the way of starting the game now
    warmer::stop_for_launch();
    let busy = warmer::BusyGuard::new();
    let mut timings: Vec<(&'static str, Duration)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut warnings: Vec<Prompt> = Vec::new();
    let t0 = Instant::now();

    let ls = LaunchSettings::resolve(settings, instance);
    let plan = lwjgl::LwjglPlan::new(dirs, ls.enable_arm_support);
    // ARM computers get replacement LWJGL libraries when the launcher's settings say so
    let version: VersionJson = plan.apply(&instance.file.version);
    let legacy_lwjgl = plan.should_use_legacy_lwjgl(instance.mc_version());
    let legacy = legacy::uses_legacy_launch(instance, ls.disable_legacy_launching);

    // libraries nobody can download (a pack's own files that went missing) can't be fixed by trying
    let features = Features::default();
    let unreachable = version.libraries.iter().filter(|l| l.applies(&features)).any(|l| {
        l.downloads
            .as_ref()
            .and_then(|d| d.artifact.as_ref())
            .is_some_and(|a| a.url.is_empty() && a.path.as_ref().is_some_and(|p| !dirs.libraries.join(p).exists()))
    });
    if unreachable {
        return Err(LaunchProblem { prompt: prompts::missing_libraries() }.into());
    }

    // Java, the game's files and the computer's graphics card don't depend on each other, so they are fetched side by
    // side: when Mojang's Java has to be downloaded, its biggest file alone can take longer than all the libraries
    let first_pick = javapick::pick(dirs, &ls, &version);
    let runtime_fut = async {
        match &first_pick.component {
            Some(c) if first_pick.runtime_missing => java::ensure_runtime(dirs, c, progress).await.map(|_| ()),
            _ => Ok(()),
        }
    };
    let files_fut = install::ensure_game_files(dirs, &version, progress, false);
    let low_end = lowend::Mode::parse(&ls.low_end_mode);
    let profile_fut = async {
        // the computer's profile also tells Low-end mode on automatic whether this computer is a slow one
        if (ls.auto_game_options && !gameoptions::has_own_options(&instance.root)) || low_end == lowend::Mode::Auto {
            let d = dirs.clone();
            tokio::task::spawn_blocking(move || Some(memory::PcProfile::get(&d))).await.ok().flatten()
        } else {
            None
        }
    };
    let (runtime, files, profile) = tokio::join!(runtime_fut, files_fut, profile_fut);
    files.context("getting the game files")?;
    if let Err(e) = runtime {
        // not fatal when a Java is configured elsewhere
        notes.push(format!("Couldn't get Mojang's Java ({e:#}); using the configured Java instead"));
    }
    timings.push(("files", t0.elapsed()));

    let t = Instant::now();
    let java_pick = javapick::pick(dirs, &ls, &version);
    for r in &java_pick.reasons {
        notes.push(format!("Java: {r}"));
    }
    notes.push(match &java_pick.home {
        Some(h) => format!(
            "Java: {} (version {})",
            h.display(),
            java_pick.major.map(|m| m.to_string()).unwrap_or_else(|| "unknown".into())
        ),
        None => "Java: whatever `java` is on the PATH".to_string(),
    });
    for w in &java_pick.warnings {
        notes.push(format!("Warning: Java: {w}"));
        warnings.push(prompts::java_problem(w));
    }
    timings.push(("java", t.elapsed()));

    // ---- natives
    let t = Instant::now();
    let natives_dir = dirs.temp.join(format!("natives-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&natives_dir)?;
    let skipped = lwjgl::extract_natives(dirs, &version, &natives_dir, ls.use_system_glfw, ls.use_system_openal)?;
    for s in skipped {
        notes.push(format!("Warning: {s}"));
    }
    let legacy_assets = install::map_legacy_assets(dirs, &version, &instance.root)?;
    timings.push(("natives", t.elapsed()));

    let mut lwjgl_natives_dir = None;
    if legacy_lwjgl {
        let t = Instant::now();
        if let Some(library) = plan.versions.legacy_lwjgl_library(&lwjgl::classifier()) {
            let dir = dirs.temp.join(format!("lwjgl-natives-{}", uuid::Uuid::new_v4().simple()));
            lwjgl::prepare_legacy_lwjgl(dirs, &library, &dir, progress)
                .await
                .context("getting the replacement LWJGL")?;
            lwjgl_natives_dir = Some(dir);
        }
        timings.push(("legacy lwjgl", t.elapsed()));
    }

    // ---- a modpack's own game jar is combined with Mojang's
    let mut custom_jar = None;
    let modpack_jar = instance.root.join("bin").join("modpack.jar");
    if modpack_jar.exists() {
        let t = Instant::now();
        let out = instance.root.join("bin").join("minecraft.jar");
        let _ = std::fs::remove_file(&out);
        combine_jars(&install::client_jar(dirs, version.game_version()), &modpack_jar, &out)
            .context("Failed to combine jars into custom minecraft.jar")?;
        custom_jar = Some(out);
        timings.push(("minecraft.jar", t.elapsed()));
    }

    // ---- mods that look like malware stop the start
    if ls.scan_mods_on_launch {
        let t = Instant::now();
        let (d, files) = (dirs.clone(), fractureiser::files_to_scan(&instance.root));
        let infected = tokio::task::spawn_blocking(move || fractureiser::scan(&d, &files)).await?;
        timings.push(("mod scan", t.elapsed()));
        if !infected.is_empty() {
            return Err(LaunchProblem { prompt: prompts::mods_infected(&infected) }.into());
        }
    }

    // ---- memory, and the first start's graphics settings
    let t = Instant::now();
    let decision = memory::decide(instance, settings, java_pick.home.as_deref());
    for w in &decision.warnings {
        notes.push(format!("Warning: {w}"));
    }
    if decision.is_short() {
        if let Some(first) = decision.warnings.first() {
            warnings.push(prompts::memory_short(first));
        }
    }
    notes.push(if decision.automatic {
        format!(
            "Memory: {} MB, picked automatically for {} (it would like {} MB; this computer can spare up to {} MB)",
            decision.heap_mb,
            memory::describe_game(&decision.game),
            decision.auto.ideal_mb,
            decision.auto.ceiling_mb
        )
    } else {
        format!("Memory: {} MB, as set in the settings", decision.heap_mb)
    });
    if !decision.automatic {
        if let Some(w) = memory::availability_warning(decision.heap_mb, memory::available_mb()) {
            notes.push(format!("Warning: {w}"));
        }
    }
    if ls.auto_game_options && !gameoptions::has_own_options(&instance.root) {
        if let Some(profile) = &profile {
            let jar = install::client_jar(dirs, version.game_version());
            match gameoptions::write_if_missing(&instance.root, profile.tier, instance.mc_version(), Some(&jar)) {
                Ok(Some(written)) => {
                    let shown: Vec<String> = written.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    notes.push(format!(
                        "First start: graphics settings picked for this computer ({} tier): {{{}}}",
                        profile.tier.name(),
                        shown.join(", ")
                    ));
                }
                Ok(None) => {}
                Err(e) => notes.push(format!("Warning: Couldn't write the first start graphics settings: {e:#}")),
            }
        }
    }
    // Low-end mode: after the first-start choice, so a file written just now is lowered too
    let slow = match &profile {
        Some(p) if low_end == lowend::Mode::Auto => lowend::looks_weak(&lowend::Computer {
            cores: p.cores,
            total_mb: p.total_mb.max(0),
            gpu: p.gpu,
            spinning_disk: crate::disk::is_spinning(settings, &dirs.base),
        }),
        _ => None,
    };
    let low_end_on = low_end.active(slow.is_some());
    let (in_use, others_running) = lowend::InUse::claim(&instance.root);
    let outcome = if others_running {
        if low_end_on || !lowend::Backup::load(&instance.root).is_empty() {
            lowend::Outcome::Skipped(
                "another game of this instance is running, so its settings file is left as it is".to_string(),
            )
        } else {
            lowend::Outcome::Nothing
        }
    } else {
        let minor = memory::minor_version(Some(instance.mc_version()));
        let presets =
            minor >= 21 && gameoptions::has_graphics_presets(&install::client_jar(dirs, version.game_version()));
        lowend::apply(&instance.root, minor, presets, low_end_on)
    };
    notes.extend(outcome.note());
    timings.push(("memory and options", t.elapsed()));

    // ---- notes the Java edition writes to its log around a start
    if let Some((kind, v)) = instance.loader() {
        notes.push(format!("Loader: {kind} {v}"));
    }
    if platform::os() == platform::Os::Linux
        && version.libraries.iter().any(|l| l.name.starts_with("org.lwjgl.lwjgl:lwjgl:2"))
        && !executable_in_path("xrandr")
    {
        notes.push("Warning: This Minecraft version needs the \"xrandr\" program on Linux, but it wasn't found. If the game crashes on start, install xrandr (x11-xserver-utils on Debian/Ubuntu, xrandr on Fedora/Arch).".to_string());
    }
    if instance.file.version.java_version.is_some()
        && !ls.use_java_provided_by_minecraft
        && ls.java_runtime_override.is_none()
    {
        notes.push("Warning: The Use Java Provided By Minecraft option has been disabled. If you're experiencing crashes, please enable this option.".to_string());
    }
    if legacy::should_use_legacy_launch(instance.mc_version())
        && ls.disable_legacy_launching
        && instance.file.version.kind == "release"
    {
        notes.push(
            "Warning: Legacy launching disabled. If you have issues with Minecraft, please enable this setting again"
                .to_string(),
        );
    }

    // ---- the wrapper that starts the oldest versions
    if legacy {
        legacy::ensure_jar(dirs)?;
    }

    // ---- who is playing
    let t = Instant::now();
    // playing under another name means playing offline as that player: sending a Microsoft account's token and id
    // under a name that isn't theirs only gets the game turned away by servers ("Failed to verify username")
    let offline_as = offline_player(extras.offline_name.as_deref())?;
    let account = offline_as.as_ref().unwrap_or(account);
    let name = account.name().to_string();
    let uuid = account.uuid_nodashes();
    let token = account.access_token.clone().unwrap_or_else(|| "0".into());
    let player = command::Player {
        name: name.clone(),
        uuid: uuid.clone(),
        access_token: token.clone(),
        user_type: if account.offline { "legacy".into() } else { "msa".into() },
    };
    let assets_id =
        version.assets.clone().or_else(|| version.asset_index.as_ref().map(|a| a.id.clone())).unwrap_or_default();
    let game_assets = legacy_assets.clone().unwrap_or_else(|| dirs.assets.join("virtual").join(&assets_id));

    // the window: the player's size, or the whole screen
    let window = if ls.maximise_minecraft {
        extras.screen_size.or_else(settings::screen_size).unwrap_or((ls.window_width, ls.window_height))
    } else {
        (ls.window_width, ls.window_height)
    };

    // one-off Quick Play wins over the instance's own
    let mut quick = ls.quick_play.clone();
    if options.server.is_some() || options.world.is_some() || extras.realm.is_some() {
        quick = QuickPlay {
            server_address: options.server.clone().filter(|s| !s.is_empty()),
            world_name: options.world.clone().filter(|s| !s.is_empty()),
            realm_id: extras.realm.clone().filter(|s| !s.is_empty()),
        };
    }

    let logging_ok = version
        .logging
        .as_ref()
        .and_then(|l| l.client.as_ref())
        .is_some_and(|c| dirs.assets.join("log_configs").join(&c.file.id).exists());
    let input = command::Input {
        dirs,
        ls: &ls,
        instance,
        version: &version,
        player: &player,
        java_major: java_pick.major,
        natives_dir: &natives_dir,
        lwjgl_natives_dir: lwjgl_natives_dir.as_deref(),
        game_assets,
        heap_mb: decision.heap_mb,
        window,
        quick_play: quick,
        legacy,
        os: platform::os(),
        cores: memory::cores(),
        available_ram_mb: memory::available_mb().max(0),
        total_ram_mb: memory::total_ram_mb().max(0),
        logging_config_exists: logging_ok,
    };
    let assembled = command::assemble(&input);
    notes.extend(assembled.notes.iter().cloned());
    let mut jvm = assembled.options;

    // the fast-start helper (a Java agent), unless it is switched off for this instance or for all of them
    let mut fast_start = false;
    if ls.fast_start {
        match boost::ensure_jar(dirs) {
            Ok(jar) => {
                let key = warmer::safe_name(instance);
                let spinning = crate::disk::is_spinning(settings, &dirs.base);
                let options =
                    boost::options(&boost::class_list(dirs, &key), &boost::setup_key(&instance.root), spinning);
                jvm.insert(0, boost::java_argument(&jar, &options));
                fast_start = true;
            }
            // not worth stopping the game for
            Err(e) => {
                notes.push(format!("Warning: Fast start is off for this start, its file couldn't be used: {e:#}"))
            }
        }
    }

    // the smoothness helper (a second Java agent) goes after it, with the same switches
    let mut lite_on = false;
    if ls.lite {
        match lite::prepare(dirs, instance, &ls.lite_off, low_end_on, &jvm) {
            Ok((lite::Added::Argument(at, argument), more)) => {
                notes.extend(more);
                jvm.insert(at.min(jvm.len()), argument);
                lite_on = true;
            }
            Ok((lite::Added::PlayersOwn, _)) => notes.push(
                "Smoothness helper: your own Java arguments start it already, so it isn't added a second time".into(),
            ),
            Ok((lite::Added::UnusablePath(path), _)) => notes.push(format!(
                "Warning: The smoothness helper is off for this start: Java can't load it from a folder whose path \
                 has a = or , in it ({})",
                path.display()
            )),
            // not worth stopping the game for
            Err(e) => notes.push(format!(
                "Warning: The smoothness helper is off for this start, its file couldn't be written: {e:#}"
            )),
        }
    }
    notes.push(helpers_note(fast_start, lite_on, low_end, low_end_on, slow.as_deref()));
    let lower_workers = workers::wanted(lite_on, memory::cores());
    if lower_workers {
        notes.push(format!(
            "Smoothness helper: Minecraft's world-generation threads run at a lower priority ({} processor threads)",
            memory::cores()
        ));
    }

    // a saved class-data archive makes the second start of the same game noticeably faster (after the helpers: on
    // Java 25 the archive has to be made for a game started with agents)
    let user_args = jvmargs::split(&ls.java_arguments);
    let java_binary = java_pick.binary.to_string_lossy().to_string();
    let applied = classcache::apply(
        dirs,
        &mut jvm,
        &java_binary,
        java_pick.home.as_deref(),
        ls.class_data_cache,
        java_pick.major,
        &user_args,
        &warmer::safe_name(instance),
    );
    if let Some(m) = &applied.message {
        notes.push(m.clone());
    }

    // ---- put the command together
    let mut full: Vec<String> = Vec::new();
    if platform::os() == platform::Os::Linux && ls.enable_feral_gamemode && executable_in_path("gamemoderun") {
        full.push("gamemoderun".into());
    }
    full.push(java_binary);
    let java_index = full.len() - 1;
    // Windows can't start a program with a command line over 32767 characters, which big modpacks reach; Java 9 and
    // newer can read their options from a file instead. Only the options before the main class go in the file, so the
    // account details that follow it are never written to disk.
    let mut whole = full.clone();
    whole.extend(jvm.iter().cloned());
    whole.push(assembled.main_class.clone());
    whole.extend(assembled.game.iter().cloned());
    if jvmargs::needs_argument_file(&whole, platform::os() == platform::Os::Windows, java_pick.major) {
        let file = natives_dir.join("java-options.txt");
        match std::fs::write(&file, jvmargs::to_argument_file(&jvm)) {
            Ok(()) => {
                notes.push(format!(
                    "The command line is too long for Windows, so the Java options are passed in {}",
                    file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                ));
                jvm = vec![format!("@{}", file.display())];
            }
            Err(e) => {
                notes.push(format!("Warning: Couldn't write the Java options file, trying the long command line: {e}"))
            }
        }
    }
    let _ = java_index;
    full.extend(jvm);
    full.push(assembled.main_class);
    full.extend(assembled.game);

    // commands only apply when the player switched them on
    let pre_launch = ls.enable_commands.then(|| ls.pre_launch_command.clone()).flatten();
    let post_exit = ls.enable_commands.then(|| ls.post_exit_command.clone()).flatten();
    if ls.enable_commands {
        if let Some(wrapper) = ls.wrapper_command.as_deref() {
            full = commands::wrap_arguments(wrapper, full);
        }
    }
    let (program, args) = full.split_first().map(|(p, a)| (PathBuf::from(p), a.to_vec())).context("empty command")?;
    timings.push(("command", t.elapsed()));

    // on a hard disk the game's own scattered reads are the slow part of a start; one sweep over its files first
    // is far quicker (nothing on a fast disk, or when the game was played a moment ago)
    let t = Instant::now();
    let swept = {
        let (d, st, inst) = (dirs.clone(), settings.clone(), instance.clone());
        let spinning = crate::disk::is_spinning(settings, &dirs.base);
        if spinning {
            progress.stage("Reading the game's files from the disk");
        }
        tokio::task::spawn_blocking(move || crate::warmer::prefetch_for_launch(&d, &st, &inst)).await.unwrap_or(0)
    };
    if swept > 0 {
        timings.push(("disk sweep", t.elapsed()));
    }

    if !ls.environment_variables.is_empty() {
        notes.push(format!("Injecting environment variables: {:?}", ls.environment_variables));
    }
    let mut env: Vec<(String, String)> = ls.environment_variables.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    // the graphics card the player chose, for computers with two (the player's own variables still win)
    let gpu_choice = crate::gpu::Choice::parse(&ls.gpu);
    if gpu_choice != crate::gpu::Choice::Auto && crate::gpu::supported() {
        let chosen = gpu_choice.clone();
        let gpus = tokio::task::spawn_blocking(crate::gpu::detect).await.unwrap_or_default();
        let effect = crate::gpu::effect(&chosen, &gpus);
        if let Some(name) = &effect.gpu {
            notes.push(format!("Graphics card: {name}"));
        }
        if let Some(note) = effect.note {
            notes.push(note);
        }
        for (k, v) in effect.env {
            if !env.iter().any(|(have, _)| *have == k) {
                env.push((k, v));
            }
        }
    }
    #[cfg(windows)]
    crate::gpu::apply_windows_preference(dirs, &java_pick.binary, &gpu_choice);
    env.sort();

    let dashed_uuid = account.uuid.clone();
    let censor = gamelog::LineCensor::new(&[], "", Some(token.as_str()).filter(|t| *t != "0"), false);
    Ok(Prepared {
        censor,
        program,
        args,
        cwd: instance.root.clone(),
        env,
        natives_dir,
        lwjgl_natives_dir,
        custom_jar,
        secrets: vec![token, uuid, dashed_uuid, name].into_iter().filter(|s| s != "0" && !s.is_empty()).collect(),
        notes,
        warnings,
        timings,
        memory: decision,
        java: java_pick,
        legacy,
        fast_start,
        lite: lite_on,
        lower_workers,
        pre_launch,
        post_exit,
        instance: instance.clone(),
        launch_settings: ls,
        class_cache: applied.pending,
        pre_done: AtomicBool::new(false),
        post_done: AtomicBool::new(false),
        cleaned: AtomicBool::new(false),
        _busy: busy,
        _in_use: in_use,
    })
}

/// The log line that says which helpers this start uses.
pub fn helpers_note(
    fast_start: bool,
    lite: bool,
    low_end: lowend::Mode,
    low_end_on: bool,
    slow: Option<&str>,
) -> String {
    let on_off = |b: bool| if b { "on" } else { "off" };
    let low = match (low_end, slow) {
        (lowend::Mode::Auto, Some(why)) => format!("automatic, on (this computer looks slow: {why})"),
        (lowend::Mode::Auto, None) => "automatic, off (this computer doesn't look slow)".to_string(),
        _ => on_off(low_end_on).to_string(),
    };
    format!("Helpers: Fast start {}, Smoothness helper {}, Low-end mode {low}", on_off(fast_start), on_off(lite))
}

/// Joins Mojang's game jar and a modpack's into one: the modpack's files win, and `META-INF` is left out (it carries
/// the signatures that no longer fit). The Java edition's `Utils.combineJars`.
pub fn combine_jars(main: &Path, add: &Path, out: &Path) -> Result<()> {
    use std::collections::HashSet;
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::fs::File::create(out)?);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut added: HashSet<String> = HashSet::new();
    for jar in [add, main] {
        let mut archive =
            zip::ZipArchive::new(std::fs::File::open(jar).with_context(|| format!("opening {}", jar.display()))?)?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let Ok(name) = entry.name().map(|n| n.to_string()) else { continue };
            if name.contains("META-INF") || added.contains(&name) {
                continue;
            }
            added.insert(name.clone());
            if entry.is_dir() {
                writer.add_directory(name, options)?;
            } else {
                writer.start_file(name, options)?;
                std::io::copy(&mut entry, &mut writer)?;
            }
        }
    }
    writer.flush()?;
    writer.finish()?;
    Ok(())
}

/// The game's exit code, or for a game that was ended by a signal (a crash in the graphics driver makes Java abort)
/// `128 + signal`, the way a shell reports it: such an end has no code of its own and used to count as a clean exit,
/// so the player was never told the game had crashed.
pub fn exit_code(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return Some(128 + signal);
        }
    }
    status.code()
}

/// The offline player to start the game as when a name to play under was given (`--offline-name`), with that
/// name's own offline id and no token; `None` when no name was given. A name Minecraft wouldn't take is refused.
fn offline_player(name: Option<&str>) -> Result<Option<Account>> {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if accounts::valid_offline_name(n) => Ok(Some(Account::offline(n))),
        Some(n) => anyhow::bail!("\"{n}\" can't be a player name: use 3 to 16 letters, numbers or underscores"),
        None => Ok(None),
    }
}

/// The longest line of game output kept whole; the rest of a longer one is cut off (a mod printing megabytes on one
/// line would otherwise fill the memory).
const MAX_LINE_BYTES: usize = 64 * 1024;

/// Send the game's output on, one line at a time, with the sign-in token hidden. The bytes are read as they come:
/// on Windows Java writes in the system's code page, not UTF-8, and a line that isn't valid UTF-8 (a path with
/// `José` in it) used to end the reading for good, losing everything after it, crash reports included.
async fn forward_lines<R: tokio::io::AsyncRead + Unpin>(
    stream: R,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    censor: gamelog::LineCensor,
) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            buf.pop();
        }
        let line = censor.apply(&String::from_utf8_lossy(&buf));
        if tx.send(line).is_err() {
            break;
        }
    }
}

/// Read up to and including the next `\n` into `buf`, keeping at most `max` bytes of it (the rest of a longer line
/// is read and dropped). Returns how many bytes were read, 0 at the end.
async fn read_line_capped<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<usize> {
    let mut total = 0;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(total);
        }
        let (chunk, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (&available[..=i], true),
            None => (available, false),
        };
        let room = max.saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = chunk.len();
        total += used;
        reader.consume(used);
        if done {
            return Ok(total);
        }
    }
}

pub struct Running {
    pub child: tokio::process::Child,
    /// Game output, one line at a time, ending when the game closes both its output streams.
    pub lines: UnboundedReceiver<String>,
    /// Keeps the game's world-generation threads at a lower priority while it runs (see `workers`).
    _workers: Option<workers::Watcher>,
}

impl Running {
    /// Stop the game and everything it started.
    pub async fn kill_tree(&mut self) {
        if let Some(pid) = self.child.id() {
            #[cfg(unix)]
            {
                let _ = crate::platform::command("kill").args(["-KILL", "--", &format!("-{pid}")]).status();
            }
            #[cfg(windows)]
            {
                let _ = crate::platform::command("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).status();
            }
        }
        let _ = self.child.kill().await;
    }
}

pub fn spawn(prepared: &Prepared) -> Result<Running> {
    // the pre-launch command runs first, and a failure stops the start (the Java edition does the same)
    if let Some(cmd) = prepared.pre_launch.as_deref() {
        if !prepared.pre_done.swap(true, Ordering::SeqCst) {
            if let Err(e) = commands::run_blocking(&prepared.instance, cmd, &prepared.launch_settings, &prepared.java) {
                return Err(LaunchProblem { prompt: prompts::command_failed(Some(&e.message)) }.into());
            }
        }
    }
    let mut cmd = crate::platform::tokio_command(&prepared.program);
    cmd.args(&prepared.args)
        .current_dir(&prepared.cwd)
        // any _JAVA_OPTIONS would override the memory this launcher worked out
        .env_remove("_JAVA_OPTIONS")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(false);
    // its own process group, so "kill" reaches the game even when a wrapper (gamemoderun, prime-run) started it
    #[cfg(unix)]
    cmd.process_group(0);
    for (k, v) in &prepared.env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().with_context(|| format!("starting {}", prepared.program.display()))?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    if let Some(out) = child.stdout.take() {
        tokio::spawn(forward_lines(out, tx.clone(), prepared.censor.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(forward_lines(err, tx, prepared.censor.clone()));
    }
    let workers = if prepared.lower_workers { child.id().and_then(workers::Watcher::start) } else { None };
    Ok(Running { child, lines: rx, _workers: workers })
}

/// Sweep natives folders that an earlier run left behind (a crash, a killed launcher).
pub fn clean_stale_temp(dirs: &Dirs) {
    if let Ok(rd) = std::fs::read_dir(&dirs.temp) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|a| a > Duration::from_secs(3600));
            if (n.starts_with("natives-") || n.starts_with("lwjgl-natives-") || n.starts_with("loader-")) && old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    // worlds the converter unpacked in a run that crashed or was killed (a running conversion keeps writing)
    if let Ok(rd) = std::fs::read_dir(dirs.temp.join("convert")) {
        for e in rd.flatten() {
            let stale = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|a| a > Duration::from_secs(24 * 3600));
            if e.file_name().to_string_lossy().starts_with("blockforge-") && stale {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

pub fn instance_log_dir(instance: &Instance) -> &Path {
    &instance.root
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub text: String,
    pub level: Level,
}

/// Mojang's logging setup writes each message as a little XML event spread over several lines. This puts them
/// back together as plain `[12:00:00] [thread/INFO] (logger): message` lines.
#[derive(Default)]
pub struct LogAssembler {
    event: Option<String>,
}

fn attr<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = header.find(&key)? + key.len();
    let end = header[start..].find('"')?;
    Some(&header[start..start + end])
}

fn cdata<'a>(body: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}><![CDATA[");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&format!("]]></{tag}>"))?;
    Some(&body[start..start + end])
}

fn guess_level(line: &str) -> Level {
    let l = line.to_ascii_uppercase();
    if l.contains("/ERROR]") || l.contains("EXCEPTION") || l.contains("FATAL") || l.starts_with("ERROR") {
        Level::Error
    } else if l.contains("/WARN]") || l.starts_with("WARN") {
        Level::Warn
    } else if l.contains("/DEBUG]") {
        Level::Debug
    } else {
        Level::Info
    }
}

impl LogAssembler {
    pub fn push(&mut self, line: &str) -> Option<LogLine> {
        if self.event.is_none() {
            if line.trim_start().starts_with("<log4j:Event") {
                self.event = Some(String::new());
            } else {
                return Some(LogLine { level: guess_level(line), text: line.to_string() });
            }
        }
        let buf = self.event.as_mut().unwrap();
        buf.push_str(line);
        buf.push('\n');
        if !line.contains("</log4j:Event>") {
            return None;
        }
        let event = self.event.take().unwrap();
        let level_name = attr(&event, "level").unwrap_or("INFO");
        let level = match level_name {
            "ERROR" | "FATAL" => Level::Error,
            "WARN" => Level::Warn,
            "DEBUG" | "TRACE" => Level::Debug,
            _ => Level::Info,
        };
        let time = attr(&event, "timestamp")
            .and_then(|t| t.parse::<i64>().ok())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
            .unwrap_or_default();
        let mut text = format!(
            "[{time}] [{}/{level_name}] ({}): {}",
            attr(&event, "thread").unwrap_or("?"),
            attr(&event, "logger").unwrap_or("?"),
            cdata(&event, "log4j:Message").unwrap_or("").trim_end()
        );
        if let Some(t) = cdata(&event, "log4j:Throwable") {
            text.push('\n');
            text.push_str(t.trim_end());
        }
        Some(LogLine { text, level })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_a_shell() {
        assert_eq!(split_args(r#"-Xms1G -Dname="a b" 'c d'"#), vec!["-Xms1G", "-Dname=a b", "c d"]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn quoted_arguments_split_back_the_same() {
        let args = [
            "-Xmx2G",
            "-DFabricMcEmu= net.minecraft.client.main.Main ",
            "",
            r"C:\Program Files\Java\bin\java.exe",
            r#"say "hi""#,
            "it's",
        ];
        let line = args.iter().map(|a| quote_argument(a)).collect::<Vec<_>>().join(" ");
        assert_eq!(split_args(&line), args);
        assert_eq!(quote_argument("-Xmx2G"), "-Xmx2G");
    }

    #[test]
    fn rebuilds_log4j_events() {
        let mut a = LogAssembler::default();
        assert!(
            a.push(r#"<log4j:Event logger="ave" timestamp="1792089606582" level="WARN" thread="Client thread">"#)
                .is_none()
        );
        assert!(a.push("<log4j:Message><![CDATA[first").is_none());
        assert!(a.push("second]]></log4j:Message>").is_none());
        let done = a.push("</log4j:Event>").unwrap();
        assert_eq!(done.level, Level::Warn);
        assert!(done.text.contains("[Client thread/WARN] (ave): first\nsecond"));
        assert_eq!(a.push("plain line").unwrap().text, "plain line");
    }

    #[test]
    fn the_log_says_which_helpers_are_on() {
        use lowend::Mode;
        assert_eq!(
            helpers_note(true, true, Mode::Auto, true, Some("4 processor threads")),
            "Helpers: Fast start on, Smoothness helper on, Low-end mode automatic, on (this computer looks slow: 4 processor threads)"
        );
        assert!(
            helpers_note(false, true, Mode::Auto, false, None)
                .ends_with("Low-end mode automatic, off (this computer doesn't look slow)")
        );
        assert_eq!(
            helpers_note(false, false, Mode::Off, false, None),
            "Helpers: Fast start off, Smoothness helper off, Low-end mode off"
        );
    }

    #[test]
    fn substitutes_known_names_only() {
        use crate::launch::command::substitute;
        use std::collections::HashMap;
        let mut v = HashMap::new();
        v.insert("auth_player_name", "Steve".to_string());
        assert_eq!(substitute("--username ${auth_player_name} ${nope}", &v), "--username Steve ${nope}");
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    async fn lines_of(bytes: &[u8], token: Option<&str>) -> Vec<String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        forward_lines(bytes, tx, gamelog::LineCensor::new(&[], "", token, false)).await;
        let mut out = Vec::new();
        while let Ok(l) = rx.try_recv() {
            out.push(l);
        }
        out
    }

    #[tokio::test]
    async fn a_line_that_is_not_utf8_doesnt_stop_the_log() {
        // Java on Windows writes in the system code page: "José" in Windows-1252, then the crash that matters
        let mut bytes = b"[main] Loading\r\n".to_vec();
        bytes.extend_from_slice(b"C:\\Users\\Jos\xe9\\AppData\r\n");
        bytes.extend_from_slice(b"java.lang.OutOfMemoryError: Java heap space\n");
        bytes.extend_from_slice(b"last line without an ending");
        let lines = lines_of(&bytes, None).await;
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert_eq!(lines[0], "[main] Loading");
        assert!(lines[1].starts_with("C:\\Users\\Jos") && lines[1].ends_with("AppData"));
        assert_eq!(lines[2], "java.lang.OutOfMemoryError: Java heap space");
        assert_eq!(lines[3], "last line without an ending");
    }

    #[tokio::test]
    async fn the_sign_in_token_never_reaches_the_log() {
        let token = "eyJhbGciOiJIUzI1NiJ9.secret-token.value";
        let text = format!("[Client thread/INFO]: (Session ID is token:{token}:0123abcd)\nnormal line\n");
        let lines = lines_of(text.as_bytes(), Some(token)).await;
        assert!(!lines.iter().any(|l| l.contains(token)), "{lines:?}");
        assert!(lines[0].contains("**ACCESSTOKEN**"));
        assert_eq!(lines[1], "normal line");
        // an offline player's "0" is not a secret and must not blank out every zero
        let lines = lines_of(b"version 1.20.1 build 0\n", Some("0")).await;
        assert_eq!(lines[0], "version 1.20.1 build 0");
    }

    #[tokio::test]
    async fn a_gigantic_line_is_cut_and_the_next_one_still_arrives() {
        let mut bytes = vec![b'x'; MAX_LINE_BYTES * 5];
        bytes.extend_from_slice(b"\nafter\n");
        let lines = lines_of(&bytes, None).await;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), MAX_LINE_BYTES);
        assert_eq!(lines[1], "after");
    }

    #[tokio::test]
    async fn empty_output_and_blank_lines() {
        assert!(lines_of(b"", None).await.is_empty());
        assert_eq!(lines_of(b"\n\r\n\n", None).await, vec!["", "", ""]);
    }
}

#[cfg(test)]
mod offline_name_tests {
    use super::*;

    #[test]
    fn playing_under_another_name_is_playing_offline_as_that_player() {
        assert!(offline_player(None).unwrap().is_none());
        assert!(offline_player(Some("   ")).unwrap().is_none());
        let p = offline_player(Some(" Alex ")).unwrap().unwrap();
        assert_eq!(p.name(), "Alex");
        assert!(p.offline, "never the Microsoft account's token under a name that isn't theirs");
        assert_eq!(p.uuid, accounts::offline_uuid("Alex"));
        for bad in ["Steve Jobs", "ab", "seventeen_letters", "Ünïcode"] {
            assert!(offline_player(Some(bad)).is_err(), "{bad}");
        }
    }
}

#[cfg(all(test, unix))]
mod exit_code_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn a_game_killed_by_a_signal_counts_as_a_crash_not_a_clean_exit() {
        // what Java does after a crash in native code: abort, signal 6
        assert_eq!(exit_code(&std::process::ExitStatus::from_raw(6)), Some(134));
        // a segmentation fault
        assert_eq!(exit_code(&std::process::ExitStatus::from_raw(11)), Some(139));
        // ordinary exits keep their own code
        assert_eq!(exit_code(&std::process::ExitStatus::from_raw(0)), Some(0));
        assert_eq!(exit_code(&std::process::ExitStatus::from_raw(1 << 8)), Some(1));
    }
}
