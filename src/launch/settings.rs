//! The settings that decide how a game starts, as one value: the launcher-wide setting unless the instance has its own.
//! Both come from files the Java edition writes (`BlockForge.json` and `instance.json`), where a lot of the keys are
//! not typed fields in this program. They are read by their Java names from the whole saved JSON, so a key that is
//! later given a typed field keeps working.

use std::path::Path;

use serde_json::{Map, Value};

use crate::config::Settings;
use crate::instances::Instance;
use crate::paths::Dirs;

fn as_map<T: serde::Serialize>(value: &T) -> Map<String, Value> {
    match serde_json::to_value(value) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn bool_of(m: &Map<String, Value>, key: &str) -> Option<bool> {
    m.get(key).and_then(Value::as_bool)
}

fn text_of(m: &Map<String, Value>, key: &str) -> Option<String> {
    m.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A text where nothing at all (empty, spaces) counts as "not set".
fn nonblank(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.trim().is_empty())
}

/// What Minecraft's Quick Play should do when the game starts (`launcher.quickPlay` in `instance.json`). At most one of
/// the three is used: a server, else a world, else a realm.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickPlay {
    pub server_address: Option<String>,
    pub world_name: Option<String>,
    pub realm_id: Option<String>,
}

/// The options Quick Play can be set to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickPlayOption {
    Disabled,
    SinglePlayer,
    MultiPlayer,
    Realm,
}

impl QuickPlayOption {
    pub const ALL: [QuickPlayOption; 4] = [
        QuickPlayOption::Disabled,
        QuickPlayOption::SinglePlayer,
        QuickPlayOption::MultiPlayer,
        QuickPlayOption::Realm,
    ];

    /// The text shown in the instance settings (the English original; pass it through `i18n::tr`).
    pub fn label(self) -> &'static str {
        match self {
            QuickPlayOption::Disabled => "The title screen",
            QuickPlayOption::SinglePlayer => "A single player world",
            QuickPlayOption::MultiPlayer => "A multiplayer server",
            QuickPlayOption::Realm => "A realm",
        }
    }

    /// The game argument that does it in Minecraft 23w14a and newer.
    pub fn argument(self) -> Option<&'static str> {
        match self {
            QuickPlayOption::Disabled => None,
            QuickPlayOption::SinglePlayer => Some("--quickPlaySingleplayer"),
            QuickPlayOption::MultiPlayer => Some("--quickPlayMultiplayer"),
            QuickPlayOption::Realm => Some("--quickPlayRealms"),
        }
    }

    /// Joining a server also works on older versions, through `--server` and `--port`.
    pub fn compatible_on_older_versions(self) -> bool {
        matches!(self, QuickPlayOption::Disabled | QuickPlayOption::MultiPlayer)
    }
}

impl QuickPlay {
    pub fn from_json(v: Option<&Value>) -> QuickPlay {
        let Some(o) = v.and_then(Value::as_object) else { return QuickPlay::default() };
        let get = |k: &str| o.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        QuickPlay { server_address: get("serverAddress"), world_name: get("worldName"), realm_id: get("realmId") }
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "serverAddress": self.server_address,
            "worldName": self.world_name,
            "realmId": self.realm_id,
        })
    }

    /// The option these values stand for.
    pub fn selected(&self) -> QuickPlayOption {
        if self.server_address.is_some() {
            QuickPlayOption::MultiPlayer
        } else if self.world_name.is_some() {
            QuickPlayOption::SinglePlayer
        } else if self.realm_id.is_some() {
            QuickPlayOption::Realm
        } else {
            QuickPlayOption::Disabled
        }
    }
}

/// If this instance's version knows the Quick Play argument for the option: it lists it among its game arguments
/// (Minecraft 23w14a and newer do). Older versions only support joining a server, with `--server`.
pub fn is_quick_play_supported(instance: &Instance, option: QuickPlayOption) -> bool {
    let Some(arg) = option.argument() else { return false };
    let Some(args) = &instance.file.version.arguments else { return false };
    args.game.iter().any(|a| match a {
        crate::model::Arg::Ruled { value, .. } => value.to_vec().iter().any(|v| v == arg),
        crate::model::Arg::Plain(_) => false,
    })
}

/// The options to offer for this instance: the ones its version supports, plus joining a server, which every version
/// can do (Java's `QuickPlayOption.compatibleValues`).
pub fn quick_play_options_for(instance: &Instance) -> Vec<QuickPlayOption> {
    QuickPlayOption::ALL
        .iter()
        .copied()
        .filter(|o| is_quick_play_supported(instance, *o) || o.compatible_on_older_versions())
        .collect()
}

