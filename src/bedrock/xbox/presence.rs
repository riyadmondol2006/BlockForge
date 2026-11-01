//! Telling Xbox Live the account is playing Minecraft, for as long as it plays, so friends see it and can join.
//!
//! Ported from BedrockOnLinux bol/presence.py (MIT, with credit). Under Wine nothing in the game writes presence
//! (XSAPI, which does it on Windows, doesn't come up), so the account reads "Offline" to its friends unless the
//! launcher says it. The write is one POST with the profile's own `http://xboxlive.com` token:
//!
//! ```text
//! POST userpresence.xboxlive.com/users/xuid(<xuid>)/devices/current/titles/current
//! Authorization: XBL3.0 x=<uhs>;<token>
//! x-xbl-contract-version: 3
//! {"state":"active"}
//! ```
//!
//! The title is taken from the token, so this can only ever say "Minecraft". Every 240 s keeps it inside the few
//! minutes Xbox holds an unfed record before it decays to "Away"; on the way out it writes "inactive". Nothing
//! here can fail a launch or hold one up: presence is what other people see, never whether the game runs.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;

use super::{Endpoints, Secret, Timing, now_unix};

const CONTRACT: &str = "3";
const PEOPLEHUB_CONTRACT: &str = "5";

/// The three payload fields a heartbeat needs.
#[derive(Clone, Debug)]
pub(crate) struct Credentials {
    token: Secret,
    uhs: String,
    xuid: String,
    /// `xbl_token_expiry_epoch`; unknown on older payloads, which count as usable (an expired token is answered
    /// 401, which ends the session by itself).
    expiry: Option<i64>,
}

impl Credentials {
    pub(crate) fn usable(&self, now: i64) -> bool {
        self.expiry.is_none_or(|e| e > now + 60)
    }
}

fn text_or_number(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => n.as_u64().map(|n| n.to_string()),
        _ => None,
    }
}

/// Read the payload, or `None` when it can't serve a heartbeat. Deliberately tolerant: every way it can be absent
/// or half-written has the same answer, publish nothing.
pub(crate) fn load_credentials(path: &Path) -> Option<Credentials> {
    let payload: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let token = payload.get("xbl_token")?.as_str().filter(|t| !t.is_empty())?;
    let uhs = text_or_number(payload.get("xbl_uhs")).filter(|u| !u.is_empty())?;
    let xuid =
        text_or_number(payload.get("xbl_xuid")).filter(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))?;
    let expiry = text_or_number(payload.get("xbl_token_expiry_epoch")).and_then(|e| e.parse().ok());
    Some(Credentials { token: Secret::new(token), uhs, xuid, expiry })
}

/// One authenticated call: the status, or `None` when it never reached Xbox Live.
async fn request(
    client: &reqwest::Client,
    creds: &Credentials,
    method: reqwest::Method,
    url: &str,
    body: Option<&Value>,
    timeout: Duration,
    contract: &str,
) -> Option<(u16, String)> {
    let mut req = client
        .request(method, url)
        .timeout(timeout)
        .header(reqwest::header::AUTHORIZATION, format!("XBL3.0 x={};{}", creds.uhs, creds.token.expose()))
        .header("x-xbl-contract-version", contract)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US");
    if let Some(body) = body {
        req = req
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body).unwrap_or_default());
    }
    let sent = req.send().await.ok()?;
    let status = sent.status().as_u16();
    Some((status, sent.text().await.unwrap_or_default()))
}

/// Publish `active` or `inactive` once.
pub(crate) async fn write_state(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    creds: &Credentials,
    state: &str,
    timeout: Duration,
) -> Option<u16> {
    let url = format!("{}/users/xuid({})/devices/current/titles/current", endpoints.presence, creds.xuid);
    let body = serde_json::json!({ "state": state });
    request(client, creds, reqwest::Method::POST, &url, Some(&body), timeout, CONTRACT).await.map(|(s, _)| s)
}

/// What Xbox Live says about the account now ("Online", "Away", "Offline"), for diagnostics only.
pub(crate) async fn read_state(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    creds: &Credentials,
    timeout: Duration,
) -> Option<String> {
    let url = format!("{}/users/xuid({})?level=all", endpoints.presence, creds.xuid);
    let (status, text) = request(client, creds, reqwest::Method::GET, &url, None, timeout, CONTRACT).await?;
    if status != 200 {
        return None;
    }
    serde_json::from_str::<Value>(&text).ok()?.get("state")?.as_str().map(str::to_string)
}

