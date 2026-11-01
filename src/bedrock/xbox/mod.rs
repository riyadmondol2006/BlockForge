//! Package D of PORT-PLAN.md: the Xbox game sign-in and the online session, all done on the host. The player signs
//! in once: the Store's sign-in window goes on, in the same login.live.com session, to WineGDK's own app's
//! authorisation page, and its code ([`sign_in_with_code`]) gives the same refresh token a device code would. The
//! device code ([`sign_in_with`]) stays as the fallback when the window can't give one.
//!
//! Ported from BedrockOnLinux (MIT, BedrockOnLinux contributors, https://github.com/Wyze3306/BedrockOnLinux, commit
//! cc6ac323): bol/auth.py (`NativeAuth`, the account cache and epoch, `xbl_preauth`) and bol/presence.py.
//!
//! The flow, per profile:
//!
//! 1. [`sign_in_with_code`] (the code from the one sign-in window) or [`sign_in_with`] (Microsoft's legacy Live
//!    device-code sign-in, microsoft.com/link and a code), both with WineGDK's own client ID and scope. Only the
//!    refresh token is kept, 0600 in the profile's 0700 `xbox/` folder.
//! 2. [`prepare`], on every Play, alongside the prefix work: renew the refresh token (the rotated one is saved
//!    first, whatever happens next), then run the signed Xbox Live chain on the host and write the payload WineGDK
//!    reads. It answers [`OnlineReadiness`]: online with the validated payload's path, or offline with the most
//!    actionable reason. Offline never stops the game: single-player and LAN still start (the Store licence is a
//!    separate matter, checked by the Store package).
//! 3. [`start_presence`] while the game runs; [`PresenceSession::stop`] when it ends.
//! 4. [`sign_out`] rotates the account epoch and deletes the account's files, keeping the device key.
//!
//! None of this shares a token with the Java sign-in in `src/msauth.rs` (another app, other endpoints, whose refresh
//! token can't mint this app's), and only the browser session with the Store sign-in: the window, not its tokens.

mod cache;
mod device_code;
#[cfg(test)]
mod mock;
mod preauth;
mod presence;
mod signature;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub(crate) use cache::Epoch;
pub(crate) use device_code::{DevicePrompt, SignedIn};
pub(crate) use preauth::{Category, Offline, OnlineReadiness, StageResult};
pub(crate) use presence::{PresenceSession, SocialSnapshot};

use super::{BedrockError, Cancel, ErrorKind, Result, SignInState};
use cache::MsaToken;
use device_code::Refreshed;

/// The environment variable that hands WineGDK the payload (as a `Z:\...` path).
pub(crate) const PREAUTH_ENV: &str = "WINEGDK_PREAUTH_DEVICE";
/// Said once when a launch meant to publish presence and could not.
pub(crate) const PRESENCE_UNAVAILABLE: &str = "Xbox Live was not told you are playing, so your account stays \
     \"Offline\" to your friends and they cannot join your world or invite you. Sign in to Xbox again if this \
     persists.";

/// A credential. Its `Debug` never shows it, so it can't reach a log by accident.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Every address this package talks to, so tests can point all of them at a local server.
#[derive(Clone, Debug)]
pub(crate) struct Endpoints {
    pub msa_connect: String,
    pub msa_token: String,
    pub device_auth: String,
    pub user_auth: String,
    pub xsts: String,
    pub sisu: String,
    /// Base of userpresence.xboxlive.com.
    pub presence: String,
    /// Base of peoplehub.xboxlive.com.
    pub peoplehub: String,
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            msa_connect: "https://login.live.com/oauth20_connect.srf".into(),
            msa_token: "https://login.live.com/oauth20_token.srf".into(),
            device_auth: "https://device.auth.xboxlive.com/device/authenticate".into(),
            user_auth: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            sisu: "https://sisu.xboxlive.com/authorize".into(),
            presence: "https://userpresence.xboxlive.com".into(),
            peoplehub: "https://peoplehub.xboxlive.com".into(),
        }
    }
}

