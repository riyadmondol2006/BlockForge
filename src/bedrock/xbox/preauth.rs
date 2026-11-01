//! The Xbox Live chain, run on the host before the game starts, and the payload WineGDK reads instead of making
//! those calls itself (`WINEGDK_PREAUTH_DEVICE`).
//!
//! Ported from BedrockOnLinux bol/auth.py (`xbl_preauth` and its helpers), MIT, with credit. It has to run on the
//! host: Azure resets every *.auth.xboxlive.com and SISU connection made through Wine's GnuTLS, while the same
//! requests from the host succeed.
//!
//! The chain: device token (proof of possession), user token (`RpsTicket=t=<access token>`), an optional
//! user-only XSTS token for achievements, then one token per audience the game needs: the Xbox profile
//! (`http://xboxlive.com`), PlayFab, multiplayer, Realms and licensing. Each is asked of SISU first (what a real
//! GDK title does, and the only path with modern gamertag claims) and, when SISU refuses but XSTS is usable for
//! this account, of XSTS (device-bound, then user-only). Only a complete, unexpired chain is written; a failed
//! refresh keeps a complete unexpired payload from before and never replaces it with part of one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::cache::{self, Epoch};
use super::signature::{DeviceKey, ProofKey};
use super::{Secret, Xbox, blocking, now_unix};

pub(super) const TITLE_APP_ID: &str = "0000000048183522";
pub(super) const XBOXLIVE_RP: &str = "http://xboxlive.com";
pub(super) const PLAYFAB_RP: &str = "https://b980a380.minecraft.playfabapi.com/";
pub(super) const MULTIPLAYER_RP: &str = "https://multiplayer.minecraft.net/";
pub(super) const REALMS_RP: &str = "https://pocket.realms.minecraft.net/";
pub(super) const LICENSING_RP: &str = "http://licensing.xboxlive.com";
const AUTH_RP: &str = "http://auth.xboxlive.com";
const USER_AGENT: &str = "XAL Xbox Live Game (Windows; SDK; 1.0.0.0)";

/// How much validity a cached payload must have left for a Play to reuse it instead of running the whole chain
/// again: enough to cover a play session, far short of the hours Xbox issues tokens for.
pub(super) const REUSE_MARGIN: i64 = 1800;
/// The least validity any payload handed to the game may have.
pub(super) const MIN_TTL: i64 = 60;

// ---------------------------------------------------------------------------------------------------------------
// Diagnostics: only whitelisted, credential-free facts about a failure are kept.

/// Why the game starts without Xbox Live. The order of `priority` decides which failure is reported when several
/// stages fail: the most actionable one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Category {
    /// No Xbox account is linked to this profile.
    NotLinked,
    Incomplete,
    Service,
    Network,
    Local,
    Session,
    Account,
    SignIn,
    Age,
}

impl Category {
    fn priority(self) -> u8 {
        match self {
            Category::NotLinked => 0,
            Category::Incomplete => 10,
            Category::Service => 20,
            Category::Network => 30,
            Category::Local | Category::Session => 40,
            Category::Account | Category::SignIn => 50,
            Category::Age => 60,
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Category::NotLinked => "No Xbox account is signed in for this profile. Sign in to Xbox to play online.",
            Category::Age => {
                "Xbox Live requires age or family-account verification. Review the account's birth date, family \
                 membership and Xbox privacy settings, then sign out of Xbox and sign in again."
            }
            Category::Account => {
                "Xbox Live rejected this Microsoft account. Sign in on xbox.com, finish creating or verifying the \
                 Xbox profile and accept any requested terms, then sign out of Xbox and sign in again."
            }
            Category::SignIn => {
                "Microsoft no longer accepts the Xbox sign-in saved for this profile, so Xbox Live could not be \
                 asked for a session. Sign out of Xbox, sign in again, then press Play."
            }
            Category::Network => {
                "Xbox Live could not be reached. Check the Internet connection, DNS, VPN or proxy and firewall, \
                 then try again."
            }
            Category::Local => "Xbox Live support is incomplete in this installation. Run Repair, then try again.",
            Category::Session => {
                "The Xbox account changed while Xbox Live was being prepared. Start the game again with the current \
                 account."
            }
            Category::Service => {
                "Xbox Live returned an unexpected response. Try again later; if it continues, turn on diagnostics \
                 and include the stage named here."
            }
            Category::Incomplete => {
                "Xbox Live did not provide every token required for multiplayer. Verify the Xbox profile and \
                 account settings, then try again."
            }
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Category::NotLinked => "not-linked",
            Category::Age => "age",
            Category::Account => "account",
            Category::SignIn => "signin",
            Category::Network => "network",
            Category::Local => "local",
            Category::Session => "session",
            Category::Service => "service",
            Category::Incomplete => "incomplete",
        }
    }
}

/// These Xbox errors have an explicit age/family-account resolution. Kept numeric: response bodies and their
/// free-form messages can contain credentials and are never kept.
const AGE_ERRORS: [u32; 3] = [
    2_148_916_236, // Adult verification is required.
    2_148_916_237, // Adult verification is unavailable for this account.
    2_148_916_238, // A child account must be added to a Microsoft family.
];
const ACCOUNT_ERRORS: [u32; 3] = [
    2_148_916_233, // The Microsoft account has no Xbox profile.
    2_148_916_234,
    2_148_916_235, // Xbox Live is unavailable for the account or region.
];

/// Classify a refused Xbox request from its status and its numeric `XErr`, keeping nothing else of the body.
pub(super) fn classify(status: u16, body: Option<&Value>) -> (Category, Option<u32>) {
    let code = body.and_then(Value::as_object).and_then(|o| {
        let lowered: Vec<(String, &Value)> = o.iter().map(|(k, v)| (k.to_lowercase(), v)).collect();
        let find = |name: &str| lowered.iter().find(|(k, _)| k == name).map(|(_, v)| *v);
        match find("xerr").filter(|v| !v.is_null()).or_else(|| find("xerrcode"))? {
            Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
            Value::String(s) => {
                if let Some(h) = s.strip_prefix("0x") {
                    (!h.is_empty() && h.len() <= 8 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                        .then(|| u32::from_str_radix(h, 16).ok())
                        .flatten()
                } else {
                    (!s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| s.parse::<u64>().ok().and_then(|n| u32::try_from(n).ok()))
                        .flatten()
                }
            }
            _ => None,
        }
    });
    let category = if code.is_some_and(|c| AGE_ERRORS.contains(&c)) {
        Category::Age
    } else if code.is_some_and(|c| ACCOUNT_ERRORS.contains(&c))
        || matches!(status, 400 | 401)
        || (status == 403 && code.is_some())
    {
        // A real Xbox Live rejection always names an XErr; a bare 403 is an edge or policy refusal instead, and
        // sending people to xbox.com to repair a healthy profile only wastes their time.
        Category::Account
    } else if matches!(status, 408 | 425 | 429) || status >= 500 {
        Category::Network
    } else {
        Category::Service
    };
    (category, code)
}

/// The one failure worth reporting from a chain, with the stage it happened at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offline {
    pub stage: &'static str,
    pub category: Category,
    pub http_status: Option<u16>,
    pub error_code: Option<u32>,
}

impl Offline {
    pub(crate) fn not_linked() -> Offline {
        Offline { stage: "account", category: Category::NotLinked, http_status: None, error_code: None }
    }

    pub(crate) fn message(&self) -> &'static str {
        self.category.message()
    }

    /// The degraded-mode notice the window shows: it starts anyway, without Xbox Live.
    pub(crate) fn notice(&self) -> String {
        let mut text = if self.category == Category::NotLinked {
            "No Xbox account is signed in for this profile.".to_string()
        } else {
            format!("Could not prepare a complete Xbox Live session (stage: {}). {}", self.stage, self.message())
        };
        text.push_str(
            " Minecraft starts without Xbox Live: single-player and LAN worlds work; servers, Realms, the \
             Marketplace and friends need it.",
        );
        text
    }
}

