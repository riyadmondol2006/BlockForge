//! Horizon: Minecraft: Java Edition rewritten in Rust by the Horizon project (https://github.com/horizon-rust/horizon,
//! MIT licence), an unofficial fan project not affiliated with Mojang or Microsoft. BlockForge plays it as an
//! instance of its own: it downloads Horizon's signed release, prepares its resources from BlockForge's own copy of
//! the Minecraft version it pins, and starts it through Horizon's Launch API (docs/api/launch.md in their repository).
//!
//! Everything of Horizon lives under `horizon/` in BlockForge's data folder and in the instance's own folder. The
//! shared Minecraft files are only read, and nothing of the official launcher (`~/.minecraft`) or of the Horizon
//! launcher (`~/.local/share/horizon`) is touched.
//!
//! The pieces: `release` (finding, checking and unpacking a release into `horizon/builds/<tag>/`), `prepare` (making
//! a build's resources with its `horizon-prepare`) and `start` (the game's command line and the account it is
//! handed).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;

pub mod policy;
mod prepare;
pub mod release;
mod start;

#[cfg(test)]
mod real_test;

use crate::accounts::Account;
use crate::config::Settings;
use crate::install::Progress;
use crate::instances::Instance;
use crate::launch::{LaunchOptions, Running};
use crate::paths::Dirs;

/// The field of an instance's file that says which game it plays (`launcher.engine`).
pub const ENGINE_KEY: &str = "engine";
/// Its value for Horizon. Instances without the field play Minecraft: Java Edition.
pub const ENGINE: &str = "horizon";

/// The project's name, as it calls itself.
pub const NAME: &str = "Horizon";
/// One line on what it is.
pub const TAGLINE: &str = "Minecraft: Java Edition rewritten in Rust";
/// Who makes it, and that it is unofficial: shown wherever Horizon is offered.
pub const CREDIT: &str = "Horizon is made by the Horizon project and its contributors, and shared under the MIT \
                          licence. It is an unofficial fan project, not affiliated with Mojang or Microsoft. \
                          BlockForge only downloads and starts it.";
pub const WEBSITE: &str = "https://playhorizon.dev";
pub const REPOSITORY: &str = "https://github.com/horizon-rust/horizon";
/// The Minecraft version the current Horizon builds pin, until a build says otherwise (its research-lock.json).
pub const MINECRAFT_VERSION: &str = "26.3";

/// Whether Horizon publishes a build for this computer (Linux x86-64, macOS on Apple Silicon, Windows x64).
pub fn supported() -> bool {
    matches!(
        (std::env::consts::OS, std::env::consts::ARCH),
        ("linux", "x86_64") | ("macos", "aarch64") | ("windows", "x86_64")
    )
}

/// Whether `inst` plays Horizon.
pub fn is_horizon(inst: &Instance) -> bool {
    inst.file.launcher.extras.get(ENGINE_KEY).and_then(|v| v.as_str()) == Some(ENGINE)
}

/// Make `inst` a Horizon instance (the caller saves it).
pub fn mark(inst: &mut Instance) {
    let l = &mut inst.file.launcher;
    l.extras.insert(ENGINE_KEY.into(), serde_json::Value::String(ENGINE.into()));
    l.pack = NAME.into();
    l.description = format!("{NAME}: {TAGLINE} ({})", inst.file.version.game_version());
}

/// Why `account` can't play Horizon, if it can't: Horizon starts only for a Microsoft account that owns Minecraft:
/// Java Edition, as its own launcher does.
pub fn account_problem(account: &Account) -> Option<String> {
    if account.offline || account.access_token.as_deref().unwrap_or("").is_empty() {
        Some(format!(
            "{NAME} plays only with a Microsoft account that owns Minecraft: Java Edition. Add one in Accounts, then \
             press Play again."
        ))
    } else {
        None
    }
}

