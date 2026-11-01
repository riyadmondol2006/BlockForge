//! Store package verification (BlockForge's own; Xodus and BedrockOnLinux check nothing here).
//!
//! The package comes over plain http (its CDN hosts have no certificate of their own), and the catalogue's `FileHash`
//! and `HashOfHashes` are empty, so nothing the catalogue says covers its bytes. What does: the package's own hash
//! tree (`msixvc::integrity`), whose root hash is in its header. The header is fetched over HTTPS from the CDN's TLS
//! edge, which serves the catalogue's hosts by name, and must agree with the catalogue's identity of the package;
//! every page that comes over http is then checked against the tree as it arrives (header, tree, region table,
//! metadata, every file, encrypted or not), and any mismatch refuses the install.
use super::*;
use msixvc::integrity::{self, HEADER_LEN, Plan};
use reqwest::header::{CONTENT_RANGE, HOST, RANGE};

/// The package's header, over HTTPS. Tries each of the catalogue's hosts; an install can't go on without it.
pub(super) async fn header(client: &reqwest::Client, package: &catalog::Package) -> Result<Box<[u8; HEADER_LEN]>> {
    let mut last = String::from("the catalogue names no CDN host");
    for host in &package.origin.hosts {
        match fetch(client, &package.origin, host, package.bytes).await {
            Ok(header) => return Ok(header),
            Err(e) => last = format!("{host}: {e}"),
        }
    }
    Err(with_cause(
        ErrorKind::Network,
        "The Store package's header couldn't be fetched over HTTPS from Microsoft's CDN, so the download can't be \
         verified; nothing was installed",
        last,
    ))
}

async fn fetch(
    client: &reqwest::Client,
    origin: &catalog::Origin,
    host: &str,
    total: u64,
) -> std::result::Result<Box<[u8; HEADER_LEN]>, String> {
    use futures::StreamExt;
    let response = client
        .get(format!("{}{}", origin.edge, origin.path))
        .header(HOST, host)
        .header(RANGE, format!("bytes=0-{}", HEADER_LEN - 1))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let range = response.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if range != format!("bytes 0-{}/{total}", HEADER_LEN - 1) {
        return Err("not the package's first pages".into());
    }
    if response.content_length().is_some_and(|l| l != HEADER_LEN as u64) {
        return Err("unexpected length".into());
    }
    let mut body = Vec::with_capacity(HEADER_LEN);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if body.len() + chunk.len() > HEADER_LEN {
            return Err("unexpected length".into());
        }
        body.extend_from_slice(&chunk);
    }
    let header: Box<[u8; HEADER_LEN]> = body.into_boxed_slice().try_into().map_err(|_| "short header".to_string())?;
    Ok(header)
}

/// The layout of the package `header` describes, once the header agrees with what the catalogue said over HTTPS:
/// its content, its build (version and package ID) and its size.
pub(super) fn plan(header: &[u8; HEADER_LEN], package: &catalog::Package) -> Result<Plan> {
    let plan = Plan::new(header).map_err(refused)?;
    let h = plan.header();
    if h.vduid.to_string() != catalog::content_id(package.build.edition) {
        return Err(refused("another content ID"));
    }
    if h.package_version.to_string() != package.build.version {
        return Err(refused(format!("version {}, not {}", h.package_version, package.build.version)));
    }
    // the catalogue's package ID ends in the build's ID (the header's PDUID) when it has one
    if let Some(id) = package.build.package_id.rsplit('.').next().and_then(|id| uuid::Uuid::parse_str(id).ok())
        && id != h.pduid
    {
        return Err(refused("another build ID"));
    }
    if plan.len() != package.bytes {
        return Err(refused("another size"));
    }
    Ok(plan)
}

/// A package that failed verification: nothing of it is kept.
pub(super) fn refused(cause: impl std::fmt::Display) -> BedrockError {
    with_cause(
        ErrorKind::Integrity,
        "The Store package failed verification against its hash tree and was not used; something on the network \
         changed it. Try again later, on a network you trust",
        cause,
    )
}

/// An error from reading the package: a verification refusal says so, anything else is invalid package data.
pub(super) fn package_error(error: Box<dyn std::error::Error>) -> BedrockError {
    match integrity::find(&*error) {
        Some(refusal) => refused(refusal),
        None => super::integrity(error),
    }
}

/// [`package_error`] for an I/O error.
pub(super) fn io_package_error(error: std::io::Error) -> BedrockError {
    package_error(Box::new(error))
}
