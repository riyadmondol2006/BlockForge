//! The `blockforge` program: the launcher window and the command line.
// Release builds for Windows are GUI programs, so double-clicking them opens no console window. `winconsole` below
// connects the command line subcommands to the terminal they were started from.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use blockforge::accounts::{self, Account};
use blockforge::config::Settings;
use blockforge::install::{self, Progress};
use blockforge::loaders::{self, Loader};
use blockforge::paths::Dirs;
use blockforge::{instances, java, launch, modrinth, msauth};

#[derive(Parser)]
#[command(name = "blockforge", version, about = "BlockForge - a fast Minecraft launcher")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    /// Write debug lines to the log (the Tools page's "Debug mode" restarts the launcher with this)
    #[arg(long, global = true)]
    debug: bool,
    /// How much debug logging: 1 is the usual debug mode, up to 5 for everything
    #[arg(long, global = true)]
    debug_level: Option<u8>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the launcher window (the default)
    Gui,
    /// List instances
    List,
    /// List Minecraft versions
    Versions {
        #[arg(long)]
        snapshots: bool,
        #[arg(long, default_value_t = 15)]
        count: usize,
    },
    /// Create an instance
    Create {
        name: String,
        #[arg(long)]
        mc: Option<String>,
        #[arg(long, default_value = "vanilla")]
        loader: String,
        #[arg(long)]
        loader_version: Option<String>,
    },
    /// Start an instance
    Launch {
        instance: String,
        /// Account name or id (defaults to the last one used)
        #[arg(long)]
        account: Option<String>,
        /// Build the command and print it, but don't start the game
        #[arg(long)]
        dry_run: bool,
        /// Join this server when the game starts (`host` or `host:port`)
        #[arg(long)]
        server: Option<String>,
        /// Open this world (its folder name in `saves`) when the game starts
        #[arg(long)]
        world: Option<String>,
        /// Join this Realm (its id) when the game starts
        #[arg(long)]
        realm: Option<String>,
        /// Play under this name instead of the account's
        #[arg(long)]
        offline_name: Option<String>,
        /// Answer "yes" to the questions the launcher asks before starting (a pack wanting more memory than set)
        #[arg(long)]
        yes: bool,
    },
    /// Show accounts, add an offline one, or sign in with Microsoft
    Account {
        #[command(subcommand)]
        action: Option<AccountCmd>,
    },
    /// Search Modrinth
    Search {
        query: String,
        #[arg(long, default_value = "mod")]
        kind: String,
        #[arg(long)]
        mc: Option<String>,
        #[arg(long, default_value = "vanilla")]
        loader: String,
    },
    /// Install a Modrinth mod, resource pack or shader into an instance
    Add {
        instance: String,
        project: String,
        #[arg(long, default_value = "mod")]
        kind: String,
    },
    /// Create an instance from a Modrinth modpack (.mrpack file or project id)
    Import { source: String },
    /// Show the Javas found on this computer
    Java,
    /// Delete an instance
    Delete { instance: String },
    /// Write the list of texts to translate (a .pot template) from the program's source code
    #[command(hide = true)]
    Pot {
        /// The source folder to read
        #[arg(long, default_value = "src")]
        src: PathBuf,
        /// Where to write it (the screen if not given)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Show where the data lives
    Paths,
    /// FTB modpacks, importing from other launchers, exporting
    Packs {
        #[command(subcommand)]
        action: blockforge::packs_cli::PacksCmd,
    },
}

#[derive(Subcommand)]
enum AccountCmd {
    Offline { name: String },
    Login,
    Remove { name: String },
}

/// Windows only: what a GUI-subsystem program needs so that `blockforge --version`, `blockforge list` and the other
/// subcommands print where they were started from.
#[cfg(windows)]
mod winconsole {
    use std::fs::OpenOptions;
    use std::os::windows::io::IntoRawHandle;

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

    /// The id the Start Menu and desktop shortcuts of the installer carry (`AppUserModelID` in installer.iss), so the
    /// taskbar groups the window with its shortcut and keeps the pin.
    const APP_USER_MODEL_ID: &str = "io.github.riyadmondol2006.BlockForge";

    fn unusable(handle: HANDLE) -> bool {
        handle.is_null() || handle == INVALID_HANDLE_VALUE
    }

    /// Points standard stream `which` at the console device `device` unless the program was given a stream already
    /// (`blockforge --version > out.txt` or `| more` hands over a file or a pipe, which must be kept as it is).
    fn reopen(which: u32, device: &str, write: bool) {
        // SAFETY: GetStdHandle only reads the process's standard handle table.
        if !unusable(unsafe { GetStdHandle(which) }) {
            return;
        }
        let opened = OpenOptions::new().read(!write).write(write).open(device);
        if let Ok(file) = opened {
            // SAFETY: the handle is valid and is handed over to the process, which keeps it until it exits.
            unsafe { SetStdHandle(which, file.into_raw_handle() as HANDLE) };
        }
    }

