//! Speed-up: the switch beside Play. It groups the changes that were measured to make Minecraft: Java Edition open
//! sooner, use less memory or stutter less, each of which can also be switched off by itself. Horizon and Bedrock have
//! no Java game to change, so it never applies to them.
//!
//! What it holds, with what was measured (Minecraft 26.3, 16 processor threads, GTX 1650, every chunk generated again):
//!
//! * fewer world-generation threads (`max.bg.threads`), so they don't compete with the frame being drawn;
//! * less memory: compact object headers on Java 25, and giving unused native memory back to the system on Linux;
//! * on Java 25 the class cache becomes Java's AOT cache (`classcache`), which also holds how the classes were linked.
//!
//! A player's own Java arguments win: with them, none of the Java options here are added (`command`).

use crate::config::Settings;
use crate::instances::Instance;

/// The first Java with the AOT cache and compact object headers as regular features.
pub const MODERN_JAVA: u32 = 25;

/// The instance's own switch for Speed-up, if it has one.
pub fn instance_override(instance: &Instance) -> Option<bool> {
    instance.file.launcher.extras.get("speedup").and_then(|v| v.as_bool())
}

/// Whether Speed-up is on for this instance: its own switch, otherwise the launcher-wide setting. Horizon is never.
pub fn enabled(settings: &Settings, instance: &Instance) -> bool {
    !crate::horizon::is_horizon(instance) && instance_override(instance).unwrap_or(settings.speedup)
}

/// Which collector a game gets: `Auto` picks by this computer's memory and processors (see `collector`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GcChoice {
    Auto,
    On,
    Off,
}

impl GcChoice {
    pub fn parse(text: &str) -> GcChoice {
        match text.trim().to_ascii_lowercase().as_str() {
            "on" => GcChoice::On,
            "off" => GcChoice::Off,
            _ => GcChoice::Auto,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            GcChoice::Auto => "auto",
            GcChoice::On => "on",
            GcChoice::Off => "off",
        }
    }
}

fn own_bool(instance: &Instance, key: &str) -> Option<bool> {
    instance.file.launcher.extras.get(key).and_then(|v| v.as_bool())
}

/// The instance's own switch for the world-generation threads, if it has one.
pub fn threads_override(instance: &Instance) -> Option<bool> {
    own_bool(instance, "speedupThreads")
}

/// The instance's own switch for using less memory, if it has one.
pub fn memory_override(instance: &Instance) -> Option<bool> {
    own_bool(instance, "speedupMemory")
}

/// The instance's own collector choice, if it has one.
pub fn gc_override(instance: &Instance) -> Option<GcChoice> {
    instance.file.launcher.extras.get("speedupGc").and_then(|v| v.as_str()).map(GcChoice::parse)
}

/// Fewer world-generation threads for this instance: under Speed-up, its own switch over the launcher's.
pub fn threads_on(settings: &Settings, instance: &Instance) -> bool {
    enabled(settings, instance) && threads_override(instance).unwrap_or(settings.speedup_threads)
}

/// Less memory for this instance: under Speed-up, its own switch over the launcher's.
pub fn memory_on(settings: &Settings, instance: &Instance) -> bool {
    enabled(settings, instance) && memory_override(instance).unwrap_or(settings.speedup_memory)
}

/// The collector choice for this instance: under Speed-up, its own over the launcher's; without Speed-up the usual one.
pub fn gc_choice(settings: &Settings, instance: &Instance) -> GcChoice {
    if !enabled(settings, instance) {
        return GcChoice::Off;
    }
    gc_override(instance).unwrap_or_else(|| GcChoice::parse(&settings.speedup_gc))
}

/// Whether this instance runs with the player's own Java arguments, which then decide everything here.
pub fn own_arguments(settings: &Settings, instance: &Instance) -> bool {
    let ls = crate::launch::settings::LaunchSettings::resolve(settings, instance);
    crate::launch::jvmargs::is_custom(Some(&ls.java_arguments))
}

/// The Java major version this instance's game asks for, when its version says so.
pub fn java_major(instance: &Instance) -> Option<u32> {
    instance.file.version.java_version.as_ref().map(|j| j.major_version)
}

/// The simulation distance in the instance's `options.txt`, when it has one.
pub fn simulation_distance(instance: &Instance) -> Option<u32> {
    let text = std::fs::read_to_string(instance.root.join("options.txt")).ok()?;
    text.lines().find_map(|l| l.strip_prefix("simulationDistance:"))?.trim().parse().ok()
}

/// Above this, the simulation distance is worth a word: the game simulates (2d+1)^2 chunks, so 32 is 4225 and 12 is 625.
pub const HIGH_SIMULATION_DISTANCE: u32 = 16;
/// What the suggestion lowers it to.
pub const SUGGESTED_SIMULATION_DISTANCE: u32 = 12;

