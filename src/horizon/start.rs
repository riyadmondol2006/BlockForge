//! Starting a prepared build through Horizon's Launch API (docs/api/launch.md in their repository): the build says
//! which options it takes (`horizon-client --launch-api`), only those are passed, and the account goes on the game's
//! standard input as the session JSON, never on the command line. Builds from before the Launch API (v0.2.97, the
//! newest release when this was written) don't answer `--launch-api`: their options are read from `--help`, and they
//! take the account as a session file, which they read once and delete.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::release::program;
use crate::accounts::Account;

/// Where a build's options are kept: a build never changes once unpacked, nor do they.
fn cache_file(build: &Path) -> PathBuf {
    build.join(".blockforge-launch-api.json")
}

/// The options of the build's Launch API answer.
pub fn parse_options(answer: &[u8]) -> Result<Vec<String>> {
    let json: Value = serde_json::from_slice(answer).context("reading horizon-client --launch-api")?;
    let options = json["options"].as_array().context("horizon-client --launch-api lists no options")?;
    Ok(options.iter().filter_map(|o| o["name"].as_str().map(str::to_string)).collect())
}

/// The options a build's `--help` lists: the lines that start with them (`  --width N, --height N   Window size`),
/// the description coming after a wider gap.
pub fn parse_help(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim_start).filter(|l| l.starts_with("--")) {
        let names = line.split("  ").next().unwrap_or_default();
        for word in names.split_whitespace().map(|w| w.trim_end_matches(',')) {
            let plain = word.len() > 2 && word[2..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            if word.starts_with("--") && plain && !found.iter().any(|f| f == word) {
                found.push(word.to_string());
            }
        }
    }
    found
}

/// The options `build`'s game takes, asking it once. The game runs with `home` as its data folder for this (a build
/// that doesn't know the question writes a log and a crash report), so nothing lands in Horizon's own folder.
pub async fn options(build: &Path, home: &Path) -> Result<Vec<String>> {
    if let Some(options) = std::fs::read(cache_file(build)).ok().and_then(|b| parse_options(&b).ok()) {
        return Ok(options);
    }
    let _ = std::fs::create_dir_all(home);
    let answer = ask(build, home, "--launch-api").await.and_then(|out| parse_options(&out));
    let options = match answer {
        Ok(options) => Ok(options),
        Err(_) => ask(build, home, "--help").await.map(|help| parse_help(&String::from_utf8_lossy(&help))),
    };
    let _ = std::fs::remove_dir_all(home);
    let options = options.context("asking horizon-client which options it takes")?;
    if options.is_empty() {
        bail!("horizon-client lists no options in --launch-api or --help");
    }
    let listed: Vec<Value> = options.iter().map(|o| serde_json::json!({ "name": o })).collect();
    let _ = std::fs::write(cache_file(build), serde_json::json!({ "options": listed }).to_string());
    Ok(options)
}

