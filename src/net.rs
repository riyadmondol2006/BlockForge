//! Network behaviour shared by every download: the proxy, the timeout, how many files go at once, and what to
//! do when a server is busy. It follows the Java edition's `Network`, `DownloadPool` and `RetryPolicy`.
//!
//! The shared client in [`crate::http`] is built from a [`NetConfig`]. Call [`crate::http::configure`] with
//! [`NetConfig::from_settings`] once at start-up, before the first request, to make the player's proxy, timeout
//! and connection limit count.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::svcsettings::{DEFAULT_CONCURRENT_CONNECTIONS, ServiceSettings};

/// Files downloaded at once when the settings were never applied (the figure used before this module existed).
pub const DEFAULT_CONCURRENCY: usize = 48;
/// How many times a request is tried in total (`RetryPolicy.MAX_ATTEMPTS`).
pub const MAX_ATTEMPTS: u32 = 5;
const MAX_DELAY_MS: u64 = 30_000;
const BASE_DELAY_MS: u64 = 750;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProxyType {
    Http,
    Socks,
    Direct,
}

impl ProxyType {
    /// In the order the Java edition shows them.
    pub const ALL: [ProxyType; 3] = [ProxyType::Http, ProxyType::Socks, ProxyType::Direct];

    pub fn key(self) -> &'static str {
        match self {
            ProxyType::Http => "HTTP",
            ProxyType::Socks => "SOCKS",
            ProxyType::Direct => "DIRECT",
        }
    }

    pub fn from_key(key: &str) -> Option<ProxyType> {
        ProxyType::ALL.into_iter().find(|t| t.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProxyConfig {
    pub kind: ProxyType,
    pub host: String,
    pub port: u16,
}

impl ProxyConfig {
    /// The address handed to the HTTP library, or `None` for a direct connection.
    pub fn url(&self) -> Option<String> {
        match self.kind {
            ProxyType::Direct => None,
            // `socks5h` lets the proxy look up host names, which is what Java's SOCKS proxies do
            ProxyType::Socks => Some(format!("socks5h://{}:{}", self.host, self.port)),
            ProxyType::Http => Some(format!("http://{}:{}", self.host, self.port)),
        }
    }
}

/// Everything the shared HTTP client needs to know from the settings.
#[derive(Clone, Debug, PartialEq)]
pub struct NetConfig {
    pub timeout: Duration,
    /// How many files `install::download_all` fetches at once, already adjusted for this computer.
    pub concurrency: usize,
    pub proxy: Option<ProxyConfig>,
}

impl NetConfig {
    pub fn from_settings(s: &ServiceSettings) -> NetConfig {
        let (total_mb, _) = crate::platform::memory_mb();
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let proxy =
            (s.enable_proxy && !s.proxy_host.trim().is_empty() && (1..=65535).contains(&s.proxy_port)).then(|| {
                ProxyConfig { kind: s.proxy_type, host: s.proxy_host.trim().to_string(), port: s.proxy_port as u16 }
            });
        NetConfig {
            timeout: Duration::from_secs(s.connection_timeout.max(1) as u64),
            concurrency: for_computer(
                s.concurrent_connections as usize,
                DEFAULT_CONCURRENT_CONNECTIONS as usize,
                total_mb,
                cores,
            ),
            proxy,
        }
    }
}

/// The user agent every request carries.
pub fn user_agent() -> String {
    format!("{}/{} (https://github.com/riyadmondol2006/BlockForge)", crate::NAME, crate::VERSION)
}

/// Build the shared client. With no config this is the client the program always had.
pub fn build_client(config: Option<&NetConfig>) -> reqwest::Client {
    let mut b = reqwest::Client::builder()
        .user_agent(user_agent())
        .pool_max_idle_per_host(64)
        .pool_idle_timeout(Duration::from_secs(60))
        .tcp_nodelay(true);
    b = match config {
        Some(c) => b.connect_timeout(c.timeout).read_timeout(c.timeout),
        None => b.connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(30)),
    };
    if let Some(proxy) = config.and_then(|c| c.proxy.as_ref()) {
        match proxy.url() {
            Some(url) => match reqwest::Proxy::all(&url) {
                Ok(p) => b = b.proxy(p),
                Err(e) => crate::logs::warn(format!("The proxy {url} can't be used ({e}); connecting directly")),
            },
            // "DIRECT" means: don't use any proxy, not even the one the system names
            None => b = b.no_proxy(),
        }
    }
    b.build().expect("HTTP client")
}

static MODRINTH_KEY: Mutex<Option<String>> = Mutex::new(None);

/// Remember the player's Modrinth API key (the `modrinthApiKey` setting). Requests to Modrinth's API sent through
/// [`send_with_retry`] (so everything fetched with `http::get_bytes` and `http::get_json`) carry it, as the Java
/// edition's `ModrinthApi` does; the key is never sent to any other host.
pub fn set_modrinth_api_key(key: Option<String>) {
    *MODRINTH_KEY.lock().unwrap_or_else(|e| e.into_inner()) = key.filter(|k| !k.trim().is_empty());
}

pub fn modrinth_api_key() -> Option<String> {
    MODRINTH_KEY.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The `Authorization` value to send to `host`, if any.
fn authorization_for(host: &str) -> Option<String> {
    if host == "api.modrinth.com" { modrinth_api_key() } else { None }
}

/// Ask `url` through `proxy` and say whether it answered with 200 (`Utils.testProxy`, used by the settings page).
pub async fn test_proxy(proxy: &ProxyConfig, url: &str) -> Result<()> {
    let config = NetConfig { timeout: Duration::from_secs(15), concurrency: 1, proxy: Some(proxy.clone()) };
    let client = build_client(Some(&config));
    let r = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("couldn't connect through the proxy {}:{}", proxy.host, proxy.port))?;
    crate::logs::info(format!("Proxy returned code {} when testing!", r.status().as_u16()));
    if r.status().as_u16() != 200 {
        anyhow::bail!("the proxy answered with HTTP {}", r.status().as_u16());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// How many downloads at once (DownloadPool)

/// What the player set, or when it is still the default, what suits this computer: 8 with two cores or 4 GB of
/// memory or less, otherwise the default.
pub fn for_computer(configured: usize, default_configured: usize, total_mb: u64, cores: usize) -> usize {
    if configured != default_configured {
        return configured;
    }
    let small = (total_mb > 0 && total_mb <= 4608) || (cores > 0 && cores <= 2);
    if small { configured.min(8) } else { configured }
}

/// How many files to fetch at once: the configured number, but no more than there are files, and on computers
/// with few cores no more than 4 per core (but never below 8).
pub fn download_threads(configured: usize, processors: usize, files: usize) -> usize {
    let threads = configured.max(1).min(8.max(processors * 4));
    threads.min(files).max(1)
}

/// How many files to check on disk at once.
pub fn check_threads(processors: usize, files: usize) -> usize {
    files.min(2.max((processors * 2).min(16))).max(1)
}

/// The order to start downloads in: the few biggest first, one per thread, since a large file on one connection is
/// what everything else ends up waiting for. The rest keep their order. Returns indices into `sizes`.
pub fn start_order(sizes: &[u64], threads: usize) -> Vec<usize> {
    let mut by_size: Vec<usize> = (0..sizes.len()).collect();
    by_size.sort_by(|a, b| sizes[*b].cmp(&sizes[*a]).then(a.cmp(b)));
    by_size.truncate(threads.max(1).min(sizes.len()));
    let mut ordered = by_size.clone();
    ordered.extend((0..sizes.len()).filter(|i| !by_size.contains(i)));
    ordered
}

// ---------------------------------------------------------------------------------------------------------
// When a download fails (RetryPolicy)

/// Answers that mean "busy, try again": request timeout, too early, too many requests, and server errors.
pub fn is_transient_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Errors that tend to go away on their own (a timeout, a dropped connection). Being offline (no route, no such
/// host, connection refused) is not one of them, so offline use fails at once instead of waiting.
pub fn is_transient_error(e: &reqwest::Error) -> bool {
    if e.is_timeout() {
        return true;
    }
    if e.is_connect() || e.is_builder() || e.is_redirect() {
        return false;
    }
    true
}

/// How long to wait before the next try. `attempt` is the one that just failed, counting from 1. A server's
/// `Retry-After` wins; otherwise the wait doubles each time, with some random extra so downloads that failed
/// together don't all come back together.
pub fn delay_millis(attempt: u32, retry_after_ms: Option<u64>) -> u64 {
    if let Some(ms) = retry_after_ms {
        return ms.min(MAX_DELAY_MS);
    }
    let exponential = MAX_DELAY_MS.min(BASE_DELAY_MS << (attempt.saturating_sub(1)).min(6));
    exponential + jitter(exponential / 2 + 1)
}

fn jitter(range: u64) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos % range.max(1)
}

/// `Retry-After` is either a number of seconds or a date.
pub fn parse_retry_after(header: &str) -> Option<u64> {
    let value = header.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(secs.saturating_mul(1000));
    }
    let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let ms = (when.timestamp_millis() - chrono::Utc::now().timestamp_millis()).max(0);
    Some(ms as u64)
}

static BLOCKED_UNTIL: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Tell every request to `host` to hold off for a while, because the host asked us to.
pub fn block_host(host: &str, millis: u64) {
    let until = Instant::now() + Duration::from_millis(millis);
    let mut guard = BLOCKED_UNTIL.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let slot = map.entry(host.to_string()).or_insert(until);
    if *slot < until {
        *slot = until;
    }
}

/// Wait out any pause the host asked for.
pub async fn await_host(host: &str) {
    let until = BLOCKED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(host).copied());
    if let Some(until) = until {
        let now = Instant::now();
        if until > now {
            tokio::time::sleep(until - now).await;
        }
    }
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("").to_string()
}

/// Send a request, trying again the way the Java edition does. `build` makes a fresh request each time. A
/// finished answer is returned whatever its status; only busy-server answers and dropped connections are retried.
pub async fn send_with_retry(build: impl Fn() -> reqwest::RequestBuilder, url: &str) -> Result<reqwest::Response> {
    let host = host_of(url);
    let mut attempt = 1;
    loop {
        await_host(&host).await;
        let request = match authorization_for(&host) {
            Some(key) => build().header("Authorization", key),
            None => build(),
        };
        match request.send().await {
            Ok(r) => {
                if attempt < MAX_ATTEMPTS && is_transient_status(r.status().as_u16()) {
                    let retry_after =
                        r.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(parse_retry_after);
                    let delay = delay_millis(attempt, retry_after);
                    if r.status().as_u16() == 429 || retry_after.is_some() {
                        block_host(&host, delay);
                    }
                    crate::logs::info(format!(
                        "Server answered HTTP {} while downloading {url}, trying again in {:.1}s (attempt {} of {MAX_ATTEMPTS})",
                        r.status().as_u16(),
                        delay as f64 / 1000.0,
                        attempt + 1
                    ));
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    attempt += 1;
                    continue;
                }
                return Ok(r);
            }
            Err(e) => {
                if attempt >= MAX_ATTEMPTS || !is_transient_error(&e) {
                    return Err(anyhow::Error::from(e).context(format!("GET {url}")));
                }
                let delay = delay_millis(attempt, None);
                crate::logs::info(format!(
                    "Network error while downloading {url}, trying again in {:.1}s (attempt {} of {MAX_ATTEMPTS})",
                    delay as f64 / 1000.0,
                    attempt + 1
                ));
                tokio::time::sleep(Duration::from_millis(delay)).await;
                attempt += 1;
            }
        }
    }
}

/// A GET that retries; the body of `http::get_bytes`. A 4xx answer other than 429 is final.
pub async fn get_bytes_with_retry(url: &str) -> Result<Vec<u8>> {
    let mut attempt = 1;
    loop {
        let r = send_with_retry(|| crate::http::client().get(url), url).await?;
        let status = r.status();
        if status.is_success() {
            match r.bytes().await {
                Ok(b) => return Ok(b.to_vec()),
                Err(e) => {
                    // the body broke off half way: the same as a dropped connection
                    if attempt >= MAX_ATTEMPTS {
                        return Err(anyhow::Error::from(e).context(format!("GET {url}")));
                    }
                    tokio::time::sleep(Duration::from_millis(delay_millis(attempt, None))).await;
                    attempt += 1;
                    continue;
                }
            }
        }
        anyhow::bail!("{url}: HTTP {status}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_statuses_are_the_java_editions() {
        for code in [408, 425, 429, 500, 502, 503, 504] {
            assert!(is_transient_status(code), "{code}");
        }
        for code in [200, 301, 400, 401, 403, 404, 410, 501] {
            assert!(!is_transient_status(code), "{code}");
        }
    }

    #[test]
    fn waits_grow_and_stop_at_the_cap() {
        let first = delay_millis(1, None);
        assert!((750..=750 + 375).contains(&first), "{first}");
        let fourth = delay_millis(4, None);
        assert!((6000..=6000 + 3000).contains(&fourth), "{fourth}");
        let late = delay_millis(20, None);
        assert!((30_000..=30_000 + 15_000).contains(&late), "{late}");
        assert_eq!(delay_millis(2, Some(5_000)), 5_000);
        assert_eq!(delay_millis(2, Some(500_000)), MAX_DELAY_MS, "a server can't make us wait for ever");
    }

    #[test]
    fn retry_after_in_both_forms() {
        assert_eq!(parse_retry_after("7"), Some(7000));
        assert_eq!(parse_retry_after(" 0 "), Some(0));
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("soon"), None);
        let future = (chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc2822();
        let ms = parse_retry_after(&future).unwrap();
        assert!((80_000..=91_000).contains(&ms), "{ms}");
        let past = (chrono::Utc::now() - chrono::Duration::seconds(90)).to_rfc2822();
        assert_eq!(parse_retry_after(&past), Some(0));
    }

    #[test]
    fn thread_counts_follow_the_download_pool() {
        // default setting on a small computer drops to 8; an explicit setting is kept
        assert_eq!(for_computer(16, 16, 4096, 8), 8);
        assert_eq!(for_computer(16, 16, 16384, 2), 8);
        assert_eq!(for_computer(16, 16, 16384, 8), 16);
        assert_eq!(for_computer(32, 16, 2048, 2), 32);
        assert_eq!(for_computer(4, 16, 2048, 2), 4);
        // never more threads than files, nor more than 4 per core (at least 8)
        assert_eq!(download_threads(16, 8, 100), 16);
        assert_eq!(download_threads(100, 2, 1000), 8);
        assert_eq!(download_threads(100, 16, 1000), 64);
        assert_eq!(download_threads(16, 8, 3), 3);
        assert_eq!(download_threads(0, 8, 0), 1);
        assert_eq!(check_threads(8, 100), 16);
        assert_eq!(check_threads(1, 100), 2);
    }

    #[test]
    fn biggest_files_start_first() {
        let order = start_order(&[5, 100, 7, 90, 1], 2);
        assert_eq!(order, vec![1, 3, 0, 2, 4]);
        assert_eq!(start_order(&[], 4), Vec::<usize>::new());
        assert_eq!(start_order(&[3, 3], 8), vec![0, 1]);
    }

    #[test]
    fn proxy_settings_become_addresses() {
        let p = |kind| ProxyConfig { kind, host: "10.0.0.2".into(), port: 3128 };
        assert_eq!(p(ProxyType::Http).url().as_deref(), Some("http://10.0.0.2:3128"));
        assert_eq!(p(ProxyType::Socks).url().as_deref(), Some("socks5h://10.0.0.2:3128"));
        assert_eq!(p(ProxyType::Direct).url(), None);
        assert_eq!(ProxyType::from_key("SOCKS"), Some(ProxyType::Socks));
        assert_eq!(ProxyType::from_key("socks"), None);
    }

    #[test]
    fn config_comes_from_settings() {
        let mut s = ServiceSettings::default();
        s.connection_timeout = 90;
        s.concurrent_connections = 24;
        s.enable_proxy = true;
        s.proxy_host = " proxy.local ".into();
        s.proxy_port = 8888;
        let c = NetConfig::from_settings(&s);
        assert_eq!(c.timeout, Duration::from_secs(90));
        assert_eq!(c.concurrency, 24);
        assert_eq!(c.proxy, Some(ProxyConfig { kind: ProxyType::Http, host: "proxy.local".into(), port: 8888 }));
        s.proxy_host.clear();
        assert!(NetConfig::from_settings(&s).proxy.is_none(), "a proxy without a host is ignored");
        // every client kind builds
        for kind in ProxyType::ALL {
            let mut c = c.clone();
            c.proxy = Some(ProxyConfig { kind, host: "127.0.0.1".into(), port: 1080 });
            let _ = build_client(Some(&c));
        }
        let _ = build_client(None);
    }

    #[test]
    fn hosts_are_taken_from_urls() {
        assert_eq!(host_of("https://api.modrinth.com/v2/search?q=a"), "api.modrinth.com");
        assert_eq!(host_of("http://localhost:8080/x"), "localhost:8080");
    }

    async fn read_head(sock: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    #[tokio::test]
    async fn requests_go_through_an_http_proxy() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let head = read_head(&mut sock).await;
            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nvia-proxy").await;
            head
        });
        let cfg = NetConfig {
            timeout: Duration::from_secs(5),
            concurrency: 4,
            proxy: Some(ProxyConfig { kind: ProxyType::Http, host: "127.0.0.1".into(), port }),
        };
        let body = build_client(Some(&cfg))
            .get("http://files.blockforge.invalid/thing.json")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "via-proxy");
        let head = seen.await.unwrap();
        assert!(head.starts_with("GET http://files.blockforge.invalid/thing.json HTTP/1.1"), "{head}");
    }

    #[tokio::test]
    async fn requests_go_through_a_socks_proxy() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut hello = vec![0u8; 3];
            use tokio::io::AsyncReadExt;
            sock.read_exact(&mut hello).await.unwrap();
            // choose "no authentication", then refuse the connection request
            let _ = sock.write_all(&[5, 0]).await;
            let mut request = vec![0u8; 64];
            let n = sock.read(&mut request).await.unwrap_or(0);
            let _ = sock.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            (hello, request[..n].to_vec())
        });
        let cfg = NetConfig {
            timeout: Duration::from_secs(5),
            concurrency: 4,
            proxy: Some(ProxyConfig { kind: ProxyType::Socks, host: "127.0.0.1".into(), port }),
        };
        let result = build_client(Some(&cfg)).get("http://files.blockforge.invalid/").send().await;
        assert!(result.is_err(), "the fake proxy refuses");
        let (hello, request) = seen.await.unwrap();
        assert_eq!(hello[0], 5, "a SOCKS5 greeting reached the proxy");
        assert!(
            request.windows(21).any(|w| w == b"files.blockforge.inva"),
            "the host name is sent to the proxy to look up (socks5h)"
        );
    }

    #[tokio::test]
    async fn the_timeout_setting_is_used() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // accepts, then says nothing
        let hold = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
            drop(sock);
        });
        let cfg = NetConfig { timeout: Duration::from_secs(1), concurrency: 4, proxy: None };
        let started = Instant::now();
        let err = build_client(Some(&cfg)).get(format!("http://127.0.0.1:{port}/")).send().await.unwrap_err();
        assert!(err.is_timeout(), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "gave up after about the configured second, took {:?}",
            started.elapsed()
        );
        assert!(is_transient_error(&err), "a timeout is worth trying again");
        hold.abort();
    }

    /// A tiny server: answers each connection with the next canned response, then closes it.
    async fn canned_server(responses: Vec<&'static str>) -> (u16, tokio::task::JoinHandle<usize>) {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let mut served = 0;
            for r in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let _ = read_head(&mut sock).await;
                let _ = sock.write_all(r.as_bytes()).await;
                served += 1;
            }
            served
        });
        (port, handle)
    }

    #[tokio::test]
    async fn a_busy_server_is_asked_again() {
        let (port, served) = canned_server(vec![
            "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
        ])
        .await;
        let bytes = get_bytes_with_retry(&format!("http://127.0.0.1:{port}/file")).await.unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(served.await.unwrap(), 3, "two busy answers, then the file");
    }

    #[tokio::test]
    async fn the_modrinth_key_only_goes_to_modrinth() {
        assert_eq!(authorization_for("api.modrinth.com"), None, "no key set yet");
        set_modrinth_api_key(Some("mrp_secret".into()));
        assert_eq!(authorization_for("api.modrinth.com").as_deref(), Some("mrp_secret"));
        assert_eq!(authorization_for("cdn.modrinth.com"), None);
        assert_eq!(authorization_for("127.0.0.1:80"), None);
        set_modrinth_api_key(Some("   ".into()));
        assert_eq!(modrinth_api_key(), None, "a blank key is no key");
        set_modrinth_api_key(None);
    }

    #[tokio::test]
    async fn a_missing_file_is_final() {
        let (port, served) =
            canned_server(vec!["HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"]).await;
        let err = get_bytes_with_retry(&format!("http://127.0.0.1:{port}/nope")).await.unwrap_err();
        assert!(format!("{err}").contains("HTTP 404"), "{err}");
        assert_eq!(served.await.unwrap(), 1, "a 404 is not asked again");
    }

    #[tokio::test]
    async fn a_refused_connection_is_not_retried() {
        // nothing listens on this port: being offline fails at once instead of waiting through five tries
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/")).send().await.unwrap_err();
        assert!(err.is_connect() && !is_transient_error(&err));
    }

    #[tokio::test]
    async fn a_busy_host_makes_requests_wait() {
        block_host("blocked.example", 120);
        let t = Instant::now();
        await_host("blocked.example").await;
        assert!(t.elapsed() >= Duration::from_millis(100));
        let t = Instant::now();
        await_host("free.example").await;
        assert!(t.elapsed() < Duration::from_millis(50));
    }
}
