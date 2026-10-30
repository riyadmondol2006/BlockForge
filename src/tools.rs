//! The Tools page: what each tool does, with the Java edition's behaviour.
//!
//! Network checker, support bundle, clear logs, debug mode, clear downloads, update skins (in [`crate::skins`]),
//! clear game start cache (in [`crate::classcache`]) and delete libraries.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::accounts::Account;
use crate::config::Settings;
use crate::diagnostics::{self, BundleInput, Diagnosis};
use crate::install::Progress;
use crate::instances::Instance;
use crate::paths::Dirs;
use crate::svcsettings::ServiceSettings;

/// The servers the Java edition talks to. The network checker tests every one of them.
pub mod hosts {
    pub const API: &str = "api.atlauncher.com";
    pub const PASTE: &str = "paste.atlauncher.com";
    pub const DOWNLOAD: &str = "download.nodecdn.net";
    pub const CURSEFORGE: &str = "api.curseforge.com";
    pub const MODRINTH: &str = "api.modrinth.com";
    pub const FTB: &str = "api.feed-the-beast.com";
    pub const FORGE: &str = "maven.minecraftforge.net";
    pub const FABRIC: &str = "maven.fabricmc.net";
    pub const LEGACY_FABRIC: &str = "maven.legacyfabric.net";
    pub const NEOFORGE: &str = "maven.neoforged.net";
    pub const QUILT: &str = "maven.quiltmc.org";
}

/// Where the network checker's result is shared, and the page that shows it.
pub const PASTE_API_URL: &str = "https://paste.atlauncher.com/api/create-v2";
pub const PASTE_CHECK_URL: &str = "https://paste.atlauncher.com";
/// The Java edition's download server and the file it speed-tests with.
pub const DOWNLOAD_SERVER: &str = "https://download.nodecdn.net/containers/atl";

/// Every host that gets a name lookup, in the Java edition's order (`ToolsViewModel.HOSTS`).
pub const ALL_HOSTS: [&str; 16] = [
    "authserver.mojang.com",
    "session.minecraft.net",
    "libraries.minecraft.net",
    "launchermeta.mojang.com",
    "launcher.mojang.com",
    hosts::API,
    hosts::PASTE,
    hosts::DOWNLOAD,
    hosts::FABRIC,
    hosts::LEGACY_FABRIC,
    hosts::NEOFORGE,
    hosts::FORGE,
    hosts::QUILT,
    hosts::CURSEFORGE,
    hosts::MODRINTH,
    hosts::FTB,
];

/// The hosts that are pinged and traced, in the order the Java edition does it.
pub const PING_HOSTS: [&str; 10] = [
    hosts::DOWNLOAD,
    hosts::API,
    hosts::CURSEFORGE,
    hosts::MODRINTH,
    hosts::FTB,
    hosts::FABRIC,
    hosts::FORGE,
    hosts::LEGACY_FABRIC,
    hosts::NEOFORGE,
    hosts::QUILT,
];

/// One card on the Tools page.
#[derive(Clone, Copy, Debug)]
pub struct ToolInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// The button's label.
    pub action: &'static str,
}

