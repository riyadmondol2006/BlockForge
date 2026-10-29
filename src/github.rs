//! Mods published only on GitHub: reading a repository's releases from a link the player pastes, and choosing the
//! file of a release that fits an instance. GitHub lets a computer ask 60 times an hour without signing in, which
//! is plenty for adding a mod now and then but not for searching.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const API: &str = "https://api.github.com";

/// A repository, and the release a link pointed at (none for "the newest").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoLink {
    pub owner: String,
    pub repo: String,
    pub tag: Option<String>,
}

impl RepoLink {
    pub fn page_url(&self) -> String {
        format!("https://github.com/{}/{}/releases", self.owner, self.repo)
    }
}

fn valid_part(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// Reads a link to a repository or one of its releases: `owner/repo`, `github.com/owner/repo`,
/// `https://github.com/owner/repo/releases/tag/v1.2`, with or without `.git` at the end.
pub fn parse_link(input: &str) -> Option<RepoLink> {
    let text = input.trim().trim_end_matches('/');
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text)
        .trim_start_matches("www.");
    let path = match rest.strip_prefix("github.com/") {
        Some(p) => p,
        // a bare `owner/repo`
        None if !rest.contains('.') || rest.matches('/').count() == 1 && !rest.contains(':') => rest,
        None => return None,
    };
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let (owner, repo) = (*parts.first()?, parts.get(1)?.trim_end_matches(".git"));
    if !valid_part(owner) || !valid_part(repo) {
        return None;
    }
    let tag = match parts.get(2..) {
        Some(["releases", "tag", tag, ..]) => Some(tag.to_string()),
        _ => None,
    };
    Some(RepoLink { owner: owner.to_string(), repo: repo.to_string(), tag })
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Release {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn title(&self) -> &str {
        self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&self.tag_name)
    }
}

async fn get<T: serde::de::DeserializeOwned>(url: &str) -> Result<Option<T>> {
    let r = crate::http::client()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("Couldn't reach GitHub. Check your internet connection")?;
    let status = r.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        bail!("GitHub only answers 60 questions an hour from one computer, and that's used up. Try again later.");
    }
    if !status.is_success() {
        bail!("GitHub answered {status}. Try again in a little while.");
    }
    Ok(Some(r.json().await.context("Couldn't read GitHub's answer")?))
}

/// The releases of the repository with files in them, newest first; just the one when the link named it.
pub async fn releases(link: &RepoLink) -> Result<Vec<Release>> {
    let (o, r) = (&link.owner, &link.repo);
    let list: Vec<Release> = match &link.tag {
        Some(tag) => {
            let tag: String = url_part(tag);
            get::<Release>(&format!("{API}/repos/{o}/{r}/releases/tags/{tag}")).await?.into_iter().collect()
        }
        None => get(&format!("{API}/repos/{o}/{r}/releases?per_page=15"))
            .await?
            .ok_or_else(|| anyhow::anyhow!("There is no public repository at github.com/{o}/{r}. Check the link."))?,
    };
    let list: Vec<Release> =
        list.into_iter().filter(|rel| !rel.draft && rel.assets.iter().any(|a| usable(&a.name))).collect();
    if list.is_empty() {
        bail!("github.com/{o}/{r} has no release with a .jar or .zip file to add");
    }
    Ok(list)
}

fn url_part(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Whether a release file could go into an instance: a jar or zip that isn't source code or a developer build.
pub fn usable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".jar") || lower.ends_with(".zip"))
        && !["-sources", "-javadoc", "-dev.", "-dev-", "-api.", "source code"].iter().any(|w| lower.contains(w))
        && crate::packutil::safe_file_name(name)
}

const LOADERS: [&str; 4] = ["neoforge", "fabric", "quilt", "forge"];

/// The loader a file name says it is for, if any ("neoforge" is not read as "forge").
fn loader_in(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    LOADERS.into_iter().find(|l| lower.match_indices(l).any(|(i, _)| *l != "forge" || !lower[..i].ends_with("neo")))
}

/// How well a file fits an instance by its name: its loader and its Minecraft version. Higher is better.
fn fit(name: &str, loader: Option<&str>, mc: &str) -> i32 {
    let mut score = 0;
    match (loader_in(name), loader) {
        (Some(l), Some(want)) if l == want => score += 4,
        // a Quilt instance runs Fabric mods too
        (Some("fabric"), Some("quilt")) => score += 2,
        (Some(_), _) => score -= 4,
        (None, _) => {}
    }
    if !mc.is_empty() && mentions(name, mc) {
        score += 2;
    }
    score
}

/// The release offered first: the newest full release (not a test version) that names Minecraft `mc` in its title
/// or files, else the newest full release, else the newest.
pub fn preferred_release(list: &[Release], mc: &str) -> usize {
    let names_mc = |r: &Release| {
        !mc.is_empty()
            && (mentions(r.title(), mc) || mentions(&r.tag_name, mc) || r.assets.iter().any(|a| mentions(&a.name, mc)))
    };
    list.iter()
        .position(|r| !r.prerelease && names_mc(r))
        .or_else(|| list.iter().position(|r| !r.prerelease))
        .unwrap_or(0)
}

