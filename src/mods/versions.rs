//! What each mod loader can run, which Minecraft versions a mod's requirement allows, and which version of a
//! project to pick. These are the pure rules behind installing and updating, kept apart from any network code so
//! they can be tested on their own.

use serde_json::Value;

pub const FABRIC: &str = "fabric";
pub const QUILT: &str = "quilt";
pub const FORGE: &str = "forge";
pub const NEOFORGE: &str = "neoforge";
pub const LEGACY_FABRIC: &str = "legacy-fabric";
pub const LITELOADER: &str = "liteloader";

pub const IRIS: &str = "YL57xq9U";
pub const SODIUM: &str = "AANobbMI";
pub const OCULUS: &str = "GchcoXML";
pub const EMBEDDIUM: &str = "sk9rgfiA";
pub const LITHIUM: &str = "gvQqBUqZ";
pub const FERRITE_CORE: &str = "uXXizFIs";
pub const MODERN_FIX: &str = "nmDcB62a";

pub const MODRINTH_FABRIC_MOD_ID: &str = "P7dR8mSH";
pub const MODRINTH_LEGACY_FABRIC_MOD_ID: &str = "9CJED7xi";
pub const MODRINTH_QSL_MOD_ID: &str = "qvIfYCYJ";
pub const MODRINTH_SINYTRA_CONNECTOR_MOD_ID: &str = "u58R1TMW";
pub const CURSEFORGE_FABRIC_MOD_ID: i64 = 306612;
pub const CURSEFORGE_LEGACY_FABRIC_MOD_ID: i64 = 400281;
pub const CURSEFORGE_SINYTRA_CONNECTOR_MOD_ID: i64 = 890127;

/// What a shader pack needs before it does anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShaderSetup {
    /// Iris, Oculus or OptiFine is already there.
    Ready,
    /// Iris (with Sodium) can be added from Modrinth.
    AddIris,
    /// Forge: Oculus can be added from Modrinth if it exists for this version, otherwise OptiFine is needed.
    AddOculus,
    /// No mod loader, or one no shader loader is made for.
    NeedsModLoader,
}

/// Modrinth's name for the loader called `loader_type` in an instance (`Fabric`, `NeoForge`, `LegacyFabric` ...),
/// or `None` for plain Minecraft and server-only loaders.
pub fn loader_key(loader_type: &str) -> Option<&'static str> {
    match loader_type.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
        "fabric" => Some(FABRIC),
        "quilt" => Some(QUILT),
        "neoforge" => Some(NEOFORGE),
        "forge" => Some(FORGE),
        "legacyfabric" => Some(LEGACY_FABRIC),
        _ => None,
    }
}

/// The loaders whose mods run on this loader: Quilt runs Fabric mods, and NeoForge runs Forge mods on the
/// Minecraft versions where the two are still compatible.
pub fn accepted_loaders(
    key: Option<&str>,
    minecraft_version: &str,
    neoforge_runs_forge_mods_on: &[String],
) -> Vec<String> {
    let Some(key) = key else { return Vec::new() };
    let mut loaders = vec![key.to_string()];
    if key == QUILT {
        loaders.push(FABRIC.to_string());
    }
    if key == NEOFORGE && neoforge_runs_forge_mods_on.iter().any(|v| v == minecraft_version) {
        loaders.push(FORGE.to_string());
    }
    loaders
}

/// Whether a mod loader is there that shader packs work with.
pub fn has_shader_loader<'a>(
    project_ids: impl IntoIterator<Item = &'a str>,
    file_names: impl IntoIterator<Item = &'a str>,
) -> bool {
    if project_ids.into_iter().any(|id| id == IRIS || id == OCULUS) {
        return true;
    }
    file_names.into_iter().any(|f| {
        let name = f.to_lowercase();
        name.starts_with("iris")
            || name.starts_with("oculus")
            || name.contains("optifine")
            || name.starts_with("optifabric")
    })
}