impl Endpoints {
    /// Everything on one local server, at the same paths as the real services (the signature covers the path).
    #[cfg(test)]
    pub(crate) fn local(base: &str) -> Endpoints {
        Endpoints {
            msa_connect: format!("{base}/oauth20_connect.srf"),
            msa_token: format!("{base}/oauth20_token.srf"),
            device_auth: format!("{base}/device/authenticate"),
            user_auth: format!("{base}/user/authenticate"),
            xsts: format!("{base}/xsts/authorize"),
            sisu: format!("{base}/authorize"),
            presence: base.to_string(),
            peoplehub: base.to_string(),
        }
    }
}

/// The waits, scaled down in tests.
#[derive(Clone, Debug)]
pub(crate) struct Timing {
    /// One "second" of the device-code flow's `interval` and `expires_in`.
    pub poll_unit: Duration,
    pub request_timeout: Duration,
    /// Inside the few minutes Xbox holds an "active" record before it decays to "Away" (measured: Away at minute 7),
    /// and far under the endpoint's rate limit.
    pub presence_heartbeat: Duration,
    /// First retry after a refused heartbeat; doubles up to the heartbeat.
    pub presence_retry: Duration,
    /// How long a stop waits for the "inactive" write before moving on.
    pub presence_stop: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            poll_unit: Duration::from_secs(1),
            request_timeout: Duration::from_secs(15),
            presence_heartbeat: Duration::from_secs(240),
            presence_retry: Duration::from_secs(30),
            presence_stop: Duration::from_secs(5),
        }
    }
}

/// One profile's Xbox side: its private folder (`profiles/<uuid>/xbox`), and how to reach Xbox Live.
#[derive(Clone)]
pub(crate) struct Xbox {
    dir: PathBuf,
    client: reqwest::Client,
    endpoints: Arc<Endpoints>,
    timing: Timing,
}

impl std::fmt::Debug for Xbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Xbox").field("dir", &self.dir).finish_non_exhaustive()
    }
}

impl Xbox {
    /// The real services, through BlockForge's shared HTTP client (its proxy and timeouts). `profile_root` is the
    /// profile's folder; nothing is created until a sign-in or a Play needs it. Call it on a worker: the first use
    /// of the shared client builds it.
    pub(crate) fn for_profile(profile_root: &Path) -> Xbox {
        Xbox::with(profile_root.join("xbox"), crate::http::client().clone(), Endpoints::default(), Timing::default())
    }

