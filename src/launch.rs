//! Turning an instance and an account into a running game: everything between "the player pressed Play" and "the game
//! has closed". It follows the Java edition's `Instance.launch`, `prepareForLaunch` and `MCLauncher` step by step:
//! checking the files and the Java, unpacking natives, choosing the game's memory, writing the first-start graphics
//! settings, building the command line (with the class cache, an argument file for Windows, the wrapper command and
//! Quick Play), running the pre-launch and post-exit commands, and cleaning up afterwards.
//!
//! The pieces live in `launch/`: `memory` (how much memory and what kind of computer), `javapick` (which Java),
//! `command` (the command line), `jvmargs`, `classcache`, `gameoptions`, `lwjgl`, `legacy`, `commands`, `prompts`
//! (questions and warnings as data), `warmer` (reading files ahead) and `fractureiser` (the mod scan).

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

pub mod classcache;
pub mod command;
pub mod commands;
pub mod fractureiser;
pub mod gamelog;
pub mod gameoptions;
pub mod javapick;
pub mod jvmargs;
pub mod legacy;
pub mod lwjgl;
pub mod memory;
pub mod prompts;
pub mod settings;
pub mod warmer;

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
}

impl Prepared {
    /// The command line with the player's token and id hidden, safe to show or save.
    pub fn display(&self) -> String {
        let mut s = format!("{} {}", self.program.display(), self.args.join(" "));
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
    let mut renewed = current;
    msauth::refresh(&mut renewed).await?;
    let mut all = accounts::load(dirs);
    match all.iter_mut().find(|a| a.uuid == renewed.uuid) {
        Some(slot) => *slot = renewed.clone(),
        None => all.push(renewed.clone()),
    }
    accounts::save(dirs, &all)?;
    Ok(renewed)
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

/// The account chosen in the launcher.
pub fn selected_account<'a>(settings: &Settings, all: &'a [Account]) -> Option<&'a Account> {
    settings.last_account.as_ref().and_then(|id| all.iter().find(|a| &a.uuid == id)).or(all.first())
}

/// Whether the "Who's playing?" question should be shown when the launcher starts: the player asked to be asked each
/// time, or no account is chosen yet (the Java edition's `WhoIsPlayingDialog.askIfNeeded`).
pub fn should_ask_who_is_playing(settings: &Settings, accounts: &[Account]) -> bool {
    settings.ask_player_on_startup || selected_account(settings, accounts).is_none()
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
    let profile_fut = async {
        if ls.auto_game_options && !gameoptions::has_own_options(&instance.root) {
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
    let name =
        extras.offline_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| account.name().to_string());
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
        total_ram_mb: memory::total_ram_mb().max(0),
        logging_config_exists: logging_ok,
    };
    let assembled = command::assemble(&input);
    notes.extend(assembled.notes.iter().cloned());
    let mut jvm = assembled.options;

    // a saved class-data archive makes the second start of the same game noticeably faster
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

    if !ls.environment_variables.is_empty() {
        notes.push(format!("Injecting environment variables: {:?}", ls.environment_variables));
    }
    let mut env: Vec<(String, String)> = ls.environment_variables.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    env.sort();

    let dashed_uuid = account.uuid.clone();
    Ok(Prepared {
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
        pre_launch,
        post_exit,
        instance: instance.clone(),
        launch_settings: ls,
        class_cache: applied.pending,
        pre_done: AtomicBool::new(false),
        post_done: AtomicBool::new(false),
        cleaned: AtomicBool::new(false),
        _busy: busy,
    })
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

pub struct Running {
    pub child: tokio::process::Child,
    /// Game output, one line at a time, ending when the game closes both its output streams.
    pub lines: UnboundedReceiver<String>,
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
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
    }
    Ok(Running { child, lines: rx })
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
    fn substitutes_known_names_only() {
        use crate::launch::command::substitute;
        use std::collections::HashMap;
        let mut v = HashMap::new();
        v.insert("auth_player_name", "Steve".to_string());
        assert_eq!(substitute("--username ${auth_player_name} ${nope}", &v), "--username Steve ${nope}");
    }
}
