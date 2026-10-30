//! Horizon for real: the newest release from GitHub installed, prepared from Minecraft 26.3 and started to its title
//! screen. It downloads about 100 MB and takes a few minutes, so it runs only when asked:
//!
//! ```sh
//! BLOCKFORGE_DIR=<a scratch data folder> cargo test --locked horizon_real -- --ignored --nocapture
//! ```
//!
//! `BLOCKFORGE_DIR` must be given (the test never writes into the real data folder) and must hold Minecraft 26.3:
//! `configs/json/minecraft/26.3.json`, `libraries/net/minecraft/client/26.3/client-26.3.jar`, `assets/` and
//! `runtimes/` (links to a real data folder's do, as they are only read when complete). Without them it skips
//! itself. Needs a screen; HORIZON_NO_AUDIO=1 is set for the game.

use std::path::Path;
use std::time::{Duration, Instant};

use super::*;

#[test]
#[ignore = "downloads Horizon and starts it; run by hand with BLOCKFORGE_DIR set"]
fn horizon_real() {
    let Some(base) = std::env::var_os("BLOCKFORGE_DIR").filter(|v| !v.is_empty()) else {
        eprintln!("skipped: set BLOCKFORGE_DIR to a scratch data folder");
        return;
    };
    let dirs = Dirs::at(base);
    let needed = [
        dirs.configs.join("json/minecraft").join(format!("{MINECRAFT_VERSION}.json")),
        crate::install::client_jar(&dirs, MINECRAFT_VERSION),
        dirs.assets.join("indexes"),
        dirs.runtimes.clone(),
    ];
    if !supported() || needed.iter().any(|p| !p.exists()) {
        eprintln!("skipped: {} doesn't hold Minecraft {MINECRAFT_VERSION}", dirs.base.display());
        return;
    }
    let rt = crate::testutil::runtime();
    rt.block_on(async {
        let progress = Progress::new();
        let began = Instant::now();
        let watch = {
            let p = progress.clone();
            tokio::spawn(async move {
                let mut last = String::new();
                loop {
                    let now = p.stage_text();
                    if now != last {
                        eprintln!("  {:6.1} s · {now}", began.elapsed().as_secs_f32());
                        last = now;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            })
        };

        let t = Instant::now();
        let inst = create_instance(&dirs, "Horizon test", &progress).await.expect("creating the instance");
        eprintln!(
            "created {} in {:.1} s; notes: {:?}",
            inst.root.display(),
            t.elapsed().as_secs_f32(),
            progress.take_notes()
        );
        assert!(is_horizon(&inst));
        let build = installed_build(&dirs).expect("a build is installed");
        eprintln!("build: {build:?}");
        assert!(build.prepared, "the build isn't prepared");

        let account = Account {
            username: "HorizonTest".into(),
            uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5".into(),
            access_token: Some("not-a-real-token-for-the-test".into()),
            ..Default::default()
        };
        let t = Instant::now();
        let mut start =
            prepare(&dirs, &Settings::default(), &inst, &account, &progress, &LaunchOptions::default(), false)
                .await
                .expect("preparing to play");
        eprintln!("ready to play in {:.1} s; notes: {:?}", t.elapsed().as_secs_f32(), start.notes);
        let shown = start.display();
        eprintln!("command line: {shown}");
        assert!(!shown.contains("not-a-real-token"));
        watch.abort();

        // the control socket lets horizon-ctl see the title screen and quit the game
        let socket = inst.root.join("ctl.sock");
        start.args.extend(["--control-socket".to_string(), socket.display().to_string()]);
        start.env.push(("HORIZON_NO_AUDIO".to_string(), "1".to_string()));
        let t = Instant::now();
        let mut running = start.spawn().expect("starting the game");
        let ctl = |args: &[&str]| {
            let build_dir = release::build_dir(&dirs, &build.tag);
            let out = crate::platform::command(release::program(&build_dir, "horizon-ctl"))
                .arg("--socket")
                .arg(&socket)
                .args(args)
                .output();
            out.map(|o| (o.status.success(), String::from_utf8_lossy(&o.stdout).to_string()))
        };
        let mut title = None;
        while t.elapsed() < Duration::from_secs(40) {
            if Path::new(&socket).exists() {
                if let Ok((true, reply)) = ctl(&["wait-for", "screen", "title", "--timeout", "20"]) {
                    title = Some((t.elapsed(), reply));
                    break;
                }
            }
            if running.child.try_wait().ok().flatten().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if title.is_some() {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let shot = inst.root.join("title.png");
            let _ = ctl(&["screenshot", &shot.display().to_string()]);
            eprintln!("screenshot: {} ({})", shot.display(), if shot.is_file() { "saved" } else { "missing" });
            let _ = ctl(&["quit"]);
        }
        let ended = tokio::time::timeout(Duration::from_secs(20), running.child.wait()).await;
        if ended.is_err() {
            running.kill_tree().await;
        }
        let mut lines = Vec::new();
        while let Ok(Some(line)) = tokio::time::timeout(Duration::from_secs(2), running.lines.recv()).await {
            lines.push(line);
        }
        eprintln!("the game said ({} lines), last ones:", lines.len());
        for line in lines.iter().rev().take(15).rev() {
            eprintln!("  | {line}");
        }
        assert!(!lines.iter().any(|l| l.contains("not-a-real-token")), "the token reached the log");
        let (at, reply) = title.expect("the game didn't reach its title screen");
        eprintln!("title screen after {:.1} s: {}", at.as_secs_f32(), reply.trim());
        if let Ok(Ok(status)) = ended {
            eprintln!("exit: {:?}", crate::launch::exit_code(&status));
        }
        assert!(crash_report(&lines).is_none(), "the game crashed");
    });
}
