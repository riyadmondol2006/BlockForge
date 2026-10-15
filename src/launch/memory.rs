//! How much memory the game gets, and what kind of computer this is. A port of the Java edition's `AutoMemory`,
//! `GameMemory`, `InstanceMemory` and `PcProfile`, with the same numbers and the same warnings.
//!
//! The idea: work out what the game wants (Minecraft version, mod loader, number of mods, shaders, big resource packs,
//! what the modpack asks for), then make sure that never takes memory the rest of the computer needs. More heap than the
//! game uses doesn't make it faster: the garbage collector has more to walk through, pauses get longer, and memory
//! taken from the system pushes the graphics driver, browser and the system itself into swap, which is far slower.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Settings;
use crate::instances::Instance;
use crate::paths::Dirs;

// ---------------------------------------------------------------------------------------------------------------
// AutoMemory: the game's memory when the player has not set it

/// 32-bit Java can rarely reserve more than about 1.4 GB in one piece, and fails to start if asked for more.
pub const JAVA_32_BIT_CAP_MB: u32 = 1408;
/// Below this Minecraft doesn't get to the title screen.
pub const FLOOR_MB: u32 = 512;
/// The most the launcher gives on its own (unless the modpack asks for more). Past 8 GB the collector has more
/// regions to scan and its pauses get longer.
pub const SOFT_CAP_MB: u32 = 8192;
/// With one or two cores the collector has little help, so pauses grow with the heap; keep it smaller there.
pub const FEW_CORES_CAP_MB: u32 = 4096;
/// What the system always keeps for itself, at the least.
pub const MIN_OS_RESERVE_MB: i64 = 1536;
/// The most kept back for the system on computers with lots of memory.
pub const MAX_OS_RESERVE_MB: i64 = 4096;
/// Memory the game's Java uses besides the heap: loaded classes, compiled code, threads, graphics buffers.
pub const NON_HEAP_MB: i64 = 512;

/// The mod loader, as far as memory is concerned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemLoader {
    None,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
    Other,
}

impl MemLoader {
    /// From the loader type stored with an instance ("Fabric", "NeoForge", "LegacyFabric", ...); nothing is none.
    pub fn of(kind: Option<&str>) -> MemLoader {
        match kind.map(|k| k.to_ascii_lowercase()).as_deref() {
            None | Some("") => MemLoader::None,
            Some("fabric") | Some("legacyfabric") => MemLoader::Fabric,
            Some("quilt") => MemLoader::Quilt,
            Some("forge") => MemLoader::Forge,
            Some("neoforge") => MemLoader::NeoForge,
            Some(_) => MemLoader::Other,
        }
    }

    fn fabric_like(self) -> bool {
        matches!(self, MemLoader::Fabric | MemLoader::Quilt)
    }

    fn forge_like(self) -> bool {
        matches!(self, MemLoader::Forge | MemLoader::NeoForge | MemLoader::Other)
    }
}

/// The computer at the moment Play is pressed. Unknown numbers are 0 or less.
#[derive(Clone, Copy, Debug)]
pub struct Pc {
    pub total_mb: i64,
    pub available_mb: i64,
    pub cores: i64,
    pub java_32_bit: bool,
}

/// The game being started.
#[derive(Clone, Debug)]
pub struct MemGame {
    pub minecraft_version: Option<String>,
    pub loader: MemLoader,
    pub mod_count: u32,
    pub shaders: bool,
    pub resource_packs_mb: u64,
    pub pack_recommended_mb: u32,
    pub pack_minimum_mb: u32,
}

impl MemGame {
    /// A plain game of this version: no mods, no packs.
    pub fn vanilla(version: Option<&str>) -> MemGame {
        MemGame {
            minecraft_version: version.map(str::to_string),
            loader: MemLoader::None,
            mod_count: 0,
            shaders: false,
            resource_packs_mb: 0,
            pack_recommended_mb: 0,
            pack_minimum_mb: 0,
        }
    }
}

/// What was picked, and why it may be less than the game would like.
#[derive(Clone, Debug)]
pub struct AutoResult {
    /// The -Xmx to use.
    pub heap_mb: u32,
    /// What the game would like, before looking at the computer.
    pub ideal_mb: u32,
    /// The most this computer can spare at all.
    pub ceiling_mb: u32,
    /// Lowered because little memory was free when Play was pressed.
    pub limited_by_free_memory: bool,
    /// Less than the modpack says it needs.
    pub below_pack_needs: bool,
    pub warnings: Vec<String>,
}

impl AutoResult {
    /// If the game gets noticeably less than it would like, so the player should be told.
    pub fn is_short(&self) -> bool {
        self.below_pack_needs || (self.heap_mb as i64) < self.ideal_mb as i64 - 256
    }
}

fn round_up(mb: u32) -> u32 {
    mb.div_ceil(256) * 256
}

fn round_down(mb: i64) -> u32 {
    (mb.max(0) / 256 * 256).min(u32::MAX as i64) as u32
}

/// The "1.x" part of a Minecraft version, or 100 for the year based versions (26.1 and newer), so they count as
/// newer than any 1.x. Weekly snapshots ("24w14a") are mapped to the release they led to; very old versions (alpha,
/// beta, classic) are 0.
pub fn minor_version(version: Option<&str>) -> u32 {
    let Some(version) = version else { return 100 };
    let v = version.trim().to_lowercase();
    if let Some(rest) = v.strip_prefix("1.") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        return digits.parse().unwrap_or(0);
    }
    let b = v.as_bytes();
    if b.len() >= 5
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2] == b'w'
        && b[3].is_ascii_digit()
        && b[4].is_ascii_digit()
    {
        let year: i32 = v[..2].parse().unwrap_or(0);
        let by_year = [10, 10, 10, 10, 10, 10, 12, 13, 15, 16, 17, 19, 20, 21, 21]; // 2011 .. 2025
        let index = year - 11;
        if index < 0 {
            return 0;
        }
        return by_year.get(index as usize).copied().unwrap_or(100);
    }
    let dot = v.find('.').filter(|&i| i > 0);
    let dash = v.find('-').filter(|&i| i > 0);
    let end = dot.or(dash).unwrap_or(v.len());
    if end > 0 && v[..end].chars().all(|c| c.is_ascii_digit()) {
        let major: u32 = v[..end].parse().unwrap_or(0);
        return if major >= 25 { 100 } else { 0 };
    }
    // alpha, beta, classic, infdev and other odd names
    if v.starts_with('a') || v.starts_with('b') || v.starts_with('c') || v.starts_with("inf") || v.starts_with("rd") {
        0
    } else {
        100
    }
}

