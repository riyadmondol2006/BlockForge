//! Reads a game's files once while the launcher sits idle, so they are in the system's file cache when Play is pressed
//! (the Java edition's `GameFileWarmer`, the `warmUpGameFiles` setting). A game's start is mostly reading: the Java
//! runtime, the libraries, the game jar and the mods, a few hundred MB that a hard drive or a slow SSD needs seconds for
//! after a reboot.
//!
//! It stays out of the way: one thread, reading one file after another, a limit on how much it reads, nothing at all
//! while a game runs, an install runs or files download, and it stops the moment Play is pressed. The files are only
//! read, never changed.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::Settings;
use crate::instances::Instance;
use crate::launch::classcache;
use crate::launch::javapick;
use crate::launch::memory;
use crate::launch::settings::LaunchSettings;
use crate::model::Features;
use crate::paths::Dirs;

/// The same instance isn't read again within this time; the system keeps its cache for much longer.
const REWARM_AFTER: Duration = Duration::from_secs(10 * 60);
const CHUNK: usize = 1024 * 1024;

/// Bumped to abandon whatever is being read.
static GENERATION: AtomicUsize = AtomicUsize::new(0);
/// How many games, installs or download batches are running. While any is, the disk is theirs.
static BUSY: AtomicUsize = AtomicUsize::new(0);
static WARMED_AT: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();

/// Hold one of these while a game runs, an install runs or files download; the warmer waits until all are gone.
pub struct BusyGuard;

impl BusyGuard {
    pub fn new() -> BusyGuard {
        BUSY.fetch_add(1, Ordering::SeqCst);
        BusyGuard
    }
}

impl Default for BusyGuard {
    fn default() -> Self {
        BusyGuard::new()
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn busy() -> bool {
    BUSY.load(Ordering::SeqCst) > 0
}

/// Play was pressed: stop reading now, the game itself is about to.
pub fn stop_for_launch() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

fn enabled(settings: &Settings) -> bool {
    serde_json::to_value(settings).ok().and_then(|v| v.get("warmUpGameFiles").and_then(|b| b.as_bool())).unwrap_or(true)
}

/// A computer where only the game played last is kept warm, as the file cache is small too.
fn small_computer() -> bool {
    let total = memory::total_ram_mb();
    memory::is_low_memory_computer(total) || (total > 0 && memory::cores() <= 2)
}

/// How many recently played games to read when the launcher has started.
pub fn instances_to_warm(light_mode: bool, total_mb: i64) -> usize {
    if light_mode || (total_mb > 0 && total_mb <= 4608) {
        return 1;
    }
    if total_mb >= 15360 { 2 } else { 1 }
}

/// How much to read at most: a quarter of the memory free right now (the cache shouldn't push out what other programs
/// use), at most 1 GB, or 256 MB on a small computer. 0 when there is too little free to bother.
pub fn budget_bytes(available_mb: i64, small: bool) -> u64 {
    if available_mb > 0 && available_mb < 1024 {
        return 0;
    }
    let cap: i64 = if small { 256 } else { 1024 };
    let mb = if available_mb > 0 { cap.min(available_mb / 4) } else { cap / 2 };
    mb as u64 * 1024 * 1024
}

/// The files a start of this game reads, roughly in the order it reads them: the Java runtime, its class cache, the
/// libraries, the game jar, the mods and the asset index with the few assets read at start.
pub fn files_for(dirs: &Dirs, settings: &Settings, instance: &Instance) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |p: PathBuf| {
        if seen.insert(p.clone()) {
            files.push(p);
        }
    };
    let ls = LaunchSettings::resolve(settings, instance);
    let java = javapick::pick(dirs, &ls, &instance.file.version);
    if let Some(home) = &java.home {
        add(home.join("lib/modules"));
        add(home.join("lib/server/libjvm.so"));
        add(home.join("lib/server/classes.jsa"));
        add(home.join("bin/server/jvm.dll"));
        add(home.join("lib/server/libjvm.dylib"));
        if let Some(major) = java.major {
            add(classcache::archive_for(dirs, &safe_name(instance), major));
        }
    }
    let features = Features::default();
    for lib in instance.file.version.active_libraries(&features) {
        if let Some(n) = lib.native() {
            if let Some(p) = n.path {
                add(dirs.libraries.join(p));
            }
        } else if let Some(p) = lib.artifact().and_then(|a| a.path) {
            add(dirs.libraries.join(p));
        }
    }
    add(crate::install::client_jar(dirs, instance.mc_version()));
    if let Ok(rd) = std::fs::read_dir(instance.root.join("mods")) {
        let mut mods: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jar")).collect();
        mods.sort();
        for m in mods {
            add(m);
        }
    }
    for p in assets_read_at_start(dirs, instance) {
        add(p);
    }
    files
}

/// The Java edition's `getSafeName`: the instance name with everything but letters and digits removed. A name with no
/// ASCII letters or digits at all (`Сборка`, `世界`) gets a short code made from its folder instead of nothing: with
/// nothing, every such instance shared one class cache, and each start threw away the other's.
pub fn safe_name(instance: &Instance) -> String {
    let ascii: String = instance.name().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !ascii.is_empty() {
        return ascii;
    }
    format!("instance{}", &crate::http::sha1_hex(instance.folder.as_bytes())[..10])
}

/// The asset index and the assets every start reads: the sound list and the language files of the game's language.
pub fn assets_read_at_start(dirs: &Dirs, instance: &Instance) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Some(index_ref) = &instance.file.version.asset_index else { return files };
    let index_file = crate::install::asset_index_file(dirs, &index_ref.id);
    if !index_file.is_file() {
        return files;
    }
    files.push(index_file);
    if let Ok(index) = crate::install::read_asset_index(dirs, &index_ref.id) {
        let language = language_of(&instance.root);
        let wanted = format!("minecraft/lang/{language}.json");
        let mut picked: Vec<&String> = index
            .objects
            .keys()
            .filter(|k| {
                k.as_str() == "minecraft/sounds.json"
                    || k.as_str() == "pack.mcmeta"
                    || **k == wanted
                    || k.starts_with("minecraft/sounds/ui/")
            })
            .collect();
        picked.sort();
        for key in picked {
            let hash = &index.objects[key].hash;
            if hash.len() > 2 {
                files.push(dirs.assets.join("objects").join(&hash[..2]).join(hash));
            }
        }
    }
    files
}

