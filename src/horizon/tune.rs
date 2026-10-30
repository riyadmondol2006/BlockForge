//! Fitting Horizon to this computer the way BlockForge fits the Java game: the graphics card chosen in Settings, and
//! BlockForge's memory system mapped onto what a native game has instead of a Java heap.
//!
//! Memory, three ways:
//! - The game has no heap to size, so the instance's memory budget (its own fixed memory, or with automatic memory
//!   what this computer can spare for a game, `memory::ceiling_mb`) chooses the render distance on its first start
//!   only, and on a small computer the number of chunk workers. What the player sets in the game later is kept.
//! - horizon-prepare runs Java, but gives every Java it starts its own `-Xmx` on the command line (2 GB for each of
//!   its two exporter workers, 512 MB to 1 GB for the one-off exporters), which wins over anything in
//!   `JAVA_TOOL_OPTIONS`. What can be changed is how many run at once: with `HORIZON_PREPARE_ISOLATED` each exporter
//!   runs in a small Java of its own and ends, instead of two 2 GB workers staying up. That is used where the
//!   memory system says the computer can't spare them.
//! - Every start compares the memory free with an estimate of what the game uses at the instance's render distance.

use std::path::Path;

use crate::config::Settings;
use crate::instances::Instance;
use crate::launch::memory;

/// Horizon's own render distance when options.txt has none.
pub const DEFAULT_RENDER_DISTANCE: u32 = 12;

/// The memory an instance's game may use, in MB, and whether it is the automatic choice: the instance's (or the
/// launcher's) fixed memory when automatic memory is off, else what this computer can spare for a game at all.
pub fn memory_budget_mb(settings: &Settings, inst: &Instance) -> (u32, bool) {
    let total = memory::total_ram_mb();
    if memory::is_automatic(inst, settings) {
        let pc = memory::Pc { total_mb: total, available_mb: -1, cores: memory::cores() as i64, java_32_bit: false };
        (memory::ceiling_mb(&pc), true)
    } else {
        let configured = inst.file.launcher.maximum_memory.unwrap_or(settings.maximum_memory);
        (memory::plan(configured, 0, total.max(0), true).heap_mb, false)
    }
}

/// The render distance for a first start with `budget_mb`, or `None` for Horizon's own default.
pub fn render_distance_for(budget_mb: u32) -> Option<u32> {
    match budget_mb {
        0..3072 => Some(6),
        3072..4096 => Some(8),
        4096..6144 => Some(10),
        _ => None,
    }
}

/// The chunk workers for a first start (`workers:` in horizon-graphics.txt): a few on a computer with little memory
/// or few cores, where Horizon's automatic count (every core but one) would leave nothing for the rest; 0, Horizon's
/// automatic count, elsewhere.
pub fn workers_for(cores: usize, low_memory: bool) -> u32 {
    if low_memory || cores <= 4 { (cores / 2).clamp(1, 2) as u32 } else { 0 }
}

/// What the game is estimated to use at render distance `rd`, in MB: 1.5 GB at the title screen, and half a MB more
/// for every chunk column loaded around the player ((2 rd + 1)²): 1.6 GB at 4, 1.8 GB at 12, 2.1 GB at 16, 3.6 GB at
/// 32. Measured with v0.2.97 on a card with its own memory: 0.7 GB at the title, 0.95 GB in a new world at 8, 1.2 GB
/// at 32 still loading; 1.0-1.7 GB at the title with 4 elsewhere. The title figure is the higher one, as a card built
/// into the processor takes its memory from the computer's.
pub fn estimate_mb(rd: u32) -> u32 {
    let side = 2 * rd.min(64) + 1;
    1536 + side * side / 2
}

/// Whether horizon-prepare should run each exporter in a small Java of its own instead of two 2 GB workers: on a
/// computer the memory system counts as low on memory, or when less is free right now than the workers take.
pub fn prepare_isolated(total_mb: i64, available_mb: i64) -> bool {
    memory::is_low_memory_computer(total_mb) || (0..PREPARE_WORKERS_MB).contains(&available_mb)
}

/// What horizon-prepare's two exporter workers and the Java that starts them may take together.
const PREPARE_WORKERS_MB: i64 = 4608;

/// The render distance in the game's options.txt, if it has one.
pub fn render_distance_in(options: &str) -> Option<u32> {
    options.lines().find_map(|l| l.trim().strip_prefix("renderDistance:")?.trim().parse().ok())
}

/// Whether options.txt text has a line for `key`.
fn has_key(options: &str, key: &str) -> bool {
    options.lines().any(|l| l.trim().strip_prefix(key).is_some_and(|rest| rest.starts_with(':')))
}