pub fn shader_setup(key: Option<&str>, has_shader_loader: bool) -> ShaderSetup {
    if has_shader_loader {
        return ShaderSetup::Ready;
    }
    match key {
        Some(FABRIC) | Some(QUILT) | Some(NEOFORGE) => ShaderSetup::AddIris,
        Some(FORGE) => ShaderSetup::AddOculus,
        _ => ShaderSetup::NeedsModLoader,
    }
}

/// A readable name for the loader, for messages.
pub fn display_name(key: Option<&str>) -> String {
    match key {
        None => "Minecraft".into(),
        Some(FABRIC) => "Fabric".into(),
        Some(QUILT) => "Quilt".into(),
        Some(FORGE) => "Forge".into(),
        Some(NEOFORGE) => "NeoForge".into(),
        Some(LEGACY_FABRIC) => "Legacy Fabric".into(),
        Some(LITELOADER) => "LiteLoader".into(),
        Some(other) => other.to_string(),
    }
}

/// Whether an instance with one loader runs a mod made for another.
pub fn loader_runs(instance_loader: &str, mod_loader: &str, minecraft_version: &str) -> bool {
    if instance_loader == mod_loader {
        return true;
    }
    if instance_loader == QUILT && mod_loader == FABRIC {
        return true;
    }
    // NeoForge still loaded Forge mods on 1.20.1, where it started
    instance_loader == NEOFORGE && mod_loader == FORGE && minecraft_version == "1.20.1"
}

// ---------------------------------------------------------------------------------------------------------
// Version requirements

/// Checks a Minecraft version against the version requirements mods write in their metadata: Fabric and Quilt style
/// (`>=1.21 <1.22`, `~1.20.1`, `1.21.x`) and Forge's Maven ranges (`[1.20.1,1.21)`). It is only used to warn, so
/// anything it can't read counts as unknown (`None`) rather than wrong.
pub fn range_matches(requirement: Option<&str>, version: Option<&str>) -> Option<bool> {
    let (requirement, version) = (requirement?, version?);
    let req = requirement.trim();
    if req.is_empty() || req.contains("${") {
        return None;
    }
    let v = parse_version(version)?;
    if req.starts_with('[') || req.starts_with('(') {
        return maven_range(req, &v);
    }
    let mut any = None;
    for alternative in req.split("||") {
        match all_predicates(alternative.trim(), &v) {
            Some(true) => return Some(true),
            Some(false) => any = Some(false),
            None => {}
        }
    }
    any
}

/// Several requirements, any of which is enough (Fabric's array form).
pub fn range_matches_any(requirements: &[String], version: &str) -> Option<bool> {
    let mut result = None;
    for r in requirements {
        match range_matches(Some(r), Some(version)) {
            Some(true) => return Some(true),
            Some(false) => result = Some(false),
            None => {}
        }
    }
    result
}

fn all_predicates(predicates: &str, v: &[u32]) -> Option<bool> {
    if predicates.is_empty() {
        return None;
    }
    for p in predicates.split_whitespace() {
        match predicate(p, v)? {
            true => {}
            false => return Some(false),
        }
    }
    Some(true)
}

