//! `BlockForge.json`, shared with the Java edition. Only the settings this program uses are named; every other
//! setting is carried along untouched so saving never loses the Java edition's.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths::Dirs;

pub const DEFAULT_JAVA_ARGS: &str = "-XX:+UnlockExperimentalVMOptions -XX:+UseG1GC -XX:G1NewSizePercent=20 \
-XX:G1ReservePercent=20 -XX:MaxGCPauseMillis=50 -XX:G1HeapRegionSize=32M";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub last_account: Option<String>,
    pub java_path: String,
    pub using_custom_java_path: bool,
    pub java_parameters: String,
    pub maximum_memory: u32,
    /// `None` (a `null` in a Java-written file) means automatic, like `true`.
    pub automatic_memory: Option<bool>,
    pub metaspace: u32,
    pub window_width: u32,
    pub window_height: u32,
    pub maximise_minecraft: bool,
    pub keep_launcher_open: bool,
    pub enable_feral_gamemode: bool,
    /// Which graphics card the game uses on computers with two: `auto`, `integrated`, `dedicated`, or a card's id
    /// (see `gpu`).
    pub gpu: String,
    pub wrapper_command: Option<String>,
    pub environment_variables: BTreeMap<String, String>,
    pub default_mod_platform: String,
    /// No longer used: BlockForge doesn't ask who is playing when it opens. Kept so old settings files load unchanged.
    pub ask_player_on_startup: bool,
    pub language: String,
    pub theme: String,
    pub class_data_cache: bool,
    /// How Java is told to start the game: `auto` picks the quickest start for the number of processors,
    /// `fast` always uses the quick compiler, `full` never changes the compiler (see `launch::jvmargs`).
    pub startup_mode: String,
    /// Start games with BlockForge's fast-start helper (a Java agent, see `launch::boost`). On unless switched off.
    pub fast_start: bool,
    /// Start games with the smoothness helper (a second Java agent, see `launch::lite`). On unless switched off.
    pub lite: bool,
    /// Changes of the smoothness helper to leave out, by id (`ID;ID`).
    pub lite_off: String,
    /// Lower the graphics settings for a slow computer: `auto` (when the computer looks slow), `on` or `off` (see
    /// `launch::lowend`).
    pub low_end_mode: String,
    /// The player closed the hint that suggests Low-end mode.
    pub low_end_hint_dismissed: bool,
    pub auto_game_options: bool,
    pub check_for_launcher_updates: bool,
    pub enable_console: bool,
    pub enable_analytics: bool,
    pub enable_logs: bool,
    /// `None` follows the computer: on for machines with 4 GB of memory or less.
    pub light_mode: Option<bool>,
    /// The size of everything in the window (Ctrl and +/-), kept for the next start. `None` is 100%.
    pub ui_zoom: Option<f32>,
    pub enable_tray_menu: bool,
    pub remember_window_size_position: bool,
    pub launcher_size: Option<Size>,
    pub launcher_position: Option<Pos>,
    pub console_size: Option<Size>,
    pub console_position: Option<Pos>,
    pub date_format: String,
    pub instance_title_format: String,
    pub default_instance_sorting: String,
    pub selected_tab_on_startup: i32,
    pub skipped_launcher_update: Option<String>,
    pub ignore_one_drive_warning: bool,
    pub ignore_program_files_warning: bool,
    pub ignore_java_options_warning: bool,
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

pub const DEFAULT_THEME: &str = "com.blockforge.launcher.themes.Material3Dark";