/// What the game would like, before looking at what the computer can spare. Rounded to 256 MB.
pub fn ideal_mb(game: &MemGame) -> u32 {
    let minor = minor_version(game.minecraft_version.as_deref());
    // the base game: tiny worlds up to 1.6, 256-high worlds to 1.12, the flattening and new lighting engine to
    // 1.17, and 384-high worlds with a longer view from 1.18
    let mut mb: u32 = if minor <= 6 {
        1024
    } else if minor <= 12 {
        1536
    } else if minor <= 17 {
        2048
    } else {
        2560
    };
    // Fabric and Quilt load mods lazily and their mods are small; Forge and NeoForge load every mod's classes and
    // registries up front, so big packs need 4-8 GB, growing with the number of mods
    if game.loader.fabric_like() {
        mb += 256 + game.mod_count * 12;
    } else if game.loader.forge_like() {
        mb += 768 + game.mod_count * if minor <= 12 { 16 } else { 20 };
    }
    // the shader loader (Iris, Oculus, OptiFine) and the pack's own textures and buffers
    if game.shaders {
        mb += 512;
    }
    // high resolution resource packs are unpacked into atlases at 4 bytes a pixel
    if game.resource_packs_mb >= 256 {
        mb += 1024;
    } else if game.resource_packs_mb >= 64 {
        mb += 512;
    }
    mb = mb.min(SOFT_CAP_MB);
    // the modpack knows itself best; what it asks for is used even past the usual cap
    mb = mb.max(game.pack_recommended_mb.max(game.pack_minimum_mb));
    round_up(mb)
}

/// The least the game should get when memory is tight: about three quarters of what it would like (that is mostly
/// headroom for the collector), never under what the modpack says it needs.
fn needed_mb(game: &MemGame, ideal: u32) -> u32 {
    let base = if minor_version(game.minecraft_version.as_deref()) <= 12 { 768 } else { 1024 };
    game.pack_minimum_mb.max(base.max(round_down((ideal * 3 / 4) as i64)))
}

/// The most the computer can spare for the game at all: everything but what the system needs, which is a quarter of
/// the memory, at least 1.5 GB (the system, the graphics driver and a browser) and at most 4 GB.
pub fn ceiling_mb(pc: &Pc) -> u32 {
    let mut cap = i32::MAX as u32;
    if pc.total_mb > 0 {
        let reserve = MIN_OS_RESERVE_MB.max(MAX_OS_RESERVE_MB.min(pc.total_mb / 4));
        cap = FLOOR_MB.max(round_down((pc.total_mb - reserve).min(i32::MAX as i64)));
    }
    if pc.java_32_bit {
        cap = cap.min(JAVA_32_BIT_CAP_MB);
    }
    cap
}

/// Picks the memory for this game on this computer.
pub fn choose(pc: &Pc, game: &MemGame) -> AutoResult {
    let mut warnings = Vec::new();
    let ideal = ideal_mb(game);
    let ceiling = ceiling_mb(pc);
    let mut heap = ideal.min(ceiling);

    // one or two cores: only when the pack doesn't need more
    if pc.cores > 0 && pc.cores <= 2 && heap > FEW_CORES_CAP_MB && game.pack_minimum_mb <= FEW_CORES_CAP_MB {
        heap = FEW_CORES_CAP_MB;
    }
    // memory not known at all: stay modest
    if pc.total_mb <= 0 {
        heap = heap.min(UNKNOWN_DEFAULT_MB.max(game.pack_minimum_mb));
    }

    // little free right now: give less, but not less than the game needs; swapping a little is better than running
    // out of memory, which crashes the game
    let mut limited_by_free = false;
    let needed = needed_mb(game, ideal).min(heap);
    if pc.available_mb > 0 {
        let spare = round_down((pc.available_mb - NON_HEAP_MB).min(i32::MAX as i64));
        if spare < heap {
            let lowered = needed.max(spare);
            if lowered < heap {
                heap = lowered;
                limited_by_free = true;
            }
        }
    }

    // never under what the game needs to reach the title screen at all, even on a tiny computer: there it will
    // swap, but with less it would just run out of memory and crash
    let start_minimum = if minor_version(game.minecraft_version.as_deref()) <= 12 { 768 } else { 1024 };
    heap = heap.max(start_minimum.min(if pc.java_32_bit { JAVA_32_BIT_CAP_MB } else { u32::MAX }));
    heap = heap.max(FLOOR_MB);

    if limited_by_free {
        warnings.push(format!(
            "Only {} MB of memory is free right now, so the game gets {} MB instead of the {} MB it should have. \
             Closing other programs (web browsers especially) lets it have more next time.",
            pc.available_mb, heap, ideal
        ));
    }
    let below_pack = game.pack_minimum_mb > 0 && heap < game.pack_minimum_mb;
    if below_pack {
        warnings.push(format!(
            "This modpack needs at least {} MB of memory, but this computer can only spare {} MB. It may run slowly or run out of memory.",
            game.pack_minimum_mb, heap
        ));
    } else if !limited_by_free && (heap as i64) < ideal as i64 - 256 {
        warnings.push(format!(
            "This game would like about {ideal} MB of memory, but this computer can only spare {heap} MB. It may run \
             slowly; fewer mods or a lower render distance will help."
        ));
    }
    AutoResult {
        heap_mb: heap,
        ideal_mb: ideal,
        ceiling_mb: ceiling,
        limited_by_free_memory: limited_by_free,
        below_pack_needs: below_pack,
        warnings,
    }
}