    pub(crate) fn with(dir: PathBuf, client: reqwest::Client, endpoints: Endpoints, timing: Timing) -> Xbox {
        Xbox { dir, client, endpoints: Arc::new(endpoints), timing }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the payload WineGDK reads lives (it exists only while it is valid for the current account).
    pub(crate) fn payload_path(&self) -> PathBuf {
        self.dir.join(cache::PAYLOAD_FILE)
    }
}

/// `Z:\home\...`: the payload's path as Wine sees the host's root.
pub(crate) fn wine_path(path: &Path) -> String {
    format!("Z:{}", path.to_string_lossy().replace('/', "\\"))
}

fn err(kind: ErrorKind, message: impl Into<String>) -> BedrockError {
    BedrockError::new(kind, message)
}

fn io_err(doing: &str, e: std::io::Error) -> BedrockError {
    err(ErrorKind::Internal, format!("Failed {doing}: {e}"))
}

fn cancelled() -> BedrockError {
    err(ErrorKind::Cancelled, "Cancelled.")
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// File work off the async workers.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| err(ErrorKind::Internal, "A background task stopped unexpectedly."))
}

/// Refreshes and chains for one profile run one at a time (in this process; the OS lock covers the files).
fn serial(dir: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(dir.to_path_buf()).or_default().clone()
}

/// Sign this profile into Xbox, announcing the code as [`Event::DeviceCode`]. Already signed in: nothing is asked.
pub(crate) async fn sign_in_with(
    x: &Xbox,
    on_prompt: impl FnOnce(DevicePrompt) + Send,
    cancel: &Cancel,
) -> Result<SignedIn> {
    device_code::sign_in(x, on_prompt, cancel).await
}

/// Sign this profile into Xbox with the code the one Microsoft sign-in window got (the Store's session, WineGDK's
/// app): from there on exactly like [`sign_in_with`]. An error means the device code is the way left.
pub(crate) async fn sign_in_with_code(x: &Xbox, code: &Secret, cancel: &Cancel) -> Result<SignedIn> {
    device_code::sign_in_with_code(x, code, cancel).await
}

/// Who the profile's Xbox account is, from its cached payload (only when the payload is the current account's):
/// XUID, gamertag and user hash. No network.
pub(crate) fn cached_identity(x: &Xbox) -> super::AccountIdentity {
    let dir = &x.dir;
    let Ok(Some(epoch)) = cache::current_epoch(dir) else { return Default::default() };
    if !dir.join(cache::TOKEN_FILE).exists() {
        return Default::default();
    }
    let Some(p) = cache::load_payload(dir).filter(|p| p.account_epoch.as_ref() == Some(&epoch)) else {
        return Default::default();
    };
    let some = |v: Option<String>| v.filter(|s| !s.is_empty());
    super::AccountIdentity {
        email: None,
        gamertag: some(p.xbl_gamertag),
        xuid: some(p.xbl_xuid),
        user_hash: some(p.xbl_uhs.or(p.sisu_uhs)),
    }
}

/// Forget this profile's Xbox account: the epoch rotates first, so nothing in flight can bring it back, then the
/// refresh token and the payload go. The device key and ID stay. The prefix's own copy of the refresh token (the
/// WineGDK registry value) must be removed before this is called, and if that fails this must not be called, so
/// the window keeps showing the real state and the player can retry.
pub(crate) async fn sign_out(x: &Xbox) -> Result<()> {
    let dir = x.dir.clone();
    if !dir.exists() {
        return Ok(());
    }
    blocking(move || cache::purge(&dir)).await?.map_err(|e| io_err("signing out of Xbox", e))
}

/// The cached sign-in state for the window: no network, and nothing created on disk.
#[cfg(test)]
pub(crate) async fn state(x: &Xbox) -> SignInState {
    let x = x.clone();
    blocking(move || cached_state(&x)).await.unwrap_or(SignInState::Unknown)
}

/// [`state`] for a caller already on a blocking worker: a few small reads, no network.
pub(crate) fn cached_state(x: &Xbox) -> SignInState {
    let dir = &x.dir;
    if !dir.join(cache::TOKEN_FILE).exists() {
        return SignInState::SignedOut;
    }
    match cache::session_snapshot(dir) {
        Err(_) => SignInState::Unknown,
        Ok((_, None)) => SignInState::NeedsSignIn {
            reason: "This profile's Xbox sign-in record is damaged: sign out of Xbox and sign in again.".into(),
        },
        Ok((None, _)) => SignInState::SignedOut,
        Ok((Some(_), Some(epoch))) => SignInState::Linked {
            display_name: cache::load_payload(dir)
                .filter(|p| p.account_epoch.as_ref() == Some(&epoch))
                .and_then(|p| p.xbl_gamertag),
        },
    }
}

pub(crate) async fn online_cached(x: &Xbox) -> bool {
    let dir = x.dir.clone();
    blocking(move || {
        if !dir.join(cache::PAYLOAD_FILE).exists() {
            return false;
        }
        let Ok(Some(epoch)) = cache::current_epoch(&dir) else { return false };
        cache::load_payload(&dir).is_some_and(|p| {
            p.account_epoch.as_ref() == Some(&epoch) && p.problems(now_unix(), preauth::MIN_TTL).is_empty()
        })
    })
    .await
    .unwrap_or(false)
}

/// What a Play gets from the Xbox side.
#[derive(Debug)]
pub(crate) struct Prepared {
    pub readiness: OnlineReadiness,
    /// For WineGDK's XUser: the prefix package writes it into the stopped prefix's HKLM
    /// [`WINEGDK_REGISTRY_KEY`] / [`WINEGDK_REFRESH_TOKEN_VALUE`] before the game starts. `None` when no Xbox
    /// account is signed in (then nothing is written).
    pub registry_refresh_token: Option<Secret>,
    /// Check it with [`epoch_is_current`] right before the game is spawned: a sign-out in between means this
    /// preparation belongs to an account that is gone.
    pub epoch: Option<Epoch>,
    /// Redacted per-step results, for the setup log.
    pub stages: Vec<StageResult>,
}

/// Get the Xbox side ready for a Play: renew the sign-in, keep the rotated refresh token, run the chain (or reuse a
/// fresh payload) and say whether the game gets Xbox Live. Runs alongside prefix preparation. Errors only for a
/// sign-out that happened during it, a cancel, or broken local files; Xbox Live itself failing is an offline
/// answer, not an error, because it never stops the game.
pub(crate) async fn prepare(x: &Xbox, cancel: &Cancel) -> Result<Prepared> {
    let serial = serial(&x.dir);
    let _one_at_a_time = tokio::select! {
        g = serial.lock_owned() => g,
        _ = cancel.cancelled() => return Err(cancelled()),
    };
    let dir = x.dir.clone();
    let (token, epoch) =
        blocking(move || cache::session_snapshot(&dir)).await?.map_err(|e| io_err("reading the Xbox sign-in", e))?;
    let Some(epoch) = epoch else {
        let damaged =
            Offline { stage: "account-cache", category: Category::Account, http_status: None, error_code: None };
        return Ok(Prepared {
            readiness: OnlineReadiness::Offline(damaged),
            registry_refresh_token: None,
            epoch: None,
            stages: Vec::new(),
        });
    };
    let Some(token) = token else {
        return Ok(Prepared {
            readiness: OnlineReadiness::Offline(Offline::not_linked()),
            registry_refresh_token: None,
            epoch: Some(epoch),
            stages: Vec::new(),
        });
    };

    // Not raced against cancel: once Microsoft has rotated the token, the new one must be saved, not dropped.
    let (access, registry, unreachable) = match device_code::refresh(x, &token.refresh_token).await {
        Refreshed::Fresh { refresh, access } => {
            let saved = MsaToken { refresh_token: refresh.clone(), obtained: now_unix() };
            let (dir, e) = (x.dir.clone(), epoch.clone());
            let kept = blocking(move || cache::save_token_for_epoch(&dir, &saved, &e))
                .await?
                .map_err(|e| io_err("saving the renewed Xbox sign-in", e))?;
            if !kept {
                return Err(err(
                    ErrorKind::SignInRequired,
                    "The profile was signed out of Xbox while Play was getting ready, so nothing of that account \
                     was kept. Press Play again.",
                ));
            }
            (access, refresh, false)
        }
        // Refused: the old token still goes to WineGDK, and a complete cached payload may still serve.
        Refreshed::Rejected => (None, token.refresh_token, false),
        Refreshed::Unreachable => (None, token.refresh_token, true),
    };
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let outcome = tokio::select! {
        o = preauth::run(x, access.as_ref(), &epoch, unreachable) => o,
        _ = cancel.cancelled() => return Err(cancelled()),
    };
    Ok(Prepared {
        readiness: outcome.readiness,
        registry_refresh_token: Some(registry),
        epoch: Some(epoch),
        stages: outcome.stages,
    })
}

/// The last check before the game is spawned (and the payload handed to it).
pub(crate) async fn epoch_is_current(x: &Xbox, epoch: &Epoch) -> Result<bool> {
    let (dir, e) = (x.dir.clone(), epoch.clone());
    blocking(move || cache::epoch_is_current(&dir, &e)).await?.map_err(|e| io_err("checking the Xbox sign-in", e))
}

/// Publish the play session on Xbox Live while the game runs. Always a session (an inert one when presence is
/// switched off, the game is offline or the payload can't serve a heartbeat), so a launch can always stop it.
/// If `enabled` and the session isn't active, show [`PRESENCE_UNAVAILABLE`] once.
pub(crate) async fn start_presence(x: &Xbox, enabled: bool, readiness: &OnlineReadiness) -> PresenceSession {
    let OnlineReadiness::Online { payload, .. } = readiness else { return PresenceSession::inert() };
    if !enabled {
        return PresenceSession::inert();
    }
    let path = payload.clone();
    match blocking(move || presence::load_credentials(&path)).await.ok().flatten() {
        Some(creds) => PresenceSession::start(x.client.clone(), x.endpoints.clone(), creds, &x.timing),
        None => PresenceSession::inert(),
    }
}

/// Ask Xbox Live what friends see (for a diagnosis report; one GET each to presence and PeopleHub).
pub(crate) async fn social_snapshot(x: &Xbox) -> SocialSnapshot {
    let path = x.payload_path();
    let creds = blocking(move || presence::load_credentials(&path)).await.ok().flatten();
    presence::social_snapshot(&x.client, &x.endpoints, creds.as_ref(), x.timing.request_timeout).await
}

#[cfg(test)]
pub(super) mod tests {
    use super::mock::{Mock, Resp};
    use super::preauth::tests::{Live, iso};
    use super::*;
    use serde_json::json;

