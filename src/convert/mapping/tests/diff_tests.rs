// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Differential test against Chunker's real resolvers. `tools/convert/check` generates cases and runs Chunker on them
//! (`Check.java`); this test replays the answers through the Rust engine. It needs a JDK and the Chunker sources to
//! produce the answers, so it only runs when `CONVERT_DIFF_ANSWERS` names the answers file:
//! `CONVERT_DIFF_ANSWERS=answers.tsv cargo test --lib convert::mapping::tests::diff -- --ignored --nocapture`.

use std::collections::HashMap;

use crate::convert::mapping::*;

fn parse_typed_pairs(s: &str) -> StateMap {
    s.split(',')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap();
            (k.into(), StateValue::parse_typed(v).unwrap())
        })
        .collect()
}

fn pairs(m: &StateMap) -> String {
    m.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
}

#[test]
#[ignore = "needs CONVERT_DIFF_ANSWERS, see the module docs"]
fn diff_against_chunker() {
    let Ok(path) = std::env::var("CONVERT_DIFF_ANSWERS") else {
        eprintln!("skipping: CONVERT_DIFF_ANSWERS not set");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut resolvers: HashMap<(String, String), BlockResolver> = HashMap::new();
    let (mut total, mut bad) = (0usize, 0usize);
    for line in text.lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        let (mode, version, dir, id, st, expect) = (f[0], f[1], f[2], f[3], f[4], f[5]);
        let r = resolvers.entry((mode.to_string(), version.to_string())).or_insert_with(|| {
            let (ed, legacy) = match mode {
                "java" => (Edition::Java, false),
                "bedrock" => (Edition::Bedrock, false),
                "java_legacy" => (Edition::Java, true),
                _ => (Edition::Bedrock, true),
            };
            let v = Version::parse(version).unwrap();
            if legacy { BlockResolver::new_legacy(ed, v) } else { BlockResolver::new(ed, v) }
        });
        let got = if dir == "T" {
            let m = parse_typed_pairs(st);
            r.to_pivot(id, &m).map(|p| {
                let s: Vec<String> = p.states.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{}|{}", p.ty, s.join(","))
            })
        } else {
            let states = st
                .split(',')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap();
                    (k.into(), v.into())
                })
                .collect();
            r.from_pivot(&PivotBlock { ty: id.into(), states }).map(|(i, s)| format!("{i}|{}", pairs(&s)))
        };
        let got = got.unwrap_or_else(|| "NONE".to_string());
        total += 1;
        if got != expect {
            bad += 1;
            if bad <= 25 {
                eprintln!("MISMATCH {mode} {version} {dir} {id} [{st}]\n   chunker: {expect}\n   rust:    {got}");
            }
        }
    }
    eprintln!("{total} cases, {bad} mismatches");
    assert_eq!(bad, 0);
}
