//! Compares the planner with the reference results of another implementation (a JSON file of gather lists).
//! `cargo run --release --example craft_diff -- <version> <client.jar> <reference.json> [show]`

use std::collections::BTreeMap;

use blockforge::craft::{Request, load, plan};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: craft_diff <version> <client.jar> <reference.json> [show]");
        std::process::exit(2);
    }
    let show = args.get(4).map(|s| s == "show").unwrap_or(false);
    let t = std::time::Instant::now();
    let ds = load::load_jar(&args[1], std::path::Path::new(&args[2])).expect("load");
    println!(
        "loaded {} items, {} recipes ({} skipped) in {:?}",
        ds.items.len(),
        ds.recipes.len(),
        ds.skipped.len(),
        t.elapsed()
    );
    let reference: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[3]).unwrap()).unwrap();
    let reference = &reference[&args[1]];
    let (mut same, mut different, mut missing, mut errors) = (0, 0, 0, 0);
    let mut worst = std::time::Duration::ZERO;
    let mut total = std::time::Duration::ZERO;
    let mut plans = 0u32;
    for (key, want) in reference.as_object().unwrap() {
        let (item, qty) = key.rsplit_once('*').unwrap();
        if !ds.items.contains_key(item) {
            missing += 1;
            continue;
        }
        let req = Request { item: item.to_string(), quantity: qty.parse().unwrap(), ..Default::default() };
        let t = std::time::Instant::now();
        let result = plan::calculate(&ds, &req);
        let took = t.elapsed();
        total += took;
        plans += 1;
        worst = worst.max(took);
        let Ok(got) = result else {
            errors += 1;
            if show {
                println!("ERR {key}: {}", result.unwrap_err());
            }
            continue;
        };
        let Some(arr) = want.get("materials").and_then(|m| m.as_array()) else {
            continue;
        };
        let want: BTreeMap<String, u64> =
            arr.iter().map(|m| (m[0].as_str().unwrap().to_string(), m[1].as_u64().unwrap())).collect();
        let have: BTreeMap<String, u64> = got.materials.iter().cloned().collect();
        if want == have {
            same += 1;
        } else {
            different += 1;
            if show {
                println!("DIFF {key}\n   p1:   {want:?}\n   rust: {have:?}");
            }
        }
    }
    println!(
        "same {same}, different {different}, errors {errors}, not in this jar {missing}; {plans} plans in {total:?} (worst {worst:?})"
    );
}
