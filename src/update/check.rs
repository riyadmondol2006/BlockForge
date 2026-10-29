//! Asking GitHub for the latest release, and the rules for when a newer one is worth showing.

use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::plan::{current_version, is_newer};
use super::release::Release;

/// Where the latest release is read from.
pub const LATEST_RELEASE_API: &str = "https://api.github.com/repos/riyadmondol2006/BlockForge/releases/latest";
/// The environment variable that points the check somewhere else (a test server).
pub const API_OVERRIDE: &str = "BLOCKFORGE_UPDATE_API";
/// Wait this long after start so the check never competes with loading.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(8);
/// How often a running launcher looks again.
pub const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// The address the latest release is read from.
pub fn api_url() -> String {
    std::env::var(API_OVERRIDE)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| LATEST_RELEASE_API.to_string())
}

/// What a check found.
#[derive(Clone, Debug)]
pub enum Found {
    /// The project has no published release yet.
    NoRelease,
    /// The newest release is not newer than this copy.
    UpToDate,
    /// A newer release.
    Newer(Box<Release>),
}

/// Decides what a release means for the running version. Drafts, pre-releases and tags that aren't versions are
/// never offered.
pub fn judge(release: Release, current: &[u32; 4]) -> Found {
    if release.draft || release.prerelease || release.tag_name.is_empty() {
        return Found::UpToDate;
    }
    if is_newer(&release.tag_name, current) { Found::Newer(Box::new(release)) } else { Found::UpToDate }
}

/// Asks GitHub for the latest release and compares it with the running version.
pub async fn check() -> Result<Found> {
    let response = crate::http::client()
        .get(api_url())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("Couldn't reach GitHub to look for a new version")?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(Found::NoRelease);
    }
    if !status.is_success() {
        bail!("GitHub answered {status} when asked for the newest version. Try again in a little while.");
    }
    let release: Release = response.json().await.context("Couldn't read GitHub's answer about the newest version")?;
    let found = judge(release, &current_version());
    if let Found::Newer(r) = &found {
        crate::logs::debug(format!("Latest release on GitHub is {} (newer)", r.tag_name));
    }
    Ok(found)
}

/// Remembers which release was already offered while the launcher is open, so a later check doesn't nag again.
#[derive(Default, Debug)]
pub struct Offers {
    offered: Option<String>,
}

impl Offers {
    /// Whether the background check should show `tag` now: not when the player skipped that version, and not when
    /// it was already shown in this session. Saying yes counts as showing it.
    pub fn should_offer(&mut self, tag: &str, skipped: Option<&str>) -> bool {
        if skipped == Some(tag) || self.offered.as_deref() == Some(tag) {
            return false;
        }
        self.offered = Some(tag.to_string());
        true
    }

    /// A check the player asked for always shows its result; it still counts as shown.
    pub fn mark_offered(&mut self, tag: &str) {
        self.offered = Some(tag.to_string());
    }
}

/// What the About page shows about updates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateStatus {
    /// No check has finished or started yet.
    #[default]
    NotChecked,
    Checking,
    UpToDate,
    /// A newer version, like `4.1.0`.
    Available(String),
    /// Why the last check failed, in words a player can read.
    Error(String),
}

impl UpdateStatus {
    /// One line for the About page.
    pub fn text(&self) -> String {
        match self {
            UpdateStatus::NotChecked => "Not checked yet".to_string(),
            UpdateStatus::Checking => "Checking for updates...".to_string(),
            UpdateStatus::UpToDate => "You have the latest version".to_string(),
            UpdateStatus::Available(v) => format!("BlockForge {v} is available"),
            UpdateStatus::Error(e) => format!("Couldn't check for updates: {e}"),
        }
    }

    pub fn is_checking(&self) -> bool {
        *self == UpdateStatus::Checking
    }

    pub fn is_error(&self) -> bool {
        matches!(self, UpdateStatus::Error(_))
    }

    /// The newer version, when one was found.
    pub fn available(&self) -> Option<&str> {
        match self {
            UpdateStatus::Available(v) => Some(v),
            _ => None,
        }
    }
}