/// The cards, in the Java edition's order and wording.
pub const TOOLS: [ToolInfo; 8] = [
    ToolInfo {
        id: "network_checker",
        name: "Network checker",
        description: "Tests your connection to the servers the launcher needs. The result stays on this computer unless you choose to share it.",
        action: "Run",
    },
    ToolInfo {
        id: "support_bundle",
        name: "Support bundle",
        description: "Packs the launcher log and the last played instance's logs into one ZIP with names, tokens and IDs removed, ready to share.",
        action: "Create",
    },
    ToolInfo {
        id: "log_clearer",
        name: "Clear logs",
        description: "Removes the logs the launcher has written, to free up space. Instance logs are kept.",
        action: "Clear",
    },
    ToolInfo {
        id: "debug_mode",
        name: "Debug mode",
        description: "Restarts the launcher with extra logging, handy when you are chasing a problem.",
        action: "Restart",
    },
    ToolInfo {
        id: "download_clearer",
        name: "Clear downloads",
        description: "Deletes cached downloads. Instances are not touched, but new installs may take longer.",
        action: "Clear",
    },
    ToolInfo {
        id: "skin_updater",
        name: "Update skins",
        description: "Downloads the latest skin for each of your Microsoft accounts.",
        action: "Update",
    },
    ToolInfo {
        id: "class_cache_clearer",
        name: "Clear game start cache",
        description: "Deletes the saved class data that makes games start faster. Each game makes it again the next time it is played.",
        action: "Clear",
    },
    ToolInfo {
        id: "libraries_deleter",
        name: "Delete libraries",
        description: "Removes the libraries Minecraft uses. An instance may need a reinstall if they don't download again.",
        action: "Delete",
    },
];

/// What is said before the network checker runs. The check itself sends nothing anywhere: sharing its result is a
/// separate step the player takes afterwards (see [`NETWORK_SHARE_NOTE`]).
pub const NETWORK_CHECK_NOTE: &str = "Pings and traces the servers the launcher needs and looks up their addresses. It can take 10 minutes or more. The result is shown here and stays on this computer.";

/// What is said before a network report is shared, and asked to be agreed to.
pub const NETWORK_SHARE_NOTE: &str = "The report is uploaded to ATLauncher's paste service (paste.atlauncher.com) so it can be read by whoever you send the link to. IP addresses, your user name and your home folder are taken out first.";

// ---------------------------------------------------------------------------------------------------------
// Network checker

#[derive(Clone, Debug)]
pub struct NetworkCheckOptions {
    /// How many echo requests `ping` sends per host.
    pub ping_count: u32,
    /// Trace the route to each host, which can take a minute each.
    pub trace_routes: bool,
    /// Download the 100 MB test file to measure speed.
    pub speed_test: bool,
    /// The longest any one ping or trace may run before it is stopped.
    pub command_timeout: Duration,
    /// Hosts to ping and trace (the rest of the tests are not affected).
    pub ping_hosts: Vec<String>,
}

impl Default for NetworkCheckOptions {
    fn default() -> Self {
        NetworkCheckOptions {
            ping_count: 10,
            trace_routes: true,
            speed_test: true,
            command_timeout: Duration::from_secs(120),
            ping_hosts: PING_HOSTS.iter().map(|h| h.to_string()).collect(),
        }
    }
}

impl NetworkCheckOptions {
    /// How many steps the check has, for a progress bar.
    pub fn steps(&self) -> usize {
        let pings = self.ping_hosts.len() * if self.trace_routes { 2 } else { 1 };
        pings + ALL_HOSTS.len() + 2 + usize::from(self.speed_test)
    }
}

async fn run_command(program: &str, args: &[String], limit: Duration) -> Result<String> {
    let mut cmd = crate::platform::tokio_command(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    let child = cmd.spawn().with_context(|| format!("couldn't run {program}"))?;
    let out = match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(r) => r?,
        Err(_) => bail!("{program} took longer than {} seconds and was stopped", limit.as_secs()),
    };
    // the Java edition joins the lines with a carriage return
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(|l| format!("{l}\r")).collect())
}

async fn first_address(host: &str) -> Option<IpAddr> {
    tokio::net::lookup_host((host, 0)).await.ok()?.map(|a| a.ip()).next()
}

/// `ping` output for `host`, or an empty string when ping can't be run (the Java edition logs and goes on).
pub async fn ping(host: &str, count: u32, limit: Duration) -> String {
    let Some(ip) = first_address(host).await else { return String::new() };
    let flag = if cfg!(windows) { "-n" } else { "-c" };
    match run_command("ping", &[flag.to_string(), count.to_string(), ip.to_string()], limit).await {
        Ok(s) => s,
        Err(e) => {
            crate::logs::error(format!("Problem while running ping on host {host}: {e:#}"));
            String::new()
        }
    }
}