/// Only one install or preparation of Horizon at a time: two at once (a Create and a Play, two Plays) would unpack
/// the same build or prepare into the same folder.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A Horizon build installed in BlockForge's data folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    /// The release's tag (`v0.2.97`).
    pub tag: String,
    /// The Minecraft version it plays (its research-lock.json).
    pub minecraft: String,
    /// Whether its resources are prepared.
    pub prepared: bool,
}

/// The newest installed build, if any (no network).
pub fn installed_build(dirs: &Dirs) -> Option<Build> {
    let (tag, dir) = release::installed(dirs).into_iter().next()?;
    let lock = prepare::read_lock(&dir).ok()?;
    Some(Build { tag, minecraft: lock.version, prepared: prepare::is_prepared(&dir) })
}

fn unsupported() -> anyhow::Error {
    anyhow::anyhow!(
        "{NAME} makes builds only for Linux on x86-64, macOS on Apple Silicon and Windows on x64, not for this \
         computer"
    )
}

/// Make a new Horizon instance called `name`: install the newest build, the Minecraft version it pins with its
/// Java, and prepare the build's resources, then write the instance.
///
/// Once the instance is written it is kept and returned whatever happens next: when preparing fails or is
/// cancelled, `progress` gets a note saying so, and Play prepares what isn't prepared yet. Builds are never moved or
/// removed here (a Horizon game may be running from one); Play does that.
pub async fn create_instance(dirs: &Dirs, name: &str, progress: &Arc<Progress>) -> Result<Instance> {
    if !supported() {
        return Err(unsupported());
    }
    let _busy = BUSY.lock().await;
    let mut notes = Vec::new();
    let build = release::ensure_newest(dirs, progress, &mut notes).await?;
    let lock = prepare::read_lock(&build)?;
    let mut inst =
        crate::instances::create(dirs, name, &lock.version, crate::loaders::Loader::Vanilla, None, progress).await?;
    mark(&mut inst);
    inst.save()?;
    match prepare::get_ready(dirs, &build, progress, false, &mut notes).await {
        Ok(_) => {}
        Err(_) if progress.cancelled.load(Ordering::Relaxed) => {
            notes.push(format!("Preparing {NAME} was stopped. Press Play to finish it."))
        }
        Err(e) => notes
            .push(format!("{NAME} is installed, but preparing its resources failed: {e:#}. Press Play to try again.")),
    }
    for note in notes {
        crate::logs::info(format!("Horizon: {note}"));
        progress.note(note);
    }
    Ok(inst)
}

/// A Horizon game ready to start.
pub struct Start {
    /// What was decided on the way, for the log.
    pub notes: Vec<String>,
    /// The build it plays (`v0.2.97`).
    pub tag: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// The session JSON the game is handed the account in.
    session: Vec<u8>,
    handover: start::Handover,
    token: String,
}

impl Start {
    /// The command line with the account's token left out, safe to show.
    pub fn display(&self) -> String {
        let mut parts = vec![crate::launch::quote_argument(&self.program.display().to_string())];
        parts.extend(self.args.iter().map(|a| crate::launch::quote_argument(a)));
        let line = parts.join(" ");
        // the token is never on the command line; this only makes sure it can't be by mistake
        if self.token.len() > 3 { line.replace(&self.token, "REDACTED") } else { line }
    }

    /// Start the game, handing it the account on its standard input, or for a build from before the Launch API in a
    /// file only the player can read, which the game deletes once read.
    pub fn spawn(&self) -> Result<Running> {
        let stdin = match &self.handover {
            start::Handover::Stdin => Some(self.session.clone()),
            start::Handover::File(path) => {
                start::write_session_file(path, &self.session)?;
                None
            }
        };
        let running =
            crate::launch::spawn_process(&self.program, &self.args, &self.cwd, &self.env, stdin, Some(&self.token));
        if running.is_err() {
            self.cleanup();
        }
        running
    }