    pub(super) fn unit() -> Duration {
        Duration::from_millis(20)
    }

    pub(super) fn test_timing() -> Timing {
        Timing {
            poll_unit: unit(),
            request_timeout: Duration::from_secs(5),
            presence_heartbeat: unit() * 3,
            presence_retry: unit() * 2,
            presence_stop: unit() * 10,
        }
    }

    pub(super) fn test_xbox(mock: &Mock, tag: &str) -> Xbox {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        Xbox::with(crate::testutil::scratch(tag).join("xbox"), client, Endpoints::local(&mock.base), test_timing())
    }

    fn link(x: &Xbox, refresh: &str) {
        let (_, epoch) = cache::session_snapshot(&x.dir).unwrap();
        let t = MsaToken { refresh_token: Secret::new(refresh), obtained: 1 };
        assert!(cache::save_token_for_epoch(&x.dir, &t, &epoch.unwrap()).unwrap());
    }

    fn saved_refresh(x: &Xbox) -> Option<String> {
        cache::session_snapshot(&x.dir).unwrap().0.map(|t| t.refresh_token.expose().to_string())
    }

    /// Xbox Live as `live` answers it, and login.live.com rotating the refresh token on each renewal.
    async fn microsoft_and(live: Live) -> Mock {
        Mock::start(move |req, nth| match req.path() {
            "/oauth20_token.srf" => Resp::json(
                200,
                json!({"access_token": "fake-access", "refresh_token": format!("fake-refresh-rotated-{nth}")}),
            ),
            _ => live.answer(req),
        })
        .await
    }

