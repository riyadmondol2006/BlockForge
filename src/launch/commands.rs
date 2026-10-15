//! The commands a player can have run before the game starts and after it closes (the `enableCommands`,
//! `preLaunchCommand` and `postExitCommand` settings), and the wrapper command that goes in front of the game's own
//! command line. A port of the Java edition's `CommandExecutor` and `MCLauncher.wrapArguments`.

use std::collections::BTreeMap;
use std::path::Path;

use crate::instances::Instance;
use crate::launch::javapick::JavaPick;
use crate::launch::settings::LaunchSettings;

/// A command that did not finish well; `message` is what to tell the player.
#[derive(Debug, Clone)]
pub struct CommandError {
    pub message: String,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

/// What a command printed.
#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    pub lines: Vec<String>,
}

/// The `$NAME` words a command can use.
///
/// * `$INST_NAME` the instance's name
/// * `$INST_ID` the name of its folder
/// * `$INST_DIR` and `$INST_MC_DIR` the full path of its folder
/// * `$INST_JAVA` the Java that runs it
/// * `$INST_JAVA_ARGS` the Java arguments (not the game's, which have the account in them)
pub fn tokens(instance: &Instance, ls: &LaunchSettings, java: &JavaPick) -> BTreeMap<&'static str, String> {
    let dir = instance.root.to_string_lossy().to_string();
    let mut t = BTreeMap::new();
    t.insert("INST_NAME", instance.name().to_string());
    t.insert("INST_ID", instance.folder.clone());
    t.insert("INST_DIR", dir.clone());
    t.insert("INST_MC_DIR", dir);
    t.insert("INST_JAVA", java.binary.to_string_lossy().to_string());
    t.insert("INST_JAVA_ARGS", ls.java_arguments.clone());
    t
}

