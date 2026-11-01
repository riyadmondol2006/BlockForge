//! The private control channel between the window (or a later BlockForge) and the session supervisor, and the session
//! state both can read.
//!
//! - A Unix socket in the session's private folder (`0700`; the socket `0600`). Each connection must be the same user
//!   (checked with the kernel's peer credentials), then speaks one JSON object per line.
//! - `state.json` in the same folder is rewritten whole and atomically at every step, so a window that was closed and
//!   reopened finds the truth about a session that went on without it, and a supervisor that died leaves a state that
//!   says so (its recorded start time no longer matches a running process).
//!
//! A socket path may not be longer than 107 bytes; a data folder deep in someone's home can be. A long path is
//! reached through the folder's `/proc/self/fd/<n>` entry instead.

use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use super::plan::{SessionEvent, Stage, State, StopOutcome};
use crate::bedrock::{BedrockError, ErrorKind, StopMode};

pub const SOCKET_NAME: &str = "control.sock";
pub const STATE_NAME: &str = "state.json";
pub const PLAN_NAME: &str = "plan.json";
const MAX_REQUEST_BYTES: usize = 4096;
/// A stop is bounded by the supervisor (grace, kill, idle check); the caller waits for it a little longer.
const STOP_WAIT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    Graceful,
    Force,
}