impl Default for Settings {
    fn default() -> Self {
        Settings {
            last_account: None,
            java_path: String::new(),
            using_custom_java_path: false,
            java_parameters: DEFAULT_JAVA_ARGS.to_string(),
            maximum_memory: 4096,
            automatic_memory: Some(true),
            metaspace: 256,
            window_width: 854,
            window_height: 480,
            maximise_minecraft: false,
            keep_launcher_open: true,
            enable_feral_gamemode: false,
            gpu: "auto".to_string(),
            wrapper_command: None,
            environment_variables: BTreeMap::new(),
            default_mod_platform: "Modrinth".to_string(),
            ask_player_on_startup: false,
            language: "English".to_string(),
            theme: DEFAULT_THEME.to_string(),
            class_data_cache: true,
            startup_mode: "auto".to_string(),
            fast_start: true,
            lite: true,
            lite_off: String::new(),
            low_end_mode: "auto".to_string(),
            low_end_hint_dismissed: false,
            auto_game_options: true,
            check_for_launcher_updates: true,
            enable_console: false,
            enable_analytics: false,
            enable_logs: false,
            light_mode: None,
            ui_zoom: None,
            enable_tray_menu: true,
            remember_window_size_position: false,
            launcher_size: None,
            launcher_position: None,
            console_size: None,
            console_position: None,
            date_format: "dd/MM/yyyy".to_string(),
            instance_title_format: "%1$s (%2$s %3$s)".to_string(),
            default_instance_sorting: "BY_NAME".to_string(),
            selected_tab_on_startup: 0,
            skipped_launcher_update: None,
            ignore_one_drive_warning: false,
            ignore_program_files_warning: false,
            ignore_java_options_warning: false,
            extras: Map::new(),
        }
    }
}

impl Settings {
    /// The settings file, or the defaults when there is none. A setting with a value of the wrong kind (a `null`
    /// the Java edition wrote, a number typed as text by hand) falls back to its default on its own instead of
    /// taking every other setting with it, and a file that can't be read at all is kept aside as
    /// `BlockForge.json.broken-<time>` before the defaults are used, so saving never destroys it.
    pub fn load(dirs: &Dirs) -> Settings {
        load_json_lenient(&dirs.settings_file()).unwrap_or_default()
    }

    pub fn save(&self, dirs: &Dirs) -> Result<()> {
        write_json_atomic(&dirs.settings_file(), self)
    }
}

/// Read a JSON object file into `T` as far as it can be. `None` when there is no file (or no object in it); a file
/// that isn't JSON is renamed out of the way first. Fields that don't fit `T` are left at their defaults, and the
/// file as it was is copied to `<name>.broken-<time>` so nothing the player had is lost when it is saved again.
pub fn load_json_lenient<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice::<Value>(strip_bom(&bytes)) {
        Ok(Value::Object(map)) => {
            let (value, dropped) = lenient_object::<T>(map);
            if !dropped.is_empty() {
                crate::logs::warn(format!(
                    "{}: these settings had values that couldn't be used and went back to their defaults: {}",
                    path.display(),
                    dropped.join(", ")
                ));
                keep_broken_copy(path, false);
            }
            value
        }
        Ok(_) | Err(_) => {
            crate::logs::warn(format!("{} could not be read; starting from the defaults", path.display()));
            keep_broken_copy(path, true);
            None
        }
    }
}

/// `T` from a JSON object, keeping every field that deserialises and dropping the ones that don't. Returns the
/// value (`None` only when even an empty object doesn't make a `T`) and the names of the dropped fields. Meant for
/// types whose fields all have defaults (`#[serde(default)]`).
pub fn lenient_object<T: DeserializeOwned>(map: Map<String, Value>) -> (Option<T>, Vec<String>) {
    if let Ok(v) = serde_json::from_value::<T>(Value::Object(map.clone())) {
        return (Some(v), Vec::new());
    }
    let mut kept = Map::new();
    let mut dropped = Vec::new();
    for (key, value) in map {
        kept.insert(key.clone(), value);
        if serde_json::from_value::<T>(Value::Object(kept.clone())).is_err() {
            kept.remove(&key);
            dropped.push(key);
        }
    }
    (serde_json::from_value::<T>(Value::Object(kept)).ok(), dropped)
}

/// A UTF-8 byte order mark, which Windows editors such as Notepad put at the start of a file and JSON doesn't allow.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

/// Copy (or with `move_away`, rename) a file that couldn't be read fully to `<name>.broken-<time>` next to it.
pub fn keep_broken_copy(path: &Path, move_away: bool) -> Option<std::path::PathBuf> {
    // a file read several times before it is saved again is copied once per run, not once per read
    static KEPT: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());
    if !move_away {
        let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
        if kept.iter().any(|p| p == path) {
            return None;
        }
        kept.push(path.to_path_buf());
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = path.file_name()?.to_string_lossy().to_string();
    let target = path.with_file_name(format!("{name}.broken-{stamp}"));
    if target.exists() {
        return Some(target);
    }
    let done = if move_away { std::fs::rename(path, &target) } else { std::fs::copy(path, &target).map(|_| ()) };
    match done {
        Ok(()) => {
            crate::logs::warn(format!("Kept the unreadable file as {}", target.display()));
            Some(target)
        }
        Err(e) => {
            crate::logs::warn(format!("Couldn't keep a copy of {}: {e}", path.display()));
            None
        }
    }
}

