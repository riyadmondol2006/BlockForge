//! Which failed requests are worth trying again and how long to wait first. A busy server answering "slow down"
//! (429) or "try again" (503) is normal, so it shouldn't end an installation; being offline is not something waiting
//! fixes, so that fails straight away.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

/// How many times a request is tried in total.
pub const MAX_ATTEMPTS: u32 = 5;

const MAX_DELAY_MS: u64 = 30_000;
const BASE_DELAY_MS: u64 = 750;

/// Per host, the time (milliseconds since the epoch) before which no new requests should be sent because the host
/// asked us to wait.
static BLOCKED_UNTIL: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn is_transient_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Network errors that tend to go away by themselves: timeouts and connections that broke halfway. Not being able
/// to connect at all (no network, unknown host, refused) is not one of them, so that offline use isn't slowed down.
pub fn is_transient_error(e: &reqwest::Error) -> bool {
    if e.is_connect() {
        return false;
    }
    e.is_timeout() || e.is_request() || e.is_body()
}

/// How long to wait before the next attempt (`attempt` is the one that just failed, starting at 1).
/// `retry_after` is what the server asked for, when it said.
pub fn delay_millis(attempt: u32, retry_after: Option<u64>) -> u64 {
    if let Some(ms) = retry_after {
        return ms.min(MAX_DELAY_MS);
    }
    let exponential = MAX_DELAY_MS.min(BASE_DELAY_MS << (attempt.saturating_sub(1)).min(6));
    // a random share is added so many downloads that failed together don't all come back together
    exponential + (uuid::Uuid::new_v4().as_u128() % (exponential as u128 / 2 + 1)) as u64
}

/// Reads a `Retry-After` header: a number of seconds, or a date. `None` when it is missing or unreadable.
pub fn parse_retry_after(header: Option<&str>) -> Option<u64> {
    let value = header?.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<i64>() {
        return Some((secs.max(0) as u64).saturating_mul(1000));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some((date.timestamp_millis() - chrono::Utc::now().timestamp_millis()).max(0) as u64)
}

/// Tells every request to that host to hold off for a while.
pub fn block_host(host: &str, millis: u64) {
    let until = chrono::Utc::now().timestamp_millis() + millis as i64;
    let mut map = BLOCKED_UNTIL.lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(host.to_string()).or_insert(until);
    *entry = (*entry).max(until);
}

/// Waits out any pause that the host asked for.
pub async fn await_host(host: &str) {
    let until = BLOCKED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()).get(host).copied();
    let Some(until) = until else { return };
    let wait = until - chrono::Utc::now().timestamp_millis();
    if wait > 0 {
        tokio::time::sleep(Duration::from_millis(wait as u64)).await;
    } else {
        let mut map = BLOCKED_UNTIL.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(host) == Some(&until) {
            map.remove(host);
        }
    }
}

/// Sends a request, trying again for the failures that go away on their own. Any answer that is not a temporary
/// failure is handed back as it is (including 404 and other client errors), so the caller decides what it means.
pub async fn send(build: impl Fn() -> reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let mut attempt = 1;
    loop {
        let request = build().build().context("building the request")?;
        let host = request.url().host_str().unwrap_or_default().to_string();
        await_host(&host).await;
        match crate::http::client().execute(request).await {
            Ok(r) => {
                let code = r.status().as_u16();
                if !is_transient_status(code) || attempt >= MAX_ATTEMPTS {
                    return Ok(r);
                }
                let asked = parse_retry_after(r.headers().get("retry-after").and_then(|v| v.to_str().ok()));
                let delay = delay_millis(attempt, asked);
                if asked.is_some() && matches!(code, 429 | 503) {
                    block_host(&host, delay);
                }
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Err(e) => {
                if !is_transient_error(&e) || attempt >= MAX_ATTEMPTS {
                    return Err(e).context("the request failed");
                }
                tokio::time::sleep(Duration::from_millis(delay_millis(attempt, None))).await;
            }
        }
        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_retry_after_headers() {
        assert_eq!(parse_retry_after(Some("3")), Some(3000));
        assert_eq!(parse_retry_after(None), None);
        assert_eq!(parse_retry_after(Some("soon")), None);
        assert_eq!(parse_retry_after(Some("")), None);
        assert!(parse_retry_after(Some("Wed, 21 Oct 2099 07:28:00 GMT")).unwrap() > 0);
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let first = delay_millis(1, None);
        let later = delay_millis(5, None);
        assert!((750..=1200).contains(&first), "{first}");
        assert!(later > first);
        assert!(delay_millis(20, None) <= 45_000);
        assert_eq!(delay_millis(1, Some(100_000)), 30_000);
    }

    #[test]
    fn only_temporary_statuses_are_retried() {
        for code in [408, 425, 429, 500, 502, 503, 504] {
            assert!(is_transient_status(code));
        }
        for code in [200, 301, 400, 401, 403, 404, 410] {
            assert!(!is_transient_status(code));
        }
    }

    /// A tiny server that answers the first `failures` requests with `status` and then with 200, to watch the
    /// retries happen for real.
    async fn serve(
        failures: usize,
        status: u16,
        retry_after: Option<&'static str>,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let n = seen.fetch_add(1, Ordering::SeqCst) + 1;
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let response = if n <= failures {
                    let extra = retry_after.map(|r| format!("Retry-After: {r}\r\n")).unwrap_or_default();
                    format!("HTTP/1.1 {status} X\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n")
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string()
                };
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{addr}/file"), count)
    }

    #[tokio::test]
    async fn recovers_from_rate_limiting_and_server_errors() {
        let (url, count) = serve(2, 429, Some("0")).await;
        let r = send(|| crate::http::client().get(&url)).await.unwrap();
        assert_eq!(r.status().as_u16(), 200);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 3);

        let (url, count) = serve(1, 503, Some("0")).await;
        assert_eq!(send(|| crate::http::client().get(&url)).await.unwrap().status().as_u16(), 200);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn does_not_retry_when_the_file_is_missing() {
        let (url, count) = serve(100, 404, None).await;
        assert_eq!(send(|| crate::http::client().get(&url)).await.unwrap().status().as_u16(), 404);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod overflow_tests {
    use super::*;

    #[test]
    fn absurd_retry_after_values_never_overflow() {
        assert_eq!(parse_retry_after(Some("99999999999999999")), Some(u64::MAX));
        assert_eq!(parse_retry_after(Some("-5")), Some(0));
        assert_eq!(parse_retry_after(Some(&i64::MAX.to_string())), Some(u64::MAX));
        assert_eq!(delay_millis(1, parse_retry_after(Some("99999999999999999"))), MAX_DELAY_MS);
        assert!(delay_millis(u32::MAX, None) <= MAX_DELAY_MS * 2);
    }
}
