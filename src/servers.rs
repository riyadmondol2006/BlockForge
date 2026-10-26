//! Servers: pack servers the Java edition installed under `servers/<name>/`, each described by a `server.json`.
//! This lists them, starts them in a terminal window, backs them up and removes them (`ServerManager` and the
//! launch/backup parts of `Server`).
//!
//! What is not here: installing a server (downloading a pack's server files and writing `server.json`), because
//! that is part of the Java edition's pack installer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::accounts::Account;
use crate::config::write_json_atomic;
use crate::instances::LoaderVersion;
use crate::model::JavaVersion;
use crate::paths::Dirs;
use crate::platform::{self, Os};

/// A server, as `server.json` describes it. Fields this program doesn't use (the mod list, CurseForge and
/// Modrinth details) are kept as they are.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Server {
    pub name: String,
    pub description: Option<String>,
    pub pack: String,
    pub vanilla_instance: Option<bool>,
    pub pack_id: Option<i64>,
    pub external_pack_id: Option<i64>,
    pub minecraft_version: String,
    pub version: String,
    pub hash: Option<String>,
    pub is_patched_for_log4_shell: bool,
    pub loader_version: Option<LoaderVersion>,
    pub java_version: Option<JavaVersion>,
    pub is_dev: bool,
    #[serde(flatten)]
    pub extras: Map<String, Value>,
    /// The folder the server lives in. Not part of the file.
    #[serde(skip)]
    pub root: PathBuf,
}

impl Server {
    /// Letters and digits of the name only (`getSafeName`).
    pub fn safe_name(&self) -> String {
        self.name.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
    }

    pub fn server_file(&self) -> PathBuf {
        self.root.join("server.json")
    }

    pub fn save(&self) -> Result<()> {
        write_json_atomic(&self.server_file(), self)
    }

    /// `Fabric 0.18.4`, or `None` for a vanilla server.
    pub fn loader_label(&self) -> Option<String> {
        self.loader_version.as_ref().filter(|l| !l.kind.is_empty()).map(|l| format!("{} {}", l.kind, l.version))
    }

