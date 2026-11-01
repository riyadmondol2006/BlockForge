//! Ported from Xodus CLI package.rs and BedrockOnLinux bol/config.py.
use super::*;
use xodus::{
    models::packagespc::{PackageDetails, PackageResponse},
    tokens::TokenManager,
};
pub struct Package {
    pub build: BuildId,
    pub url: String,
    pub bytes: u64,
    /// Where the package's header is fetched over HTTPS to verify the download (see `verify`).
    pub origin: Origin,
}
/// The package's header over HTTPS: the catalogue's CDN hosts have no TLS certificate of their own, but the CDN's
/// TLS edge (Akamai's, its certificate for `a248.e.akamai.net`) serves them by name.
#[derive(Clone, Debug)]
pub struct Origin {
    /// The TLS edge, `TLS_EDGE` outside tests.
    pub edge: String,
    /// The catalogue's CDN hosts that serve the file at `path`, in its order.
    pub hosts: Vec<String>,
    /// The file's path on them, as the catalogue gives it.
    pub path: String,
}
pub const TLS_EDGE: &str = "https://a248.e.akamai.net";
pub fn content_id(edition: Edition) -> &'static str {
    match edition {
        Edition::Release => "7792d9ce-355a-493c-afbd-768f4a77c3b0",
        Edition::Preview => "98bd2335-9b01-4e4c-bd05-ccc01614078b",
    }
}
pub async fn latest(
    client: &reqwest::Client,
    tokens: &TokenManager,
    edition: Edition,
    cancel: &Cancel,
) -> Result<Package> {
    interrupt(cancel, async {
        let ticket = license::user_ticket(client, tokens, "user.auth.xboxlive.com").await?;
        let user = xodus::api::xbox::authenticate_xbox_user(client, ticket).await.map_err(network)?;
        let xsts = xodus::api::xbox::request_xsts_token(client, user.token, "http://update.xboxlive.com")
            .await
            .map_err(network)?;
        let uhs = xsts.user_hash().ok_or_else(|| integrity("Missing user hash"))?;
        let authorization = format!("XBL3.0 x={uhs};{}", xsts.token);
        resolve(client, "https://packagespc.xboxlive.com", authorization, edition).await
    })
    .await
}
pub async fn resolve(
    client: &reqwest::Client,
    endpoint: &str,
    authorization: String,
    edition: Edition,
) -> Result<Package> {
    let response = client
        .get(format!("{endpoint}/GetBasePackage/{}", content_id(edition)))
        .header("x-xbl-contract-version", "3")
        .header("Authorization", authorization)
        .send()
        .await
        .map_err(network)?
        .error_for_status()
        .map_err(network)?;
    match response.json::<PackageResponse>().await.map_err(integrity)? {
        PackageResponse::Found(details) if details.package_found => package(details, edition),
        _ => Err(err(ErrorKind::NotEntitled, "Store package unavailable for this account")),
    }
}
fn package(details: PackageDetails, edition: Edition) -> Result<Package> {
    if !details.content_id.eq_ignore_ascii_case(content_id(edition)) {
        return Err(integrity("Unexpected content identity"));
    }
    if !super::download::valid_package_id(&details.version_id) {
        return Err(integrity(format!("Invalid package version ID {:?}", details.version_id)));
    }
    let Some(version) = numeric_version(&details.version) else {
        return Err(integrity("Invalid version"));
    };
    let file = details
        .package_files
        .iter()
        .find(|f| f.file_name.ends_with(".msixvc"))
        .ok_or_else(|| integrity("No MSIXVC file"))?;
    let root = file.cdn_root_paths.first().ok_or_else(|| integrity("Missing CDN URL"))?;
    let parsed = cdn_url(root, &file.relative_url, edition).ok_or_else(|| integrity("Unexpected Xbox CDN URL"))?;
    // Keep the scheme the catalogue gives. The package CDN roots are plain http and their hosts answer TLS with the
    // CDN's own certificate (a248.e.akamai.net), so upgrading to https fails every download. What comes over http is
    // checked page by page against the package's hash tree, whose root is in the header fetched over HTTPS from the
    // CDN's TLS edge by the same hosts' names (see `verify`).
    let mut hosts = Vec::new();
    for root in file.cdn_root_paths.iter().chain(&file.background_cdn_root_paths) {
        if let Some(other) = cdn_url(root, &file.relative_url, edition)
            && other.path() == parsed.path()
            && let Some(host) = other.host_str()
            && !hosts.iter().any(|h| h == host)
        {
            hosts.push(host.to_string());
        }
    }
    let origin = Origin { edge: TLS_EDGE.into(), hosts, path: parsed.path().to_string() };
    Ok(Package {
        build: BuildId { edition, version, package_id: details.version_id },
        url: parsed.to_string(),
        bytes: file.file_size.try_into().map_err(integrity)?,
        origin,
    })
}

/// A catalogue root and relative URL as a package URL on an Xbox CDN host, or None when it is anything else.
fn cdn_url(root: &str, relative: &str, edition: Edition) -> Option<reqwest::Url> {
    let parsed = reqwest::Url::parse(&format!("{root}{relative}")).ok()?;
    let host = parsed.host_str().unwrap_or("");
    let path = parsed.path().to_ascii_lowercase();
    (matches!(parsed.scheme(), "http" | "https")
        && (host == "xboxlive.com" || host.ends_with(".xboxlive.com"))
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && path.ends_with(".msixvc")
        && path.contains(content_id(edition)))
    .then_some(parsed)
}

/// The four numbers of a catalogue version: it gives `1.26.6005.0.<uuid>` (the build's number, then the package's id).
pub(super) fn numeric_version(version: &str) -> Option<String> {
    let parts: Vec<&str> = version.splitn(5, '.').collect();
    if parts.len() < 4 || !parts[..4].iter().all(|p| !p.is_empty() && p.parse::<u32>().is_ok()) {
        return None;
    }
    if let Some(rest) = parts.get(4) {
        uuid::Uuid::parse_str(rest).ok()?;
    }
    Some(parts[..4].join("."))
}