/// What `horizon-client <question>` prints, when it answers it.
async fn ask(build: &Path, home: &Path, question: &str) -> Result<Vec<u8>> {
    let client = program(build, "horizon-client");
    let mut cmd = crate::platform::tokio_command(&client);
    cmd.arg(question)
        .current_dir(build)
        .env("HORIZON_HOME", home)
        .env("HORIZON_TELEMETRY_URL", "off")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .with_context(|| format!("horizon-client {question} didn't answer"))?
        .with_context(|| format!("starting {}", client.display()))?;
    if !out.status.success() {
        bail!("horizon-client {question} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// How the game is handed the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handover {
    /// On its standard input (`--session-stdin`).
    Stdin,
    /// In this file, which the game reads once and deletes (`--session-file`).
    File(PathBuf),
}

/// What a start asks of the game, besides the account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub resources: PathBuf,
    pub game_dir: PathBuf,
    pub window: Option<(u32, u32)>,
    pub world: Option<String>,
    pub server: Option<String>,
    /// Where the account goes for a build that takes it only as a file.
    pub session_file: PathBuf,
}

/// The game's command line for `wanted`, with only the options the build lists (any other would be refused with
/// exit code 2), how it gets the account, and what was left out for the log. A build that can't be handed its
/// resources or the account can't be started at all. Without `--game-dir` and `--home` (builds before the Launch
/// API) the game's folder is HORIZON_HOME and the working folder, which the start sets to the instance.
pub fn command_line(options: &[String], wanted: &Wanted) -> Result<(Vec<String>, Handover, Vec<String>)> {
    let has = |o: &str| options.iter().any(|x| x == o);
    if !has("--resources") {
        bail!("this {} build doesn't take --resources, so BlockForge can't start it", super::NAME);
    }
    let mut args = vec!["--resources".to_string(), wanted.resources.display().to_string()];
    let mut notes = Vec::new();
    for option in ["--game-dir", "--home"] {
        if has(option) {
            args.extend([option.to_string(), wanted.game_dir.display().to_string()]);
        }
    }
    if let Some((w, h)) = wanted.window {
        if has("--width") && has("--height") {
            args.extend(["--width".to_string(), w.to_string(), "--height".to_string(), h.to_string()]);
        }
    }
    let handover = if has("--session-stdin") {
        args.push("--session-stdin".to_string());
        Handover::Stdin
    } else if has("--session-file") {
        args.extend(["--session-file".to_string(), wanted.session_file.display().to_string()]);
        Handover::File(wanted.session_file.clone())
    } else {
        bail!("this {} build can't be handed the account, so BlockForge can't start it", super::NAME);
    };
    if let Some(world) = wanted.world.as_deref().filter(|w| !w.is_empty()) {
        // the game opens a world by its folder name in saves/ and refuses a path
        if world.contains(['/', '\\']) || world == "." || world == ".." {
            notes.push(format!("Not opening \"{world}\": {} opens a world only by its folder name", super::NAME));
        } else if has("--quickPlaySingleplayer") {
            args.extend(["--quickPlaySingleplayer".to_string(), world.to_string()]);
        } else {
            notes.push(format!("This {} build can't open a world at start", super::NAME));
        }
    } else if let Some(server) = wanted.server.as_deref().filter(|s| !s.is_empty()) {
        if has("--quickPlayMultiplayer") {
            args.extend(["--quickPlayMultiplayer".to_string(), server.to_string()]);
        } else {
            notes.push(format!("This {} build can't join a server at start", super::NAME));
        }
    }
    Ok((args, handover, notes))
}

/// Write the session file for a build without `--session-stdin`: readable by its owner only (the game refuses it
/// otherwise), and new each time (the game refuses one older than two minutes).
pub fn write_session_file(path: &Path, session: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut file = open.open(path).with_context(|| format!("writing {}", path.display()))?;
    file.write_all(session)?;
    Ok(())
}

/// The session JSON the game is handed on its standard input: the account the game joins online-mode servers with.
pub fn session(account: &Account) -> Vec<u8> {
    serde_json::json!({
        "name": account.name(),
        "uuid": account.uuid_nodashes(),
        "access_token": account.access_token.clone().unwrap_or_default(),
        "user_type": "msa",
    })
    .to_string()
    .into_bytes()
}

/// The path in the game's last words about a crash ("Horizon: crash report saved to <path>").
pub fn crash_report<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> Option<PathBuf> {
    const SAID: &str = "Horizon: crash report saved to ";
    lines
        .into_iter()
        .filter_map(|l| l.as_ref().split_once(SAID).map(|(_, p)| p.trim().to_string()))
        .filter(|p| !p.is_empty())
        .last()
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a Nightly of 2026-10-30 answers to `horizon-client --launch-api`, cut down.
    const ANSWER: &str = r#"{"crash_report_line":"Horizon: crash report saved to ","exit_codes":[0,1,2,101],
        "horizon":"Nightly 2026-10-30","launch_api":"0.1.0","minecraft":"26.3","options":[{"name":"--resources","value":"DIR"},
        {"name":"--game-dir","value":"DIR"},{"name":"--home","value":"DIR"},{"name":"--width","value":"N"},
        {"name":"--height","value":"N"},{"name":"--fullscreen","value":null},{"name":"--session-file","value":"PATH"},
        {"name":"--session-stdin","value":null},{"name":"--quickPlaySingleplayer","value":"WORLD"},
        {"name":"--quickPlayMultiplayer","value":"HOST[:PORT]"},{"name":"--launch-api","value":null}]}"#;

    fn wanted() -> Wanted {
        Wanted {
            resources: PathBuf::from("/b/.local/horizon-client/26.3"),
            game_dir: PathBuf::from("/i/Horizon"),
            window: Some((1280, 720)),
            world: None,
            server: None,
            session_file: PathBuf::from("/t/session.json"),
        }
    }

    /// What v0.2.97's `horizon-client --help` prints, from before the Launch API, cut down.
    const HELP: &str = "Horizon \u{2014} Minecraft Java Edition 26.3 in Rust

  --resources PATH        Prepared resources (horizon-prepare)
  --render-distance N     Chunks, 2..64 (default 12)
  --width N, --height N   Window size (default 1280 x 720)
  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)
  --quickPlaySingleplayer WORLD  Open this world of saves/ at start, only if it exists (vanilla's)
  --quickPlayMultiplayer HOST[:PORT]  The same as --server (vanilla's name)
  --session-file PATH     Signed-in account from the launcher (read once, then deleted)

Controls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,
left click break, right click place/use, 1-9/scroll hotbar, E inventory,
Q drop, F3 debug, Esc pause.
";

    #[test]
    fn the_command_line_has_the_options_the_build_lists() {
        let options = parse_options(ANSWER.as_bytes()).unwrap();
        assert_eq!(options.len(), 11);
        let (args, handover, notes) = command_line(&options, &wanted()).unwrap();
        assert_eq!(
            args.join(" "),
            "--resources /b/.local/horizon-client/26.3 --game-dir /i/Horizon --home /i/Horizon --width 1280 --height \
             720 --session-stdin"
        );
        assert_eq!(handover, Handover::Stdin);
        assert!(notes.is_empty());
        let world = Wanted { world: Some("New World".into()), server: Some("example.org".into()), ..wanted() };
        let (args, _, _) = command_line(&options, &world).unwrap();
        // one at a time: the world wins, as in Minecraft's own Quick Play
        assert!(args.ends_with(&["--quickPlaySingleplayer".to_string(), "New World".to_string()]));
        let server = Wanted { server: Some("example.org:25566".into()), ..wanted() };
        assert!(command_line(&options, &server).unwrap().0.ends_with(&["example.org:25566".to_string()]));
        let path = Wanted { world: Some("../elsewhere".into()), ..wanted() };
        let (args, _, notes) = command_line(&options, &path).unwrap();
        assert!(!args.iter().any(|a| a.contains("elsewhere")) && notes.len() == 1);
    }

    #[test]
    fn a_build_from_before_the_launch_api_is_read_from_its_help_and_gets_a_session_file() {
        let options = parse_help(HELP);
        assert_eq!(
            options,
            [
                "--resources",
                "--render-distance",
                "--width",
                "--height",
                "--control-socket",
                "--quickPlaySingleplayer",
                "--quickPlayMultiplayer",
                "--session-file"
            ]
        );
        let (args, handover, _) = command_line(&options, &wanted()).unwrap();
        assert_eq!(
            args.join(" "),
            "--resources /b/.local/horizon-client/26.3 --width 1280 --height 720 --session-file /t/session.json"
        );
        assert_eq!(handover, Handover::File(PathBuf::from("/t/session.json")));
    }

    #[test]
    fn options_a_build_doesnt_list_are_left_out_and_without_the_needed_ones_it_cant_start() {
        let few: Vec<String> = ["--resources", "--game-dir", "--session-stdin", "--width"].map(String::from).to_vec();
        let w = Wanted { server: Some("example.org".into()), ..wanted() };
        let (args, _, notes) = command_line(&few, &w).unwrap();
        assert!(!args.iter().any(|a| a == "--home" || a == "--width" || a.contains("quickPlay")), "{args:?}");
        assert_eq!(notes.len(), 1);
        let no_account = vec!["--resources".to_string(), "--game-dir".to_string()];
        assert!(command_line(&no_account, &wanted()).is_err());
        let no_resources = vec!["--session-stdin".to_string()];
        assert!(command_line(&no_resources, &wanted()).is_err());
        assert!(parse_options(b"{}").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_session_file_is_for_its_owner_only_and_new() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::testutil::scratch("hz-session");
        let file = dir.join("s/session.json");
        write_session_file(&file, b"old").unwrap();
        write_session_file(&file, b"{}").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"{}");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_session_carries_the_id_without_dashes() {
        let account = Account {
            username: "Alex".into(),
            uuid: "069a79f4-44e9-4726-a5be-fca90e38aaf5".into(),
            access_token: Some("tok".into()),
            ..Default::default()
        };
        let json: Value = serde_json::from_slice(&session(&account)).unwrap();
        assert_eq!(json["uuid"], "069a79f444e94726a5befca90e38aaf5");
        assert_eq!((json["name"].as_str(), json["access_token"].as_str()), (Some("Alex"), Some("tok")));
        assert_eq!(json["user_type"], "msa");
    }

    #[test]
    fn the_crash_report_is_found_in_the_last_lines() {
        let lines =
            ["[main] INFO starting", "thread 'main' panicked", "Horizon: crash report saved to /i/crash-reports/a.txt"];
        assert_eq!(crash_report(lines), Some(PathBuf::from("/i/crash-reports/a.txt")));
        assert_eq!(crash_report(["nothing happened"]), None);
    }
}