/// Write the settings for an instance's first start in `root`: the render distance for `budget_mb` into options.txt
/// when it has none yet, and the workers into horizon-graphics.txt when there is none yet. The game fills in
/// everything else itself. Returns what was set, for the log.
pub fn first_start(root: &Path, budget_mb: u32, cores: usize, low_memory: bool) -> Vec<String> {
    let mut notes = Vec::new();
    let options = root.join("options.txt");
    // only a missing file is an empty one: a file that can't be read (or isn't text) is the player's, and is left
    let text = match std::fs::read_to_string(&options) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(String::new()),
        Err(e) => {
            notes.push(format!("Left {} as it is: {e}", options.display()));
            None
        }
    };
    if let Some((text, rd)) = text.filter(|t| render_distance_in(t).is_none()).zip(render_distance_for(budget_mb)) {
        // each setting only when the file has none of its own (the game takes the last of two)
        let mut added = format!("renderDistance:{rd}\n");
        if !has_key(&text, "simulationDistance") {
            added.push_str(&format!("simulationDistance:{}\n", rd.min(12)));
        }
        let mut text = text;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&added);
        match std::fs::write(&options, text) {
            Ok(()) => notes.push(format!("First start: render distance {rd} for {budget_mb} MB of memory")),
            Err(e) => notes.push(format!("Couldn't write {}: {e}", options.display())),
        }
    }
    let graphics = root.join("horizon-graphics.txt");
    let workers = workers_for(cores, low_memory);
    if workers > 0 && !graphics.exists() {
        match std::fs::write(&graphics, format!("workers:{workers}\n")) {
            Ok(()) => notes.push(format!("First start: {workers} chunk workers for a computer with {cores} cores")),
            Err(e) => notes.push(format!("Couldn't write {}: {e}", graphics.display())),
        }
    }
    notes
}

/// A warning when less memory is free than the game is estimated to use at the instance's render distance.
pub fn memory_warning(root: &Path, available_mb: i64) -> Option<String> {
    let rd = std::fs::read_to_string(root.join("options.txt"))
        .ok()
        .and_then(|t| render_distance_in(&t))
        .unwrap_or(DEFAULT_RENDER_DISTANCE);
    memory::availability_warning(estimate_mb(rd), available_mb)
        .map(|w| format!("{w} A lower render distance (now {rd}) needs less."))
}

/// The variables that make the game use the graphics card chosen in Settings (Linux), and what to log. The card
/// is looked for only when one was chosen; the player's own variables in `env` win.
#[cfg(target_os = "linux")]
pub async fn gpu_env(choice: &crate::gpu::Choice, env: &mut Vec<(String, String)>, notes: &mut Vec<String>) {
    if *choice == crate::gpu::Choice::Auto {
        return;
    }
    let gpus = tokio::task::spawn_blocking(crate::gpu::detect).await.unwrap_or_default();
    let (vars, said) = gpu_effect(choice, &gpus);
    notes.extend(said);
    for (k, v) in vars {
        if !env.iter().any(|(have, _)| *have == k) {
            env.push((k, v));
        }
    }
}