/// `traceroute` (`tracert` on Windows) output for `host`.
pub async fn trace_route(host: &str, limit: Duration) -> String {
    let Some(ip) = first_address(host).await else { return String::new() };
    let program = if cfg!(windows) { "tracert" } else { "traceroute" };
    match run_command(program, &[ip.to_string()], limit).await {
        Ok(s) => s,
        Err(e) => {
            crate::logs::error(format!("Problem while running {program} on host {host}: {e:#}"));
            String::new()
        }
    }
}

/// What a download speed looks like in the report: B/s, KB/s or MB/s.
pub fn format_speed(bytes: u64, seconds: f64) -> String {
    let bps = bytes as f64 / seconds.max(0.001);
    let kbps = bps / 1024.0;
    let mbps = kbps / 1024.0;
    if mbps < 1.0 {
        if kbps < 1.0 { format!("{bps:.2} B/s") } else { format!("{kbps:.2} KB/s") }
    } else {
        format!("{mbps:.2} MB/s")
    }
}

/// Run the checks and return the report text. `progress.stage` says what is being done, `files_done` and
/// `files_total` count the steps, and setting `progress.cancelled` stops after the current step.
pub async fn run_network_check(dirs: &Dirs, options: &NetworkCheckOptions, progress: &Progress) -> Result<String> {
    let total = options.steps();
    let counted = std::sync::atomic::AtomicUsize::new(0);
    let step = |what: String| {
        progress.stage(format!("{what} ({} of {total})", counted.load(Ordering::Relaxed) + 1));
        if progress.cancelled.load(Ordering::Relaxed) {
            bail!("Network Checker Tool Cancelled!");
        }
        Ok(())
    };
    let done = || {
        counted.fetch_add(1, Ordering::Relaxed);
    };

    let mut report = String::new();
    for host in &options.ping_hosts {
        step(format!("Pinging {host}"))?;
        report.push_str(&format!(
            "Ping results to {host} was {}",
            ping(host, options.ping_count, options.command_timeout).await
        ));
        done();
        if options.trace_routes {
            step(format!("Tracing the route to {host}"))?;
            report.push_str(&format!(
                "Tracert to {host} was {}\n\n----------------\n\n",
                trace_route(host, options.command_timeout).await
            ));
            done();
        } else {
            report.push_str("\n\n----------------\n\n");
        }
    }

    for host in ALL_HOSTS {
        step(format!("Looking up {host}"))?;
        match tokio::net::lookup_host((host, 0)).await {
            Ok(addrs) => {
                let mut ips: Vec<String> = Vec::new();
                for a in addrs {
                    let ip = a.ip().to_string();
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                }
                report.push_str(&format!("Resolution of {host} was {}\n\n", ips.join(", ")));
            }
            Err(e) => report.push_str(&format!("Resolution of {host} failed: {e}\n\n")),
        }
        done();
    }
    report.push_str("----------------\n\n");

    step("Testing the download server".into())?;
    let url = format!("{DOWNLOAD_SERVER}/launcher/json/files.json");
    match crate::http::client().get(&url).send().await {
        Ok(r) => report.push_str(&format!(
            "Response code to {DOWNLOAD_SERVER} was {}\n\n----------------\n\n",
            r.status().as_u16()
        )),
        Err(e) => report
            .push_str(&format!("Exception thrown when connecting to {DOWNLOAD_SERVER}\n\n----------------\n\n{e}")),
    }
    done();

    step("Pinging the download server".into())?;
    let pong = match crate::http::client().get(format!("{DOWNLOAD_SERVER}/ping")).send().await {
        Ok(r) => r.text().await.unwrap_or_default(),
        Err(e) => e.to_string(),
    };
    report.push_str(&format!("Response to ping on {DOWNLOAD_SERVER} was {pong}\n\n----------------\n\n"));
    done();

    if options.speed_test {
        step("Measuring download speed (100 MB)".into())?;
        let file = dirs.temp.join("100MB.bin");
        let _ = std::fs::create_dir_all(&dirs.temp);
        let _ = std::fs::remove_file(&file);
        let started = Instant::now();
        let result = crate::install::download_all(
            vec![crate::install::Job::new(format!("{DOWNLOAD_SERVER}/100MB.bin"), &file)],
            &Progress::new(),
            false,
        )
        .await;
        match result {
            Ok(_) => {
                let secs = started.elapsed().as_secs_f64();
                let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                report.push_str(&format!(
                    "Download speed to {DOWNLOAD_SERVER} was {}, taking {secs:.2} seconds to download 100MB",
                    format_speed(size, secs)
                ));
            }
            Err(e) => report.push_str(&format!(
                "Exception thrown when downloading 100MB.bin from {DOWNLOAD_SERVER}\n\n----------------\n\n{e:#}"
            )),
        }
        let _ = std::fs::remove_file(&file);
        done();
    }
    progress.stage("Done");
    Ok(report)
}

