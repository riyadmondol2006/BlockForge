//! Picks the version of a project to install without asking: the newest one made for this Minecraft version and
//! loader, preferring full releases over betas.

use std::cmp::Ordering;

use super::mr::Version;

/// Newest first. Modrinth dates are ISO 8601 in UTC, so they sort as text.
pub fn newest_first(a: &Version, b: &Version) -> Ordering {
    let da = a.date_published.as_deref().unwrap_or("");
    let db = b.date_published.as_deref().unwrap_or("");
    db.cmp(da)
}

/// Picks a version.
///
/// * `accepted_loaders`: loaders that work on the instance; empty for resource and shader packs, where any goes
/// * `game_version`: the instance's Minecraft version (`None` accepts any)
/// * `fallback_versions`: other Minecraft versions that are close enough when nothing is made for the exact one
///   (empty to insist on the exact version)
pub fn pick<'a>(
    versions: &'a [Version],
    accepted_loaders: &[String],
    game_version: Option<&str>,
    fallback_versions: &[String],
) -> Option<&'a Version> {
    compatible(versions, accepted_loaders, game_version, fallback_versions).into_iter().next()
}

/// Every version that fits, best first: exact game version releases, exact game version betas, then the same for
/// the close-enough versions. Nothing appears twice.
pub fn compatible<'a>(
    versions: &'a [Version],
    accepted_loaders: &[String],
    game_version: Option<&str>,
    fallback_versions: &[String],
) -> Vec<&'a Version> {
    let mut sorted: Vec<&Version> = versions.iter().collect();
    // a stable sort, so versions published at the same moment keep Modrinth's order
    sorted.sort_by(|a, b| newest_first(a, b));

    let mut out: Vec<&Version> = Vec::new();
    let mut taken = vec![false; sorted.len()];
    for tier in 0..4 {
        let exact = tier < 2;
        let releases_only = tier % 2 == 0;
        for (i, v) in sorted.iter().enumerate() {
            if taken[i] || !loader_fits(v, accepted_loaders) {
                continue;
            }
            if releases_only && !v.is_release() {
                continue;
            }
            let fits = if exact { made_for(v, game_version) } else { made_for_any(v, fallback_versions) };
            if fits {
                out.push(v);
                taken[i] = true;
            }
        }
    }
    out
}

/// True when `candidate` is a different and newer version than the one installed.
pub fn is_newer(candidate: Option<&Version>, installed: Option<&Version>) -> bool {
    let Some(candidate) = candidate.filter(|c| !c.id.is_empty()) else { return false };
    let Some(installed) = installed.filter(|i| !i.id.is_empty()) else { return true };
    if candidate.id == installed.id {
        return false;
    }
    let a = candidate.date_published.as_deref().unwrap_or("");
    let b = installed.date_published.as_deref().unwrap_or("");
    a > b
}

pub fn loader_fits(v: &Version, accepted_loaders: &[String]) -> bool {
    if accepted_loaders.is_empty() {
        return true;
    }
    v.loaders.iter().any(|l| accepted_loaders.contains(l))
}

pub fn made_for(v: &Version, game_version: Option<&str>) -> bool {
    match game_version {
        None => true,
        Some(g) => v.game_versions.iter().any(|x| x == g),
    }
}

pub fn made_for_any(v: &Version, game_versions: &[String]) -> bool {
    game_versions.iter().any(|g| v.game_versions.contains(g))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::loader_support;

    fn version(id: &str, date: &str, channel: &str, loaders: &[&str], games: &[&str]) -> Version {
        Version {
            id: id.into(),
            date_published: Some(date.into()),
            version_type: Some(channel.into()),
            loaders: loaders.iter().map(|s| s.to_string()).collect(),
            game_versions: games.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    fn id(v: Option<&Version>) -> Option<&str> {
        v.map(|v| v.id.as_str())
    }

    #[test]
    fn picks_the_newest_release_for_the_exact_version_and_loader() {
        let versions = vec![
            version("old", "2026-01-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
            version("new", "2026-06-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
            version("neo", "2026-07-01T00:00:00Z", "release", &["neoforge"], &["26.3"]),
            version("other-mc", "2026-08-01T00:00:00Z", "release", &["fabric"], &["26.2"]),
        ];
        assert_eq!(id(pick(&versions, &s(&["fabric"]), Some("26.3"), &[])), Some("new"));
        assert_eq!(id(pick(&versions, &s(&["neoforge"]), Some("26.3"), &[])), Some("neo"));
    }

    #[test]
    fn prefers_a_release_over_a_newer_beta() {
        let versions = vec![
            version("beta", "2026-09-01T00:00:00Z", "beta", &["fabric"], &["26.3"]),
            version("release", "2026-05-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
        ];
        assert_eq!(id(pick(&versions, &s(&["fabric"]), Some("26.3"), &[])), Some("release"));
        assert_eq!(id(pick(&versions[..1], &s(&["fabric"]), Some("26.3"), &[])), Some("beta"));
    }

    #[test]
    fn quilt_accepts_fabric_builds() {
        let quilt = loader_support::accepted_loaders(Some("quilt"), "26.3", &[]);
        let versions = vec![version("f", "2026-05-01T00:00:00Z", "release", &["fabric"], &["26.3"])];
        assert_eq!(id(pick(&versions, &quilt, Some("26.3"), &[])), Some("f"));
        assert_eq!(id(pick(&versions, &s(&["forge"]), Some("26.3"), &[])), None);
    }

    #[test]
    fn nothing_for_the_exact_version_means_nothing_unless_nearby_versions_are_allowed() {
        let versions = vec![version("near", "2026-05-01T00:00:00Z", "release", &["fabric"], &["26.2"])];
        assert_eq!(id(pick(&versions, &s(&["fabric"]), Some("26.2.1"), &[])), None);
        assert_eq!(id(pick(&versions, &s(&["fabric"]), Some("26.2.1"), &s(&["26.2", "26.2.1"]))), Some("near"));
    }

    #[test]
    fn packs_ignore_loaders() {
        let versions = vec![version("shader", "2026-05-01T00:00:00Z", "release", &["iris", "optifine"], &["26.3"])];
        assert_eq!(id(pick(&versions, &[], Some("26.3"), &[])), Some("shader"));
    }

    #[test]
    fn compatible_lists_best_first_without_repeats() {
        let versions = vec![
            version("beta-new", "2026-09-01T00:00:00Z", "beta", &["fabric"], &["26.3"]),
            version("rel-old", "2026-01-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
            version("rel-new", "2026-06-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
        ];
        let ids: Vec<&str> =
            compatible(&versions, &s(&["fabric"]), Some("26.3"), &[]).iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, vec!["rel-new", "rel-old", "beta-new"]);
        assert!(compatible(&[], &s(&["fabric"]), Some("26.3"), &[]).is_empty());
    }

    #[test]
    fn tells_when_a_version_is_newer() {
        let installed = version("a", "2026-01-01T00:00:00Z", "release", &["fabric"], &["26.3"]);
        let newer = version("b", "2026-02-01T00:00:00Z", "release", &["fabric"], &["26.3"]);
        let same = version("a", "2026-01-01T00:00:00Z", "release", &["fabric"], &["26.3"]);
        assert!(is_newer(Some(&newer), Some(&installed)));
        assert!(!is_newer(Some(&installed), Some(&newer)));
        assert!(!is_newer(Some(&same), Some(&installed)));
        assert!(!is_newer(None, Some(&installed)));
        assert!(is_newer(Some(&newer), None));
    }
}
