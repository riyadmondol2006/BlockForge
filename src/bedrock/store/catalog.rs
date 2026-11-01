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
}
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
    let url = format!("{root}{}", file.relative_url);
    let parsed = reqwest::Url::parse(&url).map_err(integrity)?;
    let host = parsed.host_str().unwrap_or("");
    // Keep the scheme the catalogue gives. The package CDN roots are plain http and their
    // hosts answer TLS with the CDN's own certificate (a248.e.akamai.net), so upgrading to
    // https fails every download. The URL comes from the authenticated catalogue over https,
    // the package size is checked against it, and the content key comes from the licence.
    if !matches!(parsed.scheme(), "http" | "https")
        || !(host == "xboxlive.com" || host.ends_with(".xboxlive.com"))
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !parsed.path().to_ascii_lowercase().ends_with(".msixvc")
        || !parsed.path().to_ascii_lowercase().contains(content_id(edition))
    {
        return Err(integrity("Unexpected Xbox CDN URL"));
    }
    Ok(Package {
        build: BuildId { edition, version, package_id: details.version_id },
        url: parsed.to_string(),
        bytes: file.file_size.try_into().map_err(integrity)?,
    })
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