/// [`gpu_env`]'s decision on the cards found: the variables that leave the game only the chosen card in Vulkan, and
/// the notes.
pub fn gpu_effect(choice: &crate::gpu::Choice, gpus: &[crate::gpu::Gpu]) -> (Vec<(String, String)>, Vec<String>) {
    let effect = crate::gpu::effect(choice, gpus);
    let mut notes: Vec<String> = effect.note.into_iter().collect();
    match crate::gpu::resolve(choice, gpus) {
        Some(card) => {
            notes.push(format!("Graphics card: {}", card.name));
            (crate::gpu::vulkan_env(card, gpus), notes)
        }
        None => (Vec::new(), notes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{Choice, SysCard, from_sysfs};

    #[test]
    fn the_memory_budget_picks_the_first_render_distance() {
        assert_eq!(render_distance_for(1024), Some(6));
        assert_eq!(render_distance_for(2560), Some(6));
        assert_eq!(render_distance_for(3072), Some(8));
        assert_eq!(render_distance_for(4095), Some(8));
        assert_eq!(render_distance_for(4096), Some(10));
        assert_eq!(render_distance_for(6143), Some(10));
        assert_eq!(render_distance_for(6144), None);
        assert_eq!(render_distance_for(24_000), None);
    }

    #[test]
    fn small_computers_get_a_few_workers_and_the_rest_horizons_own_count() {
        assert_eq!(workers_for(2, false), 1);
        assert_eq!(workers_for(4, false), 2);
        assert_eq!(workers_for(16, true), 2);
        assert_eq!(workers_for(1, true), 1);
        assert_eq!(workers_for(8, false), 0);
    }

    #[test]
    fn the_estimate_grows_with_the_render_distance() {
        assert_eq!(estimate_mb(4), 1536 + 40);
        assert_eq!(estimate_mb(12), 1536 + 312);
        assert!(estimate_mb(32) > estimate_mb(16));
        // a nonsense distance doesn't overflow
        assert_eq!(estimate_mb(u32::MAX), estimate_mb(64));
    }

    #[test]
    fn preparing_runs_one_java_at_a_time_when_memory_is_short() {
        assert!(prepare_isolated(4096, -1));
        assert!(prepare_isolated(16_384, 3000));
        assert!(!prepare_isolated(16_384, 9000));
        // nothing known: the workers, as Horizon does it
        assert!(!prepare_isolated(-1, -1));
    }

    #[test]
    fn the_first_start_sets_what_isnt_set_and_never_changes_the_players_choice() {
        let root = crate::testutil::scratch("hz-first-start");
        let notes = first_start(&root, 2560, 4, false);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert_eq!(
            std::fs::read_to_string(root.join("options.txt")).unwrap(),
            "renderDistance:6\nsimulationDistance:6\n"
        );
        assert_eq!(std::fs::read_to_string(root.join("horizon-graphics.txt")).unwrap(), "workers:2\n");
        // the player raised it in the game: a later start leaves it
        std::fs::write(root.join("options.txt"), "fov:0.0\nrenderDistance:16\nsimulationDistance:12\n").unwrap();
        assert!(first_start(&root, 1024, 2, true).is_empty());
        assert_eq!(render_distance_in(&std::fs::read_to_string(root.join("options.txt")).unwrap()), Some(16));
        assert_eq!(std::fs::read_to_string(root.join("horizon-graphics.txt")).unwrap(), "workers:2\n");

        // options.txt without a render distance gets one added; a big budget and many cores change nothing
        let other = crate::testutil::scratch("hz-first-start-2");
        std::fs::write(other.join("options.txt"), "lang:fr_fr").unwrap();
        first_start(&other, 3500, 8, false);
        let text = std::fs::read_to_string(other.join("options.txt")).unwrap();
        assert_eq!(text, "lang:fr_fr\nrenderDistance:8\nsimulationDistance:8\n");
        let big = crate::testutil::scratch("hz-first-start-3");
        assert!(first_start(&big, 16_000, 8, false).is_empty());
        assert!(!big.join("options.txt").exists() && !big.join("horizon-graphics.txt").exists());
        for d in [root, other, big] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn the_first_start_keeps_a_simulation_distance_and_a_file_it_cant_read() {
        let root = crate::testutil::scratch("hz-first-start-keep");
        std::fs::write(root.join("options.txt"), "simulationDistance:24\n").unwrap();
        first_start(&root, 2560, 8, false);
        assert_eq!(
            std::fs::read_to_string(root.join("options.txt")).unwrap(),
            "simulationDistance:24\nrenderDistance:6\n"
        );
        // not text: left byte for byte, with a note
        let bytes = [b'f', b'o', b'v', b':', 0xff, 0xfe, b'\n'];
        std::fs::write(root.join("options.txt"), bytes).unwrap();
        let notes = first_start(&root, 2560, 8, false);
        assert_eq!(std::fs::read(root.join("options.txt")).unwrap(), bytes);
        assert!(notes.iter().any(|n| n.starts_with("Left ")), "{notes:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_start_warns_when_less_is_free_than_the_render_distance_needs() {
        let root = crate::testutil::scratch("hz-memory-warning");
        // Horizon's default 12 needs about 1.8 GB
        assert!(memory_warning(&root, 1700).unwrap().contains("now 12"));
        assert!(memory_warning(&root, 4000).is_none());
        assert!(memory_warning(&root, -1).is_none());
        std::fs::write(root.join("options.txt"), "renderDistance:4\n").unwrap();
        assert!(memory_warning(&root, 1700).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_chosen_card_is_all_the_game_sees_and_one_card_changes_nothing() {
        let cards = [
            SysCard { pci_address: "0000:00:02.0".into(), vendor: 0x8086, device: 0x9a49, boot_vga: true },
            SysCard { pci_address: "0000:01:00.0".into(), vendor: 0x10de, device: 0x1f99, boot_vga: false },
        ];
        let gpus = from_sysfs(&cards, |_, _| None, None);
        let (env, notes) = gpu_effect(&Choice::Integrated, &gpus);
        assert!(env.contains(&("MESA_VK_DEVICE_SELECT".to_string(), "8086:9a49!".to_string())));
        assert!(env.contains(&("__VK_LAYER_NV_optimus".to_string(), "non_NVIDIA_only".to_string())));
        assert!(notes[0].starts_with("Graphics card: "));
        let (env, _) = gpu_effect(&Choice::Dedicated, &gpus);
        assert!(env.contains(&("__VK_LAYER_NV_optimus".to_string(), "NVIDIA_only".to_string())));
        // this computer's case: one NVIDIA card, nothing to choose
        let one = from_sysfs(&cards[1..], |_, _| None, None);
        assert_eq!(gpu_effect(&Choice::Dedicated, &one), (Vec::new(), Vec::new()));
        // a card that isn't there any more: a note, no variables
        let (env, notes) = gpu_effect(&Choice::Specific("pci-0000_09_00_0".into()), &gpus);
        assert!(env.is_empty() && notes.len() == 1);
    }
}