    /// The mods listed in `server.json`, as far as their names go.
    pub fn mod_names(&self) -> Vec<String> {
        self.extras
            .get("mods")
            .and_then(|m| m.as_array())
            .map(|a| a.iter().filter_map(|m| m["name"].as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    /// The script that starts this server on this computer, or `None` when the folder has none of the usual ones.
    pub fn script(&self) -> Option<&'static str> {
        script_for(platform::os(), |name| self.root.join(name).exists())
    }

    /// The Java edition's own `LaunchServer` script is the only one the launcher can hand a Java to.
    pub fn is_launcher_script(&self) -> bool {
        match platform::os() {
            Os::Windows => self.root.join("LaunchServer.bat").exists(),
            _ => self.root.join("LaunchServer.sh").exists(),
        }
    }
}

/// The start scripts to look for, best first.
pub fn script_for(os: Os, exists: impl Fn(&str) -> bool) -> Option<&'static str> {
    let names: &[&'static str] = match os {
        Os::Windows => &["LaunchServer.bat", "start.ps1", "run.bat", "StartServer.bat", "ServerStart.bat"],
        _ => &["LaunchServer.sh", "start.sh", "run.sh", "StartServer.sh", "ServerStart.sh"],
    };
    names.iter().copied().find(|n| exists(n))
}

// ---------------------------------------------------------------------------------------------------------
// The list

pub fn servers_dir(dirs: &Dirs) -> PathBuf {
    dirs.base.join("servers")
}

/// Read one server folder.
pub fn load_one(root: &Path) -> Result<Server> {
    let bytes = std::fs::read(root.join("server.json"))
        .with_context(|| format!("reading {}", root.join("server.json").display()))?;
    let mut server: Server = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a valid server file", root.join("server.json").display()))?;
    server.root = root.to_path_buf();
    if server.name.is_empty() {
        server.name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    }
    Ok(server)
}

/// Every installed server: the folders of `servers/` that hold a `server.json`, by name. A folder that can't be
/// read is logged and skipped.
pub fn load_all(dirs: &Dirs) -> Vec<Server> {
    let Ok(rd) = std::fs::read_dir(servers_dir(dirs)) else { return Vec::new() };
    let mut out: Vec<Server> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("server.json").exists())
        .filter_map(|p| match load_one(&p) {
            Ok(s) => Some(s),
            Err(e) => {
                crate::logs::error(format!("Failed to load server in the folder {}: {e:#}", p.display()));
                None
            }
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Whether a server with this name exists already (`ServerManager.isServer`): names are compared by their letters
/// and digits, without regard to case.
pub fn is_server(servers: &[Server], name: &str) -> bool {
    let wanted: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    servers.iter().any(|s| s.safe_name().eq_ignore_ascii_case(&wanted))
}

/// The server called `name`, by its name or its safe name (`--launch-server`).
pub fn find<'a>(servers: &'a [Server], name: &str) -> Option<&'a Server> {
    servers.iter().find(|s| s.name.eq_ignore_ascii_case(name) || s.safe_name().eq_ignore_ascii_case(name))
}

/// Delete a server and everything in it. This can't be undone.
pub fn remove(server: &Server) -> Result<()> {
    if server.root.as_os_str().is_empty() || !server.root.join("server.json").exists() {
        bail!("{} doesn't look like a server folder, so it was not deleted", server.root.display());
    }
    if std::fs::symlink_metadata(&server.root)?.file_type().is_symlink() {
        bail!("Not deleting {} as it's a symlink!", server.root.display());
    }
    std::fs::remove_dir_all(&server.root).with_context(|| format!("deleting {}", server.root.display()))
}

// ---------------------------------------------------------------------------------------------------------
// Collapsed cards: the Java edition remembers, per account, which servers were folded shut

/// Whether the account has this server's card folded shut.
pub fn is_collapsed(account: &Account, server: &Server) -> bool {
    account
        .extras
        .get("collapsedServers")
        .and_then(|v| v.as_array())
        .is_some_and(|a| a.iter().any(|n| n.as_str() == Some(&server.name)))
}

/// Fold a server's card shut or open it, for this account (`setServerVisibility`). The caller saves the accounts.
pub fn set_collapsed(account: &mut Account, server: &Server, collapsed: bool) {
    let entry = account.extras.entry("collapsedServers").or_insert_with(|| Value::Array(Vec::new()));
    let Some(list) = entry.as_array_mut() else { return };
    list.retain(|n| n.as_str() != Some(&server.name));
    if collapsed {
        list.push(Value::from(server.name.clone()));
    }
}

// ---------------------------------------------------------------------------------------------------------
// Starting

/// What starting a server needs to do.
#[derive(Clone, Debug, PartialEq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    /// Files to write before starting (macOS runs a throwaway script).
    pub files: Vec<(PathBuf, String)>,
}

/// Why a server can't be started right now.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LaunchProblem {
    /// None of the usual start scripts is in the folder: run it by hand.
    NoScript,
    /// No terminal program is known on this computer; the script has to be run by hand.
    NoTerminal(String),
}

impl std::fmt::Display for LaunchProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchProblem::NoScript => f.write_str("We cannot launch this server for you. Please use \"Open Folder\" and run the server manually. If unsure how to do so, please contact the packs support pages for more further help."),
            LaunchProblem::NoTerminal(script) => write!(f, "The server couldn't be launched as we don't know how to launch it. Please open the server folder and run the {script} file manually."),
        }
    }
}

impl std::error::Error for LaunchProblem {}

/// The Java the launcher provides for a server, if there is one: Mojang's runtime for the server's Java version,
/// when it is downloaded and "use Java provided by Minecraft" is on.
pub fn provided_java(dirs: &Dirs, server: &Server, use_java_provided_by_minecraft: bool) -> Option<PathBuf> {
    if !server.is_launcher_script() || !use_java_provided_by_minecraft {
        return None;
    }
    let component = &server.java_version.as_ref()?.component;
    if component.is_empty() {
        return None;
    }
    let dir = dirs.runtimes.join(component).join(platform::java_runtime_platform()).join(component);
    dir.is_dir().then_some(dir)
}

/// A server's name as the title of a cmd window, inside quotes: the characters that would end the quotes or that cmd
/// expands even inside them (`"`, `%`) are left out, and so are the ones that start another command when quotes are
/// unbalanced (`&`, `|`, `<`, `>`, `^`).
fn window_title(name: &str) -> String {
    let t: String =
        name.chars().filter(|c| !matches!(c, '"' | '%' | '&' | '|' | '<' | '>' | '^') && !c.is_control()).collect();
    if t.trim().is_empty() { "Minecraft server".to_string() } else { t }
}