/// Sets the simulation distance in the instance's `options.txt`, only when the player asks for it. Every other line
/// stays as it is, with its own line ending. The game reads the file at its next start.
pub fn set_simulation_distance(instance: &Instance, distance: u32) -> std::io::Result<()> {
    let file = instance.root.join("options.txt");
    let text = std::fs::read_to_string(&file)?;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if line.starts_with("simulationDistance:") {
            let ending = &line[line.trim_end_matches(['\r', '\n']).len()..];
            out.push_str(&format!("simulationDistance:{distance}{ending}"));
        } else {
            out.push_str(line);
        }
    }
    std::fs::write(&file, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_choice_reads_like_it_is_written() {
        for choice in [GcChoice::Auto, GcChoice::On, GcChoice::Off] {
            assert_eq!(GcChoice::parse(choice.as_setting()), choice);
        }
        assert_eq!(GcChoice::parse(" ON "), GcChoice::On);
        assert_eq!(GcChoice::parse("something else"), GcChoice::Auto);
    }
}

/// On small CPUs the worker-priority helper already gives the frame thread room.
pub fn thread_cap(processors: usize) -> Option<usize> {
    (processors > 4).then(|| processors.div_ceil(2).max(2))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collector {
    G1,
    Zgc,
}

/// Auto stays G1 until measurements justify a different default.
/// `available_ram_mb` is the memory left after reserving this start's chosen heap.
pub fn collector(
    choice: GcChoice,
    java_major: u32,
    total_ram_mb: i64,
    available_ram_mb: i64,
    _cores: usize,
) -> Collector {
    match choice {
        GcChoice::Auto => Collector::G1,
        GcChoice::On if java_major >= 21 && total_ram_mb > 1408 && available_ram_mb >= 1408 => Collector::Zgc,
        _ => Collector::G1,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    OwnArguments,
    OldJava,
    C2me,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheState {
    NotMade,
    Making,
    Ready,
}

/// Only the saved state and metadata are read; no cache building or Java process runs on the window thread.
pub fn cache_state(dirs: &crate::paths::Dirs, instance: &Instance) -> CacheState {
    crate::launch::classcache::cache_state(
        dirs,
        &crate::launch::warmer::safe_name(instance),
        java_major(instance).unwrap_or(0),
    )
}

pub fn skip_reason(settings: &Settings, instance: &Instance) -> Option<SkipReason> {
    if own_arguments(settings, instance) {
        Some(SkipReason::OwnArguments)
    } else if java_major(instance).is_some_and(|m| m < MODERN_JAVA) {
        Some(SkipReason::OldJava)
    } else if has_c2me(instance) {
        Some(SkipReason::C2me)
    } else {
        None
    }
}

pub fn has_c2me(instance: &Instance) -> bool {
    type ModCache = std::collections::HashMap<std::path::PathBuf, (Option<std::time::SystemTime>, bool)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<ModCache>> = std::sync::OnceLock::new();
    let stamp = std::fs::metadata(instance.root.join("mods")).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, result)) = cache.get(&instance.root).filter(|(previous, _)| *previous == stamp) {
        return *result;
    }
    let result = super::lite::mod_list(&instance.root).iter().any(|id| {
        let id = id.to_ascii_lowercase();
        id == "c2me" || id.starts_with("c2me-") || id.starts_with("c2me_")
    });
    cache.insert(instance.root.clone(), (stamp, result));
    result
}

pub fn unsigned_jar_on(settings: &Settings, instance: &Instance) -> bool {
    enabled(settings, instance) && settings.speedup_unsigned_jar
}

/// Runs during launch preparation, never on the window thread. Vendor backports differ, so ask this exact Java.
pub fn accepts_trim(java: &std::path::Path) -> bool {
    crate::platform::command(java)
        .args(["-XX:+UnlockExperimentalVMOptions", "-XX:TrimNativeHeapInterval=5000", "-version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod decision_tests {
    use super::*;
    #[test]
    fn caps_leave_small_cpus_alone_and_round_up() {
        for n in 0..=4 {
            assert_eq!(thread_cap(n), None);
        }
        assert_eq!(thread_cap(5), Some(3));
        assert_eq!(thread_cap(16), Some(8));
        assert_eq!(thread_cap(17), Some(9));
    }
    #[test]
    fn zgc_is_explicit_modern_and_requires_memory() {
        for major in [8, 17, 21, 25] {
            assert_eq!(collector(GcChoice::Auto, major, 32000, 24000, 16), Collector::G1);
            assert_eq!(collector(GcChoice::Off, major, 32000, 24000, 16), Collector::G1);
        }
        assert_eq!(collector(GcChoice::On, 17, 32000, 24000, 16), Collector::G1);
        assert_eq!(collector(GcChoice::On, 21, 32000, 24000, 16), Collector::Zgc);
        assert_eq!(collector(GcChoice::On, 25, 32000, 1000, 16), Collector::G1);
        assert!(!Settings::default().speedup_unsigned_jar);
    }
}