#[derive(Default)]
struct Diagnostics(Option<Offline>);

impl Diagnostics {
    fn record(&mut self, stage: &'static str, category: Category, http_status: Option<u16>, error_code: Option<u32>) {
        if self.0.as_ref().is_none_or(|p| category.priority() > p.category.priority()) {
            self.0 = Some(Offline { stage, category, http_status, error_code });
        }
    }

    fn take(&mut self, fallback_stage: &'static str) -> Offline {
        self.0.take().unwrap_or(Offline {
            stage: fallback_stage,
            category: Category::Service,
            http_status: None,
            error_code: None,
        })
    }
}

/// What each step of a chain did, for the setup log and the real-sign-in check: names, statuses and paths only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StageResult {
    Ok {
        stage: &'static str,
        via: &'static str,
    },
    Failed {
        stage: &'static str,
        category: Category,
        http_status: Option<u16>,
        error_code: Option<u32>,
    },
    /// A complete unexpired payload from before was used.
    Cached,
}

impl std::fmt::Display for StageResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageResult::Ok { stage, via } => write!(f, "{stage}: ok ({via})"),
            StageResult::Failed { stage, category, http_status, error_code } => {
                write!(f, "{stage}: failed ({}", category.as_str())?;
                if let Some(s) = http_status {
                    write!(f, ", HTTP {s}")?;
                }
                if let Some(c) = error_code {
                    write!(f, ", XErr {c}")?;
                }
                f.write_str(")")
            }
            StageResult::Cached => f.write_str("cached payload: reused"),
        }
    }
}

/// Whether the game gets Xbox Live this time. Online carries the path of a payload that was validated for the
/// current account, never a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OnlineReadiness {
    Online { payload: PathBuf, gamertag: Option<String> },
    Offline(Offline),
}

impl OnlineReadiness {
    pub(crate) fn is_online(&self) -> bool {
        matches!(self, OnlineReadiness::Online { .. })
    }

    /// `WINEGDK_PREAUTH_DEVICE` for the game, only when online.
    pub(crate) fn wine_env(&self) -> Option<(&'static str, String)> {
        match self {
            OnlineReadiness::Online { payload, .. } => Some((super::PREAUTH_ENV, super::wine_path(payload))),
            OnlineReadiness::Offline(_) => None,
        }
    }

    pub(crate) fn notice(&self) -> Option<String> {
        match self {
            OnlineReadiness::Online { .. } => None,
            OnlineReadiness::Offline(o) => Some(o.notice()),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The payload: WineGDK's `device.json`.

/// The online payload exactly as WineGDK's xgameruntime reads it (see BedrockOnLinux's native5 patches): string
/// fields, the ISO `NotAfter` expiries the launcher validates, and the decimal-string `*_epoch` copies WineGDK
/// itself parses. Absent required values are written as `null`, as the original does; optional claims are left
/// out entirely, so their absence means "not provided", never "granted".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Payload {
    /// Not a secret: the account generation this payload belongs to.
    #[serde(rename = "_account_epoch")]
    pub account_epoch: Option<Epoch>,
    pub device_id: Option<String>,
    pub ecc_private_blob_b64: Option<String>,
    pub device_token: Option<String>,
    pub device_token_expiry: Option<String>,
    pub user_token: Option<String>,
    pub user_token_expiry: Option<String>,
    pub xbl_token: Option<String>,
    pub xbl_token_expiry: Option<String>,
    pub xbl_xuid: Option<String>,
    pub xbl_gamertag: Option<String>,
    pub xbl_age_group: Option<String>,
    pub xbl_uhs: Option<String>,
    pub sisu_rp: Option<String>,
    pub sisu_token: Option<String>,
    pub sisu_uhs: Option<String>,
    pub sisu_expiry: Option<String>,
    pub mp_rp: Option<String>,
    pub mp_token: Option<String>,
    pub mp_uhs: Option<String>,
    pub mp_expiry: Option<String>,
    pub realms_rp: Option<String>,
    pub realms_token: Option<String>,
    pub realms_uhs: Option<String>,
    pub realms_expiry: Option<String>,
    pub lic_rp: Option<String>,
    pub lic_token: Option<String>,
    pub lic_uhs: Option<String>,
    pub lic_expiry: Option<String>,
    pub obtained: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achievements_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achievements_uhs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achievements_expiry: Option<String>,
    /// Modern gamertag components are separate SISU claims: GDK callers size each one's buffer on its own, and
    /// handing the classic tag back for the suffix can fail XblContext setup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xbl_modern_gamertag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xbl_modern_gamertag_suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xbl_unique_modern_gamertag: Option<String>,
    /// Present only when Xbox sent a `prv` claim. Present-but-empty means no privileges were granted and WineGDK
    /// fails closed; absent (an older payload) keeps its compatibility fallback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xbl_privileges: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_token_expiry_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xbl_token_expiry_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achievements_expiry_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sisu_expiry_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mp_expiry_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realms_expiry_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lic_expiry_epoch: Option<String>,
}

/// Parse an Xbox `NotAfter`: RFC 3339 with up to seven fractional digits (100 ns), or without an offset (UTC).
pub(super) fn parse_expiry(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.timestamp());
    }
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S%.f"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(s, f).ok())
        .map(|t| t.and_utc().timestamp())
}

fn present(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.is_empty())
}

impl Payload {
    /// Missing or expiring fields, as short credential-free phrases. Empty means complete for `min_ttl` seconds.
    pub(super) fn problems(&self, now: i64, min_ttl: i64) -> Vec<String> {
        let required: [(&str, &Option<String>, Option<(&str, &Option<String>)>); 13] = [
            ("device_token", &self.device_token, Some(("device_token_expiry", &self.device_token_expiry))),
            ("user_token", &self.user_token, Some(("user_token_expiry", &self.user_token_expiry))),
            ("xbl_token", &self.xbl_token, Some(("xbl_token_expiry", &self.xbl_token_expiry))),
            ("sisu_token", &self.sisu_token, Some(("sisu_expiry", &self.sisu_expiry))),
            ("sisu_rp", &self.sisu_rp, None),
            ("sisu_uhs", &self.sisu_uhs, None),
            ("mp_token", &self.mp_token, Some(("mp_expiry", &self.mp_expiry))),
            ("mp_rp", &self.mp_rp, None),
            ("mp_uhs", &self.mp_uhs, None),
            ("realms_token", &self.realms_token, Some(("realms_expiry", &self.realms_expiry))),
            ("realms_rp", &self.realms_rp, None),
            ("realms_uhs", &self.realms_uhs, None),
            ("xbl_xuid", &self.xbl_xuid, None),
        ];
        let mut problems = Vec::new();
        for (name, value, expiry) in required {
            if !present(value) {
                problems.push(format!("missing {name}"));
                continue;
            }
            let Some((expiry_name, raw)) = expiry else { continue };
            match raw.as_deref().filter(|s| !s.trim().is_empty()) {
                None => problems.push(format!("missing {expiry_name}")),
                Some(raw) => match parse_expiry(raw) {
                    Some(t) if t > now + min_ttl => {}
                    Some(_) => problems.push(format!("expired {name}")),
                    None => problems.push(format!("invalid {expiry_name}")),
                },
            }
        }
        problems
    }

    fn belongs_to(&self, epoch: &Epoch) -> bool {
        self.account_epoch.as_ref() == Some(epoch)
    }

