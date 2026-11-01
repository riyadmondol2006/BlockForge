//! Private Store helper wire contract. Shared verbatim with crates/bedrock-auth.
//! Length-prefixed JSON, version 1, maximum 128 KiB. Never Debug/log payloads.
//! Each program uses its own half of this file (the helper the page rules, BlockForge the framing and the URLs it may
//! open), so neither compiler sees every item used.
#![allow(dead_code)]
use serde::{Deserialize, Serialize};
pub const VERSION: u32 = 1;
pub const LIMIT: usize = 128 * 1024;
pub const LOGIN: &str = "https://login.live.com/ppsecure/InlineLogin.srf?id=80604&scid=3&mkt=en-US&Platform=Windows10&clientid=000000004424da1f&hosted=1";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub version: u32,
    pub message: Message,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Message {
    Open { url: String },
    Credentials { data: serde_json::Value },
    Cancelled,
    Complete,
    Failed,
}
pub fn encode(message: Message) -> std::io::Result<Vec<u8>> {
    let data = serde_json::to_vec(&Frame { version: VERSION, message })?;
    if data.len() > LIMIT {
        return Err(std::io::Error::other("Oversized Store IPC frame"));
    }
    let mut frame = (data.len() as u32).to_be_bytes().to_vec();
    frame.extend(data);
    Ok(frame)
}
pub fn decode(bytes: &[u8]) -> std::io::Result<Message> {
    if bytes.len() > LIMIT {
        return Err(std::io::Error::other("Oversized Store IPC frame"));
    }
    let frame: Frame = serde_json::from_slice(bytes)?;
    if frame.version != VERSION {
        return Err(std::io::Error::other("Unsupported Store IPC version"));
    }
    Ok(frame.message)
}
fn https_host(url: &str) -> Option<&str> {
    // reqwest and helper both use the url crate, but keep this schema dependency-free.
    // A user-info or port part leaves '@' or ':' in the host, which no rule accepts.
    let host = url.strip_prefix("https://")?.split(['/', '?', '#']).next()?;
    host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-').then_some(host)
}
/// Pages the helper may be told to open, and the only pages whose IPC it listens to.
/// Exact HTTPS hosts. No suffix matching, custom schemes, user-info or arbitrary ports.
pub fn allowed_url(url: &str) -> bool {
    matches!(
        https_host(url),
        Some("login.live.com" | "account.live.com" | "signup.live.com" | "login.microsoftonline.com")
    )
}
/// Where the sign-in page may navigate, frames included (WebKitGTK asks the same policy
/// for subframes). Microsoft's account pages pull in their CDNs, the account site and the
/// human-verification challenges (Arkose Labs, HUMAN "press and hold") in frames; blocking
/// those leaves a challenge that can never be solved. Credentials still come only from
/// login.live.com, so a frame from these domains can show content but never sign in.
pub fn allowed_navigation(url: &str) -> bool {
    if matches!(url, "about:blank" | "about:srcdoc") {
        return true;
    }
    https_host(url).is_some_and(|host| {
        [
            "live.com",
            "microsoft.com",
            "microsoftonline.com",
            "msauth.net",
            "msftauth.net",
            "arkoselabs.com",
            "hsprotect.net",
        ]
        .iter()
        .any(|domain| host == *domain || host.strip_suffix(domain).is_some_and(|rest| rest.ends_with('.')))
    })
}
/// The page that finishes a sign-in leg; its ServerData holds the DA token.
pub fn credential_origin(url: &str) -> bool {
    url.strip_prefix("https://login.live.com/ppsecure/post.srf")
        .is_some_and(|s| s.is_empty() || s.starts_with(['?', '#']))
}
/// Credentials are accepted from any login.live.com page: the second leg may hand them
/// over from a page other than post.srf, and upstream accepts them from anywhere.
pub fn credential_page(url: &str) -> bool {
    https_host(url) == Some("login.live.com")
}
/// The ServerData fields the parent needs (Xodus DAProperty), and nothing else: ServerData
/// carries the whole page state and can be larger than an IPC frame.
pub const DA_FIELDS: [&str; 7] =
    ["sDAToken", "sDASessionKey", "sDAStartTime", "sDAExpires", "sSTSInlineFlowToken", "sSigninName", "K"];
/// Script that posts only [`DA_FIELDS`] of ServerData back through the IPC bridge.
pub fn read_server_data() -> String {
    let fields = DA_FIELDS.map(|f| format!("{f:?}:ServerData[{f:?}]")).join(",");
    format!(
        "if (typeof ServerData !== 'undefined' && ServerData.sDAToken) window.ipc.postMessage(JSON.stringify({{{fields}}}))"
    )
}
/// Headers used for both hosted legs (BOL patch 0004).
pub fn headers(correlation: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "cxh-capabilities",
            r#"{"PrivatePropertyBag":1,"PasswordlessConnect":1,"PreferAssociate":1,"ChromelessUI":0}"#.into(),
        ),
        ("cxh-correlationId", correlation.into()),
        ("cxh-msaBinaryVersion", "55".into()),
        ("cxh-identityClientBinaryVersion", "3".into()),
        ("cxh-osVersionInfo", r#"{"platformId":2,"majorVersion":10,"minorVersion":0,"buildNumber":26100}"#.into()),
        ("cxh-platform", "CloudExperienceHost.Platform.DESKTOP".into()),
        ("cxh-protocol", "TokenBroker".into()),
        ("cxh-source", "TokenBroker".into()),
        ("hostApp", "CloudExperienceHost".into()),
    ]
}
pub fn host_context(context: &str) -> String {
    let body = serde_json::json!({"type":"callback", "value":{"name":"CloudExperienceHost.getContext", "args":["CloudExperienceHost", "TokenBroker", "TokenBroker", "{\"PrivatePropertyBag\":1,\"PasswordlessConnect\":1,\"PreferAssociate\":1,\"ChromelessUI\":0}"], "context": context}});
    format!(
        "window[\"CloudExperienceHost.Bridge.dispatchMessage\"]({});",
        serde_json::to_string(&body.to_string()).unwrap()
    )
}
