// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Bedrock -> Java block coverage: every Bedrock block state of a Bedrock version goes through the pivot to a Java
//! version (the closest release and the latest), and what comes out must be a block state that exists in that Java
//! version. Chunker has no such test, it only asserts each side against the pivot; this is the end-to-end view.
//!
//! A block that the Java table cannot write is `gap:<TYPE>` when no Java version writes the pivot type (Bedrock-only
//! content, Chunker replaces it too) and `newer:<TYPE>` when a later Java version does (content newer than the target).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use super::lossy;
use super::vdata::{self, BlockData, VersionDir};
use super::{Tally, VerReport, par_map};
use crate::convert::mapping::{BlockResolver, Edition, Version};

/// Closest Java release for a Bedrock version (lower bounds, ascending); Bedrock 26+ and unknown go to the latest.
const CLOSEST_JAVA: &[((u32, u32, u32), &str)] = &[
    ((1, 12, 0), "1.13"),
    ((1, 13, 0), "1.13.2"),
    ((1, 14, 0), "1.14.4"),
    ((1, 16, 0), "1.16.5"),
    ((1, 17, 0), "1.17.1"),
    ((1, 18, 0), "1.18.2"),
    ((1, 19, 0), "1.19.2"),
    ((1, 19, 30), "1.19.3"),
    ((1, 19, 60), "1.19.4"),
    ((1, 20, 0), "1.20.1"),
    ((1, 20, 30), "1.20.2"),
    ((1, 20, 40), "1.20.4"),
    ((1, 20, 70), "1.20.6"),
    ((1, 21, 0), "1.21.1"),
    ((1, 21, 30), "1.21.3"),
    ((1, 21, 50), "1.21.4"),
    ((1, 21, 70), "1.21.5"),
    ((1, 21, 80), "1.21.6"),
    ((1, 21, 90), "1.21.8"),
    ((1, 21, 100), "1.21.9"),
    ((1, 21, 120), "1.21.11"),
];

/// The Java version folder closest in release to a Bedrock version (the latest for unknown ones).
pub(crate) fn closest_java(v: Version, java: &[VersionDir]) -> &VersionDir {
    let mut name = None;
    for ((a, b, c), n) in CLOSEST_JAVA {
        if v >= Version::new(*a, *b, *c) {
            name = Some(*n);
        }
    }
    name.and_then(|n| java.iter().find(|d| d.name == n))
        .filter(|_| v.major == 1)
        .unwrap_or_else(|| java.last().expect("java versions"))
}

struct Target {
    dir: VersionDir,
    data: BlockData,
    resolver: BlockResolver,
}

pub(crate) fn run(dir: &Path) -> Vec<VerReport> {
    let bedrock = vdata::version_dirs(dir, "bedrock");
    let java = vdata::version_dirs(dir, "java");
    let Some(latest) = java.last() else { return Vec::new() };
    let closest = |v: Version| closest_java(v, &java);
    let mut needed: Vec<&VersionDir> = vec![latest];
    for b in &bedrock {
        let c = closest(b.version);
        if !needed.iter().any(|d| d.name == c.name) {
            needed.push(c);
        }
    }
    let targets: BTreeMap<String, Arc<Target>> = par_map(&needed, |d| {
        let data = vdata::load_java_blocks(d).unwrap_or_default();
        Arc::new(Target { dir: (*d).clone(), data, resolver: BlockResolver::new(Edition::Java, d.version) })
    })
    .into_iter()
    .map(|t| (t.dir.name.clone(), t))
    .collect();
    let latest_target = targets[&latest.name].clone();

    let pairs: Vec<(&VersionDir, Arc<Target>)> = bedrock
        .iter()
        .flat_map(|b| {
            let c = targets[&closest(b.version).name].clone();
            let mut v = vec![(b, c.clone())];
            if c.dir.name != latest_target.dir.name {
                v.push((b, latest_target.clone()));
            }
            v
        })
        .collect();
    par_map(&pairs, |(b, t)| check_pair(b, t, &latest_target))
}

fn check_pair(b: &VersionDir, target: &Target, latest: &Target) -> VerReport {
    let label = format!("{}->{}", b.name, target.dir.name);
    let blocks = match vdata::load_bedrock_blocks(b) {
        Ok(x) => x,
        Err(e) => {
            let mut t = Tally::default();
            t.fail("data", || e);
            return VerReport {
                ids: 0,
                area: "cross",
                edition: "bedrock>java",
                version: label,
                checks: vec![("load", t)],
            };
        }
    };
    let from = BlockResolver::new(Edition::Bedrock, b.version);
    let mut t = Tally::default();
    for (id, list) in &blocks.states {
        if lossy::bedrock_unsupported_block(id) {
            continue;
        }
        for st in list {
            let Some(p) = from.to_pivot(id, st) else { continue };
            match target.resolver.from_pivot(&p) {
                None => {
                    if latest.resolver.is_supported_type(&p.ty) && !std::ptr::eq(latest, target) {
                        t.allow(&format!("newer:{}", p.ty));
                    } else {
                        t.allow(&format!("gap:{}", p.ty));
                    }
                }
                Some((jid, jst)) => match target.data.allowed.get(&*jid) {
                    Some(set) if set.contains(&jst) => t.pass(),
                    Some(_) => t.fail(id, || {
                        format!(
                            "{} -> {} -> {} has no such state combination",
                            super::show(id, st),
                            p.ty,
                            super::show(&jid, &jst)
                        )
                    }),
                    None => t.fail(id, || {
                        format!("{} -> {} -> {jid} is not a block of {}", super::show(id, st), p.ty, target.dir.name)
                    }),
                },
            }
        }
    }
    VerReport {
        ids: blocks.states.len(),
        area: "cross",
        edition: "bedrock>java",
        version: label,
        checks: vec![("to_java", t)],
    }
}