/// Replaces `$UPPER_CASE` words that are in `tokens`; any other `$WORD` stays as written.
pub fn substitute(tokens: &BTreeMap<&'static str, String>, command: &str) -> String {
    let chars: Vec<char> = command.chars().collect();
    let mut out = String::with_capacity(command.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_uppercase() || chars[j] == '_') {
                j += 1;
            }
            if j > i + 1 {
                let key: String = chars[i + 1..j].iter().collect();
                if let Some(v) = tokens.get(key.as_str()) {
                    out.push_str(v);
                    i = j;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn build(command: &str, cwd: &Path) -> std::process::Command {
    // linux and macOS run it through the shell; Windows through cmd, so built-in commands like `echo` work too
    let mut c = if cfg!(windows) {
        let mut c = crate::platform::command("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = crate::platform::command("/bin/sh");
        c.arg("-c").arg(command);
        c
    };
    c.current_dir(cwd).stdin(std::process::Stdio::null());
    c
}

fn finish(out: std::process::Output) -> Result<CommandOutput, CommandError> {
    let lines: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
    if out.status.success() {
        return Ok(CommandOutput { lines });
    }
    // cut off any initial blank lines of the error output
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim_start_matches(['\r', '\n']).trim_end().to_string();
    let message = if err.is_empty() { format!("The command stopped with {}", out.status) } else { err };
    Err(CommandError { message })
}

/// Runs a command in the instance's folder and waits for it. A non-zero exit is an error.
pub fn run_blocking(
    instance: &Instance,
    command: &str,
    ls: &LaunchSettings,
    java: &JavaPick,
) -> Result<CommandOutput, CommandError> {
    let command = substitute(&tokens(instance, ls, java), command);
    let out = build(&command, &instance.root)
        .output()
        .map_err(|e| CommandError { message: format!("Couldn't run \"{command}\": {e}") })?;
    finish(out)
}

/// The same, without holding up the program's other work.
pub async fn run(
    instance: &Instance,
    command: &str,
    ls: &LaunchSettings,
    java: &JavaPick,
) -> Result<CommandOutput, CommandError> {
    let command = substitute(&tokens(instance, ls, java), command);
    let root = instance.root.clone();
    let shown = command.clone();
    tokio::task::spawn_blocking(move || build(&command, &root).output())
        .await
        .map_err(|e| CommandError { message: e.to_string() })?
        .map_err(|e| CommandError { message: format!("Couldn't run \"{shown}\": {e}") })
        .and_then(finish)
}

/// Puts the game's command line into the player's wrapper command (`gamemoderun %command%`, `taskset -c 0-3`). Where
/// `%command%` stands, the game goes in; with `%"command"%` it goes in as one quoted string (for `sh -c`); with
/// neither it is added at the end.
pub fn wrap_arguments(wrapper: &str, args: Vec<String>) -> Vec<String> {
    let mut wrap: Vec<String> = wrapper.split_whitespace().map(str::to_string).collect();
    // wrapper not set
    if wrap.is_empty() {
        return args;
    }
    if let Some(i) = wrap.iter().position(|w| w == "%command%") {
        wrap.splice(i..=i, args);
        return wrap;
    }
    // the whole command as one string, useful in the case of ''
    if let Some(i) = wrap.iter().position(|w| w == "%\"command\"%") {
        wrap[i] = format!("'{}'", args.join("' '"));
        return wrap;
    }
    // fall back to wrapping the command with the rest of the arguments added in
    wrap.extend(args);
    wrap
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::instances::InstanceFile;

    fn instance(root: &Path) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher.name = "My Pack".into();
        Instance { folder: "MyPack".into(), mod_count: 0, root: root.to_path_buf(), file }
    }

    fn java() -> JavaPick {
        crate::launch::javapick::JavaPick {
            home: None,
            binary: "/opt/jdk/bin/java".into(),
            major: None,
            arch: None,
            is_minecraft_runtime: false,
            component: None,
            runtime_missing: false,
            reasons: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn tokens_are_replaced_and_unknown_words_stay() {
        let i = instance(Path::new("/data/instances/MyPack"));
        let ls = LaunchSettings::resolve(&Settings::default(), &i);
        let t = tokens(&i, &ls, &java());
        assert_eq!(
            substitute(&t, "echo $INST_NAME in $INST_DIR ($INST_ID) $HOME $UNKNOWN"),
            "echo My Pack in /data/instances/MyPack (MyPack) $HOME $UNKNOWN"
        );
        assert_eq!(substitute(&t, "$INST_JAVA -version"), "/opt/jdk/bin/java -version");
        assert!(substitute(&t, "$INST_JAVA_ARGS").starts_with("-XX:+UnlockExperimentalVMOptions"));
        assert_eq!(substitute(&t, "cost $5 and $"), "cost $5 and $");
    }

    #[test]
    fn wrappers_put_the_game_where_the_player_says() {
        let game: Vec<String> = ["java", "-Xmx2G", "Main"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            wrap_arguments("gamemoderun %command%", game.clone()),
            vec!["gamemoderun", "java", "-Xmx2G", "Main"]
        );
        assert_eq!(
            wrap_arguments("env A=1 %command% --extra", game.clone()),
            vec!["env", "A=1", "java", "-Xmx2G", "Main", "--extra"]
        );
        assert_eq!(
            wrap_arguments("taskset -c 0-3", game.clone()),
            vec!["taskset", "-c", "0-3", "java", "-Xmx2G", "Main"]
        );
        assert_eq!(wrap_arguments("sh -c %\"command\"%", game.clone()), vec!["sh", "-c", "'java' '-Xmx2G' 'Main'"]);
        assert_eq!(wrap_arguments("   ", game.clone()), game);
    }

    #[cfg(unix)]
    #[test]
    fn commands_run_in_the_instance_folder() {
        let dir = std::env::temp_dir().join(format!("bf-cmd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let i = instance(&dir);
        let ls = LaunchSettings::resolve(&Settings::default(), &i);
        let out = run_blocking(&i, "pwd; echo $INST_ID", &ls, &java()).unwrap();
        assert_eq!(out.lines.len(), 2);
        assert!(out.lines[0].ends_with(dir.file_name().unwrap().to_str().unwrap()));
        assert_eq!(out.lines[1], "MyPack");
        let err = run_blocking(&i, "echo oops >&2; exit 3", &ls, &java()).unwrap_err();
        assert_eq!(err.message, "oops");
        let silent = run_blocking(&i, "exit 2", &ls, &java()).unwrap_err();
        assert!(silent.message.contains("stopped with"));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        assert!(rt.block_on(run(&i, "true", &ls, &java())).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