/// Work out the command that opens a terminal running the server's script, the way the Java edition does for
/// each system. `in_path` says whether a program can be found.
pub fn launch_plan(
    server: &Server,
    os: Os,
    args: &str,
    java: Option<&Path>,
    script: &str,
    is_launcher_script: bool,
    in_path: impl Fn(&str) -> bool,
) -> std::result::Result<LaunchPlan, LaunchProblem> {
    let root = &server.root;
    let java = if is_launcher_script { java } else { None };
    let mut files = Vec::new();
    let (program, args_out): (String, Vec<String>) = match os {
        Os::Windows => {
            let mut a: Vec<String> = ["/K", "start"].iter().map(|s| s.to_string()).collect();
            a.push(format!("\"{}\"", window_title(&server.name)));
            a.push("/D".into());
            a.push(format!("\"{}\"", root.display()));
            if script.ends_with(".ps1") {
                a.extend(["powershell".to_string(), "-NoExit".to_string(), format!("./{script}")]);
            } else {
                a.push(script.to_string());
                if let Some(j) = java {
                    a.push("ATLcustomjava".into());
                    a.push(format!("\"{}\\bin\\java.exe\"", j.display()));
                }
            }
            if !args.is_empty() {
                a.push(args.to_string());
            }
            ("cmd".into(), a)
        }
        Os::Linux => {
            let custom = match java {
                Some(j) => format!(" ATLcustomjava {}/bin/java ", j.display()),
                None => " ".to_string(),
            };
            if in_path("x-terminal-emulator") {
                (
                    "x-terminal-emulator".into(),
                    vec!["-e".into(), format!("{}{custom}{args}", root.join(script).display())],
                )
            } else {
                let line = format!(
                    "./{script} {}{args}",
                    match java {
                        Some(j) => format!(" ATLcustomjava {}/bin/java ", j.display()),
                        None => String::new(),
                    }
                );
                let words = || line.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect::<Vec<_>>();
                if in_path("exo-open") {
                    (
                        "exo-open".into(),
                        vec![
                            "--launch".into(),
                            "TerminalEmulator".into(),
                            "--working-directory".into(),
                            root.display().to_string(),
                            line.clone(),
                        ],
                    )
                } else if in_path("kitty") {
                    ("kitty".into(), words())
                } else if in_path("alacritty") {
                    let mut a = vec!["-e".to_string()];
                    a.extend(words());
                    ("alacritty".into(), a)
                } else if in_path("gnome-terminal") {
                    let mut a = vec!["--".to_string()];
                    a.extend(words());
                    ("gnome-terminal".into(), a)
                } else if in_path("konsole") {
                    let mut a = vec!["-e".to_string()];
                    a.extend(words());
                    ("konsole".into(), a)
                } else if in_path("xfce4-terminal") {
                    let mut a = vec!["-x".to_string()];
                    a.extend(words());
                    ("xfce4-terminal".into(), a)
                } else if in_path("xterm") {
                    let mut a = vec!["-e".to_string()];
                    a.extend(words());
                    ("xterm".into(), a)
                } else {
                    return Err(LaunchProblem::NoTerminal(script.to_string()));
                }
            }
        }
        Os::Mac => {
            let mut launch_command = script.to_string();
            if is_launcher_script {
                // macOS can't pass arguments through `open -a Terminal`, so a throwaway script is written and run
                let mut parts: Vec<String> = vec!["cd \"`dirname \"$0\"`\" ; ./LaunchServer.sh".into()];
                if let Some(j) = java {
                    parts.push("ATLcustomjava".into());
                    parts.push(format!("{}/jre.bundle/Contents/Home/bin/java", j.display()));
                }
                parts.push(args.to_string());
                parts.push("; rm -f ./.launcherrun.sh".into());
                files.push((root.join(".launcherrun.sh"), parts.join(" ")));
                launch_command = "./.launcherrun.sh".into();
            }
            ("open".into(), vec!["-a".into(), "Terminal".into(), launch_command])
        }
    };
    Ok(LaunchPlan { program, args: args_out, workdir: root.clone(), files })
}

