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
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Reply {
        Reply { status: 200, body: body.into(), cut_after: None }
    }

    pub fn status(status: u16) -> Reply {
        Reply { status, body: Vec::new(), cut_after: None }
    }

    pub fn cut(body: impl Into<Vec<u8>>, after: usize) -> Reply {
        Reply { status: 200, body: body.into(), cut_after: Some(after) }
    }
}

/// A web server on 127.0.0.1 for the length of a test. Paths it doesn't know get a 404.
pub struct Server {
    pub base: String,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    hits: Arc<Mutex<HashMap<String, usize>>>,
    total: Arc<AtomicUsize>,
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
        let (r, h, t) = (routes.clone(), hits.clone(), total.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let (r, h, t) = (r.clone(), h.clone(), t.clone());
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
                    *h.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
                    let reply = r.lock().unwrap().get(&path).cloned().unwrap_or(Reply::status(404));
                    let head = format!(
                        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
                        reply.body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let send = reply.cut_after.unwrap_or(reply.body.len()).min(reply.body.len());
                    let _ = socket.write_all(&reply.body[..send]).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Server { base, routes, hits, total }
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