/// The names of the instance's worlds (the folders in `saves`).
pub fn world_names(instance: &Instance) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(instance.root.join("saves")) else { return Vec::new() };
    let mut names: Vec<String> =
        rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// A `javaVersion` requirement a modpack can put in `instance.json` (`launcher.java`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JavaRequirement {
    pub min: u32,
    pub max: u32,
}

impl JavaRequirement {
    pub fn from_json(v: Option<&Value>) -> Option<JavaRequirement> {
        let o = v?.as_object()?;
        let n = |k: &str| o.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        Some(JavaRequirement { min: n("min"), max: n("max") })
    }

    pub fn conforms(&self, java_major: u32) -> bool {
        let mut ok = self.min == 0 || java_major >= self.min;
        if self.max != 0 && java_major > self.max {
            ok = false;
        }
        ok
    }

    /// "Java 17", "Java 8 or 9", "Java 17 minimum", "Nothing newer than Java 8".
    pub fn version_string(&self) -> String {
        if self.min != 0 && self.max != 0 && self.min == self.max {
            return format!("Java {}", self.min);
        }
        if self.min != 0 && self.max != 0 && self.min + 1 == self.max {
            return format!("Java {} or {}", self.min, self.max);
        }
        let mut s = String::new();
        if self.min != 0 {
            s.push_str(&format!("Java {}{}", self.min, if self.max == 0 { " minimum" } else { "" }));
        }
        if self.max != 0 {
            s.push_str(&format!(
                "{}Java {}",
                if !s.is_empty() { " up to, and including, " } else { "Nothing newer than " },
                self.max
            ));
        }
        s
    }
}

/// Everything about starting one instance that the settings decide.
#[derive(Clone, Debug)]
pub struct LaunchSettings {
    pub java_path: Option<String>,
    pub java_arguments: String,
    pub meta_space_mb: u32,
    pub use_java_provided_by_minecraft: bool,
    pub java_runtime_override: Option<String>,
    pub disable_legacy_launching: bool,
    pub enable_commands: bool,
    pub pre_launch_command: Option<String>,
    pub post_exit_command: Option<String>,
    pub wrapper_command: Option<String>,
    pub use_system_glfw: bool,
    pub use_system_openal: bool,
    pub window_width: u32,
    pub window_height: u32,
    pub maximise_minecraft: bool,
    pub enable_feral_gamemode: bool,
    pub class_data_cache: bool,
    pub warm_up_game_files: bool,
    pub auto_game_options: bool,
    pub ignore_java_on_instance_launch: bool,
    pub scan_mods_on_launch: bool,
    pub enable_arm_support: bool,
    pub environment_variables: std::collections::BTreeMap<String, String>,
    pub quick_play: QuickPlay,
    /// The account this instance wants to be played with (`launcher.account`), if any.
    pub account: Option<String>,
    pub java_requirement: Option<JavaRequirement>,
}

impl LaunchSettings {
    /// The launcher-wide settings with the instance's own values laid over them, the way `Optional.ofNullable(
    /// launcher.x).orElse(settings.x)` does in the Java edition.
    pub fn resolve(settings: &Settings, instance: &Instance) -> LaunchSettings {
        let g = as_map(settings);
        let l = as_map(&instance.file.launcher);
        let pick_bool = |key: &str, default: bool| bool_of(&l, key).or_else(|| bool_of(&g, key)).unwrap_or(default);
        let global_bool = |key: &str, default: bool| bool_of(&g, key).unwrap_or(default);
        let pick_text = |key: &str| text_of(&l, key).or_else(|| text_of(&g, key));
        LaunchSettings {
            java_path: nonblank(pick_text("javaPath")),
            java_arguments: text_of(&l, "javaArguments").or_else(|| text_of(&g, "javaParameters")).unwrap_or_default(),
            meta_space_mb: l
                .get("permGen")
                .and_then(Value::as_u64)
                .or_else(|| g.get("metaspace").and_then(Value::as_u64))
                .unwrap_or(256) as u32,
            use_java_provided_by_minecraft: pick_bool("useJavaProvidedByMinecraft", true),
            java_runtime_override: nonblank(text_of(&l, "javaRuntimeOverride")),
            disable_legacy_launching: pick_bool("disableLegacyLaunching", false),
            enable_commands: pick_bool("enableCommands", false),
            pre_launch_command: nonblank(pick_text("preLaunchCommand")),
            post_exit_command: nonblank(pick_text("postExitCommand")),
            wrapper_command: nonblank(pick_text("wrapperCommand")),
            use_system_glfw: pick_bool("useSystemGlfw", false),
            use_system_openal: pick_bool("useSystemOpenAl", false),
            window_width: g.get("windowWidth").and_then(Value::as_u64).unwrap_or(854) as u32,
            window_height: g.get("windowHeight").and_then(Value::as_u64).unwrap_or(480) as u32,
            maximise_minecraft: global_bool("maximiseMinecraft", false),
            enable_feral_gamemode: global_bool("enableFeralGamemode", false),
            class_data_cache: global_bool("classDataCache", true),
            warm_up_game_files: global_bool("warmUpGameFiles", true),
            auto_game_options: global_bool("autoGameOptions", true),
            ignore_java_on_instance_launch: global_bool("ignoreJavaOnInstanceLaunch", false),
            scan_mods_on_launch: global_bool("scanModsOnLaunch", true),
            enable_arm_support: global_bool("enableArmSupport", true),
            environment_variables: settings.environment_variables.clone(),
            quick_play: QuickPlay::from_json(l.get("quickPlay")),
            account: nonblank(text_of(&l, "account")),
            java_requirement: JavaRequirement::from_json(l.get("java")),
        }
    }

