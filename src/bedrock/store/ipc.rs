//! Private Store helper wire contract. Shared verbatim with crates/bedrock-auth.
//! Length-prefixed JSON, version 2, maximum 128 KiB. Never Debug/log payloads.
//! Version 2 added the Xbox step: after the Store sign-in, the same window (and so the same login.live.com session)
//! is sent to Xbox's own authorisation page, and the code it redirects to comes back here.
//! Each program uses its own half of this file (the helper the page rules, BlockForge the framing and the URLs it may
//! open), so neither compiler sees every item used.
#![allow(dead_code)]
use serde::{Deserialize, Serialize};
pub const VERSION: u32 = 2;
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
    /// Parent: open this page. The first one is always [`login_url`].
    Open {
        url: String,
    },
    /// Helper: the DA fields of a finished Store leg.
    Credentials {
        data: serde_json::Value,
    },
    Cancelled,
    Complete,
    Failed,
    /// Parent: go to Xbox's authorisation page ([`xbox_authorize_url`]) with this `state`, and send back what it
    /// redirects to. May come first, for a window that signs in to Xbox only (the Store is already signed in).
    XboxAuthorize {
        state: String,
        login_hint: Option<String>,
    },
    /// Helper: the authorisation code of the redirect, whose `state` was the one asked for. A credential: never
    /// logged, shown or kept by the helper.
    XboxCode {
        code: String,
        state: String,
    },
    /// Helper: the redirect came back without a code (Microsoft's OAuth error code, or one of ours:
    /// `state_mismatch`, `no_code`). The parent falls back to the device code.
    XboxUnavailable {
        error: String,
    },
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
/// The Store sign-in page, with the BlockForge account's sign-in name filled in when there is one.
pub fn login_url(login_hint: Option<&str>) -> String {
    match login_hint.filter(|h| valid_hint(h)) {
        Some(hint) => format!("{LOGIN}&username={}", encode_component(hint)),
        None => LOGIN.to_string(),
    }
}
/// Whether `url` is [`login_url`] for some hint: the helper's first page can be nothing else.
pub fn is_login_url(url: &str) -> bool {
    match url.strip_prefix(LOGIN) {
        Some("") => true,
        Some(rest) => rest.strip_prefix("&username=").is_some_and(|hint| {
            !hint.is_empty() && decode_component(hint).is_some_and(|h| valid_hint(&h) && encode_component(&h) == hint)
        }),
        None => false,
    }
}
/// A sign-in name worth prefilling: an e-mail address of sane length, nothing that could be read as markup.
pub fn valid_hint(hint: &str) -> bool {
    (3..=254).contains(&hint.len())
        && hint.contains('@')
        && hint.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '<' | '>' | '"' | '\\' | '\'' | '&' | '#'))
}