/// The copy of a network report that may be shared: with IP addresses, the user name, the home folder and anything
/// else personal taken out ([`crate::diagnostics::scrub`]).
pub fn shareable_report(text: &str) -> String {
    crate::diagnostics::scrub(text, &[])
}

/// Share a report on the paste service. Returns the address where it can be read. Only call this when the player
/// asked to share it and agreed to [`NETWORK_SHARE_NOTE`], and with the [`shareable_report`] copy.
pub async fn upload_report(title: &str, text: &str) -> Result<String> {
    let r = crate::http::client()
        .post(PASTE_API_URL)
        .json(&serde_json::json!({ "title": title, "text": text }))
        .send()
        .await
        .context("couldn't reach the paste service")?;
    let status = r.status();
    let body = r.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("the paste service answered HTTP {status}");
    }
    let url = parse_paste_answer(&body)?;
    crate::logs::info(format!("Network Test has finished running, you can view the results at {url}"));
    Ok(url)
}

/// The paste service answers with the address of the new paste, as a JSON string or as plain text.
pub fn parse_paste_answer(body: &str) -> Result<String> {
    let url = body.trim().trim_matches('"').to_string();
    if !url.contains(PASTE_CHECK_URL) {
        bail!("the paste service didn't return a link: {}", body.chars().take(200).collect::<String>());
    }
    Ok(url)
}

// ---------------------------------------------------------------------------------------------------------
// Clearing things

fn delete_path(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path).is_ok(),
        Ok(_) => std::fs::remove_file(path).is_ok(),
        Err(_) => true,
    }
}

/// Delete everything inside `dir` (not the folder itself) except the names in `keep`. Returns how many entries went.
fn empty_dir(dir: &Path, keep: &[&Path]) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        if keep.iter().any(|k| *k == e.path()) {
            continue;
        }
        if delete_path(&e.path()) {
            n += 1;
        }
    }
    n
}

pub fn downloads_dir(dirs: &Dirs) -> PathBuf {
    dirs.base.join("downloads")
}

pub fn failed_downloads_dir(dirs: &Dirs) -> PathBuf {
    dirs.base.join("faileddownloads")
}

/// "Clear downloads": everything in `downloads/` (Technic's too) and `faileddownloads/`. Instances are untouched.
pub fn clear_downloads(dirs: &Dirs) -> usize {
    let downloads = downloads_dir(dirs);
    let technic = downloads.join("technic");
    let n = empty_dir(&downloads, &[technic.as_path()])
        + empty_dir(&technic, &[])
        + empty_dir(&failed_downloads_dir(dirs), &[]);
    crate::logs::info(format!("Cleared {n} downloaded items"));
    n
}

/// "Delete libraries": everything in `libraries/`. They are downloaded again the next time a game starts.
pub fn delete_libraries(dirs: &Dirs) -> usize {
    let n = empty_dir(&dirs.libraries, &[]);
    crate::logs::info(format!("Deleted {n} items from the libraries folder"));
    n
}