/// The game's language from `options.txt`, `en_us` when not set.
pub fn language_of(game_directory: &Path) -> String {
    std::fs::read_to_string(game_directory.join("options.txt"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("lang:").map(|v| v.trim().to_lowercase())))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "en_us".to_string())
}

/// Reads the files one after another, up to the budget, stopping as soon as `stop` says so. Returns the bytes read.
pub fn read_all(files: &[PathBuf], budget: u64, stop: &dyn Fn() -> bool) -> u64 {
    let mut read = 0u64;
    let mut buffer = vec![0u8; CHUNK];
    for file in files {
        if read >= budget || stop() {
            break;
        }
        let Ok(mut f) = std::fs::File::open(file) else { continue };
        if !f.metadata().map(|m| m.is_file()).unwrap_or(false) {
            continue;
        }
        while read < budget {
            match f.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => read += n as u64,
            }
            if stop() {
                return read;
            }
        }
    }
    read
}

/// Reads an instance's files right away (blocks) and returns how many bytes were read. For measuring.
pub fn warm_now(dirs: &Dirs, settings: &Settings, instance: &Instance) -> u64 {
    let files = files_for(dirs, settings, instance);
    read_all(&files, budget_bytes(memory::available_mb(), small_computer()), &|| false)
}

type Job = Box<dyn FnOnce() + Send>;

struct Worker {
    queue: Mutex<VecDeque<Job>>,
    wake: Condvar,
    running: AtomicBool,
}

fn worker() -> &'static Worker {
    static W: OnceLock<Worker> = OnceLock::new();
    W.get_or_init(|| Worker {
        queue: Mutex::new(VecDeque::new()),
        wake: Condvar::new(),
        running: AtomicBool::new(false),
    })
}

