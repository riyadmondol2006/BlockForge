//! A local stand-in for login.live.com and Xbox Live, for tests: it records every request whole (method, target,
//! headers, body) and answers from a closure, so the signed bytes can be checked against the sent ones.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug)]
pub(crate) struct Req {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub at: Instant,
}

impl Req {
    pub(crate) fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// An `application/x-www-form-urlencoded` body as a map.
    pub(crate) fn form(&self) -> std::collections::HashMap<String, String> {
        let url = reqwest::Url::parse(&format!("http://form/?{}", String::from_utf8_lossy(&self.body))).unwrap();
        url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    }
}

pub(crate) struct Resp {
    pub status: u16,
    pub body: String,
    pub delay: Duration,
}

impl Resp {
    pub(crate) fn json(status: u16, v: Value) -> Resp {
        Resp { status, body: v.to_string(), delay: Duration::ZERO }
    }

    pub(crate) fn status(status: u16) -> Resp {
        Resp { status, body: String::new(), delay: Duration::ZERO }
    }

    pub(crate) fn slow(mut self, d: Duration) -> Resp {
        self.delay = d;
        self
    }
}

type Handler = Arc<dyn Fn(&Req, usize) -> Resp + Send + Sync>;

pub(crate) struct Mock {
    pub base: String,
    log: Arc<Mutex<Vec<Req>>>,
}

impl Mock {
    /// Start on 127.0.0.1. The handler gets each request and how many times its path has now been asked (from 1).
    pub(crate) async fn start(handler: impl Fn(&Req, usize) -> Resp + Send + Sync + 'static) -> Mock {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log: Arc<Mutex<Vec<Req>>> = Arc::default();
        let handler: Handler = Arc::new(handler);
        let l = log.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else { return };
                tokio::spawn(serve(socket, handler.clone(), l.clone()));
            }
        });
        Mock { base, log }
    }

    pub(crate) fn requests(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }

    pub(crate) fn hits(&self, path: &str) -> usize {
        self.log.lock().unwrap().iter().filter(|r| r.path() == path).count()
    }

    pub(crate) fn total(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}

async fn serve(mut socket: tokio::net::TcpStream, handler: Handler, log: Arc<Mutex<Vec<Req>>>) {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => data.extend_from_slice(&buf[..n]),
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or_default().split_whitespace();
    let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or("/").to_string());
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
    let length = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    while data.len() < head_end + length {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => data.extend_from_slice(&buf[..n]),
        }
    }
    let req = Req { method, target, headers, body: data[head_end..head_end + length].to_vec(), at: Instant::now() };
    let nth = {
        let mut log = log.lock().unwrap();
        log.push(req.clone());
        log.iter().filter(|r| r.path() == req.path()).count()
    };
    let resp = handler(&req, nth);
    if !resp.delay.is_zero() {
        tokio::time::sleep(resp.delay).await;
    }
    let head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        resp.body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(resp.body.as_bytes()).await;
    let _ = socket.shutdown().await;
}
