//! What the window hands the supervisor, and what the supervisor tells back. Plain data (no operating-system calls),
//! so it is the same on every platform; everything that does something is Linux only.
//!
//! The plan is a file in the session's private folder (0600 in a 0700 folder); **nothing secret goes in it**. Values
//! that must be hidden from logs (a licence token, a device code) are handed over on the supervisor's standard input
//! and live only in its memory.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::lease::LeaseSet;

/// The command line the window starts the supervisor with: `blockforge --bedrock-session <session folder>`.
pub const SESSION_ARG: &str = "--bedrock-session";

/// Prefix of the event lines the supervisor prints among the game's log lines (a record-separator character no log line
/// starts with). The window recognises them with [`parse_event_line`].
pub const EVENT_PREFIX: &str = "\u{1e}bedrock-event ";

/// Timeouts of a stop, in milliseconds. Every field has BedrockOnLinux's value unless the plan says otherwise.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimingMs {
    pub term_grace: u64,
    pub kill_grace: u64,
    pub poll: u64,
    pub idle_timeout: u64,
    pub idle_confirmations: u32,
    /// how long a stop signal sent to the supervisor itself waits for the game to close on its own before the
    /// supervisor ends it (BedrockOnLinux waited 20 s)
    pub signal_grace: u64,
    /// how long the supervisor waits for the leases at the start (the window may have only just let go of one)
    pub lease_wait: u64,
}

impl Default for TimingMs {
    fn default() -> Self {
        Self {
            term_grace: 5_000,
            kill_grace: 2_000,
            poll: 100,
            idle_timeout: 10_000,
            idle_confirmations: 3,
            signal_grace: 20_000,
            lease_wait: 5_000,
        }
    }
}

impl TimingMs {
    pub fn signal_grace(&self) -> Duration {
        Duration::from_millis(self.signal_grace)
    }

    pub fn lease_wait(&self) -> Duration {
        Duration::from_millis(self.lease_wait)
    }
}

/// One game session: how to start it and what to guard while it runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchPlan {
    /// the session's id (a UUID in text)
    pub session: String,
    /// `<bedrock>/sessions/<session>`; holds this plan, the control socket and the state
    pub session_dir: PathBuf,
    /// `<bedrock>/logs/<session>`
    pub log_dir: PathBuf,
    /// `<bedrock>`, where the graphics safety record lives
    pub bedrock_root: PathBuf,
    /// the Wine prefix; the supervisor signals only processes whose environment names exactly this
    pub prefix: PathBuf,
    /// what to run (the container wrapper or Wine), and how
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// variables the game must not inherit (the supervisor's own environment is the starting point)
    pub env_remove: Vec<String>,
    /// the game's image name: the game counts as up when a process of the prefix has it on its command line
    pub ready_image: String,
    pub leases: LeaseSet,
    /// a folder of decrypted images the supervisor takes over: emptied when the game is up or gone, removed at the end
    pub staging_dir: Option<PathBuf>,
    pub engine_rev: String,
    /// the player's explicit override of the graphics safety check
    pub allow_unsafe_gpu: bool,
    /// the card the game was told to use, to compare with what it really drew on
    pub expected_gpu: Option<String>,
    #[serde(default)]
    pub timing: Option<TimingMs>,
}

impl LaunchPlan {
    pub fn timing(&self) -> TimingMs {
        self.timing.unwrap_or_default()
    }

    /// The plan must be safe to act on before anything is started: paths absolute, a prefix that is something, an id
    /// that is a UUID (it names folders), the game's environment naming the prefix we guard.
    pub fn validate(&self) -> Result<(), String> {
        if uuid::Uuid::parse_str(&self.session).is_err() {
            return Err("the session id isn't a UUID".into());
        }
        for (what, p) in [
            ("session folder", &self.session_dir),
            ("log folder", &self.log_dir),
            ("data folder", &self.bedrock_root),
            ("prefix", &self.prefix),
            ("program", &self.program),
            ("working folder", &self.cwd),
        ] {
            if !p.is_absolute() {
                return Err(format!("the {what} must be an absolute path"));
            }
        }
        // a prefix of "/" or the home folder would make "everything using it" mean far too much
        if self.prefix.components().count() < 4 {
            return Err("the prefix path is too short to be a Wine prefix".into());
        }
        if self.ready_image.is_empty() {
            return Err("no game image name to wait for".into());
        }
        if let Some(wp) = self.env.iter().find(|(k, _)| k == "WINEPREFIX") {
            if std::path::Path::new(&wp.1) != self.prefix {
                return Err("WINEPREFIX in the environment differs from the prefix being guarded".into());
            }
        }
        if self.env.iter().any(|(k, _)| k.is_empty() || k.contains('=') || k.contains('\0')) {
            return Err("an environment variable has an impossible name".into());
        }
        Ok(())
    }
}

