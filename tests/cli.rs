//! The `blockforge` program run the way people run it, against a data folder of its own: odd names, broken files,
//! commands typed in strange places. Nothing here needs the network.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn data_dir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("bf-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Runs the program with `args` in `cwd`, killing it if it takes longer than a few seconds (a command that opened
/// the window instead of doing its job would otherwise hang the test).
fn run_in(data: &Path, cwd: &Path, args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_blockforge"))
        .args(args)
        .current_dir(cwd)
        .env("BLOCKFORGE_DIR", data)
        // no window can open in a test, whatever happens
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("blockforge {args:?} didn't finish");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run(data: &Path, args: &[&str]) -> Output {
    run_in(data, data, args)
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn version_and_paths_work_on_an_empty_data_folder() {
    let data = data_dir("version");
    let v = run(&data, &["--version"]);
    assert!(v.status.success(), "{}", text(&v));
    assert!(text(&v).contains(env!("CARGO_PKG_VERSION")));
    let p = run(&data, &["paths"]);
    assert!(p.status.success(), "{}", text(&p));
    assert!(text(&p).contains(&data.display().to_string()));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn player_names_minecraft_would_refuse_are_refused_up_front() {
    let data = data_dir("names");
    for bad in ["a b", "ab", "this_name_is_far_too_long", "Stéve", "<script>", ""] {
        let o = run(&data, &["account", "offline", bad]);
        assert!(!o.status.success(), "{bad:?} was accepted: {}", text(&o));
        assert!(text(&o).contains("player name"), "{}", text(&o));
    }
    assert!(!data.join("configs/accounts.json").exists(), "nothing was saved for a refused name");
    let ok = run(&data, &["account", "offline", "Alex_99"]);
    assert!(ok.status.success(), "{}", text(&ok));
    let list = run(&data, &["account"]);
    assert!(text(&list).contains("Alex_99"), "{}", text(&list));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_damaged_accounts_file_is_kept_and_the_program_carries_on() {
    let data = data_dir("damaged");
    std::fs::create_dir_all(data.join("configs")).unwrap();
    std::fs::write(data.join("configs/accounts.json"), "[{\"username\":\"Alex\",").unwrap();
    std::fs::write(data.join("configs/BlockForge.json"), "{\"maximumMemory\": \"lots\", \"windowWidth\": 1000")
        .unwrap();
    let list = run(&data, &["account"]);
    assert!(list.status.success(), "{}", text(&list));
    let add = run(&data, &["account", "offline", "Bob"]);
    assert!(add.status.success(), "{}", text(&add));
    let kept: Vec<String> = std::fs::read_dir(data.join("configs"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".broken-"))
        .collect();
    assert!(kept.iter().any(|n| n.starts_with("accounts.json")), "{kept:?}");
    assert!(text(&run(&data, &["account"])).contains("Bob"));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_command_is_a_command_even_next_to_a_file_of_that_name() {
    let data = data_dir("cwd");
    let cwd = data.join("work");
    std::fs::create_dir_all(cwd.join("java")).unwrap();
    std::fs::write(cwd.join("list"), b"not an instance").unwrap();
    std::fs::write(cwd.join("paths"), b"").unwrap();
    for cmd in ["list", "paths"] {
        let o = run_in(&data, &cwd, &[cmd]);
        assert!(o.status.success(), "{cmd}: {}", text(&o));
    }
    let o = run_in(&data, &cwd, &["paths"]);
    assert!(text(&o).contains("data folder"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn commands_on_instances_that_dont_exist_explain_themselves() {
    let data = data_dir("missing");
    for args in [
        vec!["delete", "Nope"],
        vec!["repair", "Nope"],
        vec!["launch", "Nope", "--dry-run"],
        vec!["delete", "../../etc"],
        vec!["launch", "", "--dry-run"],
    ] {
        let o = run(&data, &args);
        assert!(!o.status.success(), "{args:?}");
        assert!(text(&o).contains("no instance called"), "{args:?}: {}", text(&o));
    }
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn broken_instances_are_listed_around_not_crashed_on() {
    let data = data_dir("broken-list");
    for (folder, json) in [("Empty", ""), ("Garbage", "{{{{"), ("Array", "[]")] {
        std::fs::create_dir_all(data.join("instances").join(folder)).unwrap();
        std::fs::write(data.join("instances").join(folder).join("instance.json"), json).unwrap();
    }
    std::fs::create_dir_all(data.join("instances/Good")).unwrap();
    std::fs::write(data.join("instances/Good/instance.json"), r#"{"launcher":{"name":"Good Pack"},"id":"1.20.1"}"#)
        .unwrap();
    // a restore that was interrupted leaves a work folder that must not show as an instance
    std::fs::create_dir_all(data.join("instances/.restore-abc")).unwrap();
    std::fs::write(data.join("instances/.restore-abc/instance.json"), r#"{"launcher":{"name":"Ghost"}}"#).unwrap();
    let o = run(&data, &["list"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("Good Pack"));
    assert!(!text(&o).contains("Ghost"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn unknown_commands_and_options_fail_cleanly() {
    let data = data_dir("unknown");
    for args in [vec!["frobnicate"], vec!["launch"], vec!["account", "offline"], vec!["--no-such-option"]] {
        let o = run(&data, &args);
        assert!(!o.status.success(), "{args:?}");
        assert!(!text(&o).contains("panicked"), "{args:?}: {}", text(&o));
    }
    let _ = std::fs::remove_dir_all(&data);
}