    /// If the automatic memory setting is on for the launcher as a whole.
    pub fn launcher_automatic_memory(settings: &Settings) -> bool {
        settings.automatic_memory != Some(false)
    }
}

/// Look up a value in the launcher's downloaded settings (`configs/json/config.json`, the same file the Java edition
/// keeps), by a dotted key such as `useLwjglReplacement`. None when the file or the key is not there.
pub fn remote_config(dirs: &Dirs, key: &str) -> Option<Value> {
    let bytes = std::fs::read(dirs.configs.join("json").join("config.json")).ok()?;
    let root: Value = serde_json::from_slice(&bytes).ok()?;
    key.split('.').try_fold(&root, |v, part| v.get(part)).cloned()
}

pub fn remote_config_bool(dirs: &Dirs, key: &str, default: bool) -> bool {
    remote_config(dirs, key).and_then(|v| v.as_bool()).unwrap_or(default)
}

/// The size of the main screen, for "maximise Minecraft", worked out without a window toolkit: the preferred mode of
/// a connected screen on Linux, and the system's own tools elsewhere. None when it can't be found.
pub fn screen_size() -> Option<(u32, u32)> {
    use crate::platform::{Os, os};
    match os() {
        Os::Linux => linux_screen_size(),
        Os::Windows => {
            let out = crate::platform::command("powershell")
                .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \"$($b.Width)x$($b.Height)\""])
                .output()
                .ok()?;
            parse_size(String::from_utf8_lossy(&out.stdout).trim())
        }
        Os::Mac => {
            let out = crate::platform::command("system_profiler").arg("SPDisplaysDataType").output().ok()?;
            String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().strip_prefix("Resolution:")).find_map(
                |r| {
                    let mut nums = r.split_whitespace().filter_map(|w| w.parse::<u32>().ok());
                    Some((nums.next()?, nums.next()?))
                },
            )
        }
    }
}

fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once('x')?;
    let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

fn linux_screen_size() -> Option<(u32, u32)> {
    // the kernel lists each connector's modes, preferred first; this works under X11 and Wayland alike
    let mut best: Option<(u32, u32)> = None;
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        for e in rd.flatten() {
            let p = e.path();
            if std::fs::read_to_string(p.join("status")).map(|s| s.trim() == "connected").unwrap_or(false) {
                if let Some(size) =
                    std::fs::read_to_string(p.join("modes")).ok().and_then(|m| m.lines().next().and_then(parse_size))
                {
                    // the biggest connected screen is the one a game is played on
                    if best.is_none_or(|b| (size.0 as u64 * size.1 as u64) > (b.0 as u64 * b.1 as u64)) {
                        best = Some(size);
                    }
                }
            }
        }
    }
    best.or_else(|| {
        let out = crate::platform::command("xrandr").arg("--current").output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
            let l = l.strip_prefix("Screen ")?;
            let part = l.split("current ").nth(1)?.split(',').next()?.replace(' ', "");
            parse_size(&part)
        })
    })
}