/// "3 GB", "2.5 GB", "768 MB".
pub fn describe_mb(mb: u32) -> String {
    if mb < 1024 {
        return format!("{mb} MB");
    }
    if mb % 1024 == 0 {
        return format!("{} GB", mb / 1024);
    }
    // to the nearest quarter: "2.5 GB", "1.75 GB"
    let gb = (mb as f64 / 256.0).round() / 4.0;
    let text = format!("{gb:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} GB")
}

/// A few words for the kind of game: "a modern vanilla game", "a modern Fabric game with 5 mods", ...
pub fn describe_game(game: &MemGame) -> String {
    let age = if minor_version(game.minecraft_version.as_deref()) <= 12 { "an older" } else { "a modern" };
    let loader = match game.loader {
        MemLoader::Fabric => "Fabric",
        MemLoader::Quilt => "Quilt",
        MemLoader::Forge => "Forge",
        MemLoader::NeoForge => "NeoForge",
        MemLoader::Other => "modded",
        MemLoader::None => return format!("{age} vanilla game"),
    };
    format!("{age} {loader} game with {} mod{}", game.mod_count, if game.mod_count == 1 { "" } else { "s" })
}

// ---------------------------------------------------------------------------------------------------------------
// GameMemory: the player's own setting, and the defaults

/// The most the launcher picks as a default maximum; Minecraft and most modpacks run well with 4 GB.
pub const DEFAULT_CAP_MB: u32 = 4096;
/// What 32-bit Java can use at most, reliably.
pub const MAX_32_BIT_MB: u32 = 1024;
/// Below this the game doesn't start properly at all.
pub const MINIMUM_MB: u32 = 512;
/// What to use when the computer's memory can't be found out.
pub const UNKNOWN_DEFAULT_MB: u32 = 2048;

/// The default amount of memory for the game on a computer with this much memory: half of it, rounded down to 256
/// MB and at most 4 GB, so 2 GB gives 1 GB, 4 GB gives 2 GB and 8 GB or more gives 4 GB.
pub fn default_maximum_mb(physical_mb: i64, is_64_bit: bool) -> u32 {
    if physical_mb <= 0 {
        return if is_64_bit { UNKNOWN_DEFAULT_MB } else { MAX_32_BIT_MB };
    }
    safe_maximum_mb(physical_mb, is_64_bit).min(DEFAULT_CAP_MB)
}

/// The most memory the game can have without starving the rest of the computer: half of it, rounded down to 256 MB
/// (1 GB at most for 32-bit Java), or no limit if the amount of memory isn't known.
pub fn safe_maximum_mb(physical_mb: i64, is_64_bit: bool) -> u32 {
    if physical_mb <= 0 {
        return if is_64_bit { i32::MAX as u32 } else { MAX_32_BIT_MB };
    }
    let half = MINIMUM_MB.max(((physical_mb / 2) / 256 * 256).min(i32::MAX as i64) as u32);
    if is_64_bit { half } else { half.min(MAX_32_BIT_MB) }
}

/// How much memory the game gets and anything the person should know about it.
#[derive(Clone, Debug)]
pub struct MemoryPlan {
    pub heap_mb: u32,
    pub warnings: Vec<String>,
}

/// Works out the game's -Xmx from the player's own setting.
///
/// * `configured_mb` the memory set in the instance or in the settings (0 for none)
/// * `recommended_mb` what the modpack says it needs, or 0 if it doesn't say
/// * `physical_mb` the computer's memory, or 0 if unknown
pub fn plan(configured_mb: u32, recommended_mb: u32, physical_mb: i64, is_64_bit: bool) -> MemoryPlan {
    let safe = safe_maximum_mb(physical_mb, is_64_bit);
    let mut heap = if configured_mb > 0 { configured_mb } else { default_maximum_mb(physical_mb, is_64_bit) };
    let mut warnings = Vec::new();
    // a modpack that needs more than set gets more, as far as the computer can spare it
    if recommended_mb > heap {
        let raised = heap.max(recommended_mb.min(safe));
        if recommended_mb > safe {
            warnings.push(format!(
                "This modpack recommends {recommended_mb} MB of memory, but this computer ({physical_mb} MB) can only spare about {safe} MB, so the game gets {raised} MB. It may run slowly or run out of memory."
            ));
        }
        heap = raised;
    }
    // what the person set themselves is respected, but they should know when it's more than is good
    if physical_mb > 0 && heap > safe {
        warnings.push(format!(
            "The game is set to use {heap} MB of memory, which is more than half of this computer's {physical_mb} MB. That leaves little for everything else and can make the whole computer slow; about {safe} MB is recommended."
        ));
    }
    MemoryPlan { heap_mb: heap, warnings }
}

/// For settings saved before automatic memory existed: if the memory there was ever chosen by the player. The launcher
/// used to write its own default into the settings, so a value equal to one of its old defaults (4 GB in ATLauncher,
/// half the memory up to 4 GB in early BlockForge) was never picked by anyone and becomes automatic; anything else is
/// the player's choice and stays exactly as it is. Returns true when automatic memory should be used.
pub fn was_never_chosen(first_time_run: bool, maximum_memory: u32, physical_mb: i64, is_64_bit: bool) -> bool {
    first_time_run || maximum_memory == 4096 || maximum_memory == default_maximum_mb(physical_mb, is_64_bit)
}

/// If a game uses automatic memory: an instance set to automatic does, an instance with its own amount doesn't, and
/// an instance that follows the launcher does what the launcher setting says.
pub fn uses_automatic(instance_mb: Option<u32>, instance_automatic: Option<bool>, launcher_automatic: bool) -> bool {
    if instance_automatic == Some(true) {
        return true;
    }
    if instance_mb.is_some() {
        return false;
    }
    launcher_automatic
}

