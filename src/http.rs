//! One shared HTTP client, plus a small on-disk cache for the JSON documents we ask for again and again.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static CONFIG: OnceLock<crate::net::NetConfig> = OnceLock::new();

/// Apply the proxy, timeout and connection limit from the settings. Call it once at start-up, before the first
/// request; afterwards the client is already built and this returns `false` and changes nothing.
pub fn configure(config: crate::net::NetConfig) -> bool {
    CLIENT.get().is_none() && CONFIG.set(config).is_ok()
}

/// How many files the downloader fetches at once.
pub fn concurrency() -> usize {
    CONFIG.get().map_or(crate::net::DEFAULT_CONCURRENCY, |c| c.concurrency)
}

/// The client is built once and its connections are kept, so the second request to a host skips the handshake.
pub fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| crate::net::build_client(CONFIG.get()))
}

pub fn sha1_hex(data: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// A GET with retries for the failures that go away on their own (see `net::get_bytes_with_retry`).
pub async fn get_bytes(url: &str) -> Result<Vec<u8>> {
    crate::net::get_bytes_with_retry(url).await
}

pub async fn get_json<T: DeserializeOwned>(url: &str) -> Result<T> {
    let bytes = get_bytes(url).await?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading the answer from {url}"))
}

/// Like [`get_json`], but answers from `cache_dir` while the saved copy is younger than `ttl`, and falls back to
/// an older copy when the network is down, so the launcher still opens offline.
pub async fn get_json_cached<T: DeserializeOwned>(url: &str, cache_dir: &Path, ttl: Duration) -> Result<T> {
    let file = cache_dir.join(format!("{}.json", sha1_hex(url.as_bytes())));
    let age = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok());
    if let Some(age) = age {
        if age < ttl {
            if let Ok(parsed) =
                std::fs::read(&file).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice(&b)?))
            {
                return Ok(parsed);
            }
        }
    }
    match get_bytes(url).await {
        Ok(bytes) => {
            let parsed: T = serde_json::from_slice(&bytes).with_context(|| format!("reading the answer from {url}"))?;
            let _ = std::fs::create_dir_all(cache_dir);
            let _ = std::fs::write(&file, &bytes);
            Ok(parsed)
        }
        Err(e) => {
            if age.is_some() {
                if let Ok(bytes) = std::fs::read(&file) {
                    if let Ok(parsed) = serde_json::from_slice(&bytes) {
                        return Ok(parsed);
                    }
                }
            }
            Err(e)
        }
    }
}
