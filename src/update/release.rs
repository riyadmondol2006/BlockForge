//! The parts of a GitHub release (from the releases API) that the updater needs.

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub html_url: String,
    /// The release notes, in Markdown.
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Asset {
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    /// `sha256:<hex>` when GitHub has computed it for the file.
    #[serde(default)]
    pub digest: Option<String>,
}

impl Asset {
    pub fn new(name: &str, url: &str, size: u64) -> Asset {
        Asset { name: name.to_string(), browser_download_url: url.to_string(), size, digest: None }
    }

    /// The expected SHA-256 in lower case hex, or `None` when the release doesn't say.
    pub fn sha256(&self) -> Option<String> {
        let digest = self.digest.as_deref()?.trim();
        let (algorithm, hex) = digest.split_once(':')?;
        let hex = hex.trim().to_ascii_lowercase();
        (algorithm.eq_ignore_ascii_case("sha256") && is_sha256_hex(&hex)).then_some(hex)
    }
}

/// Whether `text` is exactly 64 hex digits.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}