/// A warning when the game is given more memory than is free right now.
pub fn availability_warning(heap_mb: u32, available_mb: i64) -> Option<String> {
    if available_mb <= 0 || heap_mb as i64 <= available_mb {
        return None;
    }
    Some(format!(
        "The game may use up to {heap_mb} MB of memory, but only {available_mb} MB is free right now. Closing other \
         programs (web browsers especially) will help it run smoothly."
    ))
}

/// If the computer is low on memory, so the launcher should give back what it can while the game runs.
pub fn is_low_memory_computer(physical_mb: i64) -> bool {
    physical_mb > 0 && physical_mb <= 4096 + 512
}

// ---------------------------------------------------------------------------------------------------------------
// InstanceMemory: looking at an instance and this computer

/// The memory for one launch.
#[derive(Clone, Debug)]
pub struct MemoryDecision {
    pub automatic: bool,
    pub heap_mb: u32,
    pub warnings: Vec<String>,
    /// The automatic choice, also when the player's own setting is used (for showing what it would be).
    pub auto: AutoResult,
    pub game: MemGame,
}

impl MemoryDecision {
    /// If the player should be told on the Play screen that the game is short of memory.
    pub fn is_short(&self) -> bool {
        self.automatic && self.auto.is_short()
    }
}

/// If this instance uses automatic memory.
pub fn is_automatic(instance: &Instance, settings: &Settings) -> bool {
    uses_automatic(
        instance.file.launcher.maximum_memory,
        instance.file.launcher.automatic_memory,
        settings.automatic_memory != Some(false),
    )
}

/// Decides the memory for starting this instance with the Java in `java_home`. Reads a few folders.
pub fn decide(instance: &Instance, settings: &Settings, java_home: Option<&Path>) -> MemoryDecision {
    let game = game_of(instance);
    let pc = pc_now(java_home);
    let auto = choose(&pc, &game);
    if is_automatic(instance, settings) {
        return MemoryDecision { automatic: true, heap_mb: auto.heap_mb, warnings: auto.warnings.clone(), auto, game };
    }
    let configured = instance.file.launcher.maximum_memory.unwrap_or(settings.maximum_memory);
    let plan = plan(configured, game.pack_minimum_mb, pc.total_mb.max(0), !pc.java_32_bit);
    MemoryDecision { automatic: false, heap_mb: plan.heap_mb, warnings: plan.warnings, auto, game }
}

/// The automatic memory for showing in the settings, such as "about 3 GB for a modern Fabric game with 6 mods": for
/// the given instance, or a plain modern game when there's none. Free memory isn't looked at, as it changes all the
/// time.
pub fn describe_automatic(instance: Option<&Instance>, java_home: Option<&Path>) -> String {
    let (game, java32) = match instance {
        Some(i) => (game_of(i), is_32_bit_java(java_home)),
        None => (MemGame::vanilla(None), is_32_bit_java(None)),
    };
    let pc = Pc { total_mb: total_ram_mb(), available_mb: -1, cores: cores() as i64, java_32_bit: java32 };
    let result = choose(&pc, &game);
    format!("about {} for {}", describe_mb(result.heap_mb), describe_game(&game))
}

/// What an instance is, as far as memory goes.
pub fn game_of(instance: &Instance) -> MemGame {
    let l = &instance.file.launcher;
    let loader = MemLoader::of(l.loader_version.as_ref().map(|v| v.kind.as_str()));
    // CurseForge packs say how much memory they want in their manifest
    let manifest = l.extras.get("curseForgeManifest").or_else(|| l.extras.get("curseManifest"));
    let recommended = manifest
        .and_then(|m| m.get("minecraft"))
        .and_then(|m| m.get("recommendedRam"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    MemGame {
        minecraft_version: Some(instance.mc_version().to_string()),
        loader,
        mod_count: count_mods(&instance.root.join("mods")),
        shaders: has_any(&instance.root.join("shaderpacks")),
        resource_packs_mb: size_mb(&instance.root.join("resourcepacks")),
        pack_recommended_mb: recommended,
        pack_minimum_mb: l.required_memory.max(0) as u32,
    }
}

/// This computer right now.
pub fn pc_now(java_home: Option<&Path>) -> Pc {
    Pc {
        total_mb: total_ram_mb(),
        available_mb: available_mb(),
        cores: cores() as i64,
        java_32_bit: is_32_bit_java(java_home),
    }
}

pub fn cores() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// Total memory in MB, or -1 when it can't be told.
pub fn total_ram_mb() -> i64 {
    let (total, _) = crate::platform::memory_mb();
    if total == 0 { -1 } else { total as i64 }
}

/// Mod jars in the mods folder (disabled ones don't count).
pub fn count_mods(mods: &Path) -> u32 {
    let Ok(rd) = std::fs::read_dir(mods) else { return 0 };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            (n.ends_with(".jar") || n.ends_with(".zip")) && e.path().is_file()
        })
        .count() as u32
}

/// If the folder has any shader pack (a zip or a folder).
pub fn has_any(folder: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(folder) else { return false };
    rd.flatten().any(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".zip") || e.path().is_dir())
}

/// Total size of the files directly in a folder, in MB.
pub fn size_mb(folder: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(folder) else { return 0 };
    rd.flatten().filter(|e| e.path().is_file()).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum::<u64>()
        / (1024 * 1024)
}

/// If the Java in this folder is a 32-bit one, from its "release" file. When it can't be told, this program's own
/// bitness is the best guess.
pub fn is_32_bit_java(java_home: Option<&Path>) -> bool {
    if let Some(home) = java_home {
        if let Ok(text) = std::fs::read_to_string(home.join("release")) {
            if let Some(line) = text.lines().find(|l| l.starts_with("OS_ARCH=")) {
                return is_32_bit_arch(line["OS_ARCH=".len()..].replace('"', "").trim());
            }
        }
    }
    cfg!(target_pointer_width = "32")
}