    /// The game closed (or never started): the session file is removed if the game didn't read it.
    pub fn cleanup(&self) {
        if let start::Handover::File(path) = &self.handover {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The crash report a Horizon game's output names when it crashed ("Horizon: crash report saved to <path>").
pub fn crash_report<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> Option<PathBuf> {
    start::crash_report(lines)
}

/// Get `inst` ready to play as `account` (already renewed): the build installed and up to date, its resources
/// prepared. `others_running` is true while another Horizon game runs, which keeps the builds in use where they are.
pub async fn prepare(
    dirs: &Dirs,
    settings: &Settings,
    inst: &Instance,
    account: &Account,
    progress: &Arc<Progress>,
    options: &LaunchOptions,
    others_running: bool,
) -> Result<Start> {
    if let Some(problem) = account_problem(account) {
        anyhow::bail!(problem);
    }
    if !supported() {
        return Err(unsupported());
    }
    let mut notes = Vec::new();
    let (build, lock) = {
        let _busy = BUSY.lock().await;
        let build = release::ensure_newest(dirs, progress, &mut notes).await?;
        // a running game may be playing from an older build: its resources and its folder stay where they are
        let lock = prepare::get_ready(dirs, &build, progress, !others_running, &mut notes).await?;
        if !others_running {
            prepare::prune(dirs, &build);
        }
        (build, lock)
    };
    let tag = build.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    notes.push(format!("{NAME} {tag}, playing Minecraft {}", lock.version));
    if lock.version != inst.mc_version() {
        match follow_version(dirs, inst, &lock.version).await {
            Ok(()) => notes.push(format!("The instance now says Minecraft {}, which {NAME} {tag} plays", lock.version)),
            Err(e) => notes.push(format!("Couldn't write Minecraft {} into the instance: {e:#}", lock.version)),
        }
    }

    progress.stage(format!("Starting {NAME}"));
    let listed = start::options(&build, &dirs.temp.join("horizon-asked")).await?;
    let ls = crate::launch::settings::LaunchSettings::resolve(settings, inst);
    // the window as the Java game gets it: the player's size, or the whole screen
    let window = if ls.maximise_minecraft {
        crate::launch::settings::screen_size().unwrap_or((ls.window_width, ls.window_height))
    } else {
        (ls.window_width, ls.window_height)
    };
    // a world or server for this one start wins over the instance's own Quick Play
    let (world, server) = if options.world.is_some() || options.server.is_some() {
        (options.world.clone(), options.server.clone())
    } else {
        (ls.quick_play.world_name.clone(), ls.quick_play.server_address.clone())
    };
    let wanted = start::Wanted {
        resources: prepare::resources(&build, &lock.version),
        game_dir: inst.root.clone(),
        window: Some(window),
        world,
        server,
        session_file: dirs.temp.join(format!("horizon-session-{}.json", uuid::Uuid::new_v4().simple())),
    };
    let (args, handover, left_out) = start::command_line(&listed, &wanted)?;
    notes.extend(left_out);

    let mut env = vec![("HORIZON_HOME".to_string(), inst.root.display().to_string())];
    if !crate::privacy::Privacy::from_settings(settings).analytics_allowed() {
        env.push(("HORIZON_TELEMETRY_URL".to_string(), "off".to_string()));
    }
    Ok(Start {
        notes,
        tag,
        program: release::program(&build, "horizon-client"),
        args,
        cwd: inst.root.clone(),
        env,
        session: start::session(account),
        handover,
        token: account.access_token.clone().unwrap_or_default(),
    })
}

/// Write the Minecraft version a newer build plays into the instance, so it says what it plays.
async fn follow_version(dirs: &Dirs, inst: &Instance, mc: &str) -> Result<()> {
    let mut version = crate::install::vanilla_json(dirs, mc).await?;
    version.id = mc.to_string();
    // the file as it is now: a change made meanwhile is kept
    let mut fresh = crate::instances::load_one(&inst.root).unwrap_or_else(|| inst.clone());
    fresh.file.version = version;
    fresh.file.launcher.version = mc.to_string();
    mark(&mut fresh);
    fresh.save()
}