/// The moments of a session, in order. Each is sent once.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    /// the supervisor is up, its checks passed and its leases are held
    Starting,
    /// the wrapper was started (this is not the game being up)
    Started { pid: u32 },
    /// the game itself is running (a process of the prefix has the game's image)
    Ready,
    /// a stop was asked for (`graceful` or `force`), or the supervisor itself was told to stop
    Stopping { mode: String },
    /// something worth telling the player that does not end the session
    Notice { text: String },
    /// the session is over: the wrapper and everything in the prefix are gone (`prefix_idle`), the safety record
    /// was cleared (`marker_cleared`)
    Exited { code: Option<i32>, prefix_idle: bool, marker_cleared: bool, findings: Vec<Finding> },
    /// the session could not start, or the supervisor could not do its job
    Failed { kind: FailureKind, message: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// another game, or a job on this prefix or build, holds a lease
    Busy,
    /// the graphics safety check refused (see the message for why)
    GraphicsUnsafe,
    /// a Wine process from an earlier session still uses the prefix
    PrefixInUse,
    /// the plan was refused
    BadPlan,
    /// the wrapper could not be started
    Spawn,
    Internal,
}

/// A cause found in the logs after the game ended, by kind so the window can word it.
pub use super::super::diagnostics::Finding;

/// Where a session stands, kept in `state.json` (written whole, atomically, at every change).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub session: String,
    pub supervisor_pid: u32,
    pub supervisor_start: Option<u64>,
    pub stage: Stage,
    /// everything that happened, oldest first (a handful of entries)
    pub events: Vec<SessionEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Starting,
    Running,
    Ready,
    Stopping,
    Finished,
    Failed,
}

/// Read an event the supervisor printed among the game's lines.
pub fn parse_event_line(line: &str) -> Option<SessionEvent> {
    serde_json::from_str(&line[line.find(EVENT_PREFIX)? + EVENT_PREFIX.len()..]).ok()
}

/// What `Control::stop` found out.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StopOutcome {
    /// processes of the prefix seen during the stop
    pub seen: usize,
    /// of which had to be killed
    pub killed: usize,
    /// the prefix was seen idle at the end
    pub prefix_idle: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> LaunchPlan {
        LaunchPlan {
            session: uuid::Uuid::new_v4().to_string(),
            session_dir: "/d/bedrock/sessions/x".into(),
            log_dir: "/d/bedrock/logs/x".into(),
            bedrock_root: "/d/bedrock".into(),
            prefix: "/d/bedrock/profiles/p/compat/pfx".into(),
            program: "/usr/bin/true".into(),
            args: vec![],
            cwd: "/d".into(),
            env: vec![("WINEPREFIX".into(), "/d/bedrock/profiles/p/compat/pfx".into())],
            env_remove: vec![],
            ready_image: "Minecraft.Windows.exe".into(),
            leases: LeaseSet::default(),
            staging_dir: None,
            engine_rev: "e".into(),
            allow_unsafe_gpu: false,
            expected_gpu: None,
            timing: None,
        }
    }

    #[test]
    fn a_plan_is_checked_before_anything_starts() {
        assert!(plan().validate().is_ok());
        let mut p = plan();
        p.session = "../../etc".into();
        assert!(p.validate().is_err());
        let mut p = plan();
        p.prefix = "/home".into();
        assert!(p.validate().is_err(), "a prefix that short could be anything");
        let mut p = plan();
        p.env = vec![("WINEPREFIX".into(), "/somewhere/else/entirely".into())];
        assert!(p.validate().is_err());
        let mut p = plan();
        p.cwd = "relative".into();
        assert!(p.validate().is_err());
        let mut p = plan();
        p.env = vec![("A=B".into(), "x".into())];
        assert!(p.validate().is_err());
    }

    #[test]
    fn events_round_trip_through_a_log_line() {
        let ev = SessionEvent::Exited { code: Some(0), prefix_idle: true, marker_cleared: true, findings: vec![] };
        let line = format!("{EVENT_PREFIX}{}", serde_json::to_string(&ev).unwrap());
        assert_eq!(parse_event_line(&line), Some(ev));
        assert_eq!(parse_event_line("[12:00] ordinary game output"), None);
        assert_eq!(parse_event_line(&format!("{EVENT_PREFIX}not json")), None);
        // the plan survives a file
        let p = plan();
        let back: LaunchPlan = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(back.session, p.session);
        assert_eq!(back.timing(), TimingMs::default());
    }
}