    #[tokio::test]
    async fn not_signed_in_is_offline_not_an_error() {
        let mock = Mock::start(|_, _| Resp::status(500)).await;
        let x = test_xbox(&mock, "xbox");
        let p = prepare(&x, &Cancel::default()).await.unwrap();
        assert_eq!(p.readiness, OnlineReadiness::Offline(Offline::not_linked()));
        assert!(p.registry_refresh_token.is_none());
        assert!(p.readiness.notice().unwrap().contains("single-player"));
        assert_eq!(mock.total(), 0);
        assert!(matches!(state(&x).await, SignInState::SignedOut));
    }

    #[tokio::test]
    async fn play_renews_the_token_runs_the_chain_and_hands_over_the_payload() {
        let mock = microsoft_and(Live::default()).await;
        let x = test_xbox(&mock, "xbox");
        link(&x, "fake-refresh-0");
        let p = prepare(&x, &Cancel::default()).await.unwrap();
        assert!(p.readiness.is_online(), "{:?}", p.stages);
        assert_eq!(p.registry_refresh_token.as_ref().unwrap().expose(), "fake-refresh-rotated-1");
        assert_eq!(saved_refresh(&x).as_deref(), Some("fake-refresh-rotated-1"));
        let (name, value) = p.readiness.wine_env().unwrap();
        assert_eq!(name, "WINEGDK_PREAUTH_DEVICE");
        assert!(value.starts_with("Z:\\") && value.ends_with("\\xbox\\device.json") && !value.contains('/'), "{value}");
        assert!(epoch_is_current(&x, p.epoch.as_ref().unwrap()).await.unwrap());
        assert!(online_cached(&x).await);
        assert!(matches!(state(&x).await, SignInState::Linked { display_name: Some(ref g) } if g == "FakeTag"));
        let form = mock.requests()[0].form();
        assert_eq!((form["grant_type"].as_str(), form["refresh_token"].as_str()), ("refresh_token", "fake-refresh-0"));
        // Nothing secret in what goes to the log.
        let log = format!("{p:?} {}", p.stages.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "));
        assert!(!log.contains("fake-refresh") && !log.contains("fake-access") && !log.contains("-token-"), "{log}");
    }

    #[tokio::test]
    async fn the_rotated_refresh_token_is_kept_when_xbox_live_fails_after_it() {
        let mock = microsoft_and(Live { device_status: Some(500), ..Live::default() }).await;
        let x = test_xbox(&mock, "xbox");
        link(&x, "fake-refresh-0");
        let p = prepare(&x, &Cancel::default()).await.unwrap();
        let OnlineReadiness::Offline(o) = &p.readiness else { panic!("must be offline") };
        assert_eq!((o.stage, o.category), ("device-auth", Category::Network));
        assert_eq!(saved_refresh(&x).as_deref(), Some("fake-refresh-rotated-1"));
        assert_eq!(p.registry_refresh_token.unwrap().expose(), "fake-refresh-rotated-1");
        assert!(p.readiness.notice().unwrap().contains("stage: device-auth"));
    }

    #[tokio::test]
    async fn a_rejected_refresh_keeps_the_old_token_and_says_sign_in_again() {
        let mock = Mock::start(|req, _| match req.path() {
            "/oauth20_token.srf" => Resp::json(400, json!({"error": "invalid_grant"})),
            _ => Resp::status(500),
        })
        .await;
        let x = test_xbox(&mock, "xbox");
        link(&x, "fake-refresh-0");
        let p = prepare(&x, &Cancel::default()).await.unwrap();
        let OnlineReadiness::Offline(o) = &p.readiness else { panic!() };
        assert_eq!(o.category, Category::SignIn);
        assert_eq!(saved_refresh(&x).as_deref(), Some("fake-refresh-0"));
        assert_eq!(mock.total(), 1, "Xbox Live is not asked without an access token");
    }

    #[tokio::test]
    async fn offline_with_a_valid_cache_still_plays_online() {
        let mock = microsoft_and(Live { ttl: Some(1200), ..Live::default() }).await;
        let x = test_xbox(&mock, "xbox");
        link(&x, "fake-refresh-0");
        assert!(prepare(&x, &Cancel::default()).await.unwrap().readiness.is_online());
        let gone = Xbox { endpoints: Arc::new(Endpoints::local("http://127.0.0.1:9")), ..x };
        let p = prepare(&gone, &Cancel::default()).await.unwrap();
        assert!(p.readiness.is_online(), "the cached payload (20 min left) serves while Microsoft is unreachable");
        assert_eq!(p.stages, [StageResult::Cached]);
        assert_eq!(saved_refresh(&gone).as_deref(), Some("fake-refresh-rotated-1"));
    }

    #[tokio::test]
    async fn a_sign_out_during_the_refresh_keeps_nothing() {
        let dir: Arc<Mutex<Option<PathBuf>>> = Arc::default();
        let d = dir.clone();
        let mock = Mock::start(move |req, _| match req.path() {
            "/oauth20_token.srf" => {
                cache::purge(d.lock().unwrap().as_ref().unwrap()).unwrap();
                Resp::json(200, json!({"access_token": "fake-access", "refresh_token": "fake-refresh-late"}))
            }
            _ => Live::default().answer(req),
        })
        .await;
        let x = test_xbox(&mock, "xbox");
        *dir.lock().unwrap() = Some(x.dir.clone());
        link(&x, "fake-refresh-0");
        let e = prepare(&x, &Cancel::default()).await.unwrap_err();
        assert!(matches!(e.kind, ErrorKind::SignInRequired), "{e}");
        assert!(saved_refresh(&x).is_none(), "the signed-out account's rotated token was not saved");
        assert!(!x.payload_path().exists());
        assert_eq!(mock.total(), 1);
    }

    #[tokio::test]
    async fn a_sign_out_after_prepare_is_caught_before_the_game_starts() {
        let mock = microsoft_and(Live::default()).await;
        let x = test_xbox(&mock, "xbox");
        link(&x, "fake-refresh-0");
        let p = prepare(&x, &Cancel::default()).await.unwrap();
        sign_out(&x).await.unwrap();
        assert!(!epoch_is_current(&x, p.epoch.as_ref().unwrap()).await.unwrap());
        assert!(!x.payload_path().exists());
        assert!(matches!(state(&x).await, SignInState::SignedOut));
        assert!(!online_cached(&x).await);
        // The device stays: the next account gets the same key and ID.
        assert!(x.dir.join(cache::KEY_FILE).exists() && x.dir.join(cache::DEVICE_ID_FILE).exists());
    }

    #[tokio::test]
    async fn refreshes_for_one_profile_run_one_at_a_time() {
        let mock = Mock::start(|req, nth| match req.path() {
            "/oauth20_token.srf" => {
                Resp::json(200, json!({"access_token": "a", "refresh_token": format!("fake-refresh-rotated-{nth}")}))
                    .slow(Duration::from_millis(100))
            }
            _ => Live::default().answer(req),
        })
        .await;
        let x = test_xbox(&mock, "xbox");
        link(&x, "fake-refresh-0");
        let (c1, c2) = (Cancel::default(), Cancel::default());
        let (a, b) = tokio::join!(prepare(&x, &c1), prepare(&x, &c2));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(a.readiness.is_online() && b.readiness.is_online());
        let refreshes: Vec<String> = mock
            .requests()
            .iter()
            .filter(|r| r.path() == "/oauth20_token.srf")
            .map(|r| r.form()["refresh_token"].clone())
            .collect();
        // The second refresh used the token the first one saved, not the one both started with.
        assert_eq!(refreshes, ["fake-refresh-0", "fake-refresh-rotated-1"]);
        assert_eq!(saved_refresh(&x).as_deref(), Some("fake-refresh-rotated-2"));
    }

    #[tokio::test]
    async fn presence_is_started_only_online_and_never_gates_play() {
        let mock = microsoft_and(Live::default()).await;
        let x = test_xbox(&mock, "xbox");
        let offline = OnlineReadiness::Offline(Offline::not_linked());
        assert!(!start_presence(&x, true, &offline).await.is_active());
        link(&x, "fake-refresh-0");
        let p = prepare(&x, &Cancel::default()).await.unwrap();
        assert!(!start_presence(&x, false, &p.readiness).await.is_active(), "switched off");
        // The mock answers presence with 404: a refusal ends presence quietly; nothing is raised.
        let session = start_presence(&x, true, &p.readiness).await;
        assert!(session.is_active());
        session.stop().await;
        // A payload that can't serve a heartbeat gives an inert session, not an error.
        std::fs::write(x.payload_path(), b"{}").unwrap();
        assert!(!start_presence(&x, true, &p.readiness).await.is_active());
    }

    #[test]
    fn secrets_never_show_in_debug() {
        let s = Secret::new("fake-refresh-token");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        let t = MsaToken { refresh_token: s, obtained: 1 };
        assert!(!format!("{t:?}").contains("fake-refresh"));
    }

    #[test]
    fn the_wine_path_is_z_and_backslashes() {
        assert_eq!(wine_path(Path::new("/home/a b/x/device.json")), r"Z:\home\a b\x\device.json");
    }

    #[test]
    fn the_production_endpoints_are_the_ones_winegdk_uses() {
        let e = Endpoints::default();
        assert_eq!(e.msa_connect, "https://login.live.com/oauth20_connect.srf");
        assert_eq!(e.sisu, "https://sisu.xboxlive.com/authorize");
        assert_eq!(device_code::CLIENT_ID, "0000000048183522");
        assert_eq!(preauth::TITLE_APP_ID, "0000000048183522");
        let _ = iso(0);
    }

    /// The real check, which needs the player: an Xbox device-code sign-in, then the whole chain against the real
    /// services, then a second full chain proving the stable device key is reused, then a moment of presence.
    /// Prints redacted results only (stage names, statuses, which fields are present). Run it with:
    ///
    /// ```sh
    /// BF_XBOX_REAL_DIR=$HOME/.cache/bf-bedrock-xbox-real \
    ///   cargo test --locked --lib bedrock::xbox::tests::real_xbox_sign_in_and_online_chain -- --ignored --nocapture
    /// ```
    ///
    /// and enter the printed code at the printed address. The folder keeps the sign-in for later runs; delete it
    /// (or run with a new one) to sign in afresh.
    #[test]
    #[ignore = "needs a person to sign in to Xbox with a real Microsoft account"]
    fn real_xbox_sign_in_and_online_chain() {
        let dir = std::env::var_os("BF_XBOX_REAL_DIR").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".cache/bf-bedrock-xbox-real")
        });
        let rt = crate::testutil::runtime();
        rt.block_on(async {
            let x =
                Xbox::with(dir.join("xbox"), crate::http::client().clone(), Endpoints::default(), Timing::default());
            eprintln!("Xbox folder: {}", x.dir().display());
            let got = sign_in_with(
                &x,
                |p| {
                    eprintln!(
                        "\n==> Open {} and enter the code {}  (it expires in {} s)\n",
                        p.url,
                        p.user_code,
                        p.expires_at_unix - now_unix()
                    );
                },
                &Cancel::default(),
            )
            .await
            .expect("device-code sign-in");
            eprintln!("sign-in: {got:?}");

            let report = |label: &str, p: &Prepared| {
                eprintln!("--- {label}");
                for s in &p.stages {
                    eprintln!("  {s}");
                }
                match &p.readiness {
                    OnlineReadiness::Online { gamertag, .. } => {
                        eprintln!("  ONLINE (gamertag {})", if gamertag.is_some() { "present" } else { "missing" })
                    }
                    OnlineReadiness::Offline(o) => eprintln!("  OFFLINE: {}", o.notice()),
                }
            };
            let first = prepare(&x, &Cancel::default()).await.expect("prepare");
            report("first chain", &first);
            assert!(first.readiness.is_online());
            let payload: serde_json::Value = serde_json::from_slice(&std::fs::read(x.payload_path()).unwrap()).unwrap();
            for (k, v) in payload.as_object().unwrap() {
                let shown = match v {
                    serde_json::Value::String(s)
                        if k.ends_with("_rp") || k.ends_with("_epoch") || k.ends_with("expiry") =>
                    {
                        s.clone()
                    }
                    serde_json::Value::String(s) => format!("<{} chars>", s.len()),
                    other => other.to_string(),
                };
                eprintln!("  {k}: {shown}");
            }

            let key_before = (
                std::fs::read(x.dir().join(cache::KEY_FILE)).unwrap(),
                std::fs::read_to_string(x.dir().join(cache::DEVICE_ID_FILE)).unwrap(),
            );
            // Force a full chain again, with the renewed refresh token and the saved device key.
            std::fs::remove_file(x.payload_path()).unwrap();
            let second = prepare(&x, &Cancel::default()).await.expect("second prepare");
            report("second chain (refresh, same device key)", &second);
            assert!(second.readiness.is_online());
            let key_after = (
                std::fs::read(x.dir().join(cache::KEY_FILE)).unwrap(),
                std::fs::read_to_string(x.dir().join(cache::DEVICE_ID_FILE)).unwrap(),
            );
            assert_eq!(key_before, key_after, "the device key and ID are stable");

            let session = start_presence(&x, true, &second.readiness).await;
            for _ in 0..40 {
                if session.announced() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            eprintln!("presence: active={} announced={}", session.is_active(), session.announced());
            eprintln!("social: {:?}", social_snapshot(&x).await);
            session.stop().await;
        });
    }
}