/// "Clear logs": the launcher's old logs (`logs/old/`). The log of this run and the instances' logs stay.
pub fn clear_logs(dirs: &Dirs) -> usize {
    crate::logs::clear_old_logs(dirs)
}

// ---------------------------------------------------------------------------------------------------------
// Debug mode

/// Flatpak runs can't restart themselves, so the Java edition greys the tool out there.
pub fn is_flatpak() -> bool {
    cfg!(target_os = "linux") && (std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists())
}

pub fn is_debug_enabled() -> bool {
    crate::logs::show_debug()
}

/// Whether the "Debug mode" button can be used: not already in debug mode, and not in a Flatpak.
pub fn can_launch_in_debug() -> bool {
    debug_relaunch_allowed(is_flatpak(), is_debug_enabled())
}

fn debug_relaunch_allowed(flatpak: bool, already_debugging: bool) -> bool {
    !flatpak && !already_debugging
}

/// Start another copy of the launcher with `--debug` added to the arguments it was started with. The caller
/// closes this copy once it returns `Ok`.
pub fn relaunch_in_debug() -> Result<()> {
    let exe = std::env::current_exe().context("couldn't find the launcher's own file")?;
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--debug") {
        args.push("--debug".into());
    }
    crate::platform::command(&exe)
        .args(&args)
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| format!("couldn't start {}", exe.display()))?;
    crate::logs::info("Restarting in debug mode");
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Support bundle