fn predicate(p: &str, v: &[u32]) -> Option<bool> {
    if p == "*" || p == "x" {
        return Some(true);
    }
    if let Some(rest) = p.strip_prefix(">=") {
        return parse_version(rest).map(|b| compare(v, &b) >= 0);
    }
    if let Some(rest) = p.strip_prefix("<=") {
        return parse_version(rest).map(|b| compare(v, &b) <= 0);
    }
    if let Some(rest) = p.strip_prefix('>') {
        return parse_version(rest).map(|b| compare(v, &b) > 0);
    }
    if let Some(rest) = p.strip_prefix('<') {
        return parse_version(rest).map(|b| compare(v, &b) < 0);
    }
    if let Some(rest) = p.strip_prefix('~') {
        // same major and minor, at least this patch
        let b = parse_version(rest)?;
        return Some(compare(v, &b) >= 0 && part(v, 0) == part(&b, 0) && part(v, 1) == part(&b, 1));
    }
    if let Some(rest) = p.strip_prefix('^') {
        let b = parse_version(rest)?;
        // for Minecraft's 1.x versions the minor number is what really changes, so ^1.20 stays on 1.20
        let same_line = if part(&b, 0) == 1 { part(v, 1) == part(&b, 1) } else { part(v, 0) == part(&b, 0) };
        return Some(compare(v, &b) >= 0 && part(v, 0) == part(&b, 0) && same_line);
    }
    let body = p.strip_prefix('=').unwrap_or(p);
    if body.ends_with(".x") || body.ends_with(".*") || body.ends_with(".X") {
        let prefix = parse_version(&body[..body.len() - 2])?;
        return Some(prefix.iter().enumerate().all(|(i, n)| part(v, i) == *n));
    }
    parse_version(body).map(|b| compare(v, &b) == 0)
}

/// Maven ranges: `[a,b)` and friends, several joined by commas meaning any of them.
fn maven_range(req: &str, v: &[u32]) -> Option<bool> {
    let mut ranges: Vec<&str> = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in req.char_indices() {
        match c {
            '[' | '(' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    ranges.push(&req[start..=i]);
                }
            }
            _ => {}
        }
    }
    if ranges.is_empty() {
        return None;
    }
    for range in ranges {
        let low_inclusive = range.starts_with('[');
        let high_inclusive = range.ends_with(']');
        let inner = &range[1..range.len() - 1];
        let bounds: Vec<&str> = inner.split(',').collect();
        if bounds.len() == 1 {
            let exact = parse_version(bounds[0].trim())?;
            if compare(v, &exact) == 0 {
                return Some(true);
            }
            continue;
        }
        let (low, high) = (bounds[0].trim(), bounds[1].trim());
        let mut ok = true;
        if !low.is_empty() {
            let c = compare(v, &parse_version(low)?);
            ok = if low_inclusive { c >= 0 } else { c > 0 };
        }
        if ok && !high.is_empty() {
            let c = compare(v, &parse_version(high)?);
            ok = if high_inclusive { c <= 0 } else { c < 0 };
        }
        if ok {
            return Some(true);
        }
    }
    Some(false)
}

/// "1.20.1" as [1, 20, 1]; text after a dash or plus (pre-releases) is ignored. `None` if it isn't a numbered version.
pub fn parse_version(version: &str) -> Option<Vec<u32>> {
    let mut v = version.trim();
    if let Some(i) = v.find('-').filter(|i| *i > 0) {
        v = &v[..i];
    }
    if let Some(i) = v.find('+').filter(|i| *i > 0) {
        v = &v[..i];
    }
    if v.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for p in v.split('.') {
        if p.is_empty() || p.len() > 6 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out.push(p.parse().ok()?);
    }
    Some(out)
}

fn part(v: &[u32], i: usize) -> u32 {
    v.get(i).copied().unwrap_or(0)
}

fn compare(a: &[u32], b: &[u32]) -> i32 {
    for i in 0..a.len().max(b.len()) {
        match part(a, i).cmp(&part(b, i)) {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => {}
        }
    }
    0
}

// ---------------------------------------------------------------------------------------------------------
// Picking a version

/// The parts of a Modrinth version the rules below look at.
pub trait VersionLike {
    fn id(&self) -> &str;
    fn date_published(&self) -> &str;
    fn version_type(&self) -> Option<&str>;
    fn loaders(&self) -> &[String];
    fn game_versions(&self) -> &[String];
}