/// Write through a temporary file so a crash never leaves half a settings file behind.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?, false)
}

/// Write `bytes` to `path` so that the file is either the old one or the whole new one, even when the program or
/// the computer stops half way: the bytes go to a temporary file of their own (never shared by two saves at the
/// same time), are flushed to the disk, and the temporary file then takes the old one's place. With `private`, the
/// file can only be read by its owner from the start (it holds sign-in tokens).
pub fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp =
        path.with_file_name(format!(".{name}.{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    #[cfg(not(unix))]
    let _ = private;
    let written = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::from(e).context(format!("writing {}", path.display())));
    }
    if let Err(e) = rename_replacing(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::from(e).context(format!("saving {}", path.display())));
    }
    Ok(())
}

/// `rename`, tried a few times: on Windows a virus scanner or the search indexer briefly holding the old file open
/// makes it fail with "access denied" for a moment.
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if cfg!(windows) && tries < 5 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50 * tries));
            }
            Err(e) => return Err(e),
        }
    }
}

/// How much memory a game gets when the player leaves it on automatic: what it would like, as far as this
/// computer can spare it.
pub fn auto_memory_mb(mod_count: usize, total_mb: u64, free_mb: u64) -> u32 {
    let ideal: u64 = match mod_count {
        0 => 2048,
        1..=30 => 4096,
        31..=100 => 6144,
        _ => 8192,
    };
    if total_mb == 0 {
        return ideal.min(4096) as u32;
    }
    // never more than 70% of the memory the computer has, nor more than what is free right now plus a margin
    let ceiling = (total_mb * 7 / 10).min(free_mb.saturating_add(1024).max(2048));
    ideal.min(ceiling).max(1024) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_settings_survive_a_round_trip() {
        let s: Settings = serde_json::from_str(r#"{"maximumMemory":2048,"someJavaOnlySetting":{"a":1}}"#).unwrap();
        assert_eq!(s.maximum_memory, 2048);
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["someJavaOnlySetting"]["a"], 1);
    }

    #[test]
    fn memory_stays_inside_what_the_computer_has() {
        assert_eq!(auto_memory_mb(0, 16000, 12000), 2048);
        assert_eq!(auto_memory_mb(200, 8000, 3000), 4024); // 70% of 8000 = 5600, free+1024 = 4024
        assert!(auto_memory_mb(200, 4000, 500) >= 1024);
    }
}

#[cfg(test)]
mod null_tests {
    use super::*;

    #[test]
    fn a_null_automatic_memory_from_java_still_loads() {
        let s: Settings = serde_json::from_str(r#"{"automaticMemory":null,"maximumMemory":3072}"#).unwrap();
        assert_eq!(s.automatic_memory, None);
        assert_eq!(s.maximum_memory, 3072);
    }
}

/// What players do to the settings file in real life: edit it by hand, open it in Notepad, run two copies of the
/// launcher, lose power half way through a save.
#[cfg(test)]
mod robustness_tests {
    use super::*;

    fn temp_dirs(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("bf-config-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base);
        std::fs::create_dir_all(&dirs.configs).unwrap();
        dirs
    }

    fn broken_copies(dirs: &Dirs) -> Vec<String> {
        std::fs::read_dir(&dirs.configs)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".broken-"))
            .collect()
    }

