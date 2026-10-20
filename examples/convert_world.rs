// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Convert a world from the command line: `cargo run --release --example convert_world -- <input> <output> [java-version]`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use blockforge::convert::level::encoding::{EncodingType, TargetVersion, VersionTriple};
use blockforge::convert::level::{ConvertRequest, ProgressSink};
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: convert_world <input world or .mcworld> <output folder> [java version, e.g. 1.21.4]");
        std::process::exit(2);
    }
    let target = match args.get(3) {
        Some(v) => TargetVersion::new(EncodingType::Java, VersionTriple::from_string(v).expect("version")),
        None => TargetVersion::latest(EncodingType::Java),
    };
    let request = ConvertRequest::new(&args[1], &args[2], target);
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