    /// A copy with the optional achievements block either complete and valid or gone, and WineGDK's epoch fields
    /// derived from the ISO expiries (never kept when they would disagree with a missing or invalid source).
    pub(super) fn with_epochs(&self, now: i64) -> Payload {
        let mut p = self.clone();
        let achievements = validated_achievements(
            p.achievements_token.as_deref(),
            p.achievements_uhs.as_deref(),
            p.achievements_expiry.as_deref(),
            now,
        );
        (p.achievements_token, p.achievements_uhs, p.achievements_expiry) = match achievements {
            Some((t, u, e)) => (Some(t), Some(u), Some(e)),
            None => (None, None, None),
        };
        let epoch = |iso: &Option<String>| iso.as_deref().and_then(parse_expiry).map(|t| t.to_string());
        p.user_token_expiry_epoch = epoch(&p.user_token_expiry);
        p.xbl_token_expiry_epoch = epoch(&p.xbl_token_expiry);
        p.achievements_expiry_epoch = epoch(&p.achievements_expiry);
        p.sisu_expiry_epoch = epoch(&p.sisu_expiry);
        p.mp_expiry_epoch = epoch(&p.mp_expiry);
        p.realms_expiry_epoch = epoch(&p.realms_expiry);
        p.lic_expiry_epoch = epoch(&p.lic_expiry);
        p
    }
}

/// A complete optional achievements credential, or nothing: a token, a decimal user hash in 1..=u64::MAX (kept in
/// canonical form) and an expiry at least `MIN_TTL` away.
fn validated_achievements(
    token: Option<&str>,
    uhs: Option<&str>,
    expiry: Option<&str>,
    now: i64,
) -> Option<(String, String, String)> {
    let token = token.filter(|t| !t.trim().is_empty())?;
    let uhs = uhs.filter(|u| !u.is_empty() && u.len() <= 20 && u.bytes().all(|b| b.is_ascii_digit()))?;
    let value = uhs.parse::<u64>().ok().filter(|v| *v >= 1)?;
    let expiry = expiry.filter(|e| !e.trim().is_empty())?;
    (parse_expiry(expiry)? > now + MIN_TTL).then(|| (token.to_string(), value.to_string(), expiry.to_string()))
}

/// The `prv` claim in canonical form: unique unsigned 32-bit decimal IDs, sorted, space-separated. Xbox sends a
/// space-separated string; a list is accepted too. Anything else in it is dropped, never guessed into a grant.
pub(super) fn normalize_privileges(raw: &Value) -> Option<String> {
    let items: Vec<Value> = match raw {
        Value::String(s) => s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|p| !p.is_empty())
            .map(|p| Value::String(p.to_string()))
            .collect(),
        Value::Array(a) => a.clone(),
        _ => return None,
    };
    let mut ids: Vec<u32> = items
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
            Value::String(s) if !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit()) => {
                s.parse::<u64>().ok().and_then(|n| u32::try_from(n).ok())
            }
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    (!ids.is_empty()).then(|| ids.iter().map(u32::to_string).collect::<Vec<_>>().join(" "))
}

/// `(present, canonical)` for the claim: present-but-unusable is `Some("")`, which grants nothing.
fn privilege_claim(claims: Option<&Map<String, Value>>) -> Option<String> {
    let raw = claims?.get("prv")?;
    Some(normalize_privileges(raw).unwrap_or_default())
}

// ---------------------------------------------------------------------------------------------------------------
// The signed requests.

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct DeviceAuthBody<'a> {
    relying_party: &'a str,
    token_type: &'a str,
    properties: DeviceAuthProperties<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct DeviceAuthProperties<'a> {
    auth_method: &'a str,
    id: &'a str,
    device_type: &'a str,
    version: &'a str,
    proof_key: &'a ProofKey,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct UserAuthBody<'a> {
    relying_party: &'a str,
    token_type: &'a str,
    properties: UserAuthProperties<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct UserAuthProperties<'a> {
    auth_method: &'a str,
    site_name: &'a str,
    rps_ticket: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsBody<'a> {
    relying_party: &'a str,
    token_type: &'a str,
    properties: XstsProperties<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsProperties<'a> {
    sandbox_id: &'a str,
    user_tokens: [&'a str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    device_token: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proof_key: Option<&'a ProofKey>,
}

#[derive(Serialize)]
struct SisuBody<'a> {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "AppId")]
    app_id: &'a str,
    #[serde(rename = "deviceToken")]
    device_token: &'a str,
    #[serde(rename = "Sandbox")]
    sandbox: &'a str,
    #[serde(rename = "UseModernGamertag")]
    use_modern_gamertag: bool,
    #[serde(rename = "SiteName")]
    site_name: &'a str,
    #[serde(rename = "RelyingParty")]
    relying_party: &'a str,
    #[serde(rename = "OfferTermsAcceptance")]
    offer_terms_acceptance: bool,
    #[serde(rename = "AcceptOffers")]
    accept_offers: bool,
    #[serde(rename = "ProofKey")]
    proof_key: &'a ProofKey,
}

/// The answer to a signed request: its status and its body if that was JSON. `None` is a transport failure.
type Answer = Option<(u16, Option<Value>)>;

fn token_of(v: &Value) -> Option<String> {
    v.get("Token").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn not_after(v: &Value) -> String {
    v.get("NotAfter").and_then(Value::as_str).unwrap_or_default().to_string()
}

fn xui(v: &Value) -> Option<&Map<String, Value>> {
    v.get("DisplayClaims")?.get("xui")?.get(0)?.as_object()
}

/// A claim as text: Xbox sends strings, and a number is taken as its decimal form.
fn claim(claims: Option<&Map<String, Value>>, key: &str) -> Option<String> {
    match claims?.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => n.as_u64().map(|n| n.to_string()),
        _ => None,
    }
}

struct Chain<'a> {
    x: &'a Xbox,
    key: &'a DeviceKey,
    proof: ProofKey,
    diag: Diagnostics,
    stages: Vec<StageResult>,
}

impl Chain<'_> {
    /// Serialize once, sign those bytes, send those bytes.
    async fn post(&self, url: &str, body: &impl Serialize) -> Answer {
        let bytes = serde_json::to_vec(body).ok()?;
        let path = reqwest::Url::parse(url).ok()?.path().to_string();
        let signature = self.key.signature_header("POST", &path, &bytes)?;
        let sent = self
            .x
            .client
            .post(url)
            .timeout(self.x.timing.request_timeout)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-xbl-contract-version", "1")
            .header("Signature", signature)
            .body(bytes)
            .send()
            .await
            .ok()?;
        let status = sent.status().as_u16();
        let body = sent.bytes().await.ok()?;
        Some((status, serde_json::from_slice(&body).ok()))
    }

    /// Note a failed step; `record` decides whether it may become the reported failure.
    fn failed(&mut self, stage: &'static str, answer: &Answer, record: bool) {
        let (category, status, code) = match answer {
            None => (Category::Network, None, None),
            Some((200, _)) => (Category::Service, Some(200), None),
            Some((status, body)) => {
                let (c, code) = classify(*status, body.as_ref());
                (c, Some(*status), code)
            }
        };
        if record {
            self.diag.record(stage, category, status, code);
        }
        self.stages.push(StageResult::Failed { stage, category, http_status: status, error_code: code });
    }

    /// A 200 whose body lacks what it should carry.
    fn invalid(&mut self, stage: &'static str, record: bool) {
        self.failed(stage, &Some((200, None)), record);
    }

    /// A JSON object from a 200, or the failure noted.
    fn accepted(&mut self, stage: &'static str, answer: Answer, record: bool) -> Option<Value> {
        match answer {
            Some((200, Some(v))) if v.is_object() => Some(v),
            other => {
                self.failed(stage, &other, record);
                None
            }
        }
    }

    async fn xsts(
        &mut self,
        user_token: Option<&str>,
        device_token: &str,
        rp: &str,
        stage: &'static str,
        device_bound: bool,
        record: bool,
    ) -> Option<Value> {
        let user_token = user_token?;
        let body = XstsBody {
            relying_party: rp,
            token_type: "JWT",
            properties: XstsProperties {
                sandbox_id: "RETAIL",
                user_tokens: [user_token],
                // Bound to the same device SISU would have used, so the audience sees what a real title presents.
                device_token: device_bound.then_some(device_token),
                proof_key: device_bound.then_some(&self.proof),
            },
        };
        let answer = self.post(&self.x.endpoints.xsts, &body).await;
        self.accepted(stage, answer, record)
    }

    async fn sisu(
        &mut self,
        access: &Secret,
        device_token: &str,
        rp: &str,
        stage: &'static str,
        record: bool,
    ) -> Option<Value> {
        let body = SisuBody {
            access_token: format!("t={}", access.expose()),
            app_id: TITLE_APP_ID,
            device_token,
            sandbox: "RETAIL",
            use_modern_gamertag: true,
            site_name: "user.auth.xboxlive.com",
            relying_party: rp,
            offer_terms_acceptance: true,
            accept_offers: true,
            proof_key: &self.proof,
        };
        let answer = self.post(&self.x.endpoints.sisu, &body).await;
        self.accepted(stage, answer, record)
    }

    /// One audience's token: SISU first; when SISU refuses and XSTS is usable for this account, XSTS bound to the
    /// device, then user-only. SISU can refuse what XSTS serves for the very same account (every SISU call
    /// answered 403 while XSTS kept issuing tokens), and falling back completes the chain instead of failing it.
    async fn authorize(
        &mut self,
        access: &Secret,
        user_token: Option<&str>,
        device_token: &str,
        xsts_usable: bool,
        rp: &str,
        names: (&'static str, &'static str, &'static str),
    ) -> Option<Value> {
        let (audience, sisu_stage, xsts_stage) = names;
        if let Some(v) = self.sisu(access, device_token, rp, sisu_stage, !xsts_usable).await {
            if let Some(t) = v.get("AuthorizationToken").filter(|t| token_of(t).is_some()) {
                self.stages.push(StageResult::Ok { stage: audience, via: "SISU" });
                return Some(t.clone());
            }
            // An answer without the token is noted for the log only, as the original does: the completeness check
            // below reports the missing audience.
            self.invalid(sisu_stage, false);
        }
        if !xsts_usable {
            return None;
        }
        for device_bound in [true, false] {
            let got = self.xsts(user_token, device_token, rp, xsts_stage, device_bound, !device_bound).await;
            if let Some(v) = got.filter(|v| token_of(v).is_some()) {
                self.stages.push(StageResult::Ok {
                    stage: audience,
                    via: if device_bound { "XSTS, device-bound" } else { "XSTS, user-only" },
                });
                return Some(v);
            }
        }
        None
    }
}

