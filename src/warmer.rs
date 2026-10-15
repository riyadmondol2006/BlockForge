//! Reading a game's files ahead of Play: the Java edition's `GameFileWarmer`.
//!
//! A game's start is mostly reading: the Java runtime, the libraries, the game jar and the mods, a few hundred MB
//! that a hard drive or a slow SSD needs seconds for after a reboot. This reads them once while the launcher is
//! idle, so they are in the system's file cache when Play is pressed.
//!
//! It stays out of the way: one background thread reading one file after another, a limit on how much it reads,
//! nothing at all while the caller says the disk is busy (a game running, an install, downloads), and it stops
//! the moment Play is pressed. The files are only read, never changed.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Settings;
use crate::instances::Instance;
use crate::model::Features;
use crate::paths::Dirs;
use crate::svcsettings::ServiceSettings;

/// The same instance isn't read again within this time; the system keeps its cache for much longer.
const REWARM_AFTER: Duration = Duration::from_secs(10 * 60);
const CHUNK: usize = 1024 * 1024;

/// Bumped to abandon whatever is being read.
static GENERATION: AtomicU32 = AtomicU32::new(0);
static WARMED_AT: Mutex<Option<HashMap<PathBuf, Instant>>> = Mutex::new(None);

/// Whether the disk belongs to something else right now: a game is running, an install or downloads are going.
pub type BusyFn = Arc<dyn Fn() -> bool + Send + Sync>;

/// A small computer: the Java edition's light mode, or 4.5 GB of memory or less.
pub fn small_computer(total_mb: u64) -> bool {
    total_mb > 0 && total_mb <= 4096 + 512
}

/// How many recently played games to read when the launcher has started.
pub fn instances_to_warm(small: bool, total_mb: u64) -> usize {
    if small || (total_mb > 0 && total_mb <= 4608) {
        return 1;
    }
    if total_mb >= 15360 { 2 } else { 1 }
}

/// How much to read at most: a quarter of the memory free right now (the cache shouldn't push out what other
/// programs use), at most 1 GB, or 256 MB on a small computer. 0 when there is too little free to bother.
pub fn budget_bytes(available_mb: u64, small: bool) -> u64 {
    if available_mb > 0 && available_mb < 1024 {
        return 0;
    }
    let cap: u64 = if small { 256 } else { 1024 };
    let mb = if available_mb > 0 { cap.min(available_mb / 4) } else { cap / 2 };
    mb * 1024 * 1024
}

/// The game's language from `options.txt`, `en_us` when not set.
pub fn language_of(game_dir: &Path) -> String {
    std::fs::read_to_string(game_dir.join("options.txt"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("lang:").map(|v| v.trim().to_lowercase())))
        .unwrap_or_else(|| "en_us".to_string())
}

/// The asset index and the assets every start reads: the sound list and the language files of the language.
pub fn assets_read_at_start(dirs: &Dirs, instance: &Instance) -> Vec<PathBuf> {
    let Some(index_ref) = instance.file.version.asset_index.as_ref().filter(|i| !i.id.is_empty()) else {
        return Vec::new();
    };
    let index_file = crate::install::asset_index_file(dirs, &index_ref.id);
    if !index_file.is_file() {
        return Vec::new();
    }
    let mut files = vec![index_file];
    if let Ok(index) = crate::install::read_asset_index(dirs, &index_ref.id) {
        let language = language_of(&instance.root);
        let lang_key = format!("minecraft/lang/{language}.json");
        let mut keys: Vec<_> = index.objects.iter().collect();
        keys.sort_by(|a, b| a.0.cmp(b.0));
        for (key, obj) in keys {
            if key == "minecraft/sounds.json"
                || key == "pack.mcmeta"
                || *key == lang_key
                || key.starts_with("minecraft/sounds/ui/")
            {
                if obj.hash.len() > 2 {
                    files.push(dirs.assets.join("objects").join(&obj.hash[..2]).join(&obj.hash));
                }
            }
        }
    }
    files
}

