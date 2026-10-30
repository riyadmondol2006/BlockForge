//! Preparing a build: Horizon makes its resources (block shapes, models, data, the sounds) from the player's own copy
//! of the Minecraft version it pins, with its `horizon-prepare`. BlockForge hands it its own data folder, whose layout
//! horizon-prepare reads as it is, so the shared Minecraft files are only read. What it makes goes into the build's
//! `.local/` folder: about 1.4 GB, so a newer build whose preparer makes the same takes the older one's instead of
//! preparing again.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::release::{self, program};
use crate::install::{self, Job, Progress};
use crate::paths::Dirs;

/// What a build's research-lock.json pins of Minecraft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    pub version: String,
    pub client_sha1: String,
    pub server_sha1: String,
    pub java_major: Option<u32>,
}

pub fn read_lock(build: &Path) -> Result<Lock> {
    let file = build.join("research-lock.json");
    let json: Value =
        serde_json::from_slice(&std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?)
            .with_context(|| format!("reading {}", file.display()))?;
    let text = |k: &str| json["minecraft"][k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let missing = || anyhow::anyhow!("{} doesn't pin the Minecraft version and its files", file.display());
    Ok(Lock {
        version: text("version").ok_or_else(missing)?,
        client_sha1: text("client_sha1").ok_or_else(missing)?,
        server_sha1: text("server_sha1").ok_or_else(missing)?,
        java_major: json["minecraft"]["java_major"].as_u64().map(|n| n as u32),
    })
}

/// What a build's preparer makes, by number: `OUTPUT_VERSION` in its sources (crates/horizon-prepare/src/lib.rs, in
/// the release folder), which the Horizon project raises whenever the output changes.
pub fn output_version(build: &Path) -> Option<u32> {
    let lib = std::fs::read_to_string(build.join("crates/horizon-prepare/src/lib.rs")).ok()?;
    lib.lines()
        .find_map(|line| line.trim().strip_prefix("pub const OUTPUT_VERSION: u32 = ")?.strip_suffix(';')?.parse().ok())
}

/// Whether the resources `from` prepared are the ones preparing `to` would make: both pin the same Minecraft files
/// and their preparers make the same output (as the Horizon launcher decides it). A build without the number is never
/// taken for another.
pub fn compatible(from: &Path, to: &Path) -> bool {
    match (read_lock(from), read_lock(to)) {
        (Ok(a), Ok(b)) => {
            (a.version, a.client_sha1, a.server_sha1) == (b.version, b.client_sha1, b.server_sha1)
                && output_version(from).is_some()
                && output_version(from) == output_version(to)
        }
        _ => false,
    }
}

/// The prepared resources of `build` for Minecraft `mc`, which `horizon-client --resources` reads.
pub fn resources(build: &Path, mc: &str) -> PathBuf {
    build.join(".local").join("horizon-client").join(crate::paths::safe_component(mc))
}

/// Whether `build`'s resources are prepared: horizon-prepare writes their manifest last.
pub fn is_prepared(build: &Path) -> bool {
    read_lock(build).is_ok_and(|l| resources(build, &l.version).join("horizon-resources.json").is_file())
}

/// Whether the prepared resources hold the sounds (what the game's sound registry reads).
pub fn has_sounds(build: &Path) -> bool {
    read_lock(build).is_ok_and(|l| resources(build, &l.version).join("assets/minecraft/sounds.json").is_file())
}

/// The newest other installed build whose prepared resources `build` can take.
pub fn reusable(dirs: &Dirs, build: &Path) -> Option<PathBuf> {
    release::installed(dirs)
        .into_iter()
        .map(|(_, dir)| dir)
        .find(|dir| dir != build && is_prepared(dir) && compatible(dir, build))
}

/// Move the prepared resources of `from` into `to`. What a preparation of `to` left half done is dropped first.
pub fn move_resources(from: &Path, to: &Path) -> Result<()> {
    let target = to.join(".local");
    if target.exists() {
        std::fs::remove_dir_all(&target).with_context(|| format!("clearing {}", target.display()))?;
    }
    std::fs::rename(from.join(".local"), &target).with_context(|| format!("moving the resources into {}", to.display()))
}

/// Remove everything in `horizon/` that playing `keep` doesn't need: the other builds, half-unpacked ones, old
/// downloads and the server jars of other Minecraft versions. Only while no Horizon game runs (one may be playing
/// from an older build).
pub fn prune(dirs: &Dirs, keep: &Path) {
    if let Ok(rd) = std::fs::read_dir(release::builds_dir(dirs)) {
        for e in rd.flatten() {
            let path = e.path();
            if path != keep && path.is_dir() {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => crate::logs::info(format!("Horizon: removed the old build {}", path.display())),
                    Err(e) => crate::logs::warn(format!("Horizon: couldn't remove {}: {e}", path.display())),
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(release::horizon_dir(dirs).join("downloads"));
    let jar = read_lock(keep).ok().map(|l| release::server_jar(dirs, &l.version));
    if let Ok(rd) = std::fs::read_dir(release::horizon_dir(dirs).join("server-jars")) {
        for e in rd.flatten() {
            if Some(e.path()) != jar {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Builds whose sounds couldn't be copied in this run of BlockForge: not tried again on every Play (a Minecraft
/// install without them fails the same way each time).
static SOUNDS_FAILED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Get `build`'s resources ready: taken from an older build when `reuse` allows it and one fits, else prepared
/// (with Mojang's files for the pinned version and its Java, and the server jar), then the sounds. Returns the lock.
pub async fn get_ready(
    dirs: &Dirs,
    build: &Path,
    progress: &Arc<Progress>,
    reuse: bool,
    notes: &mut Vec<String>,
) -> Result<Lock> {
    let lock = read_lock(build)?;
    if !is_prepared(build) && reuse {
        if let Some(from) = reusable(dirs, build) {
            let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match move_resources(&from, build) {
                Ok(()) => notes.push(format!("Took the prepared resources of {} for {}", name(&from), name(build))),
                Err(e) => notes.push(format!("Couldn't take the prepared resources of {}: {e:#}", name(&from))),
            }
        }
    }
    let prepare = !is_prepared(build);
    let sounds =
        !has_sounds(build) && !SOUNDS_FAILED.lock().unwrap_or_else(|e| e.into_inner()).contains(&build.to_path_buf());
    if !prepare && !sounds {
        return Ok(lock);
    }

    // Mojang's files for the pinned version, which preparing and the sounds read, and the Java it pins
    progress.stage(format!("Reading Minecraft {}", lock.version));
    let version = install::vanilla_json(dirs, &lock.version).await?;
    let component = version
        .java_version
        .as_ref()
        .map(|j| j.component.clone())
        .filter(|c| !c.is_empty())
        .with_context(|| format!("Minecraft {} names no Java", lock.version))?;
    let java_home = crate::java::ensure_runtime(dirs, &component, progress).await.context("getting Java")?;
    install::ensure_game_files(dirs, &version, progress, false).await.context("getting the game files")?;
    let temp = dirs.temp.join("horizon");
    let _ = std::fs::create_dir_all(&temp);
    let mut env = tool_env(dirs, &java_home, &temp);
    // its Javas get their memory from horizon-prepare itself; how many run at once is what can be changed
    let (total, free) = (crate::launch::memory::total_ram_mb(), crate::launch::memory::available_mb());
    if prepare && super::tune::prepare_isolated(total, free) {
        env.push(("HORIZON_PREPARE_ISOLATED".to_string(), "1".to_string()));
        notes.push(format!(
            "Preparing with one Java at a time ({free} MB of {total} MB free): slower, but it needs much less memory"
        ));
    }

    if prepare {
        let jar = server_jar(dirs, &version, &lock, progress).await?;
        idle_counts(progress);
        progress.stage(format!("Preparing {}'s resources (about a minute)", super::NAME));
        let mut args = vec!["--prepare".into(), "--server-jar".into(), jar.display().to_string()];
        args.extend(["--minecraft-dir".into(), dirs.base.display().to_string()]);
        args.extend(["--root".into(), build.display().to_string()]);
        let ran = run_tool(&program(build, "horizon-prepare"), &args, build, &env, progress).await;
        let _ = std::fs::remove_dir_all(&temp);
        ran.context("preparing Horizon's resources")?;
        if !is_prepared(build) {
            bail!(
                "horizon-prepare finished but the resources aren't there ({})",
                resources(build, &lock.version).display()
            );
        }
    }
    if sounds && !has_sounds(build) && !progress.cancelled.load(Ordering::Relaxed) {
        let _ = std::fs::create_dir_all(&temp);
        idle_counts(progress);
        progress.stage("Copying the sounds from Minecraft's files");
        let args = vec![
            "sounds".into(),
            "--minecraft-dir".into(),
            dirs.base.display().to_string(),
            "--root".into(),
            build.display().to_string(),
        ];
        let ran = run_tool(&program(build, "horizon-prepare"), &args, build, &env, progress).await;
        let _ = std::fs::remove_dir_all(&temp);
        match ran {
            Err(e) if progress.cancelled.load(Ordering::Relaxed) => return Err(e),
            // the game plays without them, silently
            Err(e) => {
                SOUNDS_FAILED.lock().unwrap_or_else(|e| e.into_inner()).push(build.to_path_buf());
                notes.push(format!("{} will be silent: its sounds couldn't be copied ({e:#})", super::NAME));
            }
            Ok(()) if !has_sounds(build) => {
                SOUNDS_FAILED.lock().unwrap_or_else(|e| e.into_inner()).push(build.to_path_buf());
                notes.push(format!("{} will be silent: its sounds couldn't be copied", super::NAME));
            }
            Ok(()) => {}
        }
    }
    Ok(lock)
}

/// The environment horizon-prepare runs in: the pinned Java first, BlockForge's data folder as the Minecraft folder
/// (also when a version of it doesn't take `--minecraft-dir`), and its temporary files in BlockForge's temp folder.
fn tool_env(dirs: &Dirs, java_home: &Path, temp: &Path) -> Vec<(String, String)> {
    let mut path = vec![java_home.join("bin")];
    path.extend(std::env::var_os("PATH").iter().flat_map(std::env::split_paths));
    let mut env = vec![
        ("JAVA_HOME".to_string(), java_home.display().to_string()),
        ("HORIZON_MINECRAFT_DIR".to_string(), dirs.base.display().to_string()),
    ];
    if let Ok(joined) = std::env::join_paths(path) {
        env.push(("PATH".to_string(), joined.to_string_lossy().to_string()));
    }
    for var in ["TMPDIR", "TMP", "TEMP"] {
        env.push((var.to_string(), temp.display().to_string()));
    }
    env
}

/// No file counts while a tool runs, so the progress bar doesn't stay full from the download before.
fn idle_counts(progress: &Progress) {
    for n in [&progress.files_total, &progress.files_done, &progress.bytes_total, &progress.bytes_done] {
        n.store(0, Ordering::Relaxed);
    }
}

/// The server jar of the pinned version (Mojang's download in its version JSON), checked against the build's pin.
async fn server_jar(
    dirs: &Dirs,
    version: &crate::model::VersionJson,
    lock: &Lock,
    progress: &Arc<Progress>,
) -> Result<PathBuf> {
    let jar = release::server_jar(dirs, &lock.version);
    let server = version.downloads.as_ref().and_then(|d| d.extras.get("server"));
    let url = server.and_then(|s| s["url"].as_str()).unwrap_or_default();
    let size = server.and_then(|s| s["size"].as_i64());
    progress.stage(format!("Downloading the Minecraft {} server", lock.version));
    // hashed even when it is there: a jar that isn't the pinned one would make resources the game can't use
    let job = Job::new(url, &jar).checked(Some(&lock.server_sha1), size);
    install::download_all(vec![job], progress, true).await.context("getting the Minecraft server jar")?;
    Ok(jar)
}

/// How many of a tool's last lines are kept for the message when it fails.
const KEPT_LINES: usize = 6;

/// Run one of Horizon's tools, each line it prints going to `progress` and the log, and stop it when the player
/// cancels.
pub(crate) async fn run_tool(
    program: &Path,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    progress: &Arc<Progress>,
) -> Result<()> {
    let name = program.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    crate::logs::info(format!("Horizon: running {name} {}", args.join(" ")));
    let mut cmd = crate::platform::tokio_command(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // its own process group, so cancelling also ends the Java helpers it starts
    #[cfg(unix)]
    cmd.process_group(0);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().with_context(|| format!("starting {}", program.display()))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    for stream in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream);
            let mut buf = Vec::new();
            while reader.read_until(b'\n', &mut buf).await.is_ok_and(|n| n > 0) {
                let line = String::from_utf8_lossy(&buf).trim().to_string();
                buf.clear();
                if !line.is_empty() && tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut last: VecDeque<String> = VecDeque::new();
    let seen = |line: String, last: &mut VecDeque<String>| {
        crate::logs::info(format!("Horizon: {line}"));
        progress.stage(line.clone());
        last.push_back(line);
        if last.len() > KEPT_LINES {
            last.pop_front();
        }
    };
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut open = true;
    let status = loop {
        tokio::select! {
            line = rx.recv(), if open => match line {
                Some(line) => seen(line, &mut last),
                None => open = false,
            },
            status = child.wait() => break status?,
            _ = tick.tick() => {
                if progress.cancelled.load(Ordering::Relaxed) {
                    kill_group(&mut child).await;
                    bail!("cancelled");
                }
            }
        }
    };
    // what it printed last, which says why it stopped (a helper it started may hold the pipe a little longer)
    while let Ok(Some(line)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
        seen(line, &mut last);
    }
    if status.success() {
        return Ok(());
    }
    let said: Vec<String> = last.into_iter().map(|l| l.trim_start_matches("error: ").to_string()).collect();
    let code = crate::launch::exit_code(&status).map(|c| format!(" (exit code {c})")).unwrap_or_default();
    bail!("{name} stopped{code}: {}", said.join(" / "))
}

async fn kill_group(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        #[cfg(unix)]
        {
            let _ = crate::platform::command("kill").args(["-KILL", "--", &format!("-{pid}")]).status();
        }
        #[cfg(windows)]
        {
            let _ = crate::platform::command("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).status();
        }
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    fn build(dir: &Path, version: &str, client: &str, server: &str, output: Option<u32>) {
        std::fs::create_dir_all(dir.join("crates/horizon-prepare/src")).unwrap();
        std::fs::write(
            dir.join("research-lock.json"),
            format!(
                r#"{{"minecraft":{{"version":"{version}","client_sha1":"{client}","server_sha1":"{server}","java_major":25}},"other":{{}}}}"#
            ),
        )
        .unwrap();
        let lib = match output {
            Some(n) => format!("//! the preparer\n\npub const OUTPUT_VERSION: u32 = {n};\n\npub fn prepare() {{}}\n"),
            None => "pub fn prepare() {}\n".to_string(),
        };
        std::fs::write(dir.join("crates/horizon-prepare/src/lib.rs"), lib).unwrap();
        std::fs::write(program(dir, "horizon-client"), b"").unwrap();
        std::fs::write(program(dir, "horizon-prepare"), b"").unwrap();
        release::seal_for_tests(dir);
    }

    fn prepared(dir: &Path, mc: &str) {
        let res = resources(dir, mc);
        std::fs::create_dir_all(res.join("assets/minecraft")).unwrap();
        std::fs::write(res.join("horizon-resources.json"), b"{}").unwrap();
    }

    #[test]
    fn the_lock_is_read_and_one_without_pins_refused() {
        let base = scratch("hz-lock");
        build(&base, "26.3", "c1", "s1", Some(2));
        let lock = read_lock(&base).unwrap();
        assert_eq!(
            lock,
            Lock { version: "26.3".into(), client_sha1: "c1".into(), server_sha1: "s1".into(), java_major: Some(25) }
        );
        assert_eq!(output_version(&base), Some(2));
        std::fs::write(base.join("research-lock.json"), r#"{"minecraft":{"version":"26.3"}}"#).unwrap();
        assert!(read_lock(&base).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn resources_fit_another_build_only_with_the_same_pins_and_output() {
        let base = scratch("hz-compatible");
        let dir = |n: &str| base.join(n);
        build(&dir("a"), "26.3", "c1", "s1", Some(2));
        build(&dir("same"), "26.3", "c1", "s1", Some(2));
        build(&dir("output"), "26.3", "c1", "s1", Some(3));
        build(&dir("client"), "26.3", "c2", "s1", Some(2));
        build(&dir("server"), "26.3", "c1", "s2", Some(2));
        build(&dir("version"), "26.4", "c1", "s1", Some(2));
        build(&dir("unknown"), "26.3", "c1", "s1", None);
        build(&dir("unknown2"), "26.3", "c1", "s1", None);
        assert!(compatible(&dir("a"), &dir("same")));
        for other in ["output", "client", "server", "version", "unknown"] {
            assert!(!compatible(&dir("a"), &dir(other)), "{other}");
            assert!(!compatible(&dir(other), &dir("a")), "{other}");
        }
        // preparers from before the number can't be told apart this way
        assert!(!compatible(&dir("unknown"), &dir("unknown2")));
        assert!(!compatible(&dir("a"), &dir("missing")));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_newer_build_takes_a_fitting_older_ones_resources_and_pruning_keeps_only_it() {
        let base = scratch("hz-reuse");
        let dirs = Dirs::at(&base);
        let at = |t: &str| release::build_dir(&dirs, t);
        build(&at("v0.2.95"), "26.3", "c1", "s1", Some(2));
        build(&at("v0.2.96"), "26.3", "c1", "s1", Some(1));
        build(&at("v0.2.97"), "26.3", "c1", "s1", Some(2));
        prepared(&at("v0.2.95"), "26.3");
        prepared(&at("v0.2.96"), "26.3");
        assert!(is_prepared(&at("v0.2.95")) && !is_prepared(&at("v0.2.97")));
        assert!(!has_sounds(&at("v0.2.95")));
        // v0.2.96 is newer but its preparer made something else
        assert_eq!(reusable(&dirs, &at("v0.2.97")), Some(at("v0.2.95")));
        // a half-done preparation of the new build is replaced
        std::fs::create_dir_all(at("v0.2.97").join(".local/half")).unwrap();
        move_resources(&at("v0.2.95"), &at("v0.2.97")).unwrap();
        assert!(is_prepared(&at("v0.2.97")) && !is_prepared(&at("v0.2.95")));
        assert!(!at("v0.2.97").join(".local/half").exists());
        assert_eq!(reusable(&dirs, &at("v0.2.97")), None);

        std::fs::create_dir_all(release::builds_dir(&dirs).join("v0.2.98.partial")).unwrap();
        let jars = release::horizon_dir(&dirs).join("server-jars");
        std::fs::create_dir_all(&jars).unwrap();
        std::fs::write(jars.join("26.3.jar"), b"jar").unwrap();
        std::fs::write(jars.join("26.2.jar"), b"jar").unwrap();
        prune(&dirs, &at("v0.2.97"));
        let left: Vec<String> = release::installed(&dirs).into_iter().map(|(t, _)| t).collect();
        assert_eq!(left, ["v0.2.97"]);
        assert!(!release::builds_dir(&dirs).join("v0.2.98.partial").exists());
        assert!(jars.join("26.3.jar").exists() && !jars.join("26.2.jar").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_fails_says_what_it_printed_last_and_one_cancelled_is_stopped() {
        let base = scratch("hz-tool");
        let rt = crate::testutil::runtime();
        rt.block_on(async {
            let progress = Progress::new();
            let args = vec!["-c".to_string(), "echo step one; echo 'error: no client jar' >&2; exit 3".to_string()];
            let e = run_tool(Path::new("/bin/sh"), &args, &base, &[], &progress).await.unwrap_err();
            let text = format!("{e:#}");
            assert!(text.contains("exit code 3") && text.contains("no client jar"), "{text}");
            assert!(!text.contains("error: "), "{text}");

            let args = vec!["-c".to_string(), "echo working; sleep 30".to_string()];
            let started = std::time::Instant::now();
            let p = progress.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                p.cancelled.store(true, Ordering::Relaxed);
            });
            let e = run_tool(Path::new("/bin/sh"), &args, &base, &[], &progress).await.unwrap_err();
            assert!(format!("{e}").contains("cancelled"));
            assert!(started.elapsed() < Duration::from_secs(10));
            assert_eq!(progress.stage_text(), "working");
        });
        let _ = std::fs::remove_dir_all(base);
    }
}