/// Every version that fits, best first: exact game version releases, exact game version betas, then the same for
/// the close-enough versions. `accepted_loaders` empty means any loader (resource and shader packs).
pub fn compatible<'a, V: VersionLike>(
    versions: &'a [V],
    accepted_loaders: &[String],
    game_version: Option<&str>,
    fallback_versions: &[String],
) -> Vec<&'a V> {
    let mut sorted: Vec<&V> = versions.iter().collect();
    // Modrinth dates are ISO 8601 in UTC, so they sort as text; newest first
    sorted.sort_by(|a, b| b.date_published().cmp(a.date_published()));
    let mut out: Vec<&V> = Vec::new();
    let mut taken = vec![false; sorted.len()];
    for tier in 0..4 {
        let exact = tier < 2;
        let releases_only = tier % 2 == 0;
        for (i, v) in sorted.iter().enumerate() {
            if taken[i] || !loader_fits(*v, accepted_loaders) {
                continue;
            }
            if releases_only && v.version_type().is_some_and(|t| t != "release") {
                continue;
            }
            let fits = if exact { made_for(*v, game_version) } else { made_for_any(*v, fallback_versions) };
            if fits {
                out.push(*v);
                taken[i] = true;
            }
        }
    }
    out
}

/// The newest, release-first version that fits, or `None`.
pub fn pick<'a, V: VersionLike>(
    versions: &'a [V],
    accepted_loaders: &[String],
    game_version: Option<&str>,
    fallback_versions: &[String],
) -> Option<&'a V> {
    compatible(versions, accepted_loaders, game_version, fallback_versions).into_iter().next()
}

/// True when `candidate` is a different and newer version than the one installed.
pub fn is_newer(candidate: Option<(&str, &str)>, installed: Option<(&str, &str)>) -> bool {
    // each pair is (id, date_published)
    let Some((cid, cdate)) = candidate.filter(|(id, _)| !id.is_empty()) else { return false };
    let Some((iid, idate)) = installed.filter(|(id, _)| !id.is_empty()) else { return true };
    if cid == iid {
        return false;
    }
    cdate > idate
}

fn loader_fits<V: VersionLike>(v: &V, accepted: &[String]) -> bool {
    if accepted.is_empty() {
        return true;
    }
    v.loaders().iter().any(|l| accepted.contains(l))
}

fn made_for<V: VersionLike>(v: &V, game_version: Option<&str>) -> bool {
    match game_version {
        None => true,
        Some(g) => v.game_versions().iter().any(|x| x == g),
    }
}

fn made_for_any<V: VersionLike>(v: &V, versions: &[String]) -> bool {
    !versions.is_empty() && v.game_versions().iter().any(|g| versions.contains(g))
}