    pub fn init() {
        // SAFETY: the string is NUL-terminated and outlives the call.
        unsafe {
            let id: Vec<u16> = APP_USER_MODEL_ID.encode_utf16().chain(Some(0)).collect();
            SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
        // Started from a shortcut or by double-clicking: no arguments, no terminal to talk to.
        if std::env::args_os().len() <= 1 {
            return;
        }
        // SAFETY: only reads the process's standard handle table.
        let missing = |which| unusable(unsafe { GetStdHandle(which) });
        if !(missing(STD_INPUT_HANDLE) || missing(STD_OUTPUT_HANDLE) || missing(STD_ERROR_HANDLE)) {
            return; // all three are redirected or inherited already (also the case of a debug build with a console)
        }
        // SAFETY: AttachConsole has no memory requirements. It fails when the parent has no console (the program was
        // started by a service or by Explorer), and then there is simply nowhere to print.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return;
        }
        reopen(STD_INPUT_HANDLE, "CONIN$", false);
        reopen(STD_OUTPUT_HANDLE, "CONOUT$", true);
        reopen(STD_ERROR_HANDLE, "CONOUT$", true);
    }
}

fn main() -> Result<()> {
    #[cfg(windows)]
    winconsole::init();
    let _ = blockforge::START.set(Instant::now());
    // files given instead of a command (the desktop's "open with", or a double-clicked .mrpack) open the window and
    // go to the import flow
    let files = files_from_arguments();
    if !files.is_empty() {
        let dirs = Dirs::detect();
        dirs.ensure()?;
        blockforge::svcsettings::init(&dirs, false, None);
        return blockforge::gui::run(dirs, files);
    }
    let cli = Cli::parse();
    let dirs = Dirs::detect();
    dirs.ensure()?;
    match cli.command {
        None | Some(Cmd::Gui) => {
            blockforge::svcsettings::init(&dirs, cli.debug, cli.debug_level);
            blockforge::gui::run(dirs, Vec::new())
        }
        Some(cmd) => {
            blockforge::svcsettings::init_network(&dirs);
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
            rt.block_on(run(cmd, dirs))
        }
    }
}

/// The arguments, if all of them are paths to files or folders that exist (also as `file://` addresses, which some
/// desktops pass). Anything else, such as a command or an option, means they are not files to open.
fn files_from_arguments() -> Vec<PathBuf> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Vec::new();
    }
    let mut files = Vec::new();
    for a in &args {
        let path = a.strip_prefix("file://").map(decode_percent).unwrap_or_else(|| a.clone());
        let path = PathBuf::from(path);
        if a.starts_with('-') || !path.exists() {
            return Vec::new();
        }
        files.push(path);
    }
    files
}

/// `%20` and friends in a `file://` address.
fn decode_percent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What a launch problem says, on one line.
fn problem_text(p: &launch::prompts::Prompt) -> String {
    let text = p.message.replace('\n', " ");
    if p.title.is_empty() || text.starts_with(&p.title) { text } else { format!("{}: {}", p.title, text) }
}

fn find_instance(dirs: &Dirs, key: &str) -> Result<instances::Instance> {
    let all = instances::load_all(dirs);
    let lower = key.to_lowercase();
    all.into_iter()
        .find(|i| i.folder.to_lowercase() == lower || i.name().to_lowercase() == lower)
        .with_context(|| format!("no instance called \"{key}\" (try `blockforge list`)"))
}

fn show_progress(progress: Arc<Progress>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last = String::new();
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let stage = progress.stage_text();
            let (d, t) = (progress.files_done.load(Ordering::Relaxed), progress.files_total.load(Ordering::Relaxed));
            let line = if t > 0 {
                format!("{stage}  {d}/{t} files  {:.0} MB", progress.bytes_done.load(Ordering::Relaxed) as f64 / 1e6)
            } else {
                stage
            };
            if line != last {
                eprintln!("  {line}");
                last = line;
            }
        }
    })
}

fn pick_account(dirs: &Dirs, settings: &Settings, wanted: Option<&str>) -> Result<Account> {
    let all = accounts::load(dirs);
    let found = match wanted {
        Some(w) => all.iter().find(|a| a.name().eq_ignore_ascii_case(w) || a.uuid == w),
        None => settings.last_account.as_ref().and_then(|id| all.iter().find(|a| &a.uuid == id)).or(all.first()),
    };
    found.cloned().context("no account yet: run `blockforge account offline <name>` or `blockforge account login`")
}