pub fn is_32_bit_arch(arch: &str) -> bool {
    let a = arch.to_lowercase();
    if a.contains("64") {
        return false;
    }
    matches!(a.as_str(), "x86" | "i386" | "i586" | "i686") || a.starts_with("arm")
}

/// Memory free for new programs right now, in MB, or -1 if unknown. Counts memory the system would give up from its
/// file cache, which is what matters for starting a game.
pub fn available_mb() -> i64 {
    if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("MemAvailable:") {
                if let Some(kb) = rest.split_whitespace().next().and_then(|n| n.parse::<i64>().ok()) {
                    return kb / 1024;
                }
            }
        }
    }
    let (total, free) = crate::platform::memory_mb();
    if total == 0 { -1 } else { free as i64 }
}

// ---------------------------------------------------------------------------------------------------------------
// PcProfile: what kind of computer this is

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Tier {
    #[serde(rename = "LOW")]
    Low,
    #[serde(rename = "MEDIUM")]
    Medium,
    #[serde(rename = "HIGH")]
    High,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Low => "low",
            Tier::Medium => "medium",
            Tier::High => "high",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gpu {
    Integrated,
    Dedicated,
    Unknown,
}

impl Gpu {
    fn name(self) -> &'static str {
        match self {
            Gpu::Integrated => "INTEGRATED",
            Gpu::Dedicated => "DEDICATED",
            Gpu::Unknown => "UNKNOWN",
        }
    }

    fn parse(s: &str) -> Gpu {
        match s {
            "INTEGRATED" => Gpu::Integrated,
            "DEDICATED" => Gpu::Dedicated,
            _ => Gpu::Unknown,
        }
    }
}

/// The graphics card is looked up again after this long, in case it changed.
const REFRESH_AFTER_DAYS: i64 = 30;

#[derive(Clone, Debug)]
pub struct PcProfile {
    pub total_mb: i64,
    pub cores: usize,
    pub gpu: Gpu,
    pub gpu_name: Option<String>,
    pub tier: Tier,
}

impl PcProfile {
    pub fn new(total_mb: i64, cores: usize, gpu: Gpu, gpu_name: Option<String>) -> PcProfile {
        PcProfile { total_mb, cores, gpu, gpu_name, tier: tier_for(total_mb, cores as i64, gpu) }
    }

    /// The profile from memory and cores alone, graphics unknown. Never slow.
    pub fn peek() -> PcProfile {
        PcProfile::new(total_ram_mb(), cores(), Gpu::Unknown, None)
    }

    /// The profile with the graphics card looked up (slow the first time, then kept in the cache folder until the
    /// memory or processor count changes or a month passes).
    pub fn get(dirs: &Dirs) -> PcProfile {
        let total_mb = total_ram_mb();
        let cores = cores();
        let file = dirs.cache.join("pc-profile.json");
        if let Some(saved) = read_saved(&file) {
            if is_still_valid(&saved, total_mb, cores, chrono::Utc::now()) {
                return PcProfile::new(total_mb, cores, Gpu::parse(saved.gpu.as_deref().unwrap_or("")), saved.gpu_name);
            }
        }
        let (gpu, name) = detect_gpu();
        let saved = Saved {
            total_mb,
            cores: cores as i64,
            gpu: Some(gpu.name().to_string()),
            gpu_name: name.clone(),
            checked_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&file, serde_json::to_vec(&saved).unwrap_or_default());
        PcProfile::new(total_mb, cores, gpu, name)
    }
}

/// What is kept in the cache folder (the same file the Java edition writes).
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    total_mb: i64,
    cores: i64,
    gpu: Option<String>,
    gpu_name: Option<String>,
    checked_at: Option<String>,
}

fn read_saved(file: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

fn is_still_valid(saved: &Saved, total_mb: i64, cores: usize, now: chrono::DateTime<chrono::Utc>) -> bool {
    if saved.gpu.is_none() {
        return false;
    }
    let Some(at) = saved.checked_at.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) else {
        return false;
    };
    // a memory upgrade or a different processor: look again (allow a little wobble in the reported memory)
    if (saved.total_mb - total_mb).abs() > 256 || saved.cores != cores as i64 {
        return false;
    }
    (now - at.with_timezone(&chrono::Utc)).num_days() < REFRESH_AFTER_DAYS
}

/// Words of a graphics card's name the way a regex `\b` sees them.
fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect()
}

/// Sorts a graphics card into integrated or dedicated from its vendor and name, as the system reports them.
/// Integrated: Intel's UHD, HD, Iris and Arc-less mobile parts, AMD's APUs (which call themselves just "Radeon
/// Graphics" or "Radeon Vega"), Apple's M chips and the virtual adapters of virtual machines. Dedicated: NVIDIA, AMD
/// Radeon RX/Pro and Intel Arc.
pub fn classify_gpu(vendor: Option<&str>, name: Option<&str>) -> Gpu {
    let v = vendor.unwrap_or("").to_lowercase();
    let n = name.unwrap_or("").to_lowercase();
    let all = format!("{v} {n}");
    if all.trim().is_empty() {
        return Gpu::Unknown;
    }
    // virtual machines and remote desktops: software rendering or a thin layer over it
    if ["vmware", "virtualbox", "llvmpipe", "microsoft basic", "virtio", "qxl", "cirrus", "hyper-v", "parallels"]
        .iter()
        .any(|k| all.contains(k))
    {
        return Gpu::Integrated;
    }
    if ["nvidia", "geforce", "quadro", "rtx", "gtx"].iter().any(|k| all.contains(k)) {
        return Gpu::Dedicated;
    }
    let tokens = words(&n);
    if all.contains("intel") {
        // Intel Arc A/B series cards are dedicated; "Arc Graphics" with no model number is the integrated GPU of Core
        // Ultra processors
        if let Some(arc) = tokens.iter().position(|t| *t == "arc") {
            let model = tokens[arc + 1..].iter().any(|t| {
                let b = t.as_bytes();
                b.len() >= 4 && (b[0] == b'a' || b[0] == b'b') && b[1..4].iter().all(u8::is_ascii_digit)
            });
            if model {
                return Gpu::Dedicated;
            }
        }
        return Gpu::Integrated;
    }
    let apple_chip =
        tokens.iter().any(|t| t.len() >= 2 && t.starts_with('m') && t[1..].bytes().all(|b| b.is_ascii_digit()));
    if all.contains("apple") || apple_chip {
        return Gpu::Integrated;
    }
    if all.contains("amd") || all.contains("ati ") || all.contains("advanced micro devices") || all.contains("radeon") {
        // the APUs report "AMD Radeon(TM) Graphics", "Radeon Vega 8 Graphics" or a 6xxM/7xxM/8xxM model
        let mobile_model = tokens.iter().any(|t| {
            let b = t.as_bytes();
            b.len() == 4 && b[3] == b'm' && matches!(b[0], b'6' | b'7' | b'8') && b[1].is_ascii_digit() && b[2] == b'0'
        });
        let apu_names = [
            "renoir",
            "cezanne",
            "rembrandt",
            "phoenix",
            "raphael",
            "barcelo",
            "lucienne",
            "picasso",
            "raven",
            "van gogh",
            "mendocino",
            "strix",
        ];
        if n.contains("radeon(tm) graphics")
            || n == "amd radeon graphics"
            || n.contains("radeon graphics")
            || (n.contains("vega") && !n.contains("vega 56") && !n.contains("vega 64"))
            || mobile_model
            || apu_names.iter().any(|k| n.contains(k))
        {
            return Gpu::Integrated;
        }
        return Gpu::Dedicated;
    }
    Gpu::Unknown
}