    #[test]
    fn one_bad_value_keeps_every_other_setting() {
        let dirs = temp_dirs("one-bad");
        // memory typed as text, a theme the Java edition wrote as null: the rest must survive
        std::fs::write(
            dirs.settings_file(),
            r#"{"maximumMemory":"8192","theme":null,"windowWidth":1280,"javaParameters":"-Xss2M","javaOnly":{"k":1}}"#,
        )
        .unwrap();
        let s = Settings::load(&dirs);
        assert_eq!(s.window_width, 1280);
        assert_eq!(s.java_parameters, "-Xss2M");
        assert_eq!(s.maximum_memory, Settings::default().maximum_memory, "the bad value falls back");
        assert_eq!(s.theme, DEFAULT_THEME);
        assert_eq!(s.extras["javaOnly"]["k"], 1, "settings of the Java edition are carried along");
        assert_eq!(broken_copies(&dirs).len(), 1, "the file as it was is kept");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn a_file_that_is_not_json_is_kept_aside_not_overwritten() {
        let dirs = temp_dirs("not-json");
        // what a crash in the middle of an old, non-atomic save leaves behind
        std::fs::write(dirs.settings_file(), r#"{"maximumMemory":6144,"windowWid"#).unwrap();
        let s = Settings::load(&dirs);
        assert_eq!(s.maximum_memory, Settings::default().maximum_memory);
        let kept = broken_copies(&dirs);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            std::fs::read_to_string(dirs.configs.join(&kept[0])).unwrap(),
            r#"{"maximumMemory":6144,"windowWid"#
        );
        s.save(&dirs).unwrap();
        assert_eq!(Settings::load(&dirs).maximum_memory, s.maximum_memory);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn notepad_byte_order_marks_and_empty_files_are_fine() {
        let dirs = temp_dirs("bom");
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(br#"{"maximumMemory":3000}"#);
        std::fs::write(dirs.settings_file(), &bytes).unwrap();
        assert_eq!(Settings::load(&dirs).maximum_memory, 3000);
        assert!(broken_copies(&dirs).is_empty());
        std::fs::write(dirs.settings_file(), b"").unwrap();
        assert_eq!(Settings::load(&dirs).maximum_memory, Settings::default().maximum_memory);
        // a JSON array or a bare number instead of an object
        std::fs::write(dirs.settings_file(), b"[1,2,3]").unwrap();
        assert_eq!(Settings::load(&dirs).maximum_memory, Settings::default().maximum_memory);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn negative_and_huge_numbers_do_not_wipe_the_file() {
        let dirs = temp_dirs("numbers");
        std::fs::write(
            dirs.settings_file(),
            r#"{"maximumMemory":-512,"metaspace":99999999999999,"windowHeight":720,"selectedTabOnStartup":2.5}"#,
        )
        .unwrap();
        let s = Settings::load(&dirs);
        assert_eq!(s.window_height, 720);
        assert_eq!(s.maximum_memory, 4096);
        assert_eq!(s.metaspace, 256);
        assert_eq!(s.selected_tab_on_startup, 0);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn saves_from_many_threads_at_once_never_mix() {
        let dirs = temp_dirs("threads");
        std::thread::scope(|scope| {
            for n in 0..16u32 {
                let dirs = &dirs;
                scope.spawn(move || {
                    for _ in 0..20 {
                        let mut s = Settings::default();
                        s.maximum_memory = 1000 + n;
                        s.java_parameters = format!("-Dthread={n} {}", "x".repeat(4000));
                        s.save(dirs).unwrap();
                    }
                });
            }
        });
        // whichever save won, the file is whole and is one save, not two mixed together
        let text = std::fs::read_to_string(dirs.settings_file()).unwrap();
        let s: Settings = serde_json::from_str(&text).unwrap();
        let n = s.maximum_memory - 1000;
        assert!(s.java_parameters.starts_with(&format!("-Dthread={n} ")), "{n}");
        let leftovers: Vec<_> = std::fs::read_dir(&dirs.configs)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left behind");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn a_failed_save_leaves_the_old_file_and_no_temporary_file() {
        let dirs = temp_dirs("fail");
        Settings::default().save(&dirs).unwrap();
        let before = std::fs::read(dirs.settings_file()).unwrap();
        // the target is a folder now: the rename can't replace it
        let target = dirs.configs.join("blocked.json");
        std::fs::create_dir_all(target.join("inside")).unwrap();
        assert!(write_json_atomic(&target, &Settings::default()).is_err());
        assert_eq!(std::fs::read(dirs.settings_file()).unwrap(), before);
        let tmp: Vec<_> = std::fs::read_dir(&dirs.configs)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmp.is_empty());
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_never_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;
        let dirs = temp_dirs("private");
        let path = dirs.configs.join("secret.json");
        write_atomic(&path, b"{}", true).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn auto_memory_never_panics_on_odd_computers() {
        for (mods, total, free) in [(0, 0, 0), (usize::MAX, u64::MAX, u64::MAX), (50, 1, 0), (500, 512, u64::MAX)] {
            let mb = auto_memory_mb(mods, total, free);
            assert!((1024..=8192).contains(&mb), "{mods} {total} {free} -> {mb}");
        }
    }
}