/// Whether `text` names version `mc`, and not a longer one that starts the same (`1.21` is not in `1.21.1`).
fn mentions(text: &str, mc: &str) -> bool {
    text.match_indices(mc).any(|(i, _)| {
        let next = text[i + mc.len()..].chars().next();
        let before = text[..i].chars().next_back();
        !matches!(next, Some(c) if c.is_ascii_digit() || c == '.' && text[i + mc.len() + 1..].starts_with(|d: char| d.is_ascii_digit()))
            && !matches!(before, Some(c) if c.is_ascii_digit() || c == '.')
    })
}

/// The usable files of a release, the one that fits `loader` (`fabric`, `neoforge`, ...) and `mc` best first.
pub fn ranked_assets(release: &Release, loader: Option<&str>, mc: &str) -> Vec<Asset> {
    let mut v: Vec<Asset> = release.assets.iter().filter(|a| usable(&a.name)).cloned().collect();
    v.sort_by_key(|a| std::cmp::Reverse(fit(&a.name, loader, mc)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(o: &str, r: &str, tag: Option<&str>) -> Option<RepoLink> {
        Some(RepoLink { owner: o.into(), repo: r.into(), tag: tag.map(String::from) })
    }

    #[test]
    fn links_are_read_in_every_usual_shape() {
        assert_eq!(parse_link("CaffeineMC/sodium"), link("CaffeineMC", "sodium", None));
        assert_eq!(parse_link("https://github.com/CaffeineMC/sodium"), link("CaffeineMC", "sodium", None));
        assert_eq!(parse_link("github.com/CaffeineMC/sodium.git/"), link("CaffeineMC", "sodium", None));
        assert_eq!(parse_link("https://www.github.com/a/b/releases"), link("a", "b", None));
        assert_eq!(
            parse_link("https://github.com/a/b/releases/tag/mc1.21.1-0.6.0?x=1"),
            link("a", "b", Some("mc1.21.1-0.6.0"))
        );
        assert_eq!(parse_link("https://github.com/a/b/tree/main/src"), link("a", "b", None));
        assert_eq!(parse_link("https://gitlab.com/a/b"), None);
        assert_eq!(parse_link("https://github.com/a"), None);
        assert_eq!(parse_link("../etc/passwd"), None);
        assert_eq!(parse_link(""), None);
    }

    #[test]
    fn only_files_that_can_go_into_an_instance_are_offered() {
        assert!(usable("sodium-fabric-0.6.0+mc1.21.1.jar"));
        assert!(usable("MyPack.zip"));
        assert!(!usable("sodium-0.6.0-sources.jar"));
        assert!(!usable("mod-1.0-dev.jar"));
        assert!(!usable("installer.exe"));
        assert!(!usable("../evil.jar"));
    }

    #[test]
    fn the_file_for_the_instance_comes_first() {
        let rel = Release {
            tag_name: "v1".into(),
            assets: ["mod-forge-1.20.1.jar", "mod-neoforge-1.21.1.jar", "mod-fabric-1.21.1.jar", "mod-sources.jar"]
                .iter()
                .map(|n| Asset { name: n.to_string(), ..Default::default() })
                .collect(),
            ..Default::default()
        };
        let names = |l, mc| ranked_assets(&rel, l, mc).into_iter().map(|a| a.name).collect::<Vec<_>>();
        assert_eq!(names(Some("neoforge"), "1.21.1")[0], "mod-neoforge-1.21.1.jar");
        assert_eq!(names(Some("forge"), "1.20.1")[0], "mod-forge-1.20.1.jar");
        assert_eq!(names(Some("quilt"), "1.21.1")[0], "mod-fabric-1.21.1.jar");
        assert_eq!(names(Some("fabric"), "1.21.1").len(), 3, "the sources are left out");
        assert_eq!(loader_in("thing-NeoForge.jar"), Some("neoforge"));
        assert_eq!(loader_in("thing-forge.jar"), Some("forge"));
    }

    #[test]
    fn the_release_for_the_instance_is_offered_first() {
        let rel = |title: &str, pre: bool| Release { tag_name: title.into(), prerelease: pre, ..Default::default() };
        let list = [rel("v22 for 26.4", true), rel("v21 for 26.3", false), rel("v11.0.3 for 1.21.1", false)];
        assert_eq!(preferred_release(&list, "1.21.1"), 2);
        assert_eq!(preferred_release(&list, "1.20.1"), 1, "none for it: the newest full release");
        assert_eq!(preferred_release(&list[..1], "1.21.1"), 0);
        assert!(mentions("mod-1.21.jar", "1.21"));
        assert!(!mentions("mod-1.21.1.jar", "1.21"));
        assert!(!mentions("mod-11.21.jar", "1.21"));
        assert!(mentions("mod+mc1.21.1.jar", "1.21.1"));
    }
}