/// The account's own presence, how many friends Xbox returned and how many of them it lists in a multiplayer
/// session: the questions behind "I can't join my friend's world". Diagnostics only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SocialSnapshot {
    pub state: Option<String>,
    pub friends: Option<usize>,
    pub in_session: Option<usize>,
    pub error: Option<String>,
}

pub(crate) async fn social_snapshot(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    creds: Option<&Credentials>,
    timeout: Duration,
) -> SocialSnapshot {
    let Some(creds) = creds else {
        return SocialSnapshot { error: Some("no linked account to ask about".into()), ..Default::default() };
    };
    if !creds.usable(now_unix()) {
        return SocialSnapshot { error: Some("the Xbox Live token has expired".into()), ..Default::default() };
    }
    let state = read_state(client, endpoints, creds, timeout).await;
    let url = format!("{}/users/me/people/social/decoration/multiplayer,presenceDetail", endpoints.peoplehub);
    let answer = request(client, creds, reqwest::Method::GET, &url, None, timeout, PEOPLEHUB_CONTRACT).await;
    let people = match answer {
        Some((200, text)) => {
            serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("people")?.as_array().cloned())
        }
        Some((status, _)) => {
            return SocialSnapshot {
                state,
                error: Some(format!("the friends list could not be read (HTTP {status})")),
                ..Default::default()
            };
        }
        None => {
            return SocialSnapshot {
                state,
                error: Some("the friends list could not be read (no answer)".into()),
                ..Default::default()
            };
        }
    };
    let Some(people) = people else {
        return SocialSnapshot {
            state,
            error: Some("the friends list could not be read (unexpected answer)".into()),
            ..Default::default()
        };
    };
    // Counts the presence of a session record only: what is inside one couldn't be measured, and a guessed field
    // name is exactly what breaks silently later.
    let in_session = people
        .iter()
        .filter(|p| p.get("multiplayerSummary").and_then(Value::as_object).is_some_and(|m| !m.is_empty()))
        .count();
    SocialSnapshot { state, friends: Some(people.len()), in_session: Some(in_session), error: None }
}

/// Whether a refused heartbeat is worth stopping over: anything the service will keep answering the same way
/// (401/403, or another 4xx, which would be a fault here). 429 means slow down, which the back-off does.
fn is_final(status: Option<u16>) -> bool {
    status.is_some_and(|s| (400..500).contains(&s) && s != 429)
}

/// One play session kept online on Xbox Live by a task of its own. Inert when it has nothing to publish, so a
/// launch can always start and stop it without asking whether it began.
pub(crate) struct PresenceSession {
    stop: Option<watch::Sender<bool>>,
    task: Option<tokio::task::JoinHandle<()>>,
    announced: Arc<AtomicBool>,
    stop_timeout: Duration,
}

impl PresenceSession {
    pub(crate) fn inert() -> PresenceSession {
        PresenceSession { stop: None, task: None, announced: Arc::default(), stop_timeout: Duration::ZERO }
    }

    /// Whether anything is being published at all.
    pub(crate) fn is_active(&self) -> bool {
        self.task.is_some()
    }

    /// Whether Xbox Live has accepted a heartbeat (and the session hasn't ended).
    pub(crate) fn announced(&self) -> bool {
        self.announced.load(Ordering::SeqCst)
    }

    /// Begin publishing in the background and return at once. Outside a tokio runtime it stays inert.
    pub(crate) fn start(
        client: reqwest::Client,
        endpoints: Arc<Endpoints>,
        creds: Credentials,
        timing: &Timing,
    ) -> PresenceSession {
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return PresenceSession::inert() };
        if !creds.usable(now_unix()) {
            return PresenceSession::inert();
        }
        let (stop, stopped) = watch::channel(false);
        let announced = Arc::new(AtomicBool::new(false));
        let t = timing.clone();
        let task = handle.spawn(run(client, endpoints, creds, t, stopped, announced.clone()));
        PresenceSession { stop: Some(stop), task: Some(task), announced, stop_timeout: timing.presence_stop }
    }

    /// Take the presence down: the task writes "inactive" on its way out. Waits at most the stop timeout; the task
    /// finishes that write by itself afterwards rather than holding a launch's teardown open on the network.
    pub(crate) async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(self.stop_timeout, task).await;
        }
    }
}

impl Drop for PresenceSession {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(true);
        }
    }
}

/// Wait `d`, or less if stopped. `true` when stopped.
async fn wait(stopped: &mut watch::Receiver<bool>, d: Duration) -> bool {
    let early = tokio::select! {
        _ = tokio::time::sleep(d) => false,
        _ = stopped.wait_for(|s| *s) => true,
    };
    early || *stopped.borrow()
}

