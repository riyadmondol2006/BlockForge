//! `BlockForge.json`, shared with the Java edition. Only the settings this program uses are named; every other
//! setting is carried along untouched so saving never loses the Java edition's.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
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
    pub auto_game_options: bool,
    pub check_for_launcher_updates: bool,
    pub enable_console: bool,
    pub enable_analytics: bool,
    pub enable_logs: bool,
    /// `None` follows the computer: on for machines with 4 GB of memory or less.
    pub light_mode: Option<bool>,
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
            auto_game_options: true,
            check_for_launcher_updates: true,
            enable_console: false,
            enable_analytics: false,
            enable_logs: false,
            light_mode: None,
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
    pub fn load(dirs: &Dirs) -> Settings {
        std::fs::read(dirs.settings_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dirs: &Dirs) -> Result<()> {
        write_json_atomic(&dirs.settings_file(), self)
    }
}

/// Write through a temporary file so a crash never leaves half a settings file behind.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
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