async fn run(cmd: Cmd, dirs: Dirs) -> Result<()> {
    let settings = Settings::load(&dirs);
    match cmd {
        Cmd::Gui => unreachable!(),
        Cmd::Pot { src, output } => {
            let pot = blockforge::i18n::template::extract_template(&src);
            match output {
                Some(path) => std::fs::write(&path, pot).with_context(|| format!("writing {}", path.display()))?,
                None => print!("{pot}"),
            }
        }
        Cmd::Paths => {
            println!("data folder: {}", dirs.base.display());
            println!("instances:   {}", dirs.instances.display());
        }
        Cmd::List => {
            let t = Instant::now();
            let all = instances::load_all(&dirs);
            for i in &all {
                let loader = i.loader().map(|(k, v)| format!("{k} {v}")).unwrap_or_else(|| "Vanilla".into());
                println!(
                    "{:<32} Minecraft {:<10} {:<22} {} mods",
                    i.name(),
                    i.mc_version(),
                    loader,
                    i.enabled_mod_count()
                );
            }
            eprintln!("{} instances in {:.1} ms", all.len(), t.elapsed().as_secs_f64() * 1000.0);
        }
        Cmd::Versions { snapshots, count } => {
            let m = install::manifest(&dirs).await?;
            println!("latest release: {}   latest snapshot: {}", m.latest["release"], m.latest["snapshot"]);
            for v in m.versions.iter().filter(|v| snapshots || v.kind == "release").take(count) {
                println!("{:<14} {}", v.id, v.kind);
            }
        }
        Cmd::Create { name, mc, loader, loader_version } => {
            let loader = Loader::parse(&loader).with_context(|| format!("unknown loader \"{loader}\""))?;
            let mc = match mc {
                Some(m) => m,
                None => install::manifest(&dirs).await?.latest["release"].clone(),
            };
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let t = Instant::now();
            let inst = instances::create(&dirs, &name, &mc, loader, loader_version.as_deref(), &progress).await;
            printer.abort();
            let inst = inst?;
            println!("created \"{}\" in {:.1}s: {}", inst.name(), t.elapsed().as_secs_f64(), inst.root.display());
        }
        Cmd::Launch { instance, account, dry_run, server, world, realm, offline_name, yes } => {
            let mut inst = find_instance(&dirs, &instance)?;
            let acc = match account.as_deref() {
                Some(w) => pick_account(&dirs, &settings, Some(w))?,
                None => launch::account_for(&dirs, &settings, &inst)
                    .context("no account yet: run `blockforge account offline <name>` or `blockforge account login`")?,
            };
            // the checks the Java edition makes before it starts anything
            let ls = launch::settings::LaunchSettings::resolve(&settings, &inst);
            let pick = launch::javapick::pick(&dirs, &ls, &inst.file.version);
            for p in launch::prompts::preflight(&settings, &inst, &ls, &pick, true, false) {
                eprintln!("  {}: {}", p.title, p.message.replace('\n', " "));
                if p.blocking || (p.kind == launch::prompts::PromptKind::Confirm && !yes) {
                    bail!("not starting: {} (use --yes to go on past questions)", p.title);
                }
            }
            let acc = launch::fresh_account(&dirs, &acc).await?;
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let t = Instant::now();
            let options = launch::LaunchOptions { server, world };
            let extras = launch::LaunchExtras { realm, offline_name, screen_size: None };
            let prepared = launch::prepare_ext(&dirs, &settings, &inst, &acc, &progress, &options, &extras).await;
            printer.abort();
            let prepared = match prepared {
                Ok(p) => p,
                Err(e) => match e.downcast_ref::<launch::prompts::LaunchProblem>() {
                    Some(problem) => bail!("{}", problem_text(&problem.prompt)),
                    None => return Err(e),
                },
            };
            for n in &prepared.notes {
                eprintln!("  {n}");
            }
            eprintln!(
                "  ready in {} (waited {:.1} ms in all)",
                prepared.timings_summary(),
                t.elapsed().as_secs_f64() * 1000.0
            );
            if dry_run {
                println!("{}", prepared.display());
                for (k, v) in &prepared.env {
                    eprintln!("  environment: {k}={v}");
                }
                if let Some(c) = &prepared.pre_launch {
                    eprintln!("  would run before the game: {c}");
                }
                if let Some(c) = &prepared.post_exit {
                    eprintln!("  would run after the game: {c}");
                }
                prepared.cleanup();
                return Ok(());
            }
            let mut running = match launch::spawn(&prepared) {
                Ok(r) => r,
                Err(e) => {
                    prepared.cleanup();
                    match e.downcast_ref::<launch::prompts::LaunchProblem>() {
                        Some(problem) => bail!("{}", problem_text(&problem.prompt)),
                        None => return Err(e),
                    }
                }
            };
            inst.mark_played()?;
            while let Some(line) = running.lines.recv().await {
                println!("{line}");
            }
            let status = running.child.wait().await?;
            prepared.cleanup();
            if launch::classcache::builders_running() > 0 {
                eprintln!("  making the class cache for quicker starts next time (20-50 seconds)...");
                launch::classcache::wait_for_builders();
            }
            if !status.success() {
                bail!("the game closed with {status}");
            }
        }
        Cmd::Account { action } => match action {
            None => {
                let all = accounts::load(&dirs);
                if all.is_empty() {
                    println!("no accounts");
                }
                for a in &all {
                    let last = settings.last_account.as_deref() == Some(a.uuid.as_str());
                    println!("{} {:<20} {:<10} {}", if last { "*" } else { " " }, a.name(), a.kind(), a.uuid);
                }
            }
            Some(AccountCmd::Offline { name }) => {
                let mut all = accounts::load(&dirs);
                all.retain(|a| !(a.offline && a.name() == name));
                let acc = Account::offline(&name);
                println!("added offline account {name} ({})", acc.uuid);
                all.push(acc);
                accounts::save(&dirs, &all)?;
            }
            Some(AccountCmd::Login) => {
                let code = msauth::start_device_login().await?;
                println!("Open {} and enter the code {}", code.verification_uri, code.user_code);
                let acc = msauth::finish_device_login(&code).await?;
                let mut all = accounts::load(&dirs);
                all.retain(|a| a.uuid != acc.uuid);
                println!("signed in as {}", acc.name());
                all.push(acc);
                accounts::save(&dirs, &all)?;
            }
            Some(AccountCmd::Remove { name }) => {
                let mut all = accounts::load(&dirs);
                let before = all.len();
                all.retain(|a| !a.name().eq_ignore_ascii_case(&name));
                accounts::save(&dirs, &all)?;
                println!("removed {} account(s)", before - all.len());
            }
        },
        Cmd::Search { query, kind, mc, loader } => {
            let kind = modrinth::Kind::from_api(&kind);
            let loader = Loader::parse(&loader).unwrap_or(Loader::Vanilla);
            let t = Instant::now();
            let (hits, total) =
                modrinth::search(&query, kind, mc.as_deref(), loader, modrinth::Sort::Relevance, 0, 10).await?;
            for h in hits {
                println!(
                    "{:<26} {:>10} downloads  {}  - {}",
                    h.slug,
                    h.downloads,
                    h.title,
                    h.description.chars().take(60).collect::<String>()
                );
            }
            eprintln!("{total} results in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
        Cmd::Add { instance, project, kind } => {
            let inst = find_instance(&dirs, &instance)?;
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let added = modrinth::install_into(&inst, &project, modrinth::Kind::from_api(&kind), &progress).await;
            printer.abort();
            for f in added? {
                println!("added {f}");
            }
        }
        Cmd::Import { source } => {
            let progress = Progress::new();
            let printer = show_progress(progress.clone());
            let path = PathBuf::from(&source);
            let inst = if path.is_file() {
                modrinth::import_mrpack(&dirs, &path, &progress).await
            } else {
                modrinth::install_modpack(&dirs, &source, &progress).await
            };
            printer.abort();
            let inst = inst?;
            println!("created \"{}\" at {}", inst.name(), inst.root.display());
        }
        Cmd::Packs { action } => blockforge::packs_cli::run(action, dirs).await?,
        Cmd::Java => {
            for j in java::detect_installed() {
                println!("Java {:<4} {}", j.major.map(|m| m.to_string()).unwrap_or("?".into()), j.home.display());
            }
        }
        Cmd::Delete { instance } => {
            let inst = find_instance(&dirs, &instance)?;
            instances::delete(&inst)?;
            println!("deleted {}", inst.name());
        }
    }
    let _ = loaders::Loader::ALL;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::decode_percent;

    #[test]
    fn file_addresses_are_decoded() {
        assert_eq!(decode_percent("/home/me/My%20Pack.mrpack"), "/home/me/My Pack.mrpack");
        assert_eq!(decode_percent("/plain/path"), "/plain/path");
        assert_eq!(decode_percent("/odd%"), "/odd%");
        assert_eq!(decode_percent("/odd%2"), "/odd%2");
        assert_eq!(decode_percent("/caf%C3%A9"), "/caf\u{e9}");
    }
}