/// What a chain ended with.
pub(super) struct Outcome {
    pub readiness: OnlineReadiness,
    pub stages: Vec<StageResult>,
}

fn online(x: &Xbox, payload: &Payload) -> OnlineReadiness {
    OnlineReadiness::Online { payload: x.payload_path(), gamertag: payload.xbl_gamertag.clone() }
}

/// Use `cached` as it is, after giving it WineGDK's epoch fields if it predates them. `false` when it could not
/// be kept for this account.
async fn keep_cache(x: &Xbox, cached: &Payload, epoch: &Epoch) -> bool {
    let upgraded = cached.with_epochs(now_unix());
    if &upgraded == cached {
        return true;
    }
    let (dir, epoch) = (x.dir.clone(), epoch.clone());
    matches!(blocking(move || cache::store_payload(&dir, &upgraded, &epoch)).await, Ok(Ok(true)))
}

/// When the chain can't complete: a complete payload still valid for the current account is used; otherwise the
/// game goes without Xbox Live, with the most actionable failure.
async fn fallback(
    x: &Xbox,
    cached: Option<&Payload>,
    epoch: &Epoch,
    mut diag: Diagnostics,
    stage: &'static str,
    mut stages: Vec<StageResult>,
) -> Outcome {
    let dir = x.dir.clone();
    let current = blocking(move || cache::current_epoch(&dir)).await.ok().and_then(|r| r.ok()).flatten();
    if let Some(cached) = cached {
        let usable = current.as_ref() == Some(epoch)
            && cached.belongs_to(epoch)
            && cached.problems(now_unix(), MIN_TTL).is_empty();
        if usable && keep_cache(x, cached, epoch).await {
            stages.push(StageResult::Cached);
            return Outcome { readiness: online(x, cached), stages };
        }
    }
    Outcome { readiness: OnlineReadiness::Offline(diag.take(stage)), stages }
}