/// Raw JSON helpers shared by the modules that keep Modrinth objects as they came from the web.
pub fn json_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_style_predicates() {
        let m = |r: &str, v: &str| range_matches(Some(r), Some(v));
        assert_eq!(m(">=1.21 <1.22", "1.21.4"), Some(true));
        assert_eq!(m(">=1.21 <1.22", "1.22"), Some(false));
        assert_eq!(m("~1.20.1", "1.20.4"), Some(true));
        assert_eq!(m("~1.20.1", "1.21"), Some(false));
        assert_eq!(m("1.21.x", "1.21.11"), Some(true));
        assert_eq!(m("1.21.x", "26.3"), Some(false));
        assert_eq!(m("*", "26.3"), Some(true));
        assert_eq!(m("1.20", "1.20.0"), Some(true));
        assert_eq!(m("^1.20.1", "1.20.6"), Some(true));
        assert_eq!(m("^1.20.1", "1.21"), Some(false));
        assert_eq!(m(">=26.1", "26.3"), Some(true));
        assert_eq!(m("1.19.2 || >=1.20", "1.20.1"), Some(true));
        let list = vec!["1.20.1".to_string(), "1.20.4".to_string()];
        assert_eq!(range_matches_any(&list, "1.20.4"), Some(true));
        assert_eq!(range_matches_any(&list, "1.21"), Some(false));
    }

    #[test]
    fn maven_ranges() {
        let m = |r: &str, v: &str| range_matches(Some(r), Some(v));
        assert_eq!(m("[1.20.1,1.21)", "1.20.4"), Some(true));
        assert_eq!(m("[1.20.1,1.21)", "1.21"), Some(false));
        assert_eq!(m("[1.20.1]", "1.20.1"), Some(true));
        assert_eq!(m("[1.20.1]", "1.20.2"), Some(false));
        assert_eq!(m("[1.21,)", "26.3"), Some(true));
        assert_eq!(m("(1.20,1.20.4]", "1.20"), Some(false));
        assert_eq!(m("[1.16,1.17),[1.18,1.19)", "1.18.2"), Some(true));
    }

    #[test]
    fn unreadable_means_unknown() {
        assert_eq!(range_matches(Some("${minecraft_version}"), Some("1.20.1")), None);
        assert_eq!(range_matches(Some(">=1.20"), Some("25w14a")), None);
        assert_eq!(range_matches(None, Some("1.20.1")), None);
        assert_eq!(range_matches(Some(""), Some("1.20.1")), None);
        assert_eq!(range_matches(Some(">=1.21-alpha.24.10.a"), Some("1.21.4")), Some(true));
    }

    #[derive(Clone)]
    struct V {
        id: String,
        date: String,
        kind: Option<String>,
        loaders: Vec<String>,
        games: Vec<String>,
    }
    impl VersionLike for V {
        fn id(&self) -> &str {
            &self.id
        }
        fn date_published(&self) -> &str {
            &self.date
        }
        fn version_type(&self) -> Option<&str> {
            self.kind.as_deref()
        }
        fn loaders(&self) -> &[String] {
            &self.loaders
        }
        fn game_versions(&self) -> &[String] {
            &self.games
        }
    }
    fn version(id: &str, date: &str, kind: &str, loaders: &[&str], games: &[&str]) -> V {
        V {
            id: id.into(),
            date: date.into(),
            kind: Some(kind.into()),
            loaders: loaders.iter().map(|s| s.to_string()).collect(),
            games: games.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn picks_the_newest_release_for_the_exact_version_and_loader() {
        let versions = vec![
            version("old", "2026-01-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
            version("new", "2026-06-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
            version("neo", "2026-07-01T00:00:00Z", "release", &["neoforge"], &["26.3"]),
            version("other-mc", "2026-08-01T00:00:00Z", "release", &["fabric"], &["26.2"]),
        ];
        assert_eq!(pick(&versions, &s(&["fabric"]), Some("26.3"), &[]).unwrap().id, "new");
        assert_eq!(pick(&versions, &s(&["neoforge"]), Some("26.3"), &[]).unwrap().id, "neo");
    }

    #[test]
    fn prefers_a_release_over_a_newer_beta() {
        let versions = vec![
            version("beta", "2026-09-01T00:00:00Z", "beta", &["fabric"], &["26.3"]),
            version("release", "2026-05-01T00:00:00Z", "release", &["fabric"], &["26.3"]),
        ];
        assert_eq!(pick(&versions, &s(&["fabric"]), Some("26.3"), &[]).unwrap().id, "release");
        assert_eq!(pick(&versions[..1], &s(&["fabric"]), Some("26.3"), &[]).unwrap().id, "beta");
    }

    #[test]
    fn quilt_accepts_fabric_builds() {
        let quilt = accepted_loaders(Some(QUILT), "26.3", &[]);
        let versions = vec![version("f", "2026-05-01T00:00:00Z", "release", &["fabric"], &["26.3"])];
        assert_eq!(pick(&versions, &quilt, Some("26.3"), &[]).unwrap().id, "f");
        assert!(pick(&versions, &s(&["forge"]), Some("26.3"), &[]).is_none());
    }

    #[test]
    fn nearby_versions_only_when_allowed() {
        let versions = vec![version("near", "2026-05-01T00:00:00Z", "release", &["fabric"], &["26.2"])];
        assert!(pick(&versions, &s(&["fabric"]), Some("26.2.1"), &[]).is_none());
        assert_eq!(pick(&versions, &s(&["fabric"]), Some("26.2.1"), &s(&["26.2", "26.2.1"])).unwrap().id, "near");
    }

    #[test]
    fn packs_ignore_loaders() {
        let versions = vec![version("shader", "2026-05-01T00:00:00Z", "release", &["iris", "optifine"], &["26.3"])];
        assert_eq!(pick(&versions, &[], Some("26.3"), &[]).unwrap().id, "shader");
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
        assert!(compatible::<V>(&[], &s(&["fabric"]), Some("26.3"), &[]).is_empty());
    }

    #[test]
    fn tells_when_a_version_is_newer() {
        let installed = ("a", "2026-01-01T00:00:00Z");
        let newer = ("b", "2026-02-01T00:00:00Z");
        assert!(is_newer(Some(newer), Some(installed)));
        assert!(!is_newer(Some(installed), Some(newer)));
        assert!(!is_newer(Some(("a", "2026-01-01T00:00:00Z")), Some(installed)));
        assert!(!is_newer(None, Some(installed)));
        assert!(is_newer(Some(newer), None));
    }

    #[test]
    fn reads_the_loader_of_an_instance() {
        assert_eq!(loader_key("Fabric"), Some(FABRIC));
        assert_eq!(loader_key("Quilt"), Some(QUILT));
        assert_eq!(loader_key("NeoForge"), Some(NEOFORGE));
        assert_eq!(loader_key("Forge"), Some(FORGE));
        assert_eq!(loader_key("LegacyFabric"), Some(LEGACY_FABRIC));
        assert_eq!(loader_key("Paper"), None);
    }

    #[test]
    fn quilt_runs_fabric_mods_and_neoforge_sometimes_runs_forge_mods() {
        assert_eq!(accepted_loaders(Some("quilt"), "26.3", &[]), vec!["quilt", "fabric"]);
        assert_eq!(accepted_loaders(Some("fabric"), "26.3", &[]), vec!["fabric"]);
        assert_eq!(accepted_loaders(Some("neoforge"), "1.20.1", &s(&["1.20.1"])), vec!["neoforge", "forge"]);
        assert_eq!(accepted_loaders(Some("neoforge"), "1.21.1", &s(&["1.20.1"])), vec!["neoforge"]);
        assert!(accepted_loaders(None, "26.3", &[]).is_empty());
    }

    #[test]
    fn finds_shader_loaders_by_project_or_file_name() {
        assert!(has_shader_loader([IRIS], []));
        assert!(has_shader_loader([], ["iris-fabric-1.8.0.jar"]));
        assert!(has_shader_loader([], ["OptiFine_1.20.1_HD_U_I6.jar"]));
        assert!(has_shader_loader([], ["oculus-mc1.20.1-1.7.0.jar"]));
        assert!(!has_shader_loader([SODIUM], ["sodium-fabric-0.6.jar"]));
    }

    #[test]
    fn says_what_shaders_need_per_loader() {
        assert_eq!(shader_setup(Some(FABRIC), true), ShaderSetup::Ready);
        assert_eq!(shader_setup(Some(FABRIC), false), ShaderSetup::AddIris);
        assert_eq!(shader_setup(Some(QUILT), false), ShaderSetup::AddIris);
        assert_eq!(shader_setup(Some(NEOFORGE), false), ShaderSetup::AddIris);
        assert_eq!(shader_setup(Some(FORGE), false), ShaderSetup::AddOculus);
        assert_eq!(shader_setup(None, false), ShaderSetup::NeedsModLoader);
        assert_eq!(shader_setup(Some(LEGACY_FABRIC), false), ShaderSetup::NeedsModLoader);
    }
}