/// Start the server in a terminal window. Returns the plan that was run (for the log). The server keeps running
/// in its own window after the launcher closes.
pub fn launch(
    dirs: &Dirs,
    server: &Server,
    extra_args: &str,
    use_java_provided_by_minecraft: bool,
) -> Result<LaunchPlan> {
    let script = server.script().ok_or(LaunchProblem::NoScript)?;
    let java = provided_java(dirs, server, use_java_provided_by_minecraft);
    if let Some(j) = &java {
        crate::logs::info(format!("Using Java runtime at path {}", j.display()));
    }
    crate::logs::info(format!("Starting server {}", server.name));
    let plan = launch_plan(
        server,
        platform::os(),
        extra_args,
        java.as_deref(),
        script,
        server.is_launcher_script(),
        crate::launch::executable_in_path,
    )?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script_path = server.root.join(script);
        if let Ok(meta) = std::fs::metadata(&script_path) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o111);
            let _ = std::fs::set_permissions(&script_path, perms);
        }
    }
    for (path, content) in &plan.files {
        std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
        }
    }
    crate::logs::info(format!("Launching server with the following arguments: {:?}", plan.args));
    let mut command = std::process::Command::new(&plan.program);
    // on Windows the arguments are already written for cmd, quotes included: passed as they are, the way Java's
    // ProcessBuilder did (Rust's own quoting turns them into \" , which cmd doesn't understand, and start failed)
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        for a in &plan.args {
            command.raw_arg(a);
        }
    }
    #[cfg(not(windows))]
    command.args(&plan.args);
    command
        .current_dir(&plan.workdir)
        .stdin(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("starting {}", plan.program))?;
    crate::logs::info(
        "Server has started. No further logs will be shown in this console. Please check for a separate window or tab for the server logs and provide those logs (not these ones) if asking for help.",
    );
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(root: &Path) -> Server {
        Server { name: "Minecraft 1.21.11 with Fabric".into(), root: root.to_path_buf(), ..Default::default() }
    }

    #[test]
    fn reads_a_java_written_server_file_and_keeps_the_rest() {
        let json = r#"{"name":"Minecraft 1.21.11 with Fabric","pack":"Minecraft","vanillaInstance":true,"packId":0,"externalPackId":0,
            "minecraftVersion":"1.21.11","version":"1.21.11","isPatchedForLog4Shell":true,
            "loaderVersion":{"version":"0.18.4","rawVersion":"0.18.4","recommended":false,"type":"Fabric","downloadables":{}},
            "javaVersion":{"component":"java-runtime-delta","majorVersion":21},"isDev":false,
            "mods":[{"name":"Fabric API","file":"fabric-api.jar","curseForgeProject":{"id":1}}],"ignoredUpdates":[]}"#;
        let s: Server = serde_json::from_str(json).unwrap();
        assert_eq!(s.safe_name(), "Minecraft12111withFabric");
        assert_eq!(s.loader_label().as_deref(), Some("Fabric 0.18.4"));
        assert_eq!(s.java_version.as_ref().unwrap().component, "java-runtime-delta");
        assert_eq!(s.mod_names(), vec!["Fabric API"]);
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["mods"][0]["curseForgeProject"]["id"], 1, "unknown parts survive a save");
        assert_eq!(back["ignoredUpdates"], serde_json::json!([]));
        assert!(back.get("root").is_none());
        assert_eq!(back["isPatchedForLog4Shell"], true);
    }

    #[test]
    fn the_scripts_are_looked_for_in_the_java_editions_order() {
        let have = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
        assert_eq!(script_for(Os::Linux, have(&["run.sh", "start.sh"])), Some("start.sh"));
        assert_eq!(script_for(Os::Linux, have(&["LaunchServer.sh", "start.sh"])), Some("LaunchServer.sh"));
        assert_eq!(script_for(Os::Linux, have(&["LaunchServer.bat"])), None);
        assert_eq!(script_for(Os::Windows, have(&["ServerStart.bat", "run.bat"])), Some("run.bat"));
        assert_eq!(script_for(Os::Mac, have(&["ServerStart.sh"])), Some("ServerStart.sh"));
    }

    #[test]
    fn linux_commands_follow_the_java_edition() {
        let s = sample(Path::new("/srv/pack"));
        let java = Path::new("/data/runtimes/minecraft/jre/linux/jre");
        let with = |only: &'static str| move |n: &str| n == only;
        let p = launch_plan(&s, Os::Linux, "nogui", Some(java), "LaunchServer.sh", true, with("x-terminal-emulator"))
            .unwrap();
        assert_eq!(p.program, "x-terminal-emulator");
        assert_eq!(
            p.args,
            vec!["-e", "/srv/pack/LaunchServer.sh ATLcustomjava /data/runtimes/minecraft/jre/linux/jre/bin/java nogui"]
        );

        let p = launch_plan(&s, Os::Linux, "", None, "start.sh", false, with("gnome-terminal")).unwrap();
        assert_eq!(
            (p.program.as_str(), p.args.clone()),
            ("gnome-terminal", vec!["--".to_string(), "./start.sh".to_string()])
        );

        let p = launch_plan(&s, Os::Linux, "", None, "start.sh", false, with("exo-open")).unwrap();
        assert_eq!(p.args, vec!["--launch", "TerminalEmulator", "--working-directory", "/srv/pack", "./start.sh "]);

        let p = launch_plan(&s, Os::Linux, "", None, "start.sh", false, with("kitty")).unwrap();
        assert_eq!((p.program.as_str(), p.args), ("kitty", vec!["./start.sh".to_string()]));

        let p = launch_plan(&s, Os::Linux, "", None, "start.sh", false, with("alacritty")).unwrap();
        assert_eq!(p.args, vec!["-e", "./start.sh"]);

        assert_eq!(
            launch_plan(&s, Os::Linux, "", None, "start.sh", false, |_| false),
            Err(LaunchProblem::NoTerminal("start.sh".into()))
        );
        // a provided Java is only handed to the launcher's own script
        let p = launch_plan(&s, Os::Linux, "", Some(java), "start.sh", false, with("x-terminal-emulator")).unwrap();
        assert!(!p.args[1].contains("ATLcustomjava"));
    }

    #[test]
    fn windows_commands_follow_the_java_edition() {
        let s = sample(Path::new("C:\\srv\\pack"));
        let java = Path::new("C:\\data\\jre");
        let p = launch_plan(&s, Os::Windows, "nogui", Some(java), "LaunchServer.bat", true, |_| false).unwrap();
        assert_eq!(p.program, "cmd");
        assert_eq!(
            p.args,
            vec![
                "/K",
                "start",
                "\"Minecraft 1.21.11 with Fabric\"",
                "/D",
                "\"C:\\srv\\pack\"",
                "LaunchServer.bat",
                "ATLcustomjava",
                "\"C:\\data\\jre\\bin\\java.exe\"",
                "nogui"
            ]
        );
        let p = launch_plan(&s, Os::Windows, "", None, "start.ps1", false, |_| false).unwrap();
        assert_eq!(&p.args[5..], ["powershell", "-NoExit", "./start.ps1"]);
    }

    #[test]
    fn mac_runs_a_throwaway_script_for_the_launcher_script() {
        let s = sample(Path::new("/Users/x/servers/pack"));
        let p = launch_plan(&s, Os::Mac, "nogui", Some(Path::new("/data/jre")), "LaunchServer.sh", true, |_| false)
            .unwrap();
        assert_eq!(
            (p.program.as_str(), p.args),
            ("open", vec!["-a".to_string(), "Terminal".to_string(), "./.launcherrun.sh".to_string()])
        );
        assert_eq!(p.files.len(), 1);
        assert_eq!(
            p.files[0].1,
            "cd \"`dirname \"$0\"`\" ; ./LaunchServer.sh ATLcustomjava /data/jre/jre.bundle/Contents/Home/bin/java nogui ; rm -f ./.launcherrun.sh"
        );
        let p = launch_plan(&s, Os::Mac, "", None, "start.sh", false, |_| false).unwrap();
        assert_eq!(p.args[2], "start.sh");
        assert!(p.files.is_empty());
    }

    fn scratch(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("blockforge-servers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        Dirs::at(base)
    }

    #[test]
    fn list_find_collapse_backup_and_remove() {
        let dirs = scratch("all");
        for (folder, name) in [("PackB", "Pack B"), ("PackA", "Pack A")] {
            let root = servers_dir(&dirs).join(folder);
            std::fs::create_dir_all(root.join("world")).unwrap();
            std::fs::write(root.join("world/level.dat"), "w").unwrap();
            std::fs::write(
                root.join("server.json"),
                format!(r#"{{"name":"{name}","pack":"X","minecraftVersion":"1.20.1","isPatchedForLog4Shell":true}}"#),
            )
            .unwrap();
        }
        std::fs::create_dir_all(servers_dir(&dirs).join("NotAServer")).unwrap();
        std::fs::create_dir_all(servers_dir(&dirs).join("Broken")).unwrap();
        std::fs::write(servers_dir(&dirs).join("Broken/server.json"), "{ nope").unwrap();

        let all = load_all(&dirs);
        assert_eq!(
            all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Pack A", "Pack B"],
            "folders without a readable server.json are skipped"
        );
        assert!(is_server(&all, "pack-a!"));
        assert!(!is_server(&all, "Pack C"));
        assert_eq!(find(&all, "packa").unwrap().name, "Pack A");
        assert_eq!(find(&all, "Pack B").unwrap().root, servers_dir(&dirs).join("PackB"));

        let mut acct = Account::offline("Steve");
        assert!(!is_collapsed(&acct, &all[0]));
        set_collapsed(&mut acct, &all[0], true);
        set_collapsed(&mut acct, &all[0], true);
        assert!(is_collapsed(&acct, &all[0]) && !is_collapsed(&acct, &all[1]));
        assert_eq!(acct.extras["collapsedServers"], serde_json::json!(["Pack A"]));
        set_collapsed(&mut acct, &all[0], false);
        assert!(!is_collapsed(&acct, &all[0]));

        let svc = crate::svcsettings::ServiceSettings::default();
        let zip = crate::backups::create_server(&dirs, &svc, &all[0].root, &all[0].name).unwrap();
        assert!(zip.file_name().unwrap().to_string_lossy().starts_with("Server-PackA-"));
        assert!(crate::backups::entry_names(&zip).contains("world/level.dat"));
        assert_eq!(crate::backups::list_server(&dirs, &svc, "Pack A").len(), 1);

        remove(&all[1]).unwrap();
        assert!(!servers_dir(&dirs).join("PackB").exists());
        assert!(
            remove(&Server { root: dirs.base.clone(), ..Default::default() }).is_err(),
            "only server folders are deleted"
        );
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn a_provided_java_needs_the_runtime_on_disk() {
        let dirs = scratch("java");
        let root = servers_dir(&dirs).join("S");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("LaunchServer.sh"), "#!/bin/bash").unwrap();
        std::fs::write(root.join("LaunchServer.bat"), "").unwrap();
        let mut s = sample(&root);
        s.java_version = Some(JavaVersion { component: "java-runtime-delta".into(), major_version: 21 });
        assert_eq!(provided_java(&dirs, &s, true), None, "not downloaded");
        let runtime =
            dirs.runtimes.join("java-runtime-delta").join(platform::java_runtime_platform()).join("java-runtime-delta");
        std::fs::create_dir_all(&runtime).unwrap();
        assert_eq!(provided_java(&dirs, &s, true), Some(runtime));
        assert_eq!(provided_java(&dirs, &s, false), None, "the setting is off");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    /// The real folder of a server the Java edition installed, read only: skipped when this computer has none.
    #[test]
    fn reads_a_real_java_installed_server() {
        let real = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/atlauncher/servers"));
        let Some(real) = real.filter(|p| p.is_dir()) else { return };
        let mut seen = 0;
        for e in std::fs::read_dir(&real).unwrap().flatten() {
            if e.path().join("server.json").exists() {
                let s = load_one(&e.path()).unwrap();
                assert!(!s.name.is_empty() && !s.minecraft_version.is_empty());
                assert!(s.script().is_some() || platform::os() == Os::Windows, "{} has a start script", s.name);
                seen += 1;
            }
        }
        println!("read {seen} real server folders");
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;

    #[test]
    fn server_names_cant_break_out_of_the_window_title() {
        assert_eq!(window_title("Rock & Stone"), "Rock  Stone");
        let t = window_title(r#"x" & calc & echo "#);
        assert!(!t.contains(['"', '&']), "{t}");
        assert_eq!(window_title("%PATH%"), "PATH");
        assert_eq!(window_title("\"\"\""), "Minecraft server");
        assert_eq!(window_title("Minecraft 1.21.11 with Fabric"), "Minecraft 1.21.11 with Fabric");
    }
}