/// One thread, gone when idle, and at most a few waiting jobs (older hovers are dropped).
fn submit(job: Job) {
    let w = worker();
    {
        let mut q = w.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= 4 {
            q.pop_front();
        }
        q.push_back(job);
    }
    w.wake.notify_one();
    if !w.running.swap(true, Ordering::SeqCst) {
        let _ = std::thread::Builder::new().name("game-file-warmer".into()).spawn(|| {
            let w = worker();
            loop {
                let job = {
                    let mut q = w.queue.lock().unwrap_or_else(|e| e.into_inner());
                    if q.is_empty() {
                        let (guard, timeout) =
                            w.wake.wait_timeout(q, Duration::from_secs(30)).unwrap_or_else(|e| e.into_inner());
                        q = guard;
                        if timeout.timed_out() && q.is_empty() {
                            w.running.store(false, Ordering::SeqCst);
                            return;
                        }
                    }
                    q.pop_front()
                };
                if let Some(job) = job {
                    job();
                }
            }
        });
    }
}

/// The player is pointing at or opening an instance: read its files if that hasn't been done lately. `all` is every
/// instance, to know which one was played last. Returns at once; the reading happens on another thread.
pub fn warm_soon(dirs: &Dirs, settings: &Settings, instance: &Instance, all: &[Instance]) {
    if !enabled(settings) || busy() {
        return;
    }
    // on a small computer only the game played last is kept warm
    if small_computer() {
        let latest = all.iter().max_by_key(|i| i.last_played());
        if latest.is_some_and(|l| l.folder != instance.folder) {
            return;
        }
    }
    {
        let mut warmed = WARMED_AT.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        if warmed.get(&instance.root).is_some_and(|t| t.elapsed() < REWARM_AFTER) {
            return;
        }
        warmed.insert(instance.root.clone(), Instant::now());
    }
    let generation = GENERATION.load(Ordering::SeqCst);
    let (dirs, settings, instance) = (dirs.clone(), settings.clone(), instance.clone());
    submit(Box::new(move || {
        if GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let files = files_for(&dirs, &settings, &instance);
        let budget = budget_bytes(memory::available_mb(), small_computer());
        read_all(&files, budget, &|| GENERATION.load(Ordering::SeqCst) != generation || busy());
    }));
}