/// The files a start of this game reads, roughly in the order it reads them: the Java runtime, its class cache,
/// the libraries, the game jar, the mods and the asset index with the few assets read at start.
pub fn files_for(dirs: &Dirs, settings: &Settings, instance: &Instance) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut add = |p: PathBuf| {
        if seen.insert(p.clone()) {
            files.push(p);
        }
    };

    let launcher = &instance.file.launcher;
    let component = instance.file.version.java_version.as_ref().map(|j| j.component.clone());
    let choice = crate::java::choose(
        dirs,
        settings,
        launcher.java_path.as_deref(),
        component.as_deref(),
        launcher.use_java_provided_by_minecraft.unwrap_or(true),
    );
    if let Some(home) = &choice.home {
        add(home.join("lib/modules"));
        add(home.join("lib/server/libjvm.so"));
        add(home.join("lib/server/classes.jsa"));
        add(home.join("bin/server/jvm.dll"));
        add(home.join("lib/server/libjvm.dylib"));
        if let Some(major) = crate::java::major_version(home) {
            add(crate::classcache::archive_for(dirs, &crate::backups::safe_name(instance.name()), major));
        }
    }

    let features = Features::default();
    for lib in instance.file.version.active_libraries(&features) {
        if let Some(native) = lib.native() {
            if let Some(path) = native.path {
                add(dirs.libraries.join(path));
                continue;
            }
        }
        if let Some(path) = lib.artifact().and_then(|a| a.path) {
            add(dirs.libraries.join(path));
        }
    }
    add(crate::install::client_jar(dirs, instance.file.version.game_version()));

    if let Ok(rd) = std::fs::read_dir(instance.mods_dir()) {
        let mut mods: Vec<PathBuf> =
            rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jar")).collect();
        mods.sort();
        for m in mods {
            add(m);
        }
    }
    for a in assets_read_at_start(dirs, instance) {
        add(a);
    }
    files
}

