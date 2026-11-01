//! The Xbox game sign-in: Microsoft's legacy Live device-code flow (microsoft.com/link and a code), and renewing
//! the refresh token it leaves.
//!
//! Ported from BedrockOnLinux bol/auth.py (`NativeAuth`, `msa_refresh_result`), MIT, with credit. This is the
//! sign-in WineGDK's XUser was written against: its client ID, its `MBI_SSL` scope and the bare `device_code`
//! grant of login.live.com. It is not, and must never share anything with, BlockForge's Java sign-in
//! (`src/msauth.rs`), which uses another app and the modern endpoints.

use std::time::{Duration, Instant};

use serde_json::Value;

use super::cache::{self, MsaToken};
use super::{Secret, Xbox, blocking, cancelled, err, io_err, now_unix};
use crate::bedrock::{Cancel, ErrorKind, Result};

pub(super) const CLIENT_ID: &str = "0000000048183522";
pub(super) const SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
const FALLBACK_VERIFICATION_URI: &str = "https://www.microsoft.com/link";

/// What the player is shown. Never the `device_code` itself, which is the polling credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevicePrompt {
    pub url: String,
    pub user_code: String,
    pub expires_at_unix: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignedIn {
    /// A refresh token was already saved: nothing was asked.
    AlreadyLinked,
    Linked,
}

/// POST a form and read the JSON answer whatever the status: OAuth endpoints answer their errors with a 4xx and a
/// JSON body. `Err` is a transport failure or an answer that isn't JSON.
async fn post_form(x: &Xbox, url: &str, form: &[(&str, &str)]) -> std::result::Result<(u16, Value), ()> {
    let sent = x
        .client
        .post(url)
        .timeout(x.timing.request_timeout)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(form)
        .send()
        .await
        .map_err(|_| ())?;
    let status = sent.status().as_u16();
    let bytes = sent.bytes().await.map_err(|_| ())?;
    let json = serde_json::from_slice(&bytes).map_err(|_| ())?;
    Ok((status, json))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Seconds as Microsoft sent them (a number, or now and then a numeric string).
fn seconds(v: &Value, key: &str, default: u64) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

/// Run the device-code sign-in. `on_prompt` is called once, with the code to show; the future then waits for the
/// player, honouring Microsoft's `authorization_pending` and `slow_down`, the code's expiry and `cancel`. The
/// refresh token is saved only if no sign-out happened since this started.
pub(super) async fn sign_in(
    x: &Xbox,
    on_prompt: impl FnOnce(DevicePrompt) + Send,
    cancel: &Cancel,
) -> Result<SignedIn> {
    // The account generation is captured before anything goes out: an answer may only be kept for this one.
    let dir = x.dir.clone();
    let (token, epoch) =
        blocking(move || cache::session_snapshot(&dir)).await?.map_err(|e| io_err("reading the Xbox sign-in", e))?;
    if token.is_some() {
        return Ok(SignedIn::AlreadyLinked);
    }
    let Some(epoch) = epoch else {
        return Err(err(
            ErrorKind::SignInRequired,
            "This profile's Xbox sign-in record is damaged. Sign out of Xbox for this profile, then sign in again.",
        ));
    };

    let form = [("client_id", CLIENT_ID), ("scope", SCOPE), ("response_type", "device_code")];
    let started = tokio::select! {
        r = post_form(x, &x.endpoints.msa_connect, &form) => r,
        _ = cancel.cancelled() => return Err(cancelled()),
    };
    let start = started
        .map_err(|_| {
            err(
                ErrorKind::Network,
                "Microsoft's sign-in service could not be reached. Check the Internet connection and try again.",
            )
        })?
        .1;
    let Some(device_code) = text(&start, "device_code").map(Secret::new) else {
        let why = text(&start, "error_description")
            .or_else(|| text(&start, "error"))
            .unwrap_or_else(|| "no code in the answer".into());
        return Err(err(ErrorKind::SignInRequired, format!("Microsoft didn't start the Xbox sign-in: {why}")));
    };
    let user_code = text(&start, "user_code")
        .ok_or_else(|| err(ErrorKind::SignInRequired, "Microsoft didn't send a sign-in code. Try again."))?;
    let url = text(&start, "verification_uri").unwrap_or_else(|| FALLBACK_VERIFICATION_URI.into());
    let expires_in = seconds(&start, "expires_in", 900).max(1);
    let mut interval = seconds(&start, "interval", 5).max(1);
    on_prompt(DevicePrompt { url, user_code, expires_at_unix: now_unix() + expires_in as i64 });

    let unit = x.timing.poll_unit;
    let deadline = Instant::now() + unit * expires_in as u32;
    let poll = [("client_id", CLIENT_ID), ("grant_type", "device_code"), ("device_code", device_code.expose())];
    loop {
        let wait = unit * interval as u32;
        if Instant::now() + wait > deadline {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = cancel.cancelled() => return Err(cancelled()),
        }
        let answer = tokio::select! {
            r = post_form(x, &x.endpoints.msa_token, &poll) => r,
            _ = cancel.cancelled() => return Err(cancelled()),
        };
        // The player may take minutes at the browser: a dropped connection or a server's error page meanwhile is
        // asked again on the next turn, until the code runs out.
        let Ok((status, body)) = answer else { continue };
        if status >= 500 {
            continue;
        }
        match text(&body, "error").as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval += 5;
                continue;
            }
            Some("expired_token") => break,
            Some("authorization_declined" | "access_denied") => {
                return Err(err(ErrorKind::SignInRequired, "The Xbox sign-in was declined."));
            }
            Some(other) => {
                let why = text(&body, "error_description").unwrap_or_else(|| other.to_string());
                return Err(err(ErrorKind::SignInRequired, format!("The Xbox sign-in failed: {why}")));
            }
            None => {}
        }
        let Some(refresh) = text(&body, "refresh_token") else { continue };
        let saved = MsaToken { refresh_token: Secret::new(refresh), obtained: now_unix() };
        let dir = x.dir.clone();
        let kept = blocking(move || cache::save_token_for_epoch(&dir, &saved, &epoch))
            .await?
            .map_err(|e| io_err("saving the Xbox sign-in", e))?;
        if !kept {
            return Err(err(
                ErrorKind::Cancelled,
                "The profile was signed out of Xbox while this sign-in was waiting, so its answer was not kept.",
            ));
        }
        return Ok(SignedIn::Linked);
    }
    Err(err(ErrorKind::SignInRequired, "The Xbox sign-in code ran out before it was entered. Start the sign-in again."))
}