impl From<StopMode> for StopKind {
    fn from(m: StopMode) -> Self {
        match m {
            StopMode::Graceful => StopKind::Graceful,
            StopMode::Force => StopKind::Force,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// stop the game; the answer comes when the stop is over
    Stop { mode: StopKind },
    /// the state now
    Status,
    /// every event from the start, then each new one, until the session ends
    Subscribe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Stopped { outcome: StopOutcome },
    State { state: State },
    Event { event: SessionEvent },
    Error { message: String },
}

/// A way to reach a socket whatever the length of its folder's path.
pub struct SocketAddr {
    /// keeps the folder open so `/proc/self/fd/<n>` stays valid
    _dir: Option<std::fs::File>,
    pub path: PathBuf,
}

impl SocketAddr {
    pub fn for_folder(dir: &Path) -> std::io::Result<SocketAddr> {
        let direct = dir.join(SOCKET_NAME);
        if direct.as_os_str().len() < 100 {
            return Ok(SocketAddr { _dir: None, path: direct });
        }
        let handle = std::fs::File::open(dir)?;
        let path = PathBuf::from(format!("/proc/self/fd/{}/{}", handle.as_raw_fd(), SOCKET_NAME));
        Ok(SocketAddr { _dir: Some(handle), path })
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn write_state(dir: &Path, state: &State) -> std::io::Result<()> {
    write_private(&dir.join(STATE_NAME), &serde_json::to_vec(state)?)
}

pub fn read_state(dir: &Path) -> Option<State> {
    let meta = std::fs::symlink_metadata(dir.join(STATE_NAME)).ok()?;
    if !meta.is_file() || meta.len() > 1 << 20 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(dir.join(STATE_NAME)).ok()?).ok()
}

pub fn write_plan(dir: &Path, plan: &super::plan::LaunchPlan) -> std::io::Result<()> {
    write_private(&dir.join(PLAN_NAME), &serde_json::to_vec(plan)?)
}

pub fn read_plan(dir: &Path) -> Result<super::plan::LaunchPlan, String> {
    let meta = std::fs::symlink_metadata(dir.join(PLAN_NAME)).map_err(|e| format!("no plan: {e}"))?;
    if !meta.is_file() || meta.len() > 1 << 20 {
        return Err("the plan isn't a plain file of a sensible size".into());
    }
    let bytes = std::fs::read(dir.join(PLAN_NAME)).map_err(|e| format!("can't read the plan: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("the plan is invalid: {e}"))
}

// ---- the server side

/// A request that reached the supervisor, with the way to answer it.
pub struct Incoming {
    pub kind: StopKind,
    pub reply: tokio::sync::oneshot::Sender<Reply>,
}

/// Listens for control connections. Dropping it removes the socket.
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    _addr: SocketAddr,
}

impl Server {
    pub fn bind(dir: &Path) -> std::io::Result<Server> {
        let addr = SocketAddr::for_folder(dir)?;
        // a socket of a supervisor that died
        let _ = std::fs::remove_file(dir.join(SOCKET_NAME));
        let listener = UnixListener::bind(&addr.path)?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir.join(SOCKET_NAME), std::fs::Permissions::from_mode(0o600));
        }
        Ok(Server { listener, path: dir.join(SOCKET_NAME), _addr: addr })
    }

    /// Serve until the task is dropped: stops go to `stops`, the rest is answered from `snapshot` and `events`.
    pub async fn serve(
        self,
        stops: tokio::sync::mpsc::Sender<Incoming>,
        snapshot: std::sync::Arc<dyn Fn() -> State + Send + Sync>,
        events: tokio::sync::broadcast::Sender<SessionEvent>,
    ) {
        let me = super::procs::current_uid();
        loop {
            let Ok((stream, _)) = self.listener.accept().await else { break };
            // only the same user
            if stream.peer_cred().map(|c| c.uid() != me).unwrap_or(true) {
                continue;
            }
            tokio::spawn(handle(stream, stops.clone(), snapshot.clone(), events.clone()));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn send(stream: &mut UnixStream, reply: &Reply) -> bool {
    let mut line = serde_json::to_vec(reply).unwrap_or_default();
    line.push(b'\n');
    stream.write_all(&line).await.is_ok()
}

async fn handle(
    mut stream: UnixStream,
    stops: tokio::sync::mpsc::Sender<Incoming>,
    snapshot: std::sync::Arc<dyn Fn() -> State + Send + Sync>,
    events: tokio::sync::broadcast::Sender<SessionEvent>,
) {
    let request = {
        let mut reader = BufReader::new(&mut stream).take(MAX_REQUEST_BYTES as u64);
        let mut line = String::new();
        // a client that connects and says nothing is dropped after a while
        match tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line)).await {
            Ok(Ok(n)) if n > 0 => serde_json::from_str::<Request>(line.trim()),
            _ => return,
        }
    };
    match request {
        Err(e) => {
            send(&mut stream, &Reply::Error { message: format!("bad request: {e}") }).await;
        }
        Ok(Request::Status) => {
            send(&mut stream, &Reply::State { state: snapshot() }).await;
        }
        Ok(Request::Stop { mode }) => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if stops.send(Incoming { kind: mode, reply: tx }).await.is_err() {
                send(&mut stream, &Reply::Error { message: "the session is already over".into() }).await;
                return;
            }
            let reply = rx.await.unwrap_or(Reply::Error { message: "the session ended before answering".into() });
            send(&mut stream, &reply).await;
        }
        Ok(Request::Subscribe) => {
            // subscribe before reading the history, so nothing falls between the two
            let mut rx = events.subscribe();
            for event in snapshot().events {
                if !send(&mut stream, &Reply::Event { event }).await {
                    return;
                }
            }
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let last = matches!(event, SessionEvent::Exited { .. } | SessionEvent::Failed { .. });
                        if !send(&mut stream, &Reply::Event { event }).await || last {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return,
                }
            }
        }
    }
}

// ---- the client side

/// The window's handle on a session: talks to the supervisor, and reads what it left when it is gone. It holds no
/// process and no lease, so a window that exits and a new one that starts can both have one.
#[derive(Clone, Debug)]
pub struct Control {
    dir: PathBuf,
}

impl Control {
    /// Reach the session whose folder is `session_dir` (`<bedrock>/sessions/<id>`).
    pub fn attach(session_dir: &Path) -> Control {
        Control { dir: session_dir.to_path_buf() }
    }

    /// What the supervisor last recorded, `None` before it has written anything.
    pub fn state(&self) -> Option<State> {
        read_state(&self.dir)
    }

    /// Whether the supervisor of this session is still running (its recorded start time matches a live process).
    pub fn supervisor_alive(&self) -> bool {
        match self.state() {
            Some(s) => {
                !matches!(s.stage, Stage::Finished | Stage::Failed)
                    && super::staging::process_start(s.supervisor_pid) == s.supervisor_start
                    && s.supervisor_start.is_some()
            }
            None => false,
        }
    }

    async fn connect(&self) -> std::io::Result<UnixStream> {
        let addr = SocketAddr::for_folder(&self.dir)?;
        UnixStream::connect(&addr.path).await
    }

