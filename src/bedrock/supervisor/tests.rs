//! End-to-end fixtures for the supervisor with fake native process trees. No Wine, no shell scripts: the "game" is a
//! copy of this test binary started in a role (see [`role_entry`]), so the trees really fork, escape their process
//! group, ignore SIGTERM and spawn children while being stopped. The supervisor under test is also a copy of this
//! binary (see [`supervisor_role`]) started through the real [`spawn_with`], with a stand-in for the computer's graphics
//! state so nothing here touches a real GPU, journal or display.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::ipc::Control;
use super::lease::{Kind, Lease, LeaseSet};
use super::plan::{FailureKind, LaunchPlan, SessionEvent, Stage, State, TimingMs};
use super::procs::{ProcTable, RealSignals, Reaper, Scope, Timing};
use super::run::{Deps, Spawned, spawn_with};
use crate::bedrock::StopMode;
use crate::bedrock::gpu_safety::{self, Env, Host};
use crate::testutil::scratch;

const IMAGE: &str = "Minecraft.Windows.exe";
const TEST_PATH: &str = "bedrock::supervisor::tests::";

// ---- the fake game: a tree of processes told what to do by BFB_SPEC

fn exe() -> PathBuf {
    std::env::current_exe().unwrap()
}

/// How a fake node is started: this binary, running only [`role_entry`], with the image's name on its command line so
/// the supervisor can tell the game is up.
fn node_args() -> Vec<String> {
    vec![
        "--exact".into(),
        format!("{TEST_PATH}role_entry"),
        "--nocapture".into(),
        "--test-threads=1".into(),
        IMAGE.into(),
    ]
}

unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

static TERMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn note_term(_: i32) {
    TERMED.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn spawn_node(spec: &str, own_group: bool, scrub_env: bool) -> std::process::Child {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(exe());
    cmd.args(node_args()).stdin(std::process::Stdio::null());
    if scrub_env {
        // a process that cleared its environment: only the descendant relation still says whose it is
        cmd.env_clear();
        cmd.env("PATH", std::env::var_os("PATH").unwrap_or_default());
    }
    cmd.env("BFB_ROLE", "node").env("BFB_SPEC", spec);
    if own_group {
        cmd.process_group(0);
    }
    cmd.spawn().unwrap()
}

/// Everything after `prefix` up to the end of the spec (a spec may hold specs: `child:` and friends take the rest).
fn run_spec(spec: &str) {
    // steps are separated by ';'. A step that takes a spec takes everything after its first ':' with ';' written '+'.
    for step in spec.split(';').filter(|s| !s.is_empty()) {
        let (verb, arg) = step.split_once(':').unwrap_or((step, ""));
        let inner = arg.replace('+', ";");
        match verb {
            "print" => {
                println!("{arg}");
            }
            "big" => println!("{}", "z".repeat(arg.parse().unwrap())),
            "sleep" => std::thread::sleep(Duration::from_millis(arg.parse().unwrap())),
            "exit" => std::process::exit(arg.parse().unwrap()),
            "child" => {
                std::mem::forget(spawn_node(&inner, false, false));
            }
            "escaped" => {
                std::mem::forget(spawn_node(&inner, true, false));
            }
            "scrubbed" => {
                std::mem::forget(spawn_node(&inner, false, true));
            }
            "pidfile" => std::fs::write(arg, std::process::id().to_string()).unwrap(),
            "ignoreterm" => unsafe {
                signal(15, 1);
            },
            // on SIGTERM start a child (a service that starts during shutdown), then go on running
            "onterm" => {
                unsafe {
                    signal(15, note_term as extern "C" fn(i32) as usize);
                }
                while !TERMED.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                std::mem::forget(spawn_node(&inner, true, false));
                // and ignore the rest of the polite requests
                std::thread::sleep(Duration::from_secs(120));
            }
            // the loader of the patched Wine: open the staged image named in the map, unlink it, report what it saw
            "load" => {
                let map = std::env::var("WINE_DLL_FILE_MAP").unwrap();
                let entry = map.split('|').find(|e| e.contains(IMAGE)).unwrap();
                let (path, nt) = entry.split_once(':').unwrap();
                let bytes = std::fs::read(path).unwrap();
                std::fs::remove_file(path).unwrap();
                println!("loaded {} bytes as {nt}", bytes.len());
            }
            other => panic!("unknown step {other}"),
        }
    }
    // a node that has finished its steps waits to be stopped
    std::thread::sleep(Duration::from_secs(120));
}

/// Not a test: the body of every fake game process and supervisor (a test binary run again with BFB_ROLE set).
#[test]
fn role_entry() {
    if std::env::var("BFB_ROLE").as_deref() == Ok("node") {
        run_spec(&std::env::var("BFB_SPEC").unwrap());
    }
}

/// A stand-in for the computer's graphics state, read from the environment of the supervisor under test.
struct TestHost {
    kernel_log: Option<String>,
}

impl Host for TestHost {
    fn boot_id(&self) -> String {
        super::staging::boot_id()
    }
    fn process_start(&self, pid: u32) -> Option<u64> {
        super::staging::process_start(pid)
    }
    fn now(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
    }
    fn kernel_log(&self, boot: i32) -> Option<String> {
        if boot == 0 { self.kernel_log.clone() } else { None }
    }
    fn x_providers(&self, _env: &Env) -> Option<u32> {
        Some(1)
    }
    fn xorg_software_fallback(&self) -> bool {
        false
    }
    fn nvidia_with_mesa_glx(&self) -> bool {
        false
    }
}

/// Not a test: the supervisor process (the real `run` with a stand-in host).
#[test]
fn supervisor_role() {
    if std::env::var("BFB_ROLE").as_deref() != Ok("supervisor") {
        return;
    }
    let dir = PathBuf::from(std::env::var("BFB_SESSION_DIR").unwrap());
    let plan = super::ipc::read_plan(&dir).unwrap();
    let mut input = String::new();
    {
        use std::io::Read;
        std::io::stdin().read_to_string(&mut input).unwrap();
    }
    #[derive(serde::Deserialize)]
    struct S {
        secrets: Vec<String>,
    }
    let secrets = serde_json::from_str::<S>(&input).map(|s| s.secrets).unwrap_or_default();
    let host = TestHost { kernel_log: std::env::var("BFB_FAKE_KERNEL_LOG").ok() };
    let deps = Deps {
        host: Arc::new(host),
        signals: Arc::new(RealSignals),
        table: ProcTable::system(),
        check_env: Env(vec![("XDG_SESSION_TYPE".into(), "wayland".into())]),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let code = rt.block_on(super::run::run(plan, secrets, deps));
    std::process::exit(code);
}

// ---- helpers

struct Rig {
    root: PathBuf,
    bedrock: PathBuf,
    prefix: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Rig {
        let root = scratch(tag);
        let bedrock = root.join("bedrock");
        let prefix = bedrock.join("profiles/p1/compat/pfx");
        std::fs::create_dir_all(&prefix).unwrap();
        Rig { root, bedrock, prefix }
    }

    fn leases(&self) -> LeaseSet {
        LeaseSet {
            game: Some(self.bedrock.join("locks/game.lease")),
            prefix: Some(self.bedrock.join("profiles/p1/locks/launch.lease")),
            assets: vec![self.bedrock.join("locks/engine.lease")],
        }
    }

    fn plan(&self, spec: &str) -> LaunchPlan {
        let session = uuid::Uuid::new_v4().to_string();
        let mut env = vec![("BFB_ROLE".to_string(), "node".to_string()), ("BFB_SPEC".to_string(), spec.to_string())];
        env.push(("WINEPREFIX".into(), self.prefix.to_string_lossy().into_owned()));
        LaunchPlan {
            session_dir: self.bedrock.join("sessions").join(&session),
            log_dir: self.bedrock.join("logs").join(&session),
            session,
            bedrock_root: self.bedrock.clone(),
            prefix: self.prefix.clone(),
            program: exe(),
            args: node_args(),
            cwd: self.root.clone(),
            env,
            env_remove: vec![],
            ready_image: IMAGE.into(),
            leases: self.leases(),
            staging_dir: None,
            engine_rev: "test-engine".into(),
            allow_unsafe_gpu: false,
            expected_gpu: None,
            timing: Some(fast()),
        }
    }

    fn start(&self, plan: &LaunchPlan, secrets: &[String]) -> Spawned {
        start_with(plan, secrets, &[])
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // whatever a failed test left running in this prefix
        let table = ProcTable::system();
        let scope = Scope::prefix_only(&self.prefix);
        Reaper { table: &table, signals: &RealSignals, scope }.stop(
            Timing { term_grace: Duration::ZERO, kill_grace: Duration::from_secs(1), ..Timing::default() },
            true,
            |_| {},
            std::thread::sleep,
        );
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fast() -> TimingMs {
    TimingMs {
        term_grace: 600,
        kill_grace: 800,
        poll: 20,
        idle_timeout: 3_000,
        idle_confirmations: 2,
        signal_grace: 1_500,
        lease_wait: 300,
    }
}

fn start_with(plan: &LaunchPlan, secrets: &[String], extra_env: &[(&str, &str)]) -> Spawned {
    let mut env = vec![
        ("BFB_ROLE".to_string(), "supervisor".to_string()),
        ("BFB_SESSION_DIR".to_string(), plan.session_dir.to_string_lossy().into_owned()),
    ];
    env.extend(extra_env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let args = vec![
        "--exact".to_string(),
        format!("{TEST_PATH}supervisor_role"),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ];
    spawn_with(&exe(), &args, &env, plan, secrets).unwrap()
}

fn wait_for(what: &str, limit: Duration, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn events(c: &Control) -> Vec<SessionEvent> {
    c.state().map(|s| s.events).unwrap_or_default()
}

fn stage(c: &Control) -> Option<Stage> {
    c.state().map(|s| s.stage)
}

fn finished(c: &Control) -> bool {
    matches!(stage(c), Some(Stage::Finished) | Some(Stage::Failed))
}

fn exited(c: &Control) -> Option<SessionEvent> {
    events(c).into_iter().find(|e| matches!(e, SessionEvent::Exited { .. }))
}

fn alive(pid: u32) -> bool {
    super::staging::process_start(pid).is_some() && ProcTable::system().info(pid).is_some_and(|i| !i.is_zombie())
}

fn pid_of(file: &Path) -> u32 {
    std::fs::read_to_string(file).unwrap().trim().parse().unwrap()
}

fn read_log(plan: &LaunchPlan) -> String {
    std::fs::read_to_string(plan.log_dir.join("game.log")).unwrap_or_default()
}

fn kill(pid: u32, sig: &str) {
    assert!(std::process::Command::new("kill").args([sig, &pid.to_string()]).status().unwrap().success());
}

fn marker_paths(rig: &Rig) -> gpu_safety::Paths {
    gpu_safety::Paths::at(&rig.bedrock)
}

fn game_lease_free(rig: &Rig) -> bool {
    Lease::take_within(&rig.bedrock.join("locks/game.lease"), Kind::Exclusive, Duration::from_secs(2), &|| false)
        .is_ok()
}

// ---- the fixtures

#[tokio::test(flavor = "multi_thread")]
async fn a_game_that_ends_on_its_own_is_followed_to_a_clean_finish() {
    let rig = Rig::new("sup-clean");
    let mut plan = rig.plan("print:starting up;sleep:2500;print:bye;exit:0");
    let staging_root = rig.root.join("ram");
    let staged = super::staging::StagingDir::create(&staging_root, &plan.session, 0).unwrap();
    staged.stage_bytes("never-opened.img", b"plaintext that no loader asked for").unwrap();
    plan.staging_dir = Some(staged.hand_over());
    let mut s = rig.start(&plan, &[]);
    s.control.wait_started(Duration::from_secs(20)).await.unwrap();
    // the window sees the same events among the lines
    let mut seen_events = vec![];
    let mut lines = vec![];
    let end = Instant::now() + Duration::from_secs(20);
    while let Ok(Some(l)) =
        tokio::time::timeout(end.saturating_duration_since(Instant::now()), s.running.lines.recv()).await
    {
        match super::parse_event_line(&l) {
            Some(e) => seen_events.push(e),
            None => lines.push(l),
        }
    }
    assert!(lines.iter().any(|l| l.ends_with("starting up")) && lines.iter().any(|l| l == "bye"), "{lines:?}");
    let names: Vec<&str> = seen_events
        .iter()
        .map(|e| match e {
            SessionEvent::Starting => "starting",
            SessionEvent::Started { .. } => "started",
            SessionEvent::Ready => "ready",
            SessionEvent::Exited { .. } => "exited",
            _ => "other",
        })
        .collect();
    assert_eq!(names, vec!["starting", "started", "ready", "exited"], "{seen_events:?}");
    let status = s.running.child.wait().await.unwrap();
    assert_eq!(status.code(), Some(0));
    match exited(&s.control).unwrap() {
        SessionEvent::Exited { code, prefix_idle, marker_cleared, findings } => {
            assert_eq!(code, Some(0));
            assert!(prefix_idle && marker_cleared && findings.is_empty());
        }
        _ => unreachable!(),
    }
    assert_eq!(stage(&s.control), Some(Stage::Finished));
    // nothing is left: the marker, the leases, the staged image, the socket
    assert!(!marker_paths(&rig).marker.exists());
    assert!(game_lease_free(&rig));
    assert!(!staging_root.join(&plan.session).exists(), "staged images must not outlive the session");
    assert!(!plan.session_dir.join("control.sock").exists());
    assert!(read_log(&plan).contains("bye"));
    assert!(ProcTable::system().in_prefix(&rig.prefix).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn services_that_leave_the_group_are_found_by_the_prefix_and_ended() {
    let rig = Rig::new("sup-escaped");
    let files = rig.root.clone();
    let svc = files.join("service.pid");
    let late = files.join("late.pid");
    // a service in a group of its own that outlives the wrapper; another that scrubbed its environment
    let spec = format!(
        "escaped:pidfile:{}+sleep:60000;scrubbed:pidfile:{}+sleep:60000;sleep:600;exit:0",
        svc.display(),
        late.display()
    );
    let plan = rig.plan(&spec);
    // a bystander of another prefix that starts the same way, and one that only shares its start
    let other = rig.bedrock.join("profiles/p1/compat/pfx2");
    let mut bystander = std::process::Command::new("sleep").arg("60").env("WINEPREFIX", &other).spawn().unwrap();
    let mut stranger = std::process::Command::new("sleep").arg("60").spawn().unwrap();
    let s = rig.start(&plan, &[]);
    wait_for("the session to finish", Duration::from_secs(30), || finished(&s.control));
    let (service, scrubbed) = (pid_of(&svc), pid_of(&late));
    assert!(!alive(service), "a service that left the group must be ended");
    assert!(!alive(scrubbed), "a child that cleared its environment is still the game's");
    match exited(&s.control).unwrap() {
        SessionEvent::Exited { prefix_idle, marker_cleared, .. } => assert!(prefix_idle && marker_cleared),
        _ => unreachable!(),
    }
    assert!(alive(bystander.id()) && alive(stranger.id()), "only our own processes are ever signalled");
    bystander.kill().unwrap();
    stranger.kill().unwrap();
    bystander.wait().unwrap();
    stranger.wait().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_graceful_stop_waits_for_the_end_and_reports_it() {
    let rig = Rig::new("sup-stop");
    let plan = rig.plan("print:running;sleep:60000");
    let s = rig.start(&plan, &[]);
    wait_for("the game to be up", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
    let outcome = s.control.stop(StopMode::Graceful).await.unwrap();
    assert!(outcome.prefix_idle);
    assert!(outcome.seen >= 1);
    wait_for("the supervisor to finish", Duration::from_secs(10), || finished(&s.control));
    assert!(events(&s.control).iter().any(|e| matches!(e, SessionEvent::Stopping { mode } if mode == "graceful")));
    assert!(ProcTable::system().in_prefix(&rig.prefix).is_empty());
    assert!(!marker_paths(&rig).marker.exists() && game_lease_free(&rig));
    // stopping a session that is over is not an error
    assert!(s.control.stop(StopMode::Graceful).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_that_ignores_the_polite_request_is_killed_after_the_grace() {
    let rig = Rig::new("sup-escalate");
    let pid = rig.root.join("stubborn.pid");
    let plan = rig.plan(&format!("pidfile:{};ignoreterm;print:ignoring;sleep:60000", pid.display()));
    let s = rig.start(&plan, &[]);
    wait_for("the game to be up", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
    let started = Instant::now();
    let outcome = s.control.stop(StopMode::Graceful).await.unwrap();
    assert!(outcome.killed >= 1, "it had to be killed: {outcome:?}");
    // the polite stage lasted about its grace, not forever and not zero
    assert!(
        started.elapsed() >= Duration::from_millis(500) && started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    assert!(!alive(pid_of(&pid)));
    // and a forced stop skips the grace entirely
    let plan2 = rig.plan(&format!("pidfile:{};ignoreterm;sleep:60000", rig.root.join("s2.pid").display()));
    wait_for("the first session to release its leases", Duration::from_secs(10), || game_lease_free(&rig));
    let s2 = rig.start(&plan2, &[]);
    wait_for("the second game", Duration::from_secs(20), || events(&s2.control).contains(&SessionEvent::Ready));
    let t = Instant::now();
    assert!(s2.control.stop(StopMode::Force).await.unwrap().prefix_idle);
    assert!(t.elapsed() < Duration::from_millis(550) + Duration::from_secs(3), "{:?}", t.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
async fn children_started_while_the_tree_shuts_down_are_ended_too() {
    let rig = Rig::new("sup-late");
    let late = rig.root.join("late-child.pid");
    // on the first polite request the node starts a service of its own group and then ignores the request
    let plan = rig.plan(&format!("onterm:pidfile:{}+sleep:60000", late.display()));
    let s = rig.start(&plan, &[]);
    wait_for("the game", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
    let outcome = s.control.stop(StopMode::Graceful).await.unwrap();
    assert!(outcome.prefix_idle, "{outcome:?}");
    wait_for("the late child's pid", Duration::from_secs(5), || late.exists());
    assert!(!alive(pid_of(&late)), "a child born during the grace must not survive the stop");
    assert!(ProcTable::system().in_prefix(&rig.prefix).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_and_sighup_close_the_game_and_clear_the_record_first() {
    for (sig, number) in [("-TERM", 15), ("-HUP", 1)] {
        let rig = Rig::new("sup-signal");
        let plan = rig.plan("print:up;sleep:60000");
        let mut s = rig.start(&plan, &[]);
        wait_for("the game", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
        let supervisor = s.running.child.id().unwrap();
        kill(supervisor, sig);
        // a second signal right after (systemd does this at power-off) neither restarts nor cuts short
        kill(supervisor, "-HUP");
        let status = tokio::time::timeout(Duration::from_secs(30), s.running.child.wait())
            .await
            .expect("the supervisor must end")
            .unwrap();
        use std::os::unix::process::ExitStatusExt;
        let code = status.code().or(status.signal().map(|s| 128 + s));
        assert_eq!(code, Some(128 + number), "it ends as the signal would have ended it");
        // but only after the teardown: the game is gone and the record cleared
        assert!(ProcTable::system().in_prefix(&rig.prefix).is_empty(), "{sig}");
        assert!(!marker_paths(&rig).marker.exists(), "{sig}: the record is cleared once the prefix is idle");
        assert!(game_lease_free(&rig));
        match exited(&s.control) {
            Some(SessionEvent::Exited { prefix_idle, marker_cleared, .. }) => assert!(prefix_idle && marker_cleared),
            other => panic!("no Exited event after {sig}: {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signal_stop_is_bounded_even_for_a_game_that_will_not_close() {
    let rig = Rig::new("sup-signal-force");
    let plan = rig.plan("ignoreterm;print:up;sleep:60000");
    let mut s = rig.start(&plan, &[]);
    wait_for("the game", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
    kill(s.running.child.id().unwrap(), "-TERM");
    tokio::time::timeout(Duration::from_secs(30), s.running.child.wait()).await.expect("bounded").unwrap();
    assert!(ProcTable::system().in_prefix(&rig.prefix).is_empty());
    assert!(!marker_paths(&rig).marker.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crashed_supervisor_leaves_a_truthful_marker_and_the_game_keeps_its_leases() {
    let rig = Rig::new("sup-crash");
    let pidfile = rig.root.join("game.pid");
    let plan = rig.plan(&format!("pidfile:{};print:up;sleep:60000", pidfile.display()));
    let mut s = rig.start(&plan, &[]);
    wait_for("the game", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
    // the supervisor dies without any chance to tidy up
    let supervisor = s.running.child.id().unwrap();
    kill(supervisor, "-KILL");
    s.running.child.wait().await.unwrap();
    let game = pid_of(&pidfile);
    assert!(alive(game), "the game is its own process group: it goes on");
    // the record says an unreturned session of this boot, owned by a supervisor that is gone
    let host = TestHost { kernel_log: None };
    let m = gpu_safety::read_marker(&marker_paths(&rig)).unwrap();
    assert_eq!(m.phase, gpu_safety::Phase::Running);
    assert_eq!(m.owner_pid, supervisor);
    assert_eq!(m.session, plan.session);
    let r =
        gpu_safety::check(&marker_paths(&rig), &host, &Env(vec![("XDG_SESSION_TYPE".into(), "wayland".into())]), false)
            .refusal
            .unwrap();
    assert_eq!(r.kind, gpu_safety::RefusalKind::InterruptedLaunch);
    // ... the game still holds the single-game lease and the prefix lease it inherited from the dead supervisor
    assert!(!game_lease_free(&rig), "a killed supervisor must not free the leases while the game runs");
    assert!(Lease::try_take(&rig.bedrock.join("profiles/p1/locks/launch.lease"), Kind::Exclusive).is_err());
    // the state file tells the window the supervisor is gone without having finished
    assert!(!s.control.supervisor_alive());
    assert_ne!(stage(&s.control), Some(Stage::Finished));
    let e = s.control.stop(StopMode::Graceful).await.unwrap_err();
    assert!(e.message.contains("supervisor is gone"), "{}", e.message);
    // the next start refuses while the game is up; when the orphan is stopped, the record is retired and the lease free
    let second = rig.plan("print:second;sleep:100");
    let s2 = rig.start(&second, &[]);
    wait_for("the second session to fail", Duration::from_secs(20), || finished(&s2.control));
    assert!(
        events(&s2.control)
            .iter()
            .any(|e| matches!(e, SessionEvent::Failed { kind: FailureKind::Busy | FailureKind::PrefixInUse, .. })),
        "{:?}",
        events(&s2.control)
    );
    let table = ProcTable::system();
    let reaper = Reaper { table: &table, signals: &RealSignals, scope: Scope::prefix_only(&rig.prefix) };
    let timing = Timing {
        term_grace: Duration::from_secs(2),
        kill_grace: Duration::from_secs(2),
        poll: Duration::from_millis(20),
        idle_timeout: Duration::from_secs(3),
        idle_confirmations: 2,
    };
    reaper.stop(timing, false, |_| {}, std::thread::sleep);
    assert!(reaper.wait_idle(timing, std::thread::sleep).is_empty());
    assert!(game_lease_free(&rig));
    let said = gpu_safety::retire_idle_current_boot_marker(&marker_paths(&rig), &host, true)
        .expect("retired once nothing is left");
    assert!(said.contains("supervisor stopped"));
    // and a start is possible again
    let third = rig.plan("print:third;sleep:200;exit:0");
    let s3 = rig.start(&third, &[]);
    wait_for("the third session", Duration::from_secs(30), || finished(&s3.control));
    assert!(matches!(exited(&s3.control), Some(SessionEvent::Exited { prefix_idle: true, marker_cleared: true, .. })));
    // the sweep of dead sessions removes the first one's folder, and only once its game is gone
    let report = super::run::cleanup_stale_sessions(&rig.bedrock, None);
    assert!(report.removed.contains(&plan.session_dir), "{report:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_window_closing_does_not_end_the_session() {
    let rig = Rig::new("sup-orphan");
    let plan = rig.plan("print:line one;sleep:1500;print:line two;exit:3");
    let Spawned { running, control, .. } = rig.start(&plan, &[]);
    control.wait_started(Duration::from_secs(20)).await.unwrap();
    // the window goes away: its pipes close and its handle on the supervisor is dropped
    drop(running);
    let reattached = Control::attach(&plan.session_dir);
    wait_for("the session to finish without anyone reading", Duration::from_secs(30), || finished(&reattached));
    match exited(&reattached).unwrap() {
        SessionEvent::Exited { code, prefix_idle, marker_cleared, .. } => {
            assert_eq!(code, Some(3));
            assert!(prefix_idle && marker_cleared);
        }
        _ => unreachable!(),
    }
    let log = read_log(&plan);
    assert!(log.contains("line one") && log.contains("line two"), "the whole log is kept: {log}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_game_is_refused_with_a_reason_and_leaves_the_first_alone() {
    let rig = Rig::new("sup-busy");
    let first = rig.plan("print:first;sleep:60000");
    let s1 = rig.start(&first, &[]);
    wait_for("the first game", Duration::from_secs(20), || events(&s1.control).contains(&SessionEvent::Ready));
    // another profile (another prefix) wants to play at the same time
    let mut second = rig.plan("print:second;sleep:1000");
    second.prefix = rig.bedrock.join("profiles/p2/compat/pfx");
    second.env.retain(|(k, _)| k != "WINEPREFIX");
    second.leases.prefix = Some(rig.bedrock.join("profiles/p2/locks/launch.lease"));
    std::fs::create_dir_all(&second.prefix).unwrap();
    let mut s2 = rig.start(&second, &[]);
    let err = s2.control.wait_started(Duration::from_secs(20)).await.unwrap_err();
    assert!(matches!(err.kind, crate::bedrock::ErrorKind::Busy), "{err:?}");
    assert!(err.message.contains("Another Bedrock game"), "{}", err.message);
    let status = s2.running.child.wait().await.unwrap();
    assert_eq!(status.code(), Some(75));
    // nothing of the refused one touched the first's record
    assert_eq!(gpu_safety::read_marker(&marker_paths(&rig)).unwrap().session, first.session);
    assert!(s1.control.supervisor_alive());
    s1.control.stop(StopMode::Force).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsafe_graphics_session_is_refused_before_anything_starts() {
    let rig = Rig::new("sup-gpu");
    let plan = rig.plan("print:never;sleep:1000");
    let mut s =
        start_with(&plan, &[], &[("BFB_FAKE_KERNEL_LOG", "NVRM: GPU 0000:01:00.0: GPU has fallen off the bus.\n")]);
    let err = s.control.wait_started(Duration::from_secs(20)).await.unwrap_err();
    assert!(err.message.contains("fatal kernel fault"), "{}", err.message);
    assert!(err.message.contains("did not start Wine"), "{}", err.message);
    assert_eq!(s.running.child.wait().await.unwrap().code(), Some(78));
    assert!(!marker_paths(&rig).marker.exists(), "a refused start leaves no record of a game");
    assert!(!read_log(&plan).contains("never"));
    assert!(game_lease_free(&rig));
    assert_eq!(stage(&s.control), Some(Stage::Failed));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_plan_is_refused_without_side_effects() {
    let rig = Rig::new("sup-badplan");
    let mut plan = rig.plan("sleep:100");
    plan.prefix = "/home".into();
    let err = spawn_with(&exe(), &[], &[], &plan, &[]).map(|_| ()).unwrap_err();
    assert!(matches!(err.kind, crate::bedrock::ErrorKind::InvalidRequest));
    assert!(!plan.session_dir.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn licensing_and_display_failures_are_reported_as_structured_findings() {
    use crate::bedrock::diagnostics::FindingKind;
    let rig = Rig::new("sup-findings");
    let plan = rig.plan("print:Error: Device group is full;print:x11drv: Can't open display :0;sleep:300;exit:0");
    let mut s = rig.start(&plan, &[]);
    let status = tokio::time::timeout(Duration::from_secs(30), s.running.child.wait()).await.unwrap().unwrap();
    // a game that "returned" 0 without ever opening is not a success
    assert_eq!(status.code(), Some(1));
    match exited(&s.control).unwrap() {
        SessionEvent::Exited { findings, .. } => {
            let kinds: Vec<_> = findings.iter().map(|f| f.kind).collect();
            assert!(
                kinds.contains(&FindingKind::LicenceRefused) && kinds.contains(&FindingKind::LostDisplay),
                "{kinds:?}"
            );
            assert!(
                findings
                    .iter()
                    .find(|f| f.kind == FindingKind::LicenceRefused)
                    .unwrap()
                    .text
                    .contains("ten Microsoft Store")
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_never_reach_the_log_or_the_window() {
    let rig = Rig::new("sup-redact");
    let jwt = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJkdW1teSJ9.c2lnbmF0dXJl";
    let spec = format!(
        "print:Authorization: Bearer dummyBearer123;print:{{\"refresh_token\":\"dummyRefresh456\"}};print:got {jwt} back;print:licence=SECRET-LICENCE-VALUE-99;big:150000;print:DAProperty=dummyDA789;print:ok;sleep:300;exit:0"
    );
    let plan = rig.plan(&spec);
    let mut s = rig.start(&plan, &["SECRET-LICENCE-VALUE-99".to_string()]);
    let mut window = String::new();
    let end = Instant::now() + Duration::from_secs(30);
    while let Ok(Some(l)) =
        tokio::time::timeout(end.saturating_duration_since(Instant::now()), s.running.lines.recv()).await
    {
        window.push_str(&l);
        window.push('\n');
    }
    let log = read_log(&plan);
    for text in [&log, &window] {
        for hidden in
            ["dummyBearer123", "dummyRefresh456", "eyJzdWIiOiJkdW1teSJ9", "SECRET-LICENCE-VALUE-99", "dummyDA789"]
        {
            assert!(!text.contains(hidden), "{hidden} leaked");
        }
        assert!(text.contains("REDACTED"), "nothing was redacted: {text}");
        assert!(text.lines().any(|l| l == "ok"), "the plain line after them is kept: {text}");
        // a 150 kB line is cut where the line reader cuts, not carried whole
        assert!(!text.contains(&"z".repeat(100_000)));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_loader_gets_the_map_it_expects_and_the_staging_is_gone_afterwards() {
    let rig = Rig::new("sup-map");
    let staging_root = rig.root.join("ram");
    let mut plan = rig.plan("load;print:loaded;sleep:700;exit:0");
    let staging = super::staging::StagingDir::create(&staging_root, &plan.session, 0).unwrap();
    let image = staging.stage_bytes("game.img", b"MZ decrypted image").unwrap();
    let spare = staging.stage_bytes("spare.img", b"not opened by the loader").unwrap();
    let mut map = super::staging::FileMap::new();
    map.add(&image, r"\??\Z:\games\mc\Minecraft.Windows.exe").unwrap();
    plan.env.push(("WINE_DLL_FILE_MAP".into(), map.value()));
    plan.staging_dir = Some(staging.hand_over());
    let mut s = rig.start(&plan, &[]);
    let mut lines = vec![];
    let end = Instant::now() + Duration::from_secs(30);
    while let Ok(Some(l)) =
        tokio::time::timeout(end.saturating_duration_since(Instant::now()), s.running.lines.recv()).await
    {
        lines.push(l);
    }
    assert!(
        lines.iter().any(|l| l.contains("loaded 18 bytes as \\??\\Z:\\games\\mc\\Minecraft.Windows.exe")),
        "{lines:?}"
    );
    assert!(!image.exists(), "the loader unlinked the image the moment it opened it");
    assert!(!spare.exists() && !staging_root.join(&plan.session).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_socket_path_longer_than_a_unix_socket_allows_still_works() {
    let rig = Rig::new("sup-longpath");
    let mut deep = rig.bedrock.join("sessions");
    for _ in 0..6 {
        deep = deep.join("a-rather-long-directory-name-for-a-profile");
    }
    let mut plan = rig.plan("print:up;sleep:60000");
    plan.session_dir = deep.join(&plan.session);
    assert!(plan.session_dir.join("control.sock").as_os_str().len() > 108);
    let s = rig.start(&plan, &[]);
    wait_for("the game", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
    assert!(s.control.subscribe().await.is_ok());
    assert!(s.control.stop(StopMode::Force).await.unwrap().prefix_idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn followers_see_the_history_and_then_the_end() {
    let rig = Rig::new("sup-subscribe");
    let plan = rig.plan("print:up;sleep:3000;exit:0");
    let s = rig.start(&plan, &[]);
    wait_for("the game", Duration::from_secs(20), || events(&s.control).contains(&SessionEvent::Ready));
    let mut rx = s.control.subscribe().await.unwrap();
    let mut got: Vec<SessionEvent> = vec![];
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await {
        got.push(e);
    }
    assert!(matches!(got.first(), Some(SessionEvent::Starting)), "history first: {got:?}");
    assert!(got.contains(&SessionEvent::Ready));
    assert!(matches!(got.last(), Some(SessionEvent::Exited { .. })), "{got:?}");
}

/// The state file of a finished session reads back as written.
#[test]
fn state_files_survive_a_round_trip() {
    let dir = scratch("sup-state");
    let state = State {
        session: "s".into(),
        supervisor_pid: 7,
        supervisor_start: Some(3),
        stage: Stage::Ready,
        events: vec![SessionEvent::Ready],
    };
    super::ipc::write_state(&dir, &state).unwrap();
    assert_eq!(super::ipc::read_state(&dir), Some(state));
    std::fs::write(dir.join("state.json"), b"{ torn").unwrap();
    assert!(super::ipc::read_state(&dir).is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

// ---- the output towards the window

#[tokio::test]
async fn a_window_that_stops_reading_never_stalls_the_supervisor() {
    // a writer that never accepts a byte
    struct Stuck;
    impl tokio::io::AsyncWrite for Stuck {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Pending
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }
    let (out, task) = super::run::output(8);
    let handle = tokio::spawn(task.run(Stuck));
    let started = Instant::now();
    for i in 0..200_000 {
        out.line(format!("line {i}"));
    }
    out.event(&SessionEvent::Ready);
    assert!(started.elapsed() < Duration::from_secs(5), "producing must never wait for the reader");
    drop(out);
    handle.abort();
}

#[tokio::test]
async fn a_slow_reader_is_told_how_many_lines_it_missed_and_events_always_arrive() {
    let (out, task) = super::run::output(4);
    // fill the queue before the writer runs
    for i in 0..100 {
        out.line(format!("line {i}"));
    }
    out.event(&SessionEvent::Ready);
    drop(out);
    let (mut a, b) = tokio::io::duplex(1 << 20);
    task.run(b).await;
    use tokio::io::AsyncReadExt;
    let mut text = String::new();
    a.read_to_string(&mut text).await.unwrap();
    assert!(text.contains(super::EVENT_PREFIX), "{text}");
    assert!(text.contains("lines were not shown here"), "{text}");
    assert!(text.contains("line 0") && !text.contains("line 99"), "the queue keeps the oldest lines: {text}");
}

// ---- the real thing

/// The hardware check of package E. Needs a prepared profile (packages A–D) and a person at the computer.
///
/// 1. Build a plan file for a prepared profile (the integrator's `play` writes `sessions/<id>/plan.json`) and set
///    `BFB_REAL_PLAN` to the folder holding it.
/// 2. `BFB_REAL_EXE=$(pwd)/target/debug/blockforge BFB_REAL_PLAN=<session folder> cargo test --locked \
///    real_supervised_launch -- --ignored --nocapture`
/// 3. The game window opens (first on the NVIDIA GTX 1650 Mobile, windowed/XWayland). Look at the log for
///    `Found device:` naming that card, use the menu, mouse and keyboard, load a single-player world, join a server.
/// 4. Close the window normally. The test then asserts the prefix is idle, the staging folder is gone and the
///    graphics record cleared. Run again with `BFB_REAL_STOP=1` to stop it through the control socket instead
///    (graceful), and `BFB_REAL_STOP=force` for the forced path on a disposable world.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a prepared Bedrock profile, a display and a person; see the doc comment"]
async fn real_supervised_launch() {
    let dir = PathBuf::from(std::env::var("BFB_REAL_PLAN").expect("BFB_REAL_PLAN: the session folder with plan.json"));
    let exe = PathBuf::from(
        std::env::var("BFB_REAL_EXE").expect("BFB_REAL_EXE: the blockforge binary with the session hook"),
    );
    let plan = super::ipc::read_plan(&dir).unwrap();
    let mut s = super::run::spawn(&exe, &plan, &[]).unwrap();
    s.control.wait_started(Duration::from_secs(60)).await.unwrap();
    let ctl = s.control.clone();
    let stopper = std::env::var("BFB_REAL_STOP").ok();
    if let Some(mode) = stopper {
        wait_for("the game to be up", Duration::from_secs(180), || events(&ctl).contains(&SessionEvent::Ready));
        std::thread::sleep(Duration::from_secs(20));
        ctl.stop(if mode == "force" { StopMode::Force } else { StopMode::Graceful }).await.unwrap();
    }
    while let Some(l) = s.running.lines.recv().await {
        println!("{l}");
    }
    let status = s.running.child.wait().await.unwrap();
    println!("supervisor ended: {status}");
    match exited(&ctl).expect("an Exited event") {
        SessionEvent::Exited { prefix_idle, marker_cleared, findings, .. } => {
            println!("findings: {findings:#?}");
            assert!(prefix_idle && marker_cleared);
        }
        _ => unreachable!(),
    }
    assert!(ProcTable::system().in_prefix(&plan.prefix).is_empty());
    if let Some(d) = &plan.staging_dir {
        assert!(!d.exists(), "staged images must be gone");
    }
}