/// Read the files one after another, up to `budget` bytes, stopping as soon as `stop` says so. Returns the bytes read.
pub fn read_all(files: &[PathBuf], budget: u64, stop: &dyn Fn() -> bool) -> u64 {
    let mut read = 0u64;
    let mut buffer = vec![0u8; CHUNK];
    for file in files {
        if read >= budget || stop() {
            break;
        }
        if !file.is_file() {
            continue;
        }
        // a file in use or gone: the next one
        let Ok(mut f) = std::fs::File::open(file) else { continue };
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

fn enabled(settings: &Settings) -> bool {
    ServiceSettings::from_settings(settings).warm_up_game_files
}

/// Read one instance's files now, on this thread. Returns the bytes read.
pub fn warm_now(dirs: &Dirs, settings: &Settings, instance: &Instance, stop: &dyn Fn() -> bool) -> u64 {
    let (total, available) = crate::platform::memory_mb();
    let budget = budget_bytes(available, small_computer(total));
    read_all(&files_for(dirs, settings, instance), budget, stop)
}

/// The player is pointing at or opening an instance, or the launcher has just become idle: read its files in the
/// background if that hasn't been done lately. `latest_played` is the folder name of the game played last: on a
/// small computer only that one is kept warm.
pub fn warm_soon(dirs: &Dirs, settings: &Settings, instance: &Instance, latest_played: Option<&str>, busy: BusyFn) {
    if !enabled(settings) || busy() {
        return;
    }
    let (total, _) = crate::platform::memory_mb();
    if small_computer(total) && latest_played.is_some_and(|l| l != instance.folder) {
        return;
    }
    {
        let mut guard = WARMED_AT.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if map.get(&instance.root).is_some_and(|t| t.elapsed() < REWARM_AFTER) {
            return;
        }
        map.insert(instance.root.clone(), Instant::now());
    }
    let generation = GENERATION.load(Ordering::Relaxed);
    let (dirs, settings, instance) = (dirs.clone(), settings.clone(), instance.clone());
    let _ = std::thread::Builder::new().name("game-file-warmer".into()).spawn(move || {
        if GENERATION.load(Ordering::Relaxed) != generation {
            return;
        }
        let stop = || GENERATION.load(Ordering::Relaxed) != generation || busy();
        let read = warm_now(&dirs, &settings, &instance, &stop);
        crate::logs::debug(format!("Read {} MB of {}'s files ahead of Play", read / (1024 * 1024), instance.name()));
    });
}

/// A few seconds after the launcher's window shows: read the most recently played game(s). `instances` is
/// the instance list, most recently played first (as `instances::load_all` returns it).
pub fn launcher_idle(dirs: &Dirs, settings: &Settings, instances: &[Instance], busy: BusyFn) {
    if !enabled(settings) {
        return;
    }
    let (total, _) = crate::platform::memory_mb();
    let count = instances_to_warm(small_computer(total), total);
    let latest = instances.iter().max_by_key(|i| i.last_played()).map(|i| i.folder.clone());
    let mut played: Vec<&Instance> = instances.iter().filter(|i| i.last_played() > 0).collect();
    played.sort_by_key(|i| std::cmp::Reverse(i.last_played()));
    for i in played.into_iter().take(count) {
        warm_soon(dirs, settings, i, latest.as_deref(), busy.clone());
    }
}

/// Play was pressed: stop reading now, the game itself is about to start.
pub fn stop_for_launch() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn how_many_games_to_warm() {
        assert_eq!(instances_to_warm(true, 32000), 1);
        assert_eq!(instances_to_warm(false, 4096), 1);
        assert_eq!(instances_to_warm(false, 8192), 1);
        assert_eq!(instances_to_warm(false, 16384), 2);
        assert_eq!(instances_to_warm(false, 0), 1);
        assert!(small_computer(4096) && !small_computer(8192) && !small_computer(0));
    }

    #[test]
    fn the_budget_is_a_quarter_of_free_memory_with_caps() {
        assert_eq!(budget_bytes(512, false), 0, "too little free to bother");
        assert_eq!(budget_bytes(2048, false), 512 * 1024 * 1024);
        assert_eq!(budget_bytes(16384, false), 1024 * 1024 * 1024);
        assert_eq!(budget_bytes(16384, true), 256 * 1024 * 1024);
        assert_eq!(budget_bytes(0, false), 512 * 1024 * 1024, "unknown free memory: half the cap");
    }

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("blockforge-warm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn reading_stops_at_the_budget_and_when_told() {
        let dir = scratch("read");
        let files: Vec<PathBuf> = (0..3)
            .map(|i| {
                let f = dir.join(format!("f{i}.bin"));
                std::fs::write(&f, vec![1u8; 3 * CHUNK]).unwrap();
                f
            })
            .collect();
        let mut with_missing = vec![dir.join("missing.bin")];
        with_missing.extend(files.clone());
        assert_eq!(
            read_all(&with_missing, 100 * CHUNK as u64, &|| false),
            9 * CHUNK as u64,
            "a missing file is skipped"
        );
        assert_eq!(read_all(&files, 4 * CHUNK as u64, &|| false), 4 * CHUNK as u64, "stops at the budget");
        assert_eq!(read_all(&files, 0, &|| false), 0);
        let calls = std::cell::Cell::new(0);
        let n = read_all(&files, 100 * CHUNK as u64, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 3
        });
        assert!(n < 9 * CHUNK as u64, "told to stop part way: {n}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn language_comes_from_options() {
        let dir = scratch("lang");
        assert_eq!(language_of(&dir), "en_us");
        std::fs::write(dir.join("options.txt"), "fov:70\nlang:De_DE\nsound:1").unwrap();
        assert_eq!(language_of(&dir), "de_de");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_of_a_real_instance_in_a_scratch_data_folder() {
        // an instance with one library, a client jar, a mod and an asset index, in a throwaway data folder
        let base = scratch("files");
        let dirs = Dirs::at(&base);
        let root = dirs.instances.join("Test");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::write(root.join("mods/b.jar"), "x").unwrap();
        std::fs::write(root.join("mods/a.jar"), "x").unwrap();
        std::fs::write(root.join("mods/readme.txt"), "x").unwrap();
        std::fs::write(root.join("options.txt"), "lang:fr_fr").unwrap();
        let json = serde_json::json!({
            "id": "1.21", "mainClass": "m",
            "assetIndex": {"id": "17", "sha1": "", "size": 0, "url": ""},
            "libraries": [{"name": "org.ow2.asm:asm:9.11", "downloads": {"artifact": {"path": "org/ow2/asm/asm/9.11/asm-9.11.jar", "url": "u", "sha1": "s", "size": 1}}}]
        });
        std::fs::write(root.join("instance.json"), serde_json::to_vec(&json).unwrap()).unwrap();
        std::fs::create_dir_all(dirs.assets.join("indexes")).unwrap();
        let index = serde_json::json!({"objects": {
            "minecraft/sounds.json": {"hash": "aabbcc", "size": 1},
            "minecraft/lang/fr_fr.json": {"hash": "ddeeff", "size": 1},
            "minecraft/lang/en_us.json": {"hash": "112233", "size": 1},
            "minecraft/sounds/ui/click.ogg": {"hash": "445566", "size": 1},
            "minecraft/textures/x.png": {"hash": "778899", "size": 1}
        }});
        std::fs::write(dirs.assets.join("indexes/17.json"), serde_json::to_vec(&index).unwrap()).unwrap();

        let instance = crate::instances::load_one(&root).unwrap();
        let files = files_for(&dirs, &Settings::default(), &instance);
        let rel = |p: &PathBuf| {
            p.strip_prefix(&base)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_else(|_| p.to_string_lossy().to_string())
        };
        let names: Vec<String> = files.iter().map(rel).collect();
        let pos = |needle: &str| {
            names.iter().position(|n| n.ends_with(needle)).unwrap_or_else(|| panic!("{needle} missing from {names:?}"))
        };
        assert!(pos("asm-9.11.jar") < pos("a.jar"), "libraries come before mods");
        assert!(pos("a.jar") < pos("b.jar") && pos("b.jar") < pos("indexes/17.json"));
        assert!(names.iter().any(|n| n.contains("client-1.21.jar")));
        assert!(!names.iter().any(|n| n.ends_with("readme.txt")));
        assert!(
            names.iter().any(|n| n.ends_with("objects/aa/aabbcc"))
                && names.iter().any(|n| n.ends_with("objects/dd/ddeeff"))
                && names.iter().any(|n| n.ends_with("objects/44/445566"))
        );
        assert!(!names.iter().any(|n| n.ends_with("objects/11/112233")), "only the chosen language");
        assert!(!names.iter().any(|n| n.ends_with("objects/77/778899")));
        let mut dedup = names.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), names.len(), "no file twice");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn nothing_happens_when_turned_off_or_busy() {
        let dirs = Dirs::at(scratch("off"));
        let mut settings = Settings::default();
        settings.extras.insert("warmUpGameFiles".into(), serde_json::Value::Bool(false));
        let root = dirs.instances.join("T");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("instance.json"), "{}").unwrap();
        let instance = crate::instances::load_one(&root).unwrap();
        warm_soon(&dirs, &settings, &instance, None, Arc::new(|| false));
        assert!(
            WARMED_AT.lock().unwrap().as_ref().is_none_or(|m| !m.contains_key(&root)),
            "turned off: not even noted"
        );
        settings.extras.insert("warmUpGameFiles".into(), serde_json::Value::Bool(true));
        warm_soon(&dirs, &settings, &instance, None, Arc::new(|| true));
        assert!(WARMED_AT.lock().unwrap().as_ref().is_none_or(|m| !m.contains_key(&root)), "busy: not even noted");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }
}
