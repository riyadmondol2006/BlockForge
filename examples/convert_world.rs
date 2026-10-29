// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Convert a world from the command line: `cargo run --release --example convert_world -- <input> <output> [java-version]
//! [options]`. Everything comes along by default; the options turn parts off or set what the world should be:
//!
//! - `--owner <uuid>`: the Java account that owns the world (the host's player file and pets)
//! - `--mobs all|no-hostile|pets|none`, `--no-item-frames`, `--keep-dropped` (dropped items, orbs, arrows ...)
//! - `--no-ender-chest`, `--no-stats`, `--no-other-players`
//! - `--inventory <player uuid>` (another player's inventory for the owner), `--merge-inventory` (keep the host's too)
//! - `--difficulty peaceful|easy|normal|hard`, `--gamemode survival|creative|adventure|spectator`,
//!   `--cheats on|off`, `--hardcore on|off`

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use blockforge::convert::level::encoding::{EncodingType, TargetVersion, VersionTriple};
use blockforge::convert::level::{
    ConversionSettings, ConvertRequest, Difficulty, EntityFilter, GameType, InventorySource, ProgressSink,
};
use blockforge::convert::pipeline::convert_world;

struct Print;

impl ProgressSink for Print {
    fn stage(&self, text: &str) {
        eprintln!("== {text}");
    }
    fn progress(&self, fraction: f64) {
        eprint!("\r{:5.1}%", fraction * 100.0);
    }
    fn warning(&self, message: &str) {
        eprintln!("warning: {message}");
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

fn on_off(value: Option<&String>, what: &str) -> bool {
    match value.map(String::as_str) {
        Some("on") => true,
        Some("off") => false,
        _ => fail(&format!("{what} takes on or off")),
    }
}

/// The options after the positional arguments, applied to the settings.
fn apply_options(options: &[String], settings: &mut ConversionSettings) {
    let mut merge = false;
    let mut it = options.iter();
    while let Some(opt) = it.next() {
        match opt.as_str() {
            "--owner" => {
                let v = it.next().unwrap_or_else(|| fail("--owner takes a uuid"));
                settings.player.owner_uuid = Some(uuid::Uuid::parse_str(v).unwrap_or_else(|_| fail("not a uuid")));
            }
            "--mobs" => {
                let filter = match it.next().map(String::as_str) {
                    Some("all") => EntityFilter::All,
                    Some("no-hostile") => EntityFilter::NoHostile,
                    Some("pets") => EntityFilter::PetsAndNamedOnly,
                    Some("none") => EntityFilter::None,
                    _ => fail("--mobs takes all, no-hostile, pets or none"),
                };
                settings.process_entities = filter != EntityFilter::None;
                settings.entity_filter = filter;
            }
            "--no-item-frames" => settings.convert_item_frames = false,
            "--keep-dropped" => settings.drop_transient_entities = false,
            "--no-ender-chest" => settings.player.include_ender_chest = false,
            "--no-stats" => settings.player.include_stats = false,
            "--no-other-players" => settings.player.export_guest_profiles = false,
            "--inventory" => {
                let id = it.next().unwrap_or_else(|| fail("--inventory takes a player's uuid")).clone();
                settings.player.inventory_source = InventorySource::Profile(id);
            }
            "--merge-inventory" => merge = true,
            "--difficulty" => {
                settings.level_overrides.difficulty = Some(match it.next().map(String::as_str) {
                    Some("peaceful") => Difficulty::Peaceful,
                    Some("easy") => Difficulty::Easy,
                    Some("normal") => Difficulty::Normal,
                    Some("hard") => Difficulty::Hard,
                    _ => fail("--difficulty takes peaceful, easy, normal or hard"),
                })
            }
            "--gamemode" => {
                settings.level_overrides.game_type = Some(match it.next().map(String::as_str) {
                    Some("survival") => GameType::Survival,
                    Some("creative") => GameType::Creative,
                    Some("adventure") => GameType::Adventure,
                    Some("spectator") => GameType::Spectator,
                    _ => fail("--gamemode takes survival, creative, adventure or spectator"),
                })
            }
            "--cheats" => settings.level_overrides.allow_commands = Some(on_off(it.next(), "--cheats")),
            "--hardcore" => settings.level_overrides.hardcore = Some(on_off(it.next(), "--hardcore")),
            other => fail(&format!("unknown option {other}")),
        }
    }
    if merge {
        match &settings.player.inventory_source {
            InventorySource::Profile(id) => {
                settings.player.inventory_source = InventorySource::MergeHostAnd(id.clone())
            }
            _ => fail("--merge-inventory needs --inventory"),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        fail("usage: convert_world <input world or .mcworld> <output folder> [java version, e.g. 1.21.4] [options]");
    }
    // the version is optional: the first argument after the folders that is not an option
    let (version, options) = match args.get(3) {
        Some(v) if !v.starts_with("--") => (Some(v), &args[4..]),
        _ => (None, &args[3.min(args.len())..]),
    };
    let target = match version {
        Some(v) => TargetVersion::new(EncodingType::Java, VersionTriple::from_string(v).expect("version")),
        None => TargetVersion::latest(EncodingType::Java),
    };
    let mut request = ConvertRequest::new(&args[1], &args[2], target);
    apply_options(options, &mut request.settings);
    let cancel = Arc::new(AtomicBool::new(false));
    match convert_world(&request, &Print, &cancel) {
        Ok(summary) => {
            eprintln!("\ndone in {:?}: {} columns", summary.elapsed, summary.columns);
            for (name, n) in summary.blocks_unmapped.iter().take(20) {
                eprintln!("  unmapped {name} x{n}");
            }
            for w in summary.warnings.iter().take(20) {
                eprintln!("  warning: {w}");
            }
        }
        Err(e) => {
            eprintln!("\nfailed: {e}");
            std::process::exit(1);
        }
    }
}