    async fn ask(&self, request: &Request) -> Result<UnixStream, BedrockError> {
        let mut stream = self
            .connect()
            .await
            .map_err(|e| BedrockError::new(ErrorKind::Runtime, format!("can't reach the Bedrock session: {e}")))?;
        let mut line = serde_json::to_vec(request).unwrap_or_default();
        line.push(b'\n');
        stream
            .write_all(&line)
            .await
            .map_err(|e| BedrockError::new(ErrorKind::Runtime, format!("can't talk to the Bedrock session: {e}")))?;
        Ok(stream)
    }

    /// Stop the game and wait for the end of the stop: for `Graceful` a polite request first and bounded escalation
    /// after, for `Force` the escalation at once. Success means the wrapper is reaped and the prefix was seen idle; if
    /// processes were left, it is an error naming how many. A session that is already over is a success.
    pub async fn stop(&self, mode: StopMode) -> Result<StopOutcome, BedrockError> {
        let mut stream = match self.ask(&Request::Stop { mode: mode.into() }).await {
            Ok(s) => s,
            Err(e) => {
                // nobody answers: either it is over, or the supervisor died
                return match self.state() {
                    Some(s) if matches!(s.stage, Stage::Finished | Stage::Failed) => Ok(finished_outcome(&s)),
                    Some(_) if !self.supervisor_alive() => Err(BedrockError::new(
                        ErrorKind::Runtime,
                        "the Bedrock session's supervisor is gone without finishing; the next start cleans up after it",
                    )),
                    _ => Err(e),
                };
            }
        };
        let mut line = String::new();
        let read = tokio::time::timeout(STOP_WAIT, BufReader::new(&mut stream).read_line(&mut line)).await;
        match read {
            Ok(Ok(n)) if n > 0 => match serde_json::from_str::<Reply>(line.trim()) {
                Ok(Reply::Stopped { outcome }) => {
                    if outcome.prefix_idle {
                        Ok(outcome)
                    } else {
                        Err(BedrockError::new(
                            ErrorKind::Runtime,
                            "the game was stopped but some Wine processes of its prefix could not be ended",
                        ))
                    }
                }
                Ok(Reply::Error { message }) => match self.state() {
                    // asked in the moment between the end of the session and the supervisor's exit
                    Some(s) if matches!(s.stage, Stage::Finished) => Ok(finished_outcome(&s)),
                    _ => Err(BedrockError::new(ErrorKind::Runtime, message)),
                },
                _ => Err(BedrockError::new(ErrorKind::Internal, "an unexpected answer from the Bedrock session")),
            },
            // the supervisor closed the connection: look at what it recorded
            Ok(_) => match self.state() {
                Some(s) if matches!(s.stage, Stage::Finished) => Ok(finished_outcome(&s)),
                _ => Err(BedrockError::new(ErrorKind::Runtime, "the Bedrock session ended without answering")),
            },
            Err(_) => Err(BedrockError::new(ErrorKind::Busy, "the Bedrock session is still stopping")),
        }
    }

    /// Wait until the session has started its wrapper (`Ok`), refused to (`Err` with the reason), or `limit` passes.
    /// Reads the state file, so it works for a session the supervisor has already finished.
    pub async fn wait_started(&self, limit: Duration) -> Result<(), BedrockError> {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            if let Some(state) = self.state() {
                for e in &state.events {
                    match e {
                        SessionEvent::Failed { message, kind } => {
                            let ek = match kind {
                                super::plan::FailureKind::Busy | super::plan::FailureKind::PrefixInUse => {
                                    ErrorKind::Busy
                                }
                                _ => ErrorKind::Runtime,
                            };
                            return Err(BedrockError::new(ek, message.clone()));
                        }
                        SessionEvent::Started { .. } => return Ok(()),
                        _ => {}
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BedrockError::new(ErrorKind::Runtime, "the Bedrock session didn't start in time"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Follow the session's events: everything so far, then each new one, until it ends.
    pub async fn subscribe(&self) -> Result<tokio::sync::mpsc::Receiver<SessionEvent>, BedrockError> {
        let stream = self.ask(&Request::Subscribe).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(Reply::Event { event }) = serde_json::from_str::<Reply>(&line) {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok(rx)
    }
}

fn finished_outcome(state: &State) -> StopOutcome {
    for e in state.events.iter().rev() {
        if let SessionEvent::Exited { prefix_idle, .. } = e {
            return StopOutcome { seen: 0, killed: 0, prefix_idle: *prefix_idle };
        }
    }
    StopOutcome { seen: 0, killed: 0, prefix_idle: true }
}