/// The folder the support bundle is saved in: the "downloads folder" setting, or the user's Downloads folder.
pub fn user_downloads_dir(settings: &ServiceSettings) -> PathBuf {
    if let Some(p) = settings.custom_downloads_path.as_deref().map(PathBuf::from).filter(|p| p.is_dir()) {
        return p;
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Downloads")
}

/// The instance that was played last, which is where a person most likely wants help with.
pub fn last_played_instance(instances: &[Instance]) -> Option<&Instance> {
    instances.iter().max_by_key(|i| i.last_played())
}

/// The facts at the top of a bundle's summary, as the Java edition lists them.
pub fn support_facts(dirs: &Dirs, settings: &Settings, instance: Option<&Instance>) -> Vec<String> {
    use sysinfo::System;
    let mut facts = vec![
        format!("Launcher: {} {}", crate::NAME, crate::VERSION),
        format!(
            "System: {} {} ({})",
            System::name().unwrap_or_else(|| std::env::consts::OS.to_string()),
            System::os_version().unwrap_or_default(),
            std::env::consts::ARCH
        ),
    ];
    if let Some(i) = instance {
        facts.push(format!("Minecraft: {}", i.mc_version()));
        if let Some((kind, version)) = i.loader() {
            facts.push(format!("Loader: {kind} {version}"));
        }
        let launcher = &i.file.launcher;
        let component = i.file.version.java_version.as_ref().map(|j| j.component.clone());
        let choice = crate::java::choose(
            dirs,
            settings,
            launcher.java_path.as_deref(),
            component.as_deref(),
            launcher.use_java_provided_by_minecraft.unwrap_or(true),
        );
        facts
            .push(format!("Java used by the instance: {}", choice.home.as_deref().unwrap_or(&choice.binary).display()));
        facts.push(format!("Memory: {} MB", launcher.maximum_memory.unwrap_or(settings.maximum_memory)));
    }
    facts
}

/// Make a support bundle zip in the downloads folder for `instance` (or just the launcher's log) and return
/// where it is, plus what was recognised in the logs.
pub fn create_support_bundle(
    dirs: &Dirs,
    settings: &Settings,
    accounts: &[Account],
    instance: Option<&Instance>,
) -> Result<(PathBuf, Vec<Diagnosis>)> {
    let svc = ServiceSettings::from_settings(settings);
    let mut input = BundleInput {
        launcher_log: Some(crate::logs::log_file(dirs)),
        facts: support_facts(dirs, settings, instance),
        ..Default::default()
    };
    for a in accounts {
        input.secrets.push(a.minecraft_username.clone().unwrap_or_default());
        input.secrets.push(a.username.clone());
        input.secrets.push(a.uuid.clone());
        input.secrets.push(a.access_token.clone().unwrap_or_default());
    }
    if let Some(i) = instance {
        input.instance_root = Some(i.root.clone());
        input.instance_name = Some(i.name().to_string());
    }
    let zip = user_downloads_dir(&svc)
        .join(format!("BlockForge-support-{}.zip", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    let found = diagnostics::create_bundle(&zip, &input)?;
    crate::logs::info(format!(
        "Created support bundle {}{}",
        zip.display(),
        found.first().map(|d| format!(" ({d})")).unwrap_or_default()
    ));
    Ok((zip, found))
}

/// After a crash: read the game log, and if the cause is recognised say what it was (the Java edition then
/// offers a support bundle).
pub fn crash_help(instance: &Instance) -> Option<Diagnosis> {
    let latest = instance.root.join("logs").join("latest.log");
    let log = std::fs::read_to_string(latest).ok()?;
    let found = diagnostics::classify(&log);
    let d = found.into_iter().next()?;
    crate::logs::error(format!("Likely cause [{}]: {}. {}", d.code, d.title, d.fix));
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("blockforge-tools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        Dirs::at(base)
    }

    #[test]
    fn the_tools_page_has_every_card() {
        let names: Vec<_> = TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "Network checker",
                "Support bundle",
                "Clear logs",
                "Debug mode",
                "Clear downloads",
                "Update skins",
                "Clear game start cache",
                "Delete libraries"
            ]
        );
        assert_eq!(ALL_HOSTS.len(), 16);
        assert_eq!(ALL_HOSTS[0], "authserver.mojang.com");
    }

    #[test]
    fn clearing_downloads_keeps_the_folders() {
        let dirs = scratch("dl");
        let d = downloads_dir(&dirs);
        std::fs::create_dir_all(d.join("technic/pack")).unwrap();
        std::fs::create_dir_all(d.join("folder")).unwrap();
        std::fs::write(d.join("a.zip"), "x").unwrap();
        std::fs::write(d.join("technic/pack/b.zip"), "x").unwrap();
        std::fs::create_dir_all(failed_downloads_dir(&dirs)).unwrap();
        std::fs::write(failed_downloads_dir(&dirs).join("c.part"), "x").unwrap();
        std::fs::create_dir_all(dirs.instances.join("Keep")).unwrap();
        std::fs::write(dirs.instances.join("Keep/instance.json"), "{}").unwrap();

        let n = clear_downloads(&dirs);
        assert_eq!(n, 4);
        assert!(d.is_dir() && d.join("technic").is_dir(), "the folders stay");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "only the technic folder is left");
        assert_eq!(std::fs::read_dir(d.join("technic")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(failed_downloads_dir(&dirs)).unwrap().count(), 0);
        assert!(dirs.instances.join("Keep/instance.json").exists(), "instances are untouched");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn deleting_libraries_empties_the_folder() {
        let dirs = scratch("lib");
        std::fs::create_dir_all(dirs.libraries.join("net/minecraft")).unwrap();
        std::fs::write(dirs.libraries.join("net/minecraft/a.jar"), "x").unwrap();
        std::fs::write(dirs.libraries.join("b.jar"), "x").unwrap();
        assert_eq!(delete_libraries(&dirs), 2);
        assert!(dirs.libraries.is_dir());
        assert_eq!(std::fs::read_dir(&dirs.libraries).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn the_paste_services_answer_must_be_a_link() {
        assert_eq!(
            parse_paste_answer("\"https://paste.atlauncher.com/view/abc123\"\n").unwrap(),
            "https://paste.atlauncher.com/view/abc123"
        );
        assert_eq!(
            parse_paste_answer("https://paste.atlauncher.com/view/x").unwrap(),
            "https://paste.atlauncher.com/view/x"
        );
        assert!(parse_paste_answer("{\"error\":\"nope\"}").is_err());
        assert!(parse_paste_answer("").is_err());
    }

    #[test]
    fn speeds_are_shown_in_the_right_unit() {
        assert_eq!(format_speed(500, 1.0), "500.00 B/s");
        assert_eq!(format_speed(512 * 1024, 1.0), "512.00 KB/s");
        assert_eq!(format_speed(100 * 1024 * 1024, 4.0), "25.00 MB/s");
    }

    #[test]
    fn network_checker_counts_its_steps() {
        let o = NetworkCheckOptions::default();
        assert_eq!(o.steps(), 20 + 16 + 2 + 1);
        let quick = NetworkCheckOptions { trace_routes: false, speed_test: false, ..Default::default() };
        assert_eq!(quick.steps(), 10 + 16 + 2);
    }

    #[test]
    fn a_shared_network_report_has_addresses_and_the_home_folder_taken_out() {
        let report = "Resolution of example.com was 93.184.216.34\nLog in /home/someone/.blockforge\n";
        let shared = crate::diagnostics::scrub_with(report, &[], Some("/home/someone"), Some("someone"));
        assert!(!shared.contains("93.184.216.34") && shared.contains("<ip>"));
        assert!(!shared.contains("/home/someone") && shared.contains("~/.blockforge"));
        // the same scrubbing as the shared copy, which uses this computer's home folder
        assert!(!shareable_report(report).contains("93.184.216.34"));
    }

    #[test]
    fn downloads_folder_follows_the_setting() {
        let dirs = scratch("dlpath");
        let mut s = ServiceSettings::default();
        s.custom_downloads_path = Some(dirs.base.to_string_lossy().to_string());
        assert_eq!(user_downloads_dir(&s), dirs.base);
        s.custom_downloads_path = Some("/not/a/real/folder".into());
        assert!(user_downloads_dir(&s).ends_with("Downloads"));
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn debug_relaunch_is_refused_when_already_debugging_or_in_a_flatpak() {
        assert!(debug_relaunch_allowed(false, false));
        assert!(!debug_relaunch_allowed(false, true));
        assert!(!debug_relaunch_allowed(true, false));
    }

    // ---- live checks: `cargo test --lib live_ -- --ignored --nocapture` ----

    #[tokio::test]
    #[ignore = "uses the network"]
    async fn live_network_check_without_the_big_download() {
        let dirs = scratch("live");
        // two hosts pinged (3 packets) and one traced, every host looked up, the download server asked; no 100 MB file
        let options = NetworkCheckOptions {
            ping_count: 3,
            trace_routes: false,
            speed_test: false,
            command_timeout: Duration::from_secs(40),
            ping_hosts: vec![hosts::MODRINTH.to_string(), hosts::FABRIC.to_string()],
        };
        let progress = Progress::new();
        let report = run_network_check(&dirs, &options, &progress).await.unwrap();
        println!("{report}");
        assert!(report.contains(&format!("Ping results to {} was ", hosts::MODRINTH)));
        for host in ALL_HOSTS {
            assert!(
                report.contains(&format!("Resolution of {host} was "))
                    || report.contains(&format!("Resolution of {host} failed")),
                "{host}"
            );
        }
        assert!(
            report.contains("Response code to https://download.nodecdn.net/containers/atl was")
                || report.contains("Exception thrown when connecting")
        );
        assert!(!report.contains("Download speed"));
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[tokio::test]
    #[ignore = "uses the network"]
    async fn live_traceroute_of_one_host() {
        let out = trace_route("api.modrinth.com", Duration::from_secs(60)).await;
        println!("{out}");
        assert!(
            out.contains("traceroute to") || out.contains("Tracing route") || out.is_empty(),
            "unexpected output: {out}"
        );
    }
}