/// Where Java's `release` file says the architecture is.
pub fn java_arch(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join("release")).ok()?;
    let line = text.lines().find(|l| l.starts_with("OS_ARCH="))?;
    Some(line["OS_ARCH=".len()..].replace('"', "").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instances::{Instance, InstanceFile};

    fn instance(launcher_json: &str) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher = serde_json::from_str(launcher_json).unwrap();
        Instance { folder: "x".into(), mod_count: 0, root: "/nonexistent".into(), file }
    }

    #[test]
    fn instance_values_win_over_the_launcher() {
        let mut s = Settings::default();
        s.wrapper_command = Some("gamemoderun %command%".into());
        s.extras.insert("enableCommands".into(), true.into());
        s.extras.insert("preLaunchCommand".into(), "echo global".into());
        s.extras.insert("useSystemGlfw".into(), true.into());
        let i = instance(
            r#"{"name":"A","preLaunchCommand":"echo mine","useSystemGlfw":false,"useJavaProvidedByMinecraft":false,
            "javaRuntimeOverride":"java-runtime-gamma","permGen":512,"enableCommands":false,"java":{"min":17,"max":21}}"#,
        );
        let r = LaunchSettings::resolve(&s, &i);
        assert!(!r.enable_commands, "the instance turns commands off");
        assert_eq!(r.pre_launch_command.as_deref(), Some("echo mine"));
        assert_eq!(r.wrapper_command.as_deref(), Some("gamemoderun %command%"));
        assert!(!r.use_system_glfw);
        assert!(!r.use_java_provided_by_minecraft);
        assert_eq!(r.java_runtime_override.as_deref(), Some("java-runtime-gamma"));
        assert_eq!(r.meta_space_mb, 512);
        assert_eq!(r.java_requirement, Some(JavaRequirement { min: 17, max: 21 }));
        let plain = LaunchSettings::resolve(&s, &instance(r#"{"name":"B"}"#));
        assert!(plain.enable_commands && plain.use_system_glfw && plain.use_java_provided_by_minecraft);
        assert_eq!(plain.meta_space_mb, 256);
        assert!(
            plain.class_data_cache && plain.warm_up_game_files && plain.auto_game_options && plain.scan_mods_on_launch
        );
        assert!(!plain.disable_legacy_launching && !plain.ignore_java_on_instance_launch);
    }

    #[test]
    fn quick_play_round_trips_java_json() {
        let q = QuickPlay::from_json(Some(
            &serde_json::json!({"serverAddress":"play.example.com:25566","worldName":null,"realmId":null}),
        ));
        assert_eq!(q.selected(), QuickPlayOption::MultiPlayer);
        assert_eq!(QuickPlay::from_json(Some(&q.to_json())), q);
        assert_eq!(QuickPlay::default().selected(), QuickPlayOption::Disabled);
        let w = QuickPlay { world_name: Some("New World".into()), ..Default::default() };
        assert_eq!(w.selected(), QuickPlayOption::SinglePlayer);
        assert_eq!(QuickPlayOption::Realm.argument(), Some("--quickPlayRealms"));
        assert!(QuickPlay::from_json(None).server_address.is_none());
    }

    #[test]
    fn quick_play_support_comes_from_the_version_arguments() {
        let mut i = instance(r#"{"name":"A"}"#);
        assert!(!is_quick_play_supported(&i, QuickPlayOption::SinglePlayer));
        assert_eq!(quick_play_options_for(&i), vec![QuickPlayOption::Disabled, QuickPlayOption::MultiPlayer]);
        i.file.version = serde_json::from_str(
            r#"{"arguments":{"game":["--username","${auth_player_name}",
            {"rules":[{"action":"allow","features":{"is_quick_play_singleplayer":true}}],"value":["--quickPlaySingleplayer","${quickPlaySingleplayer}"]}],"jvm":[]}}"#,
        )
        .unwrap();
        assert!(is_quick_play_supported(&i, QuickPlayOption::SinglePlayer));
        assert!(!is_quick_play_supported(&i, QuickPlayOption::Realm));
        assert_eq!(quick_play_options_for(&i).len(), 3);
    }

    #[test]
    fn java_requirements() {
        let r = JavaRequirement { min: 17, max: 0 };
        assert!(r.conforms(17) && r.conforms(21) && !r.conforms(8));
        assert_eq!(r.version_string(), "Java 17 minimum");
        let r = JavaRequirement { min: 8, max: 8 };
        assert!(r.conforms(8) && !r.conforms(17));
        assert_eq!(r.version_string(), "Java 8");
        assert_eq!(JavaRequirement { min: 8, max: 9 }.version_string(), "Java 8 or 9");
        assert_eq!(JavaRequirement { min: 0, max: 8 }.version_string(), "Nothing newer than Java 8");
        assert_eq!(JavaRequirement { min: 8, max: 17 }.version_string(), "Java 8 up to, and including, Java 17");
        assert!(JavaRequirement::default().conforms(25));
    }

    #[test]
    fn sizes_parse() {
        assert_eq!(parse_size("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_size(" 2560x1440 "), Some((2560, 1440)));
        assert_eq!(parse_size("0x0"), None);
        assert_eq!(parse_size("nonsense"), None);
    }
}