/// Picks the dedicated card if there is one, as that's what games run on.
pub fn best_of(cards: &[Gpu]) -> Gpu {
    if cards.contains(&Gpu::Dedicated) {
        Gpu::Dedicated
    } else if cards.contains(&Gpu::Integrated) {
        Gpu::Integrated
    } else {
        Gpu::Unknown
    }
}

/// How capable the computer is.
///
/// * LOW: 4 GB of memory or less, two cores or fewer, or integrated graphics with 8 GB or less and four cores or fewer
///   (a typical budget laptop). Minecraft's defaults (12 chunks, fancy) run poorly on all of these.
/// * HIGH: a dedicated graphics card with 16 GB or more and at least six cores.
/// * MEDIUM: everything in between, including integrated graphics on otherwise good computers and any computer whose
///   graphics couldn't be identified.
pub fn tier_for(total_mb: i64, cores: i64, gpu: Gpu) -> Tier {
    let low_memory = total_mb > 0 && total_mb <= 4608;
    let few_cores = cores > 0 && cores <= 2;
    if low_memory || few_cores {
        return Tier::Low;
    }
    if gpu == Gpu::Integrated && total_mb > 0 && total_mb <= 8704 && cores <= 4 {
        return Tier::Low;
    }
    if gpu == Gpu::Dedicated && total_mb >= 15360 && cores >= 6 {
        return Tier::High;
    }
    Tier::Medium
}

/// How many files to download at once by default. Downloads are many small files, so the number in flight matters more
/// than bandwidth; but each finished file is also hashed and written, and on a computer with two cores or 4 GB of
/// memory 16 at once only makes the launcher stutter, so those get 8.
pub fn download_threads(total_mb: i64, cores: i64) -> u32 {
    if (total_mb > 0 && total_mb <= 4608) || (cores > 0 && cores <= 2) { 8 } else { 16 }
}

/// Finds the graphics card by asking the system (slow, which is why the answer is cached).
fn detect_gpu() -> (Gpu, Option<String>) {
    let cards = list_graphics_cards();
    let mut kinds = Vec::new();
    let mut name: Option<String> = None;
    for (vendor, card) in &cards {
        let kind = classify_gpu(vendor.as_deref(), Some(card));
        kinds.push(kind);
        if name.is_none() || kind == Gpu::Dedicated {
            name = Some(card.clone());
        }
    }
    (best_of(&kinds), name)
}

fn run_text(program: &str, args: &[&str]) -> Option<String> {
    let out = crate::platform::command(program).args(args).stdin(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// (vendor, name) of each graphics card.
fn list_graphics_cards() -> Vec<(Option<String>, String)> {
    use crate::platform::{Os, os};
    let mut cards = Vec::new();
    match os() {
        Os::Linux => {
            if let Some(text) = run_text("lspci", &["-mm"]) {
                cards.extend(parse_lspci(&text));
            }
            if cards.is_empty() {
                // no lspci: the kernel's own list has the vendor and device ids
                if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
                    for e in rd.flatten() {
                        let n = e.file_name().to_string_lossy().to_string();
                        if !(n.starts_with("card") && n[4..].bytes().all(|b| b.is_ascii_digit())) {
                            continue;
                        }
                        let vendor = std::fs::read_to_string(e.path().join("device/vendor")).unwrap_or_default();
                        let name = match vendor.trim() {
                            "0x10de" => "NVIDIA",
                            "0x1002" => "AMD Radeon",
                            "0x8086" => "Intel",
                            _ => continue,
                        };
                        cards.push((Some(name.to_string()), name.to_string()));
                    }
                }
            }
        }
        Os::Windows => {
            if let Some(text) =
                run_text("powershell", &["-NoProfile", "-Command", "(Get-CimInstance Win32_VideoController).Name"])
            {
                cards.extend(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| (None, l.to_string())));
            }
        }
        Os::Mac => {
            if let Some(text) = run_text("system_profiler", &["SPDisplaysDataType"]) {
                cards.extend(
                    text.lines()
                        .filter_map(|l| l.trim().strip_prefix("Chipset Model:"))
                        .map(|n| (None, n.trim().to_string())),
                );
            }
        }
    }
    cards
}