async fn run(
    client: reqwest::Client,
    endpoints: Arc<Endpoints>,
    creds: Credentials,
    timing: Timing,
    mut stopped: watch::Receiver<bool>,
    announced: Arc<AtomicBool>,
) {
    let mut published = false;
    let mut backoff = timing.presence_retry;
    loop {
        if *stopped.borrow() || !creds.usable(now_unix()) {
            // Hours in, the token has run out: stop rather than spend the rest of the session collecting 401s.
            break;
        }
        let status = tokio::select! {
            s = write_state(&client, &endpoints, &creds, "active", timing.request_timeout) => s,
            _ = stopped.wait_for(|s| *s) => break,
        };
        if status == Some(200) {
            published = true;
            backoff = timing.presence_retry;
            announced.store(true, Ordering::SeqCst);
            if wait(&mut stopped, timing.presence_heartbeat).await {
                break;
            }
            continue;
        }
        if is_final(status) {
            break;
        }
        // No answer, a rate limit or a bad gateway is a moment, not a verdict: a lost Wi-Fi minute must not leave
        // someone invisible for the rest of a session. Retry for as long as the game runs, backing off to the
        // ordinary heartbeat rather than hammering a service that is already unhappy.
        if wait(&mut stopped, backoff).await {
            break;
        }
        backoff = (backoff * 2).min(timing.presence_heartbeat);
    }
    announced.store(false, Ordering::SeqCst);
    if published {
        let _ = write_state(&client, &endpoints, &creds, "inactive", timing.presence_stop).await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::mock::{Mock, Resp};
    use super::super::tests::{test_timing, unit};
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    fn creds(expiry: Option<i64>) -> Credentials {
        Credentials {
            token: Secret::new("fake-xbl-token"),
            uhs: "1234".into(),
            xuid: "2535400000000001".into(),
            expiry,
        }
    }

    const PATH: &str = "/users/xuid(2535400000000001)/devices/current/titles/current";

    fn start(mock: &Mock, c: Credentials) -> PresenceSession {
        PresenceSession::start(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            Arc::new(Endpoints::local(&mock.base)),
            c,
            &test_timing(),
        )
    }

    fn states(mock: &Mock) -> Vec<String> {
        mock.requests()
            .iter()
            .filter(|r| r.path() == PATH)
            .map(|r| r.json()["state"].as_str().unwrap().to_string())
            .collect()
    }

    async fn until(mut f: impl FnMut() -> bool) {
        let started = Instant::now();
        while !f() {
            assert!(started.elapsed() < Duration::from_secs(10), "timed out waiting");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn the_heartbeat_is_what_xbox_live_accepts() {
        let mock = Mock::start(|_, _| Resp::status(200)).await;
        let session = start(&mock, creds(None));
        until(|| session.announced()).await;
        let r = mock.requests().into_iter().next().unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.target, PATH);
        assert_eq!(r.header("authorization"), Some("XBL3.0 x=1234;fake-xbl-token"));
        assert_eq!(r.header("x-xbl-contract-version"), Some("3"));
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.body, br#"{"state":"active"}"#, "never names a title");
        session.stop().await;
        assert_eq!(states(&mock), ["active", "inactive"]);
    }

    #[tokio::test]
    async fn it_keeps_beating_while_the_game_runs() {
        let mock = Mock::start(|_, _| Resp::status(200)).await;
        let session = start(&mock, creds(None));
        until(|| states(&mock).len() >= 3).await;
        session.stop().await;
        let s = states(&mock);
        assert!(s[..s.len() - 1].iter().all(|x| x == "active"));
        assert_eq!(s.last().unwrap(), "inactive");
    }

    #[tokio::test]
    async fn a_rejected_token_ends_the_session_instead_of_retrying() {
        let mock = Mock::start(|_, _| Resp::status(401)).await;
        let session = start(&mock, creds(None));
        tokio::time::sleep(unit() * 40).await;
        assert_eq!(states(&mock), ["active"], "one try, no inactive for what was never published");
        assert!(!session.announced());
        session.stop().await;
    }

    #[tokio::test]
    async fn a_lost_minute_or_a_rate_limit_is_retried() {
        let mock = Mock::start(|_, nth| match nth {
            1 => Resp::status(503),
            2 => Resp::status(429),
            _ => Resp::status(200),
        })
        .await;
        let session = start(&mock, creds(None));
        until(|| session.announced()).await;
        let at: Vec<Instant> = mock.requests().iter().map(|r| r.at).collect();
        // the retry, then twice it, capped at the heartbeat
        assert!(at[1] - at[0] >= test_timing().presence_retry);
        let t = test_timing();
        assert!(at[2] - at[1] >= (t.presence_retry * 2).min(t.presence_heartbeat));
        session.stop().await;
    }

    #[tokio::test]
    async fn an_expired_token_publishes_nothing() {
        let mock = Mock::start(|_, _| Resp::status(200)).await;
        let session = start(&mock, creds(Some(now_unix() + 30)));
        assert!(!session.is_active());
        session.stop().await;
        assert_eq!(mock.total(), 0);
    }

    #[tokio::test]
    async fn starting_never_waits_and_stopping_is_bounded() {
        // A service that takes far longer than the stop timeout to answer.
        let mock =
            Mock::start(
                |_, nth| if nth == 1 { Resp::status(200) } else { Resp::status(200).slow(Duration::from_secs(30)) },
            )
            .await;
        let begun = Instant::now();
        let session = start(&mock, creds(None));
        assert!(begun.elapsed() < Duration::from_millis(50), "start returns at once");
        until(|| session.announced()).await;
        let stopping = Instant::now();
        session.stop().await;
        let took = stopping.elapsed();
        assert!(took >= test_timing().presence_stop.saturating_sub(Duration::from_millis(5)), "{took:?}");
        assert!(took < test_timing().presence_stop + Duration::from_millis(500), "{took:?}");
    }

    #[tokio::test]
    async fn an_unreachable_service_never_stops_anything() {
        let session = PresenceSession::start(
            reqwest::Client::builder().no_proxy().build().unwrap(),
            Arc::new(Endpoints::local("http://127.0.0.1:9")),
            creds(None),
            &test_timing(),
        );
        tokio::time::sleep(unit() * 5).await;
        assert!(!session.announced());
        let stopping = Instant::now();
        session.stop().await;
        assert!(stopping.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn credentials_come_from_the_payload_or_not_at_all() {
        let dir = crate::testutil::scratch("xbox-presence");
        let path = dir.join("device.json");
        let write = |v: Value| std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        write(json!({"xbl_token": "t", "xbl_uhs": "1", "xbl_xuid": "25", "xbl_token_expiry_epoch": "1900000000"}));
        let c = load_credentials(&path).unwrap();
        assert_eq!((c.uhs.as_str(), c.xuid.as_str(), c.expiry), ("1", "25", Some(1_900_000_000)));
        write(json!({"xbl_token": "t", "xbl_uhs": 1, "xbl_xuid": 25}));
        let c = load_credentials(&path).unwrap();
        assert_eq!((c.xuid.as_str(), c.expiry), ("25", None));
        assert!(c.usable(now_unix()), "an unknown expiry stays usable");
        write(json!({"xbl_token": "t", "xbl_uhs": "1", "xbl_xuid": "abc"}));
        assert!(load_credentials(&path).is_none());
        write(json!({"xbl_uhs": "1", "xbl_xuid": "25"}));
        assert!(load_credentials(&path).is_none());
        std::fs::write(&path, b"{\"xbl_tok").unwrap();
        assert!(load_credentials(&path).is_none());
        assert!(load_credentials(&dir.join("missing.json")).is_none());
        assert!(!format!("{:?}", creds(None)).contains("fake-xbl-token"));
    }

    #[tokio::test]
    async fn the_social_snapshot_counts_friends_and_their_sessions() {
        let mock = Mock::start(|req, _| match req.path() {
            "/users/xuid(2535400000000001)" => Resp::json(200, json!({"state": "Online"})),
            "/users/me/people/social/decoration/multiplayer,presenceDetail" => {
                assert_eq!(req.header("x-xbl-contract-version"), Some("5"));
                Resp::json(200, json!({"people": [{"multiplayerSummary": {"x": 1}}, {"multiplayerSummary": {}}, {}]}))
            }
            _ => Resp::status(404),
        })
        .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let e = Endpoints::local(&mock.base);
        let snap = social_snapshot(&client, &e, Some(&creds(None)), Duration::from_secs(5)).await;
        assert_eq!(
            snap,
            SocialSnapshot { state: Some("Online".into()), friends: Some(3), in_session: Some(1), error: None }
        );
        let none = social_snapshot(&client, &e, None, Duration::from_secs(5)).await;
        assert!(none.error.is_some() && mock.total() == 2);
    }
}
