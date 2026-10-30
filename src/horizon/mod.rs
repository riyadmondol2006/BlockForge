//! Horizon: Minecraft: Java Edition rewritten in Rust by the Horizon project (https://github.com/horizon-rust/horizon,
//! MIT licence), an unofficial fan project not affiliated with Mojang or Microsoft. BlockForge plays it as an
//! instance of its own: it downloads Horizon's signed release, prepares its resources from BlockForge's own copy of
//! the Minecraft version it pins, and starts it through Horizon's Launch API (docs/api/launch.md in their repository).
//!
//! Everything of Horizon lives under `horizon/` in BlockForge's data folder and in the instance's own folder. The
//! shared Minecraft files are only read, and nothing of the official launcher (`~/.minecraft`) or of the Horizon
//! launcher (`~/.local/share/horizon`) is touched.

pub mod policy;

use std::sync::Arc;

use anyhow::Result;

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
pub fn installed_build(_dirs: &Dirs) -> Option<Build> {
    None
}

/// Make a new Horizon instance called `name`: install the newest build, the Minecraft version it pins with its
/// Java, and prepare the build's resources, then write the instance.
pub async fn create_instance(_dirs: &Dirs, _name: &str, _progress: &Arc<Progress>) -> Result<Instance> {
    anyhow::bail!("Horizon isn't ready yet")
}

/// A Horizon game ready to start.
pub struct Start {
    /// What was decided on the way, for the log.
    pub notes: Vec<String>,
}

impl Start {
    /// The command line with the account's token left out, safe to show.
    pub fn display(&self) -> String {
        String::new()
    }

    /// Start the game, handing it the account on its standard input.
    pub fn spawn(&self) -> Result<Running> {
        anyhow::bail!("Horizon isn't ready yet")
    }

    /// The game closed (or never started).
    pub fn cleanup(&self) {}
}

/// Get `inst` ready to play as `account` (already renewed): the build installed and up to date, its resources
/// prepared. `others_running` is true while another Horizon game runs, which keeps the builds in use where they are.
pub async fn prepare(
    _dirs: &Dirs,
    _settings: &Settings,
    _inst: &Instance,
    _account: &Account,
    _progress: &Arc<Progress>,
    _options: &LaunchOptions,
    _others_running: bool,
) -> Result<Start> {
    anyhow::bail!("Horizon isn't ready yet")
}