/// WineGDK's XUser app on login.live.com (the same as the device-code sign-in's, `xbox/device_code.rs`), its
/// scope, and the desktop redirect Microsoft gives public clients. The code is exchanged for the refresh token by
/// the parent, at oauth20_token.srf, exactly like the device code.
pub const XBOX_CLIENT_ID: &str = "0000000048183522";
pub const XBOX_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
pub const XBOX_AUTHORIZE: &str = "https://login.live.com/oauth20_authorize.srf";
pub const XBOX_REDIRECT: &str = "https://login.live.com/oauth20_desktop.srf";
/// Xbox's authorisation page in the window. No `prompt=none`: with the Store sign-in's session it answers at once
/// when the app has been allowed before, and otherwise asks the player to allow it right there, which `none` would
/// turn into an error and a device code.
pub fn xbox_authorize_url(state: &str, login_hint: Option<&str>) -> String {
    let mut url = format!(
        "{XBOX_AUTHORIZE}?client_id={XBOX_CLIENT_ID}&scope={}&response_type=code&redirect_uri={}&state={}",
        encode_component(XBOX_SCOPE),
        encode_component(XBOX_REDIRECT),
        encode_component(state)
    );
    if let Some(hint) = login_hint.filter(|h| valid_hint(h)) {
        url.push_str(&format!("&login_hint={}", encode_component(hint)));
    }
    url
}
/// A `state` the parent may send: random and URL-safe, so it needs no escaping and can't smuggle anything.
pub fn valid_state(state: &str) -> bool {
    (16..=128).contains(&state.len()) && state.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
/// Whether `url` is the desktop redirect (exact scheme, host and path; any query or fragment).
pub fn is_xbox_redirect(url: &str) -> bool {
    url.strip_prefix(XBOX_REDIRECT).is_some_and(|rest| rest.is_empty() || rest.starts_with(['?', '#']))
}
/// What the window hands back when it reaches the desktop redirect: the code if `state` matches and Microsoft
/// gave one, otherwise why not. `None` for any other page (the window just keeps going). The code is only ever
/// put in the message, never in a log.
pub fn xbox_capture(url: &str, state: &str) -> Option<Message> {
    if !is_xbox_redirect(url) {
        return None;
    }
    let rest = &url[XBOX_REDIRECT.len()..];
    // A code flow answers in the query; a fragment is read too, in case Microsoft ever answers there.
    let (query, fragment) = match rest.split_once('#') {
        Some((q, f)) => (q, f),
        None => (rest, ""),
    };
    let query = query.strip_prefix('?').unwrap_or(query);
    let pairs = query.split('&').chain(fragment.split('&')).filter_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        Some((decode_component(k)?, decode_component(v)?))
    });
    let (mut code, mut got_state, mut error) = (None, None, None);
    for (k, v) in pairs {
        match k.as_str() {
            "code" if code.is_none() => code = Some(v),
            "state" if got_state.is_none() => got_state = Some(v),
            "error" if error.is_none() => error = Some(v),
            _ => {}
        }
    }
    let unavailable = |e: &str| Some(Message::XboxUnavailable { error: e.to_string() });
    if got_state.as_deref() != Some(state) || !valid_state(state) {
        return unavailable("state_mismatch");
    }
    if let Some(e) = error {
        // Only the bare OAuth code (`consent_required`, `access_denied`, ...); never the description.
        let e: String = e.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(64).collect();
        return unavailable(if e.is_empty() { "error" } else { &e });
    }
    match code.filter(|c| !c.is_empty() && c.len() <= 4096 && c.bytes().all(|b| b.is_ascii_graphic())) {
        Some(code) => Some(Message::XboxCode { code, state: state.to_string() }),
        None => unavailable("no_code"),
    }
}
/// Percent-encode everything but RFC 3986's unreserved characters.
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
/// Percent- and plus-decode a query component; `None` for broken escapes or text that isn't UTF-8.
pub fn decode_component(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
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
/// those leaves a challenge that can never be solved. The page's own IPC messages are checked against the page
/// the window is on, which WebKit reports for the main frame only, so this does not stop a frame from these
/// domains from posting a message: what keeps frame script from signing anyone in is the nonce in
/// [`credentials_from_page`], which only the helper's own evaluated script knows.
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
/// Script that posts only [`DA_FIELDS`] of ServerData back through the IPC bridge, with `nonce` in the message.
/// The nonce is random per evaluation and is never put anywhere a page can read it before the script runs (the
/// script text is evaluated in the main frame; a frame from another origin cannot see it), so a message that
/// carries it was posted by this script and not by something else on the page. `None` for a nonce that is not
/// plain letters and digits (it is pasted into JavaScript).
pub fn read_server_data(nonce: &str) -> Option<String> {
    if !valid_state(nonce) {
        return None;
    }
    let fields = DA_FIELDS.map(|f| format!("{f:?}:ServerData[{f:?}]")).join(",");
    Some(format!(
        "if (typeof ServerData !== 'undefined' && ServerData.sDAToken) window.ipc.postMessage(JSON.stringify({{\"nonce\":\"{nonce}\",{fields}}}))"
    ))
}
/// What the helper forwards for a message the page posted: `Credentials` with exactly [`DA_FIELDS`] (each a
/// string) when `body` came from the page `uri`, carries `nonce` (the one [`read_server_data`] was last given)
/// and that nonce is still unused; nothing otherwise. Nothing the page posts can become an Xbox message: those
/// are built from the window's own navigation, never from page script.
pub fn credentials_from_page(body: &str, uri: &str, nonce: Option<&str>) -> Option<Message> {
    if body.len() > LIMIT || !allowed_url(uri) || !credential_page(uri) {
        return None;
    }
    let nonce = nonce.filter(|n| valid_state(n))?;
    let data: serde_json::Value = serde_json::from_str(body).ok()?;
    if data.get("nonce")?.as_str()? != nonce {
        return None;
    }
    let fields: serde_json::Map<String, serde_json::Value> = DA_FIELDS
        .iter()
        .map(|f| Some((f.to_string(), serde_json::Value::String(data.get(*f)?.as_str()?.into()))))
        .collect::<Option<_>>()?;
    Some(Message::Credentials { data: fields.into() })
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
