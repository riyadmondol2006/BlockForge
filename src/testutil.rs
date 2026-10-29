//! Helpers for tests: a scratch folder and a small local web server that answers what a test tells it to, so
//! downloads, retries and failures can be tried for real without the internet.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A fresh, empty folder for one test.
pub fn scratch(tag: &str) -> PathBuf {
    let p =
        std::env::temp_dir().join(format!("bf-test-{tag}-{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// What the server answers for one path.
#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    /// Send only this many bytes of the body and then hang up, while announcing all of it (a dropped connection).
    pub cut_after: Option<usize>,
    /// Wait this long before answering (a far-away server).
    pub delay_ms: u64,
    /// The first time only: send this many bytes of the body, then keep the connection open and send nothing more (a
    /// connection that stalls part-way). Later requests get all of it.
    pub stall_first: Option<usize>,
    /// Say `Accept-Ranges: bytes` and answer a `Range: bytes=a-b` request with just those bytes (206).
    pub ranges: bool,
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Reply {
        Reply { status: 200, body: body.into(), cut_after: None, delay_ms: 0, stall_first: None, ranges: false }
    }

    pub fn status(status: u16) -> Reply {
        Reply { status, body: Vec::new(), cut_after: None, delay_ms: 0, stall_first: None, ranges: false }
    }

    pub fn cut(body: impl Into<Vec<u8>>, after: usize) -> Reply {
        Reply { status: 200, body: body.into(), cut_after: Some(after), delay_ms: 0, stall_first: None, ranges: false }
    }

    /// All of `body`, but the first request gets only `after` bytes of it and then nothing more.
    pub fn stall_first(body: impl Into<Vec<u8>>, after: usize) -> Reply {
        Reply { status: 200, body: body.into(), cut_after: None, delay_ms: 0, stall_first: Some(after), ranges: false }
    }

    /// The same answer, also in pieces to whoever asks for a range.
    pub fn with_ranges(mut self) -> Reply {
        self.ranges = true;
        self
    }

    /// The same answer, given after `ms` milliseconds.
    pub fn slow(mut self, ms: u64) -> Reply {
        self.delay_ms = ms;
        self
    }
}

/// A web server on 127.0.0.1 for the length of a test. Paths it doesn't know get a 404.
pub struct Server {
    pub base: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    hits: Arc<Mutex<HashMap<String, usize>>>,
    total: Arc<AtomicUsize>,
    /// Requests being answered right now, and the most there ever were at once.
    busy: Arc<AtomicUsize>,
    most_busy: Arc<AtomicUsize>,
}

impl Server {
    /// Start it on the current tokio runtime.
    pub async fn start() -> Server {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
        let hits: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let total = Arc::new(AtomicUsize::new(0));
        let (busy, most_busy) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (r, h, t, b, m) = (routes.clone(), hits.clone(), total.clone(), busy.clone(), most_busy.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let (r, h, t, b, m) = (r.clone(), h.clone(), t.clone(), b.clone(), m.clone());
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 4096];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                    t.fetch_add(1, Ordering::SeqCst);
                    let nth = {
                        let mut hits = h.lock().unwrap();
                        let n = hits.entry(path.clone()).or_insert(0);
                        *n += 1;
                        *n
                    };
                    let reply = r.lock().unwrap().get(&path).cloned().unwrap_or(Reply::status(404));
                    let now = b.fetch_add(1, Ordering::SeqCst) + 1;
                    m.fetch_max(now, Ordering::SeqCst);
                    if reply.delay_ms > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(reply.delay_ms)).await;
                    }
                    b.fetch_sub(1, Ordering::SeqCst);
                    let range = text
                        .lines()
                        .filter_map(|l| l.split_once(':'))
                        .find(|(k, _)| k.trim().eq_ignore_ascii_case("range"))
                        .and_then(|(_, v)| v.trim().strip_prefix("bytes=")?.split_once('-'))
                        .and_then(|(a, z)| Some((a.parse::<usize>().ok()?, z.parse::<usize>().ok()?)))
                        .filter(|_| reply.ranges && reply.status == 200);
                    let total = reply.body.len();
                    let (status, body, extra) = match range {
                        Some((a, z)) => {
                            let z = z.min(total.saturating_sub(1));
                            (206, &reply.body[a.min(z + 1)..=z], format!("Content-Range: bytes {a}-{z}/{total}\r\n"))
                        }
                        None => (reply.status, &reply.body[..], String::new()),
                    };
                    let accept = if reply.ranges { "Accept-Ranges: bytes\r\n" } else { "" };
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n{accept}{extra}Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let stall = reply.stall_first.filter(|_| nth == 1);
                    let send = stall.or(reply.cut_after).unwrap_or(body.len()).min(body.len());
                    let _ = socket.write_all(&body[..send]).await;
                    if stall.is_some() {
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    }
                    let _ = socket.shutdown().await;
                });
            }
        });
        Server { base, routes, hits, total, busy, most_busy }
    }

    /// The most requests that were being answered at the same moment.
    pub fn most_at_once(&self) -> usize {
        let _ = &self.busy;
        self.most_busy.load(Ordering::SeqCst)
    }

    /// Answer `path` (starting with `/`) with `reply` from now on, and return its full address.
    pub fn route(&self, path: &str, reply: Reply) -> String {
        self.routes.lock().unwrap().insert(path.to_string(), reply);
        format!("{}{path}", self.base)
    }

    /// How many requests `path` got.
    pub fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }

    /// How many requests came in altogether.
    pub fn total(&self) -> usize {
        self.total.load(Ordering::SeqCst)
    }
}

/// A runtime for tests that call async code from a plain `#[test]`.
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

/// The SHA-1 of `bytes` in hex, as download jobs expect it.
pub fn sha1(bytes: &[u8]) -> String {
    crate::http::sha1_hex(bytes)
}