/// Run the chain for the account `epoch` belongs to, with a fresh Microsoft access token (`None` when the refresh
/// failed; `refresh_unreachable` tells an offline machine from a refused sign-in).
pub(super) async fn run(x: &Xbox, access: Option<&Secret>, epoch: &Epoch, refresh_unreachable: bool) -> Outcome {
    let mut diag = Diagnostics::default();
    let stages = Vec::new();
    let dir = x.dir.clone();
    let read = blocking(move || cache::current_epoch(&dir).map(|e| (e, cache::load_payload(&dir)))).await;
    let (current, cached) = match read {
        Ok(Ok(v)) => v,
        _ => {
            diag.record("account-cache", Category::Local, None, None);
            return Outcome { readiness: OnlineReadiness::Offline(diag.take("account-cache")), stages };
        }
    };
    match current {
        None => {
            diag.record("account-cache", Category::Account, None, None);
            return Outcome { readiness: OnlineReadiness::Offline(diag.take("account-cache")), stages };
        }
        Some(c) if &c != epoch => {
            diag.record("account-cache", Category::Session, None, None);
            return Outcome { readiness: OnlineReadiness::Offline(diag.take("account-cache")), stages };
        }
        Some(_) => {}
    }
    let cached = cached.filter(|c| c.belongs_to(epoch));
    // A fresh cache is worth more than eight sequential round trips: reuse one with a session's worth left.
    if let Some(c) = cached.as_ref().filter(|c| c.problems(now_unix(), REUSE_MARGIN).is_empty()) {
        if keep_cache(x, c, epoch).await {
            return Outcome { readiness: online(x, c), stages: vec![StageResult::Cached] };
        }
    }

    let Some(access) = access.filter(|a| !a.expose().is_empty()) else {
        let category = if refresh_unreachable { Category::Network } else { Category::SignIn };
        diag.record("microsoft-token", category, None, None);
        return fallback(x, cached.as_ref(), epoch, diag, "microsoft-token", stages).await;
    };
    let dir = x.dir.clone();
    let key = match blocking(move || DeviceKey::load_or_create(&dir)).await {
        Ok(Ok(key)) => key,
        _ => {
            diag.record("device-key", Category::Local, None, None);
            return fallback(x, cached.as_ref(), epoch, diag, "device-key", stages).await;
        }
    };
    let proof = key.proof_key();
    let mut chain = Chain { x, key: &key, proof, diag, stages };

    // The device token.
    let body = DeviceAuthBody {
        relying_party: AUTH_RP,
        token_type: "JWT",
        properties: DeviceAuthProperties {
            auth_method: "ProofOfPossession",
            id: &key.device_id,
            device_type: "Win32",
            version: "10.0.22631",
            proof_key: &chain.proof,
        },
    };
    let answer = chain.post(&x.endpoints.device_auth, &body).await;
    let device = match chain.accepted("device-auth", answer, true) {
        Some(v) if token_of(&v).is_some() => v,
        other => {
            if other.is_some() {
                chain.invalid("device-auth", true);
            }
            let Chain { diag, stages, .. } = chain;
            return fallback(x, cached.as_ref(), epoch, diag, "device-auth", stages).await;
        }
    };
    chain.stages.push(StageResult::Ok { stage: "device-auth", via: "device" });
    let device_token = token_of(&device).unwrap_or_default();

    // The user token. A failure is noted and the chain goes on: the payload check below decides.
    let body = UserAuthBody {
        relying_party: AUTH_RP,
        token_type: "JWT",
        properties: UserAuthProperties {
            auth_method: "RPS",
            site_name: "user.auth.xboxlive.com",
            rps_ticket: format!("t={}", access.expose()),
        },
    };
    let answer = chain.post(&x.endpoints.user_auth, &body).await;
    let user = match chain.accepted("user-auth", answer, true) {
        Some(v) if token_of(&v).is_some() => {
            chain.stages.push(StageResult::Ok { stage: "user-auth", via: "RPS" });
            Some(v)
        }
        Some(_) => {
            chain.invalid("user-auth", true);
            None
        }
        None => None,
    };
    let user_token = user.as_ref().and_then(token_of);
    let user_ref = user_token.as_deref();

    // Optional user-only XSTS for achievements. Its outcome also says whether an XSTS fallback is worth asking:
    // offline or a rejected user token means it is not.
    let achievements =
        chain.xsts(user_ref, &device_token, XBOXLIVE_RP, "xsts-achievements", false, true).await.unwrap_or_default();
    let xsts_usable = token_of(&achievements).is_some();
    if xsts_usable {
        chain.stages.push(StageResult::Ok { stage: "achievements", via: "XSTS, user-only" });
    }

    let profile = chain
        .authorize(
            access,
            user_ref,
            &device_token,
            xsts_usable,
            XBOXLIVE_RP,
            ("profile", "sisu-profile", "xsts-profile"),
        )
        .await
        .unwrap_or_default();
    let playfab = chain
        .authorize(
            access,
            user_ref,
            &device_token,
            xsts_usable,
            PLAYFAB_RP,
            ("playfab", "sisu-playfab", "xsts-playfab"),
        )
        .await
        .unwrap_or_default();
    // Joining a server needs this audience even when pinging one already works.
    let multiplayer = chain
        .authorize(
            access,
            user_ref,
            &device_token,
            xsts_usable,
            MULTIPLAYER_RP,
            ("multiplayer", "sisu-multiplayer", "xsts-multiplayer"),
        )
        .await
        .unwrap_or_default();
    // Realms still validates the canonical legacy Bedrock audience.
    let realms = chain
        .authorize(access, user_ref, &device_token, xsts_usable, REALMS_RP, ("realms", "sisu-realms", "xsts-realms"))
        .await
        .unwrap_or_default();
    // The Marketplace's catalog and entitlement endpoints need this one.
    let licensing = chain
        .authorize(
            access,
            user_ref,
            &device_token,
            xsts_usable,
            LICENSING_RP,
            ("licensing", "sisu-licensing", "xsts-licensing"),
        )
        .await
        .unwrap_or_default();

    let now = now_unix();
    let profile_claims = xui(&profile);
    // Each audience keeps its own token and its own user hash: they are not interchangeable.
    let audience = |v: &Value, rp: &str| {
        let token = token_of(v);
        (token.is_some().then(|| rp.to_string()), token, claim(xui(v), "uhs"), Some(not_after(v)))
    };
    let (sisu_rp, sisu_token, sisu_uhs, sisu_expiry) = audience(&playfab, PLAYFAB_RP);
    let (mp_rp, mp_token, mp_uhs, mp_expiry) = audience(&multiplayer, MULTIPLAYER_RP);
    let (realms_rp, realms_token, realms_uhs, realms_expiry) = audience(&realms, REALMS_RP);
    let (lic_rp, lic_token, lic_uhs, lic_expiry) = audience(&licensing, LICENSING_RP);
    let achievements_block = validated_achievements(
        token_of(&achievements).as_deref(),
        claim(xui(&achievements), "uhs").as_deref(),
        Some(not_after(&achievements).as_str()),
        now,
    );
    use base64::Engine;
    let mut out = Payload {
        account_epoch: Some(epoch.clone()),
        device_id: Some(key.device_id.clone()),
        ecc_private_blob_b64: Some(base64::engine::general_purpose::STANDARD.encode(key.bcrypt_blob())),
        device_token: Some(device_token.clone()),
        device_token_expiry: Some(not_after(&device)),
        user_token: user_token.clone(),
        user_token_expiry: user.as_ref().map(not_after),
        xbl_token: token_of(&profile),
        xbl_token_expiry: Some(not_after(&profile)),
        xbl_xuid: claim(profile_claims, "xid"),
        xbl_gamertag: claim(profile_claims, "gtg"),
        // Copied as sent, never filled in: a missing age group is not an adult.
        xbl_age_group: claim(profile_claims, "agg"),
        xbl_uhs: claim(profile_claims, "uhs"),
        sisu_rp,
        sisu_token,
        sisu_uhs,
        sisu_expiry,
        mp_rp,
        mp_token,
        mp_uhs,
        mp_expiry,
        realms_rp,
        realms_token,
        realms_uhs,
        realms_expiry,
        lic_rp,
        lic_token,
        lic_uhs,
        lic_expiry,
        obtained: Some(now),
        xbl_modern_gamertag: profile_claims.and_then(|c| c.get("mgt")?.as_str().map(str::to_string)),
        xbl_modern_gamertag_suffix: profile_claims.and_then(|c| c.get("mgs")?.as_str().map(str::to_string)),
        xbl_unique_modern_gamertag: profile_claims.and_then(|c| c.get("umg")?.as_str().map(str::to_string)),
        xbl_privileges: privilege_claim(profile_claims),
        ..Payload::default()
    };
    if let Some((t, u, e)) = achievements_block {
        (out.achievements_token, out.achievements_uhs, out.achievements_expiry) = (Some(t), Some(u), Some(e));
    }
    let out = out.with_epochs(now);
    let Chain { mut diag, mut stages, .. } = chain;
    let problems = out.problems(now, MIN_TTL);
    if !problems.is_empty() {
        diag.record("online-chain", Category::Incomplete, None, None);
        stages.push(StageResult::Failed {
            stage: "online-chain",
            category: Category::Incomplete,
            http_status: None,
            error_code: None,
        });
        return fallback(x, cached.as_ref(), epoch, diag, "online-chain", stages).await;
    }
    let (dir, e, stored) = (x.dir.clone(), epoch.clone(), out.clone());
    match blocking(move || cache::store_payload(&dir, &stored, &e)).await {
        Ok(Ok(true)) => Outcome { readiness: online(x, &out), stages },
        Ok(Ok(false)) => {
            // Signed out while this ran: neither the new payload nor the old one is the current account's.
            diag.record("account-cache", Category::Session, None, None);
            Outcome { readiness: OnlineReadiness::Offline(diag.take("account-cache")), stages }
        }
        _ => {
            diag.record("account-cache", Category::Local, None, None);
            fallback(x, cached.as_ref(), epoch, diag, "account-cache", stages).await
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::mock::{Mock, Req, Resp};
    use super::super::tests::test_xbox;
    use super::*;
    use serde_json::json;

    pub(in crate::bedrock::xbox) fn iso(offset: i64) -> String {
        (chrono::Utc::now() + chrono::Duration::seconds(offset)).to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
    }

    /// A short name for each audience, so every token and user hash in a test chain is told apart.
    fn short(rp: &str) -> &'static str {
        match rp {
            XBOXLIVE_RP => "xbl",
            PLAYFAB_RP => "pf",
            MULTIPLAYER_RP => "mp",
            REALMS_RP => "realms",
            LICENSING_RP => "lic",
            _ => "other",
        }
    }

    fn minted(rp: &str, via: &str, ttl: i64) -> Value {
        let s = short(rp);
        let mut xui = json!({"uhs": format!("{s}-uhs-{via}")});
        if rp == XBOXLIVE_RP {
            xui = json!({"uhs": format!("{s}-uhs-{via}"), "xid": "2535400000000001", "gtg": "FakeTag",
                         "agg": "Adult", "mgt": "FakeTag", "mgs": "1234", "umg": "FakeTag#1234",
                         "prv": "254 185 185 abc 99999999999"});
        }
        json!({"Token": format!("fake-{s}-token-{via}"), "NotAfter": iso(ttl), "DisplayClaims": {"xui": [xui]}})
    }

    /// Answers like Xbox Live, with knobs for what to refuse.
    #[derive(Clone, Default)]
    pub(in crate::bedrock::xbox) struct Live {
        pub sisu_status: Option<u16>,
        pub xsts_device_bound_status: Option<u16>,
        pub xsts_status: Option<u16>,
        pub refuse_rp: Option<&'static str>,
        pub device_status: Option<u16>,
        pub ttl: Option<i64>,
        pub achievements_uhs: Option<&'static str>,
    }

    impl Live {
        pub(in crate::bedrock::xbox) fn answer(&self, req: &Req) -> Resp {
            let ttl = self.ttl.unwrap_or(16 * 3600);
            let body = req.json();
            match req.path() {
                "/device/authenticate" => match self.device_status {
                    Some(s) => Resp::status(s),
                    None => Resp::json(200, json!({"Token": "fake-device-token", "NotAfter": iso(ttl)})),
                },
                "/user/authenticate" => Resp::json(
                    200,
                    json!({"Token": "fake-user-token", "NotAfter": iso(ttl),
                    "DisplayClaims": {"xui": [{"uhs": "user-uhs"}]}}),
                ),
                "/authorize" => {
                    let rp = body["RelyingParty"].as_str().unwrap().to_string();
                    if self.refuse_rp == Some(short(&rp)) {
                        return Resp::json(401, json!({"XErr": 2_148_916_233u64}));
                    }
                    match self.sisu_status {
                        Some(s) => Resp::status(s),
                        None => Resp::json(
                            200,
                            json!({"AuthorizationToken": minted(&rp, "sisu", ttl),
                            "UserToken": {"Token": "u"}, "DeviceToken": "d"}),
                        ),
                    }
                }
                "/xsts/authorize" => {
                    let rp = body["RelyingParty"].as_str().unwrap().to_string();
                    let bound = body["Properties"].get("DeviceToken").is_some();
                    if self.refuse_rp == Some(short(&rp)) {
                        return Resp::json(401, json!({"XErr": 2_148_916_233u64}));
                    }
                    if bound && let Some(s) = self.xsts_device_bound_status {
                        return Resp::status(s);
                    }
                    if let Some(s) = self.xsts_status {
                        return Resp::status(s);
                    }
                    let mut v = minted(&rp, if bound { "xsts-bound" } else { "xsts" }, ttl);
                    if let Some(u) = self.achievements_uhs.filter(|_| rp == XBOXLIVE_RP && !bound) {
                        v["DisplayClaims"]["xui"][0]["uhs"] = json!(u);
                    }
                    Resp::json(200, v)
                }
                _ => Resp::status(404),
            }
        }
    }

    pub(in crate::bedrock::xbox) async fn live(knobs: Live) -> Mock {
        Mock::start(move |req, _| knobs.answer(req)).await
    }

    async fn fresh_epoch(x: &Xbox) -> Epoch {
        cache::current_epoch(&x.dir).unwrap().unwrap()
    }

    async fn run_ok(x: &Xbox, epoch: &Epoch) -> Outcome {
        run(x, Some(&Secret::new("fake-access")), epoch, false).await
    }

    fn stored(x: &Xbox) -> Payload {
        cache::load_payload(&x.dir).unwrap()
    }

    #[tokio::test]
    async fn a_full_chain_through_sisu_writes_the_whole_payload() {
        let mock = live(Live::default()).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        let out = run_ok(&x, &epoch).await;
        assert_eq!(
            out.readiness,
            OnlineReadiness::Online { payload: x.payload_path(), gamertag: Some("FakeTag".into()) }
        );
        let p = stored(&x);
        assert!(p.problems(now_unix(), REUSE_MARGIN).is_empty());
        assert_eq!(p.account_epoch.as_ref(), Some(&epoch));
        // Every audience has its own token and its own user hash.
        assert_eq!(p.xbl_token.as_deref(), Some("fake-xbl-token-sisu"));
        assert_eq!(p.xbl_uhs.as_deref(), Some("xbl-uhs-sisu"));
        assert_eq!((p.sisu_token.as_deref(), p.sisu_uhs.as_deref()), (Some("fake-pf-token-sisu"), Some("pf-uhs-sisu")));
        assert_eq!((p.mp_token.as_deref(), p.mp_uhs.as_deref()), (Some("fake-mp-token-sisu"), Some("mp-uhs-sisu")));
        assert_eq!(
            (p.realms_token.as_deref(), p.realms_uhs.as_deref()),
            (Some("fake-realms-token-sisu"), Some("realms-uhs-sisu"))
        );
        assert_eq!((p.lic_token.as_deref(), p.lic_uhs.as_deref()), (Some("fake-lic-token-sisu"), Some("lic-uhs-sisu")));
        assert_eq!(p.sisu_rp.as_deref(), Some(PLAYFAB_RP));
        assert_eq!(p.mp_rp.as_deref(), Some(MULTIPLAYER_RP));
        assert_eq!(p.realms_rp.as_deref(), Some(REALMS_RP));
        assert_eq!(p.lic_rp.as_deref(), Some(LICENSING_RP));
        assert_eq!(p.xbl_xuid.as_deref(), Some("2535400000000001"));
        assert_eq!(p.xbl_modern_gamertag_suffix.as_deref(), Some("1234"));
        assert_eq!(p.xbl_unique_modern_gamertag.as_deref(), Some("FakeTag#1234"));
        assert_eq!(p.xbl_privileges.as_deref(), Some("185 254"));
        // The achievements user hash ("xbl-uhs-xsts") is not decimal, so that optional block is left out whole.
        assert!(p.achievements_token.is_none() && p.achievements_expiry_epoch.is_none());
        // WineGDK's epoch fields are decimal strings that agree with the ISO expiries.
        for (iso, epoch) in [
            (&p.user_token_expiry, &p.user_token_expiry_epoch),
            (&p.xbl_token_expiry, &p.xbl_token_expiry_epoch),
            (&p.sisu_expiry, &p.sisu_expiry_epoch),
            (&p.mp_expiry, &p.mp_expiry_epoch),
            (&p.realms_expiry, &p.realms_expiry_epoch),
            (&p.lic_expiry, &p.lic_expiry_epoch),
        ] {
            assert_eq!(parse_expiry(iso.as_deref().unwrap()).unwrap().to_string(), epoch.clone().unwrap());
        }
        // The blob is the device key's, and the file is private.
        let blob = base64::engine::general_purpose::STANDARD.decode(p.ecc_private_blob_b64.unwrap()).unwrap();
        assert_eq!(blob.len(), 104);
        use base64::Engine;
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(x.payload_path()).unwrap().permissions().mode() & 0o777, 0o600);
        // The raw file has every key WineGDK reads, as strings.
        let raw: Value = serde_json::from_slice(&std::fs::read(x.payload_path()).unwrap()).unwrap();
        for k in [
            "device_id",
            "ecc_private_blob_b64",
            "device_token",
            "user_token",
            "xbl_token",
            "xbl_xuid",
            "xbl_gamertag",
            "xbl_uhs",
            "xbl_age_group",
            "sisu_rp",
            "sisu_token",
            "sisu_uhs",
            "mp_rp",
            "mp_token",
            "mp_uhs",
            "realms_rp",
            "realms_token",
            "realms_uhs",
            "lic_rp",
            "lic_token",
            "lic_uhs",
            "xbl_token_expiry_epoch",
            "user_token_expiry_epoch",
            "sisu_expiry_epoch",
            "mp_expiry_epoch",
            "realms_expiry_epoch",
            "lic_expiry_epoch",
            "xbl_modern_gamertag",
            "xbl_modern_gamertag_suffix",
            "xbl_unique_modern_gamertag",
            "xbl_privileges",
        ] {
            assert!(raw[k].is_string(), "{k} is {:?}", raw[k]);
        }
        // Each SISU call named its own audience and the title's app.
        let sisu: Vec<Value> = mock.requests().iter().filter(|r| r.path() == "/authorize").map(Req::json).collect();
        let rps: Vec<&str> = sisu.iter().map(|b| b["RelyingParty"].as_str().unwrap()).collect();
        assert_eq!(rps, [XBOXLIVE_RP, PLAYFAB_RP, MULTIPLAYER_RP, REALMS_RP, LICENSING_RP]);
        assert!(sisu.iter().all(|b| b["AppId"] == TITLE_APP_ID
            && b["AccessToken"] == "t=fake-access"
            && b["deviceToken"] == "fake-device-token"
            && b["Sandbox"] == "RETAIL"
            && b["UseModernGamertag"] == true));
    }

    #[tokio::test]
    async fn every_request_is_signed_over_the_bytes_that_were_sent() {
        let mock = live(Live::default()).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        run_ok(&x, &epoch).await;
        let key = DeviceKey::load_or_create(&x.dir).unwrap();
        let (kx, ky) = key.public_xy();
        let public = [&[4u8][..], &kx, &ky].concat();
        let reqs = mock.requests();
        assert_eq!(
            reqs.len(),
            8,
            "device, user, achievements and five SISU, and nothing else: {:?}",
            reqs.iter().map(|r| r.path().to_string()).collect::<Vec<_>>()
        );
        for r in &reqs {
            let header = r.header("signature").expect("every auth request is signed");
            assert!(super::super::signature::tests::verify_header(&public, header, r.path(), &r.body), "{}", r.path());
            assert_eq!(r.header("x-xbl-contract-version"), Some("1"));
            assert_eq!(r.header("content-type"), Some("application/json"));
            assert_eq!(r.header("user-agent"), Some(USER_AGENT));
            // Compact JSON: what was signed is exactly serde's compact form of what was meant.
            let reparsed: Value = serde_json::from_slice(&r.body).unwrap();
            assert!(!String::from_utf8_lossy(&r.body).contains(": "));
            assert_eq!(serde_json::to_vec(&reparsed).unwrap().len(), r.body.len());
        }
        let device = reqs.iter().find(|r| r.path() == "/device/authenticate").unwrap();
        let proof = key.proof_key();
        let expected = format!(
            r#"{{"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT","Properties":{{"AuthMethod":"ProofOfPossession","Id":"{}","DeviceType":"Win32","Version":"10.0.22631","ProofKey":{{"alg":"ES256","crv":"P-256","kty":"EC","use":"sig","x":"{}","y":"{}"}}}}}}"#,
            key.device_id, proof.x, proof.y
        );
        assert_eq!(String::from_utf8(device.body.clone()).unwrap(), expected);
        let user = reqs.iter().find(|r| r.path() == "/user/authenticate").unwrap();
        assert_eq!(
            String::from_utf8(user.body.clone()).unwrap(),
            r#"{"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT","Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":"t=fake-access"}}"#
        );
    }

    #[test]
    fn the_device_auth_body_matches_the_fixture_byte_for_byte() {
        let key = super::super::signature::tests::fixture_key();
        let proof = key.proof_key();
        let body = DeviceAuthBody {
            relying_party: AUTH_RP,
            token_type: "JWT",
            properties: DeviceAuthProperties {
                auth_method: "ProofOfPossession",
                id: &key.device_id,
                device_type: "Win32",
                version: "10.0.22631",
                proof_key: &proof,
            },
        };
        let bytes = serde_json::to_vec(&body).unwrap();
        assert_eq!(bytes, include_bytes!("../../../tests/bedrock-fixtures/xbox/device-auth-body.json"));
    }

    #[tokio::test]
    async fn sisu_refused_falls_back_to_device_bound_xsts() {
        let mock = live(Live { sisu_status: Some(403), ..Live::default() }).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        let out = run_ok(&x, &epoch).await;
        assert!(out.readiness.is_online(), "{:?}", out.stages);
        let p = stored(&x);
        assert_eq!(p.mp_token.as_deref(), Some("fake-mp-token-xsts-bound"));
        assert_eq!(p.realms_uhs.as_deref(), Some("realms-uhs-xsts-bound"));
        assert!(out.stages.contains(&StageResult::Ok { stage: "multiplayer", via: "XSTS, device-bound" }));
        let bound: Vec<Value> = mock
            .requests()
            .iter()
            .filter(|r| r.path() == "/xsts/authorize")
            .map(Req::json)
            .filter(|b| b["Properties"].get("DeviceToken").is_some())
            .collect();
        assert_eq!(bound.len(), 5);
        assert!(bound.iter().all(|b| b["Properties"]["ProofKey"]["crv"] == "P-256"
            && b["Properties"]["UserTokens"] == json!(["fake-user-token"])
            && b["Properties"]["SandboxId"] == "RETAIL"));
    }

    #[tokio::test]
    async fn and_then_to_user_only_xsts() {
        let mock = live(Live { sisu_status: Some(403), xsts_device_bound_status: Some(400), ..Live::default() }).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        let out = run_ok(&x, &epoch).await;
        assert!(out.readiness.is_online(), "{:?}", out.stages);
        assert_eq!(stored(&x).sisu_token.as_deref(), Some("fake-pf-token-xsts"));
    }

    #[tokio::test]
    async fn without_usable_xsts_a_refused_sisu_is_the_reported_failure() {
        let mock = live(Live { sisu_status: Some(403), xsts_status: Some(401), ..Live::default() }).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        let out = run_ok(&x, &epoch).await;
        let OnlineReadiness::Offline(o) = out.readiness else { panic!("must not be online") };
        // XSTS achievements (401, no XErr: account) outranks the SISU stages' bare 403 (service).
        assert_eq!((o.stage, o.category, o.http_status), ("xsts-achievements", Category::Account, Some(401)));
        assert_eq!(mock.hits("/xsts/authorize"), 1, "no fallback is tried when XSTS already refused");
        assert!(cache::load_payload(&x.dir).is_none(), "nothing partial is written");
    }

    #[tokio::test]
    async fn an_audience_missing_from_the_chain_writes_nothing_and_keeps_a_valid_cache() {
        // First a complete chain...
        let mock = live(Live::default()).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        assert!(run_ok(&x, &epoch).await.readiness.is_online());
        let before = std::fs::read(x.payload_path()).unwrap();
        // ...that is no longer fresh enough to reuse without asking, but still valid...
        let mut aged = stored(&x);
        aged.mp_expiry = Some(iso(600));
        cache::store_payload(&x.dir, &aged.with_epochs(now_unix()), &epoch).unwrap();
        // ...then a chain where Realms refuses.
        let refusing = live(Live { refuse_rp: Some("realms"), ..Live::default() }).await;
        let x = Xbox { endpoints: std::sync::Arc::new(super::super::Endpoints::local(&refusing.base)), ..x };
        let out = run_ok(&x, &epoch).await;
        assert!(out.readiness.is_online(), "the complete cached payload still serves");
        assert_eq!(out.stages.last(), Some(&StageResult::Cached));
        assert_ne!(std::fs::read(x.payload_path()).unwrap(), before);
        assert_eq!(stored(&x).mp_expiry, aged.mp_expiry, "not replaced by the partial chain");

        // Once that cache has expired, the same refusal means no Xbox Live, and still no partial file.
        let mut expired = stored(&x);
        expired.realms_expiry = Some(iso(-10));
        cache::store_payload(&x.dir, &expired, &epoch).unwrap();
        let out = run_ok(&x, &epoch).await;
        let OnlineReadiness::Offline(o) = out.readiness else { panic!("must not be online") };
        assert_eq!(o.category, Category::Account);
        assert_eq!(stored(&x).realms_expiry, expired.realms_expiry);
    }

    #[tokio::test]
    async fn a_fresh_cache_is_reused_without_a_single_request() {
        let mock = live(Live::default()).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        run_ok(&x, &epoch).await;
        let asked = mock.total();
        let out = run(&x, None, &epoch, true).await;
        assert!(out.readiness.is_online());
        assert_eq!(out.stages, [StageResult::Cached]);
        assert_eq!(mock.total(), asked);
    }

    #[tokio::test]
    async fn a_short_lived_cache_is_refreshed_and_falls_back_when_offline() {
        let mock = live(Live { ttl: Some(900), ..Live::default() }).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        run_ok(&x, &epoch).await;
        let asked = mock.total();
        // 900 s left is under the reuse margin: Play tries the chain again...
        run_ok(&x, &epoch).await;
        assert!(mock.total() > asked);
        // ...and offline (no access token: the refresh couldn't reach Microsoft) the still-valid cache serves.
        let out = run(&x, None, &epoch, true).await;
        assert!(out.readiness.is_online());
        // Without one, the failure says "network", not "sign in again".
        std::fs::remove_file(x.payload_path()).unwrap();
        let OnlineReadiness::Offline(o) = run(&x, None, &epoch, true).await.readiness else { panic!() };
        assert_eq!((o.stage, o.category), ("microsoft-token", Category::Network));
        let OnlineReadiness::Offline(o) = run(&x, None, &epoch, false).await.readiness else { panic!() };
        assert_eq!(o.category, Category::SignIn);
    }

    #[tokio::test]
    async fn a_payload_from_a_previous_account_is_never_replayed() {
        let mock = live(Live::default()).await;
        let x = test_xbox(&mock, "pre");
        let old = fresh_epoch(&x).await;
        run_ok(&x, &old).await;
        let old_payload = stored(&x);
        cache::purge(&x.dir).unwrap();
        let new = fresh_epoch(&x).await;
        // A crash-left copy of the old account's payload, put back after the sign-out.
        cache::write_private(&x.payload_path(), &serde_json::to_vec(&old_payload).unwrap()).unwrap();
        let asked = mock.total();
        let out = run(&x, None, &new, true).await;
        assert!(!out.readiness.is_online(), "the old account's payload is not used");
        assert_eq!(mock.total(), asked);
        // A run that still holds the old epoch is refused outright.
        let OnlineReadiness::Offline(o) = run_ok(&x, &old).await.readiness else { panic!() };
        assert_eq!(o.category, Category::Session);
        // And storing for the old epoch is refused under the lock.
        assert!(!cache::store_payload(&x.dir, &old_payload, &old).unwrap());
    }

    #[tokio::test]
    async fn a_device_auth_failure_is_classified_without_keeping_the_body() {
        let mock = live(Live { device_status: Some(503), ..Live::default() }).await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        let OnlineReadiness::Offline(o) = run_ok(&x, &epoch).await.readiness else { panic!() };
        assert_eq!((o.stage, o.category, o.http_status), ("device-auth", Category::Network, Some(503)));
        assert_eq!(mock.total(), 1);
    }

    #[test]
    fn errors_are_classified_like_the_original() {
        let body = |v: Value| Some(v);
        assert_eq!(
            classify(401, body(json!({"XErr": 2_148_916_238u64})).as_ref()),
            (Category::Age, Some(2_148_916_238))
        );
        assert_eq!(
            classify(401, body(json!({"xerr": "2148916233"})).as_ref()),
            (Category::Account, Some(2_148_916_233))
        );
        assert_eq!(classify(403, body(json!({"XErrCode": "0x8015DC0C"})).as_ref()), (Category::Age, Some(0x8015_DC0C)));
        assert_eq!(classify(403, None), (Category::Service, None), "a bare 403 is not an account problem");
        assert_eq!(classify(429, None).0, Category::Network);
        assert_eq!(classify(502, body(json!({"Message": "secret t=abc"})).as_ref()), (Category::Network, None));
        assert_eq!(classify(404, body(json!({"XErr": -1})).as_ref()), (Category::Service, None));
        assert_eq!(classify(400, body(json!({"XErr": "12345678901"})).as_ref()), (Category::Account, None));
    }

    #[test]
    fn the_most_actionable_failure_is_kept() {
        let mut d = Diagnostics::default();
        d.record("a", Category::Service, None, None);
        d.record("b", Category::Age, Some(401), Some(1));
        d.record("c", Category::Network, None, None);
        assert_eq!(d.take("x").stage, "b");
    }

    #[test]
    fn expiries_parse_with_seven_digits_offsets_or_none() {
        assert_eq!(parse_expiry("2026-11-02T03:04:05.1234567Z"), Some(1_793_588_645));
        assert_eq!(parse_expiry("2026-11-02T03:04:05Z"), Some(1_793_588_645));
        assert_eq!(parse_expiry("2026-11-02T05:04:05.5+02:00"), Some(1_793_588_645));
        assert_eq!(parse_expiry("2026-11-02T03:04:05.123456"), Some(1_793_588_645));
        assert_eq!(parse_expiry(" "), None);
        assert_eq!(parse_expiry("tomorrow"), None);
    }

    #[test]
    fn privilege_claims_never_grow() {
        assert_eq!(normalize_privileges(&json!("254 185, 185\t191")).as_deref(), Some("185 191 254"));
        assert_eq!(
            normalize_privileges(&json!([254, "185", true, -1, 1.5, "4294967296", "x"])).as_deref(),
            Some("185 254")
        );
        assert_eq!(normalize_privileges(&json!("")), None);
        assert_eq!(normalize_privileges(&json!({"185": true})), None);
        let claims = |v: Value| v.as_object().cloned();
        // Absent: left out (WineGDK's compatibility path). Present but unusable: empty, which grants nothing.
        assert_eq!(privilege_claim(claims(json!({"xid": "1"})).as_ref()), None);
        assert_eq!(privilege_claim(claims(json!({"prv": "abc"})).as_ref()).as_deref(), Some(""));
        assert_eq!(privilege_claim(claims(json!({"prv": null})).as_ref()).as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_missing_age_or_privilege_claim_is_not_invented() {
        let mock = Mock::start(|req, _| {
            if req.path() == "/authorize" && req.json()["RelyingParty"] == XBOXLIVE_RP {
                return Resp::json(
                    200,
                    json!({"AuthorizationToken": {"Token": "fake-xbl", "NotAfter": iso(36000),
                    "DisplayClaims": {"xui": [{"uhs": "1", "xid": "2", "gtg": "G"}]}}}),
                );
            }
            Live::default().answer(req)
        })
        .await;
        let x = test_xbox(&mock, "pre");
        let epoch = fresh_epoch(&x).await;
        assert!(run_ok(&x, &epoch).await.readiness.is_online());
        let raw: Value = serde_json::from_slice(&std::fs::read(x.payload_path()).unwrap()).unwrap();
        assert!(raw.get("xbl_privileges").is_none());
        assert!(raw["xbl_age_group"].is_null());
        assert!(raw.get("xbl_modern_gamertag").is_none());
    }

    #[test]
    fn the_completeness_check_names_what_is_missing() {
        let now = 1_800_000_000;
        let soon = chrono::DateTime::from_timestamp(now + 30, 0).unwrap().to_rfc3339();
        let p = Payload {
            device_token: Some("d".into()),
            device_token_expiry: Some(soon),
            user_token: Some(String::new()),
            ..Payload::default()
        };
        let problems = p.problems(now, MIN_TTL);
        assert!(problems.contains(&"expired device_token".to_string()), "{problems:?}");
        assert!(problems.contains(&"missing user_token".to_string()));
        assert!(problems.contains(&"missing xbl_xuid".to_string()));
        assert_eq!(problems.len(), 13, "one per required field");
    }

    #[test]
    fn the_achievements_block_is_all_or_nothing() {
        let now = now_unix();
        let p = Payload {
            achievements_token: Some("t".into()),
            achievements_uhs: Some("00042".into()),
            achievements_expiry: Some(iso(3600)),
            ..Payload::default()
        };
        let q = p.with_epochs(now);
        assert_eq!(q.achievements_uhs.as_deref(), Some("42"));
        assert!(q.achievements_expiry_epoch.is_some());
        for broken in [
            Payload { achievements_uhs: Some("0".into()), ..p.clone() },
            Payload { achievements_uhs: Some("18446744073709551616".into()), ..p.clone() },
            Payload { achievements_expiry: Some(iso(30)), ..p.clone() },
            Payload { achievements_token: Some(" ".into()), ..p.clone() },
        ] {
            let q = broken.with_epochs(now);
            assert!(
                q.achievements_token.is_none() && q.achievements_uhs.is_none() && q.achievements_expiry_epoch.is_none()
            );
        }
        // An epoch is never kept for an expiry that can't be read.
        let q = Payload { xbl_token_expiry: Some("garbage".into()), xbl_token_expiry_epoch: Some("1".into()), ..p }
            .with_epochs(now);
        assert!(q.xbl_token_expiry_epoch.is_none());
    }
}
