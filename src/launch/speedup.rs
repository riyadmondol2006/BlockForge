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