/// `lspci -mm` lines for display controllers: `00:02.0 "VGA compatible controller" "Intel Corporation" "UHD Graphics" ...`.
fn parse_lspci(text: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('"').collect();
        // fields: [slot, class, _, vendor, _, device, ...]
        if fields.len() < 6 {
            continue;
        }
        let class = fields[1];
        if class.contains("VGA") || class.contains("3D controller") || class.contains("Display controller") {
            out.push((Some(fields[3].to_string()), fields[5].to_string()));
        }
    }
    out
}

/// Where the profile is cached (for tests and tools).
pub fn profile_cache_file(dirs: &Dirs) -> PathBuf {
    dirs.cache.join("pc-profile.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(total: i64, avail: i64, cores: i64) -> Pc {
        Pc { total_mb: total, available_mb: avail, cores, java_32_bit: false }
    }

    #[test]
    fn minor_versions() {
        let m = |v: &str| minor_version(Some(v));
        assert_eq!(m("1.8.9"), 8);
        assert_eq!(m("1.21.1"), 21);
        assert_eq!(m("1.5"), 5);
        assert_eq!(m("1."), 0);
        assert_eq!(m("24w14a"), 21);
        assert_eq!(m("13w16a"), 10);
        assert_eq!(m("11w49a"), 10);
        assert_eq!(m("26.1"), 100);
        assert_eq!(m("26w03a"), 100);
        assert_eq!(m("b1.7.3"), 0);
        assert_eq!(m("a1.2.6"), 0);
        assert_eq!(m("c0.30_01c"), 0);
        assert_eq!(m("rd-132211"), 0);
        assert_eq!(m("inf-20100618"), 0);
        assert_eq!(m("3D Shareware v1.34"), 100);
        assert_eq!(minor_version(None), 100);
    }

    #[test]
    fn ideal_memory_grows_with_the_game() {
        assert_eq!(ideal_mb(&MemGame::vanilla(Some("1.5.2"))), 1024);
        assert_eq!(ideal_mb(&MemGame::vanilla(Some("1.8.9"))), 1536);
        assert_eq!(ideal_mb(&MemGame::vanilla(Some("1.16.5"))), 2048);
        assert_eq!(ideal_mb(&MemGame::vanilla(Some("1.21.1"))), 2560);
        let mut fabric = MemGame::vanilla(Some("1.21.1"));
        fabric.loader = MemLoader::Fabric;
        fabric.mod_count = 6;
        assert_eq!(ideal_mb(&fabric), 3072); // 2560 + 256 + 72 = 2888 -> 3072
        let mut forge = MemGame::vanilla(Some("1.20.1"));
        forge.loader = MemLoader::Forge;
        forge.mod_count = 200;
        forge.shaders = true;
        forge.resource_packs_mb = 300;
        assert_eq!(ideal_mb(&forge), SOFT_CAP_MB);
        // the pack's own request wins over the cap
        forge.pack_recommended_mb = 10000;
        assert_eq!(ideal_mb(&forge), 10240);
    }

    #[test]
    fn automatic_memory_respects_the_computer() {
        let g = MemGame::vanilla(Some("1.21.1"));
        // plenty of everything
        let r = choose(&pc(16384, 12000, 8), &g);
        assert_eq!(r.heap_mb, 2560);
        assert!(!r.is_short() && r.warnings.is_empty());
        // 4 GB computer: reserve 1.5 GB, ceiling 2560 -> exactly what it wants
        assert_eq!(ceiling_mb(&pc(4096, 0, 4)), 2560);
        // 8 GB computer with the browser open: little free memory lowers the heap but never below what is needed
        let low = choose(&pc(8192, 2000, 8), &g);
        assert_eq!(low.heap_mb, 1792);
        assert!(low.limited_by_free_memory && !low.warnings.is_empty());
        // memory unknown: modest
        let unknown = choose(
            &pc(-1, -1, 4),
            &MemGame { mod_count: 50, loader: MemLoader::Forge, ..MemGame::vanilla(Some("1.20.1")) },
        );
        assert_eq!(unknown.heap_mb, 2048);
        // few cores cap big heaps
        let mut big = MemGame::vanilla(Some("1.20.1"));
        big.loader = MemLoader::Forge;
        big.mod_count = 150;
        assert_eq!(choose(&pc(32768, 30000, 2), &big).heap_mb, 4096);
        // a pack that needs more than the computer has
        let mut pack = MemGame::vanilla(Some("1.20.1"));
        pack.pack_minimum_mb = 8192;
        let r = choose(&pc(8192, 6000, 8), &pack);
        assert!(r.below_pack_needs && r.is_short());
        assert!(r.warnings[0].starts_with("This modpack needs at least 8192 MB"));
        // 32-bit Java cannot go past 1408
        assert_eq!(ceiling_mb(&Pc { java_32_bit: true, ..pc(32768, 0, 8) }), 1408);
        // never under the floor
        assert!(choose(&pc(1500, 300, 2), &g).heap_mb >= 1024);
    }

    #[test]
    fn describing_memory() {
        assert_eq!(describe_mb(768), "768 MB");
        assert_eq!(describe_mb(3072), "3 GB");
        assert_eq!(describe_mb(2560), "2.5 GB");
        assert_eq!(describe_mb(1792), "1.75 GB");
        let mut g = MemGame::vanilla(Some("1.21.1"));
        assert_eq!(describe_game(&g), "a modern vanilla game");
        g.loader = MemLoader::Fabric;
        g.mod_count = 1;
        assert_eq!(describe_game(&g), "a modern Fabric game with 1 mod");
        g.minecraft_version = Some("1.8.9".into());
        g.mod_count = 5;
        assert_eq!(describe_game(&g), "an older Fabric game with 5 mods");
    }

    #[test]
    fn the_players_own_memory_is_respected() {
        // half of 16 GB is 8 GB, so 6 GB is fine
        let p = plan(6144, 0, 16384, true);
        assert_eq!(p.heap_mb, 6144);
        assert!(p.warnings.is_empty());
        // more than half warns but is kept
        let p = plan(12000, 0, 16384, true);
        assert_eq!(p.heap_mb, 12000);
        assert_eq!(p.warnings.len(), 1);
        // a modpack asking for more is given more, as far as the computer can spare
        let p = plan(2048, 6144, 16384, true);
        assert_eq!(p.heap_mb, 6144);
        let p = plan(2048, 12288, 8192, true);
        assert_eq!(p.heap_mb, 4096);
        assert!(p.warnings[0].starts_with("This modpack recommends 12288 MB"));
        assert_eq!(default_maximum_mb(2048, true), 1024);
        assert_eq!(default_maximum_mb(4096, true), 2048);
        assert_eq!(default_maximum_mb(32768, true), 4096);
        assert_eq!(default_maximum_mb(0, true), 2048);
        assert_eq!(default_maximum_mb(0, false), 1024);
        assert_eq!(safe_maximum_mb(32768, false), 1024);
        assert!(was_never_chosen(true, 1234, 8192, true));
        assert!(was_never_chosen(false, 4096, 8192, true));
        assert!(was_never_chosen(false, 2048, 4096, true));
        assert!(!was_never_chosen(false, 3000, 4096, true));
    }

    #[test]
    fn automatic_follows_the_instance_then_the_launcher() {
        assert!(uses_automatic(Some(4096), Some(true), false));
        assert!(!uses_automatic(Some(4096), None, true));
        assert!(uses_automatic(None, None, true));
        assert!(!uses_automatic(None, Some(false), false));
        assert!(availability_warning(4096, 2000).unwrap().contains("only 2000 MB is free"));
        assert!(availability_warning(1000, 2000).is_none());
        assert!(availability_warning(1000, -1).is_none());
        assert!(is_low_memory_computer(4096));
        assert!(!is_low_memory_computer(8192));
    }

    #[test]
    fn graphics_cards_are_sorted() {
        let c = |v: &str, n: &str| classify_gpu(Some(v), Some(n));
        assert_eq!(c("NVIDIA Corporation", "GA104 [GeForce RTX 3070]"), Gpu::Dedicated);
        assert_eq!(c("Intel Corporation", "UHD Graphics 620"), Gpu::Integrated);
        assert_eq!(c("Intel Corporation", "Arc A770"), Gpu::Dedicated);
        assert_eq!(c("Intel Corporation", "Arc Graphics"), Gpu::Integrated);
        assert_eq!(c("Advanced Micro Devices, Inc. [AMD/ATI]", "Navi 21 [Radeon RX 6800 XT]"), Gpu::Dedicated);
        assert_eq!(c("Advanced Micro Devices, Inc. [AMD/ATI]", "Cezanne [Radeon Vega Series]"), Gpu::Integrated);
        assert_eq!(c("AMD", "AMD Radeon(TM) Graphics"), Gpu::Integrated);
        assert_eq!(c("AMD", "Radeon 680M"), Gpu::Integrated);
        assert_eq!(c("Apple", "Apple M2"), Gpu::Integrated);
        assert_eq!(c("VMware", "SVGA II Adapter"), Gpu::Integrated);
        assert_eq!(classify_gpu(None, None), Gpu::Unknown);
        assert_eq!(best_of(&[Gpu::Integrated, Gpu::Dedicated]), Gpu::Dedicated);
        assert_eq!(best_of(&[]), Gpu::Unknown);
    }

    #[test]
    fn tiers() {
        assert_eq!(tier_for(4096, 8, Gpu::Dedicated), Tier::Low);
        assert_eq!(tier_for(16384, 2, Gpu::Dedicated), Tier::Low);
        assert_eq!(tier_for(8192, 4, Gpu::Integrated), Tier::Low);
        assert_eq!(tier_for(16384, 8, Gpu::Integrated), Tier::Medium);
        assert_eq!(tier_for(16384, 8, Gpu::Dedicated), Tier::High);
        assert_eq!(tier_for(16384, 8, Gpu::Unknown), Tier::Medium);
        assert_eq!(download_threads(4096, 8), 8);
        assert_eq!(download_threads(16384, 8), 16);
    }

    #[test]
    fn lspci_output_is_read() {
        let text = "00:02.0 \"VGA compatible controller\" \"Intel Corporation\" \"Raptor Lake-S GT1 [UHD Graphics 770]\" -r04 \"ASUSTeK\" \"x\"\n01:00.0 \"VGA compatible controller\" \"NVIDIA Corporation\" \"GA106 [GeForce RTX 3060]\" -ra1 \"x\" \"y\"\n00:1f.3 \"Audio device\" \"Intel Corporation\" \"Tiger Lake\" -r20 \"a\" \"b\"\n";
        let cards = parse_lspci(text);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[1].1, "GA106 [GeForce RTX 3060]");
    }

    #[test]
    fn saved_profile_expires() {
        let now = chrono::Utc::now();
        let mut s = Saved {
            total_mb: 16000,
            cores: 8,
            gpu: Some("DEDICATED".into()),
            gpu_name: None,
            checked_at: Some(now.to_rfc3339()),
        };
        assert!(is_still_valid(&s, 16100, 8, now));
        assert!(!is_still_valid(&s, 32000, 8, now));
        assert!(!is_still_valid(&s, 16000, 4, now));
        s.checked_at = Some((now - chrono::Duration::days(31)).to_rfc3339());
        assert!(!is_still_valid(&s, 16000, 8, now));
        // the Java edition's own file
        let java: Saved = serde_json::from_str(
            r#"{"totalMb":15769,"cores":16,"gpu":"INTEGRATED","gpuName":"x","checkedAt":"2026-10-15T12:00:00.123Z"}"#,
        )
        .unwrap();
        assert_eq!(java.cores, 16);
    }

    #[test]
    fn arch_is_judged_from_the_release_file() {
        assert!(is_32_bit_arch("x86"));
        assert!(is_32_bit_arch("i386"));
        assert!(is_32_bit_arch("arm"));
        assert!(!is_32_bit_arch("aarch64"));
        assert!(!is_32_bit_arch("amd64"));
        assert!(!is_32_bit_arch("x86_64"));
    }
}
