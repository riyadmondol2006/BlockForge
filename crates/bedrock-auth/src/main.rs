//! Ported from Xodus webview.rs and BedrockOnLinux's hosted-login patch 0004.
//! Only this process links GTK/WebKit; broker HTTP exchanges run in the parent.
//! One window signs in to both: the Store's hosted sign-in, then (same webview, same login.live.com session) Xbox's
//! authorisation page, whose redirect code goes to the parent. Nothing secret is printed or kept here.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[cfg(target_os = "linux")]
#[path = "../../../src/bedrock/store/ipc.rs"]
mod ipc;
#[cfg(target_os = "linux")]
mod linux {
    use super::ipc::{self, Message};
    use std::{
        io::{Read, Write},
        os::unix::{fs::MetadataExt, net::UnixStream},
        path::PathBuf,
        sync::{
            Arc, Mutex,
            mpsc::{self, SyncSender},
        },
        time::{Duration, Instant},
    };
    use tao::{
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder},
        platform::{run_return::EventLoopExtRunReturn, unix::WindowExtUnix},
        window::WindowBuilder,
    };
    use wry::{PageLoadEvent, WebViewBuilder, WebViewBuilderExtUnix};
    enum Command {
        Message(Message),
        Context(String),
        /// The window reached the Xbox redirect: what to hand the parent.
        Xbox(Message),
        Disconnected,
    }
    /// The `state` of the Xbox authorisation under way, taken by the first redirect that answers it.
    type Pending = Arc<Mutex<Option<String>>>;
    fn capture(pending: &Pending, url: &str) -> Option<Message> {
        if !ipc::is_xbox_redirect(url) {
            return None;
        }
        let state = pending.lock().ok()?.take()?;
        ipc::xbox_capture(url, &state)
    }
    fn send(sender: &SyncSender<Message>, message: Message) {
        let _ = sender.try_send(message);
    }
    fn read(stream: &mut UnixStream) -> std::io::Result<Message> {
        let mut size = [0; 4];
        stream.read_exact(&mut size)?;
        let size = u32::from_be_bytes(size) as usize;
        if size > ipc::LIMIT {
            return Err(std::io::Error::other("Oversized IPC"));
        }
        let mut data = vec![0; size];
        stream.read_exact(&mut data)?;
        ipc::decode(&data)
    }
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let self_test = args.as_slice() == [std::ffi::OsStr::new("--self-test")];
        let mut event_loop = EventLoopBuilder::<Command>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let connection = if self_test {
            None
        } else {
            if args.len() != 2 || args[0] != "--socket" {
                return Err("Usage: blockforge-bedrock-auth --socket PRIVATE_PATH | --self-test".into());
            }
            let path = PathBuf::from(&args[1]);
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.uid() != current_uid()? || metadata.mode() & 0o777 != 0o600 {
                return Err("Unsafe Store IPC socket".into());
            }
            let stream = UnixStream::connect(path)?;
            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
            let mut reader = stream.try_clone()?;
            let proxy = proxy.clone();
            std::thread::spawn(move || {
                loop {
                    match read(&mut reader) {
                        Ok(message) => {
                            if proxy.send_event(Command::Message(message)).is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = proxy.send_event(Command::Disconnected);
                            break;
                        }
                    }
                }
            });
            let (sender, receiver) = mpsc::sync_channel(4);
            std::thread::spawn(move || {
                let mut stream = stream;
                while let Ok(message) = receiver.recv() {
                    let Ok(data) = ipc::encode(message) else {
                        break;
                    };
                    if stream.write_all(&data).is_err() {
                        break;
                    }
                }
            });
            Some(sender)
        };
        let window = WindowBuilder::new()
            .with_title("Microsoft account sign-in — BlockForge")
            .with_inner_size(tao::dpi::LogicalSize::new(500.0, 700.0))
            .build(&event_loop)?;
        let mut webview: Option<wry::WebView> = None;
        let mut context = wry::WebContext::new(None); // ephemeral cookies; durable identity belongs to parent's slot
        let mut loaded = false;
        let mut opened = false;
        let pending: Pending = Arc::default();
        // The nonce of the ServerData read under way (see `ipc::credentials_from_page`).
        let nonce: Pending = Arc::default();
        let deadline = Instant::now() + Duration::from_secs(15);
        if self_test {
            proxy
                .send_event(Command::Message(Message::Open { url: "about:blank".into() }))
                .map_err(|_| std::io::Error::other("Helper event loop closed"))?;
        }
        event_loop.run_return(|event, _, flow| {
            *flow = if self_test { ControlFlow::WaitUntil(deadline) } else { ControlFlow::Wait };
            match event {
                Event::UserEvent(Command::Message(first @ (Message::Open { .. } | Message::XboxAuthorize { .. }))) => {
                    // The Store pages get the hosted-login headers; Xbox's authorisation page is a plain page.
                    let (url, hosted) = match first {
                        Message::Open { url } => {
                            if !self_test && !opened && !ipc::is_login_url(&url) { *flow = ControlFlow::Exit; return; }
                            if !(self_test && url == "about:blank" || ipc::allowed_url(&url)) { *flow = ControlFlow::Exit; return; }
                            (url, true)
                        }
                        Message::XboxAuthorize { state, login_hint } => {
                            if !ipc::valid_state(&state) { *flow = ControlFlow::Exit; return; }
                            let url = ipc::xbox_authorize_url(&state, login_hint.as_deref());
                            if let Ok(mut p) = pending.lock() { *p = Some(state); } else { *flow = ControlFlow::Exit; return; }
                            (url, false)
                        }
                        _ => return,
                    };
                    opened = true;
                    let headers = if hosted { ipc::headers(&uuid::Uuid::new_v4().to_string()) } else { Vec::new() }.into_iter().map(|(k,v)| Ok((wry::http::HeaderName::from_bytes(k.as_bytes())?, wry::http::HeaderValue::from_str(&v)?))).collect::<Result<wry::http::HeaderMap, Box<dyn std::error::Error>>>();
                    let Ok(headers) = headers else { *flow = ControlFlow::Exit; return; };
                    if let Some(view) = &webview {
                        let loaded = if hosted { view.load_url_with_headers(&url, headers) } else { view.load_url(&url) };
                        if loaded.is_err() { *flow = ControlFlow::Exit; }
                        return;
                    }
                    let connection = connection.clone();
                    let proxy_load = proxy.clone();
                    let (proxy_nav, pending_nav, nonce_ipc) = (proxy.clone(), pending.clone(), nonce.clone());
                    let builder = WebViewBuilder::new_with_web_context(&mut context)
                        .with_url(&url).with_headers(headers).with_incognito(true)
                        .with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64; MSAppHost/3.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/70.0.3538.102 Safari/537.36 Edge/18.26100")
                        .with_initialization_script("window.external = {notify: window.ipc.postMessage}")
                        .with_navigation_handler(move |url: String| {
                            // The desktop redirect is read, never loaded: its code goes to the parent alone.
                            if ipc::is_xbox_redirect(&url) {
                                if let Some(message) = capture(&pending_nav, &url) { let _ = proxy_nav.send_event(Command::Xbox(message)); }
                                return false;
                            }
                            ipc::allowed_navigation(&url)
                        })
                        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                        .with_ipc_handler(move |request| {
                            let uri = request.uri().to_string();
                            if request.body().len() > ipc::LIMIT || !ipc::allowed_url(&uri) { return; }
                            // wry reports the main frame's address whichever frame posted, and WebKit lets every frame
                            // reach this handler. So credentials count only with the nonce of the read this window
                            // asked for, used once; any other page message is at most the host-bridge command below.
                            let expected = nonce_ipc.lock().ok().and_then(|n| n.clone());
                            if let Some(message) = ipc::credentials_from_page(request.body(), &uri, expected.as_deref()) {
                                if let Ok(mut n) = nonce_ipc.lock() { *n = None; }
                                if let Some(stream) = &connection { send(stream, message); }
                                return;
                            }
                            let Ok(data) = serde_json::from_str::<serde_json::Value>(request.body()) else { return; };
                            if data.get("type").and_then(|v| v.as_str()) == Some("invoke") && data.pointer("/value/name").and_then(|v| v.as_str()) == Some("CloudExperienceHost.getContext")
                                && let Some(context) = data.pointer("/value/context").and_then(|v| v.as_str()) {
                                // Post an internal UI command; never interpolate context into executable JS.
                                let _ = proxy_load.send_event(Command::Context(context.into()));
                            }
                        })
                        .with_on_page_load_handler({ let (proxy, pending) = (proxy.clone(), pending.clone()); move |event, url| {
                            // Should a redirect ever get past the navigation policy, its load is caught here.
                            if let Some(message) = capture(&pending, &url) { let _ = proxy.send_event(Command::Xbox(message)); return; }
                            if matches!(event, PageLoadEvent::Finished) {
                                let _ = proxy.send_event(Command::Message(if self_test { Message::Complete } else if ipc::credential_origin(&url) { Message::Credentials { data: serde_json::Value::Null } } else { return; }));
                            }
                        }});
                    match builder.build_gtk(window.default_vbox().expect("GTK window box")) {
                        Ok(view) => webview = Some(view),
                        Err(_) => { *flow = ControlFlow::Exit; },
                    }
                },
                Event::UserEvent(Command::Xbox(message)) => { if let Some(stream) = &connection { send(stream, message); } },
                Event::UserEvent(Command::Context(context)) => {
                    if let Some(view) = &webview { let _ = view.evaluate_script(&ipc::host_context(&context)); }
                },
                Event::UserEvent(Command::Message(Message::Credentials { data })) if data.is_null() => {
                    // A fresh nonce for every read: whatever answers must carry it, once.
                    let fresh = uuid::Uuid::new_v4().simple().to_string();
                    if let (Some(view), Some(script)) = (&webview, ipc::read_server_data(&fresh)) {
                        if let Ok(mut n) = nonce.lock() { *n = Some(fresh); }
                        let _ = view.evaluate_script(&script);
                    }
                },
                Event::UserEvent(Command::Message(Message::Complete)) => { loaded = true; *flow = ControlFlow::Exit; },
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => { if let Some(stream) = &connection { send(stream, Message::Cancelled); } *flow = ControlFlow::Exit; },
                Event::UserEvent(Command::Disconnected) => *flow = ControlFlow::Exit,
                _ => (),
            }
            if self_test && Instant::now() >= deadline { *flow = ControlFlow::Exit; }
        });
        if self_test && !loaded {
            return Err("WebKitGTK self-test did not finish loading".into());
        }
        Ok(())
    }
    fn current_uid() -> std::io::Result<u32> {
        Ok(std::fs::metadata("/proc/self")?.uid())
    }
}
fn main() {
    #[cfg(target_os = "linux")]
    if linux::run().is_err() {
        eprintln!("Store sign-in helper failed; check display and WebKitGTK 4.1");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("Store sign-in is supported on Linux only");
        std::process::exit(1);
    }
}