/// Called a few seconds after the launcher's window shows: reads the most recently played game(s).
pub fn launcher_idle(dirs: &Dirs, settings: &Settings, instances: &[Instance]) {
    if !enabled(settings) {
        return;
    }
    let mut played: Vec<&Instance> = instances.iter().filter(|i| i.last_played() > 0).collect();
    played.sort_by_key(|i| std::cmp::Reverse(i.last_played()));
    for instance in played.into_iter().take(instances_to_warm(small_computer(), memory::total_ram_mb())) {
        warm_soon(dirs, settings, instance, instances);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instances::InstanceFile;

    #[test]
    fn budgets() {
        assert_eq!(budget_bytes(500, false), 0);
        assert_eq!(budget_bytes(2048, false), 512 * 1024 * 1024);
        assert_eq!(budget_bytes(16384, false), 1024 * 1024 * 1024);
        assert_eq!(budget_bytes(16384, true), 256 * 1024 * 1024);
        assert_eq!(budget_bytes(-1, false), 512 * 1024 * 1024);
        assert_eq!(instances_to_warm(true, 32768), 1);
        assert_eq!(instances_to_warm(false, 4096), 1);
        assert_eq!(instances_to_warm(false, 8192), 1);
        assert_eq!(instances_to_warm(false, 16384), 2);
    }

    #[test]
    fn reading_stops_at_the_budget_and_when_told() {
        let dir = std::env::temp_dir().join(format!("bf-warm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::write(&a, vec![1u8; 3 * 1024 * 1024]).unwrap();
        std::fs::write(&b, vec![2u8; 1024]).unwrap();
        let files = vec![dir.join("missing"), a.clone(), b.clone()];
        assert_eq!(read_all(&files, u64::MAX, &|| false), 3 * 1024 * 1024 + 1024);
        // the budget ends the first file at a whole chunk
        assert_eq!(read_all(&files, 1024 * 1024, &|| false), 1024 * 1024);
        assert_eq!(read_all(&files, u64::MAX, &|| true), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn busy_guards_count() {
        let before = busy();
        {
            let _g = BusyGuard::new();
            assert!(busy());
        }
        assert_eq!(busy(), before);
    }

    #[test]
    fn what_a_start_reads() {
        let dir = std::env::temp_dir().join(format!("bf-warm2-{}", std::process::id()));
        let dirs = Dirs::at(&dir);
        let root = dir.join("instances/A");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::write(root.join("mods/b.jar"), b"x").unwrap();
        std::fs::write(root.join("mods/a.jar"), b"x").unwrap();
        std::fs::write(root.join("mods/readme.txt"), b"x").unwrap();
        std::fs::write(root.join("options.txt"), "lang:de_de\n").unwrap();
        assert_eq!(language_of(&root), "de_de");
        assert_eq!(language_of(&dir), "en_us");
        let mut file = InstanceFile::default();
        file.version = serde_json::from_str(
            r#"{"id":"1.21.1","assetIndex":{"id":"17","sha1":"x","size":1,"url":"u"},"libraries":[{"name":"a.b:c:1.0","downloads":{"artifact":{"path":"a/b/c/1.0/c-1.0.jar","url":"u","sha1":"x","size":1}}}]}"#,
        )
        .unwrap();
        let i = Instance { folder: "A".into(), mod_count: 0, root: root.clone(), file };
        std::fs::create_dir_all(dirs.assets.join("indexes")).unwrap();
        std::fs::write(
            dirs.assets.join("indexes/17.json"),
            r#"{"objects":{"minecraft/sounds.json":{"hash":"aabbcc","size":1},"minecraft/lang/de_de.json":{"hash":"ddeeff","size":1},"minecraft/lang/fr_fr.json":{"hash":"112233","size":1},"minecraft/sounds/ui/click.ogg":{"hash":"445566","size":1}}}"#,
        )
        .unwrap();
        let files = files_for(&dirs, &Settings::default(), &i);
        let names: Vec<String> =
            files.iter().map(|f| f.strip_prefix(&dir).unwrap().to_string_lossy().to_string()).collect();
        let pos = |needle: &str| {
            names.iter().position(|n| n.ends_with(needle)).unwrap_or_else(|| panic!("{needle} missing in {names:?}"))
        };
        assert!(pos("c-1.0.jar") < pos("client-1.21.1.jar"));
        assert!(pos("client-1.21.1.jar") < pos("mods/a.jar"));
        assert!(pos("mods/a.jar") < pos("mods/b.jar"));
        assert!(pos("mods/b.jar") < pos("indexes/17.json"));
        assert!(names.iter().any(|n| n.ends_with("objects/aa/aabbcc")));
        assert!(names.iter().any(|n| n.ends_with("objects/dd/ddeeff")));
        assert!(!names.iter().any(|n| n.contains("112233")));
        assert!(!names.iter().any(|n| n.ends_with("readme.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crate::instances::InstanceFile;

    fn inst(folder: &str, name: &str) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher.name = name.into();
        Instance { folder: folder.into(), mod_count: 0, root: "/x".into(), file }
    }

    #[test]
    fn instances_named_outside_ascii_get_cache_names_of_their_own() {
        let a = safe_name(&inst("Сборка", "Сборка"));
        let b = safe_name(&inst("Мир", "Мир"));
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()), "{a}");
        assert_eq!(a, safe_name(&inst("Сборка", "Сборка")), "the same instance keeps its name");
        // ASCII names are as before, so existing caches are still found
        assert_eq!(safe_name(&inst("MyPack", "My Pack!")), "MyPack");
        assert_ne!(
            crate::launch::lite::instance_key(&inst("Сборка", "Сборка")),
            crate::launch::lite::instance_key(&inst("Мир", "Мир"))
        );
    }
}