/// How renewing the saved sign-in went.
pub(super) enum Refreshed {
    /// Microsoft answered with a new (rotated) refresh token, and an access token for Xbox Live.
    Fresh { refresh: Secret, access: Option<Secret> },
    /// Microsoft answered and refused (`invalid_grant` and the like): signing in again is the fix.
    Rejected,
    /// Microsoft couldn't be reached, or answered with a server error: nothing is wrong with the sign-in.
    Unreachable,
}

pub(super) async fn refresh(x: &Xbox, refresh_token: &Secret) -> Refreshed {
    let form = [
        ("client_id", CLIENT_ID),
        ("scope", SCOPE),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.expose()),
    ];
    match post_form(x, &x.endpoints.msa_token, &form).await {
        Err(()) => Refreshed::Unreachable,
        Ok((status, _)) if status >= 500 => Refreshed::Unreachable,
        Ok((_, body)) => match text(&body, "refresh_token") {
            Some(refresh) => {
                Refreshed::Fresh { refresh: Secret::new(refresh), access: text(&body, "access_token").map(Secret::new) }
            }
            None => Refreshed::Rejected,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::mock::{Mock, Resp};
    use super::super::tests::{test_xbox, unit};
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn start_reply(interval: u64, expires_in: u64) -> Resp {
        Resp::json(
            200,
            json!({"device_code": "fake-device-code", "user_code": "ABCD-EFGH",
                   "verification_uri": "https://www.microsoft.com/link", "interval": interval, "expires_in": expires_in}),
        )
    }

    fn prompts() -> (Arc<Mutex<Vec<DevicePrompt>>>, impl FnOnce(DevicePrompt) + Send) {
        let seen: Arc<Mutex<Vec<DevicePrompt>>> = Arc::default();
        let s = seen.clone();
        (seen, move |p| s.lock().unwrap().push(p))
    }

    #[tokio::test]
    async fn pending_then_slow_down_then_linked() {
        let mock = Mock::start(|req, nth| match req.path() {
            "/oauth20_connect.srf" => start_reply(1, 600),
            "/oauth20_token.srf" => match nth {
                1 => Resp::json(400, json!({"error": "authorization_pending"})),
                2 => Resp::json(400, json!({"error": "slow_down"})),
                _ => Resp::json(200, json!({"access_token": "fake-access", "refresh_token": "fake-refresh-1"})),
            },
            _ => Resp::status(404),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        let (seen, on_prompt) = prompts();
        let got = sign_in(&x, on_prompt, &Cancel::default()).await.unwrap();
        assert_eq!(got, SignedIn::Linked);
        let shown = seen.lock().unwrap().clone();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].user_code, "ABCD-EFGH");
        assert!(!format!("{shown:?}").contains("fake-device-code"), "the polling credential is never shown");

        let reqs = mock.requests();
        let start = reqs.iter().find(|r| r.path() == "/oauth20_connect.srf").unwrap().form();
        assert_eq!(start["client_id"], CLIENT_ID);
        assert_eq!(start["scope"], SCOPE);
        assert_eq!(start["response_type"], "device_code");
        let polls: Vec<_> = reqs.iter().filter(|r| r.path() == "/oauth20_token.srf").collect();
        assert_eq!(polls.len(), 3);
        let form = polls[0].form();
        assert_eq!(form["grant_type"], "device_code", "the legacy live.com grant WineGDK's XUser uses");
        assert_eq!(form["device_code"], "fake-device-code");
        // After slow_down the wait grows by five intervals: 1 unit, then 6.
        let gap = polls[2].at - polls[1].at;
        assert!(gap >= unit() * 6, "{gap:?}");
        assert!(polls[1].at - polls[0].at < unit() * 6);

        let (token, _) = cache::session_snapshot(&x.dir).unwrap();
        assert_eq!(token.unwrap().refresh_token.expose(), "fake-refresh-1");
        // Signing in again with a saved token asks nothing.
        let before = mock.total();
        assert_eq!(sign_in(&x, |_| panic!("no prompt"), &Cancel::default()).await.unwrap(), SignedIn::AlreadyLinked);
        assert_eq!(mock.total(), before);
    }

    #[tokio::test]
    async fn the_code_runs_out() {
        let mock = Mock::start(|req, _| match req.path() {
            "/oauth20_connect.srf" => start_reply(1, 4),
            _ => Resp::json(400, json!({"error": "authorization_pending"})),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        let e = sign_in(&x, |_| {}, &Cancel::default()).await.unwrap_err();
        assert!(matches!(e.kind, ErrorKind::SignInRequired), "{e}");
        assert!(e.message.contains("ran out"), "{e}");
        assert!(mock.hits("/oauth20_token.srf") <= 4);
        assert!(cache::session_snapshot(&x.dir).unwrap().0.is_none());
    }

    #[tokio::test]
    async fn microsoft_saying_expired_ends_it_too() {
        let mock = Mock::start(|req, _| match req.path() {
            "/oauth20_connect.srf" => start_reply(1, 600),
            _ => Resp::json(400, json!({"error": "expired_token"})),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        let e = sign_in(&x, |_| {}, &Cancel::default()).await.unwrap_err();
        assert!(e.message.contains("ran out"), "{e}");
        assert_eq!(mock.hits("/oauth20_token.srf"), 1);
    }

    #[tokio::test]
    async fn cancel_stops_waiting_at_once() {
        let mock = Mock::start(|req, _| match req.path() {
            "/oauth20_connect.srf" => start_reply(1, 600),
            _ => Resp::json(400, json!({"error": "authorization_pending"})),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        let cancel = Cancel::default();
        let c = cancel.clone();
        let started = Instant::now();
        let flow = sign_in(&x, move |_| {}, &cancel);
        let stopper = async move {
            tokio::time::sleep(unit() * 3).await;
            c.cancel();
        };
        let (result, ()) = tokio::join!(flow, stopper);
        assert!(matches!(result.unwrap_err().kind, ErrorKind::Cancelled));
        // Long before the code (600 units) would have run out.
        assert!(started.elapsed() < unit() * 100);
        assert!(cache::session_snapshot(&x.dir).unwrap().0.is_none());
    }

    #[tokio::test]
    async fn declined_and_other_errors_are_reported() {
        let mock = Mock::start(|req, _| match req.path() {
            "/oauth20_connect.srf" => start_reply(1, 600),
            _ => Resp::json(400, json!({"error": "authorization_declined"})),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        assert!(sign_in(&x, |_| {}, &Cancel::default()).await.unwrap_err().message.contains("declined"));

        let mock = Mock::start(|req, _| match req.path() {
            "/oauth20_connect.srf" => {
                Resp::json(400, json!({"error": "invalid_client", "error_description": "bad app"}))
            }
            _ => Resp::status(500),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        let e = sign_in(&x, |_| panic!("no prompt without a code"), &Cancel::default()).await.unwrap_err();
        assert!(e.message.contains("bad app"), "{e}");
    }

    #[tokio::test]
    async fn a_server_hiccup_while_waiting_is_asked_again() {
        let mock = Mock::start(|req, nth| match req.path() {
            "/oauth20_connect.srf" => {
                Resp::json(200, json!({"device_code": "d", "user_code": "U", "interval": "1", "expires_in": "600"}))
            }
            _ if nth == 1 => Resp::status(502),
            _ => Resp::json(200, json!({"refresh_token": "fake-refresh"})),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        let (seen, on_prompt) = prompts();
        assert_eq!(sign_in(&x, on_prompt, &Cancel::default()).await.unwrap(), SignedIn::Linked);
        assert_eq!(seen.lock().unwrap()[0].url, FALLBACK_VERIFICATION_URI, "no verification_uri: microsoft.com/link");
    }

    #[tokio::test]
    async fn a_sign_out_during_the_wait_discards_the_answer() {
        let x_dir: Arc<Mutex<Option<std::path::PathBuf>>> = Arc::default();
        let d = x_dir.clone();
        let mock = Mock::start(move |req, nth| match req.path() {
            "/oauth20_connect.srf" => start_reply(1, 600),
            _ if nth == 1 => {
                // The player signs out while Microsoft is still waiting for the code.
                cache::purge(d.lock().unwrap().as_ref().unwrap()).unwrap();
                Resp::json(400, json!({"error": "authorization_pending"}))
            }
            _ => Resp::json(200, json!({"refresh_token": "fake-refresh-of-the-old-flow"})),
        })
        .await;
        let x = test_xbox(&mock, "dc");
        *x_dir.lock().unwrap() = Some(x.dir.clone());
        let e = sign_in(&x, |_| {}, &Cancel::default()).await.unwrap_err();
        assert!(matches!(e.kind, ErrorKind::Cancelled), "{e}");
        assert!(cache::session_snapshot(&x.dir).unwrap().0.is_none(), "the old flow's token was not resurrected");
    }

    #[tokio::test]
    async fn refresh_tells_rejected_from_unreachable() {
        let mock = Mock::start(|req, _| {
            let form = req.form();
            assert_eq!(form["grant_type"], "refresh_token");
            assert_eq!(form["scope"], SCOPE);
            match form["refresh_token"].as_str() {
                "good" => Resp::json(200, json!({"access_token": "a", "refresh_token": "rotated"})),
                "bad" => Resp::json(400, json!({"error": "invalid_grant"})),
                _ => Resp::status(503),
            }
        })
        .await;
        let x = test_xbox(&mock, "dc");
        match refresh(&x, &Secret::new("good")).await {
            Refreshed::Fresh { refresh, access } => {
                assert_eq!(refresh.expose(), "rotated");
                assert_eq!(access.unwrap().expose(), "a");
            }
            _ => panic!("expected a fresh token"),
        }
        assert!(matches!(refresh(&x, &Secret::new("bad")).await, Refreshed::Rejected));
        assert!(matches!(refresh(&x, &Secret::new("busy")).await, Refreshed::Unreachable));
        let gone = Xbox { endpoints: Arc::new(super::super::Endpoints::local("http://127.0.0.1:9")), ..x };
        assert!(matches!(refresh(&gone, &Secret::new("good")).await, Refreshed::Unreachable));
    }
}
