//! Ported from Xodus webview.rs and BedrockOnLinux's hosted-login patch 0004.
//! Only this process links GTK/WebKit; broker HTTP exchanges run in the parent.
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
        sync::mpsc::{self, SyncSender},
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
        Disconnected,
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
            .with_title("Microsoft Store sign-in — BlockForge")
            .with_inner_size(tao::dpi::LogicalSize::new(500.0, 700.0))
            .build(&event_loop)?;
        let mut webview: Option<wry::WebView> = None;
        let mut context = wry::WebContext::new(None); // ephemeral cookies; durable identity belongs to parent's slot
        let mut loaded = false;
        let mut opened = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        if self_test {
            proxy
                .send_event(Command::Message(Message::Open { url: "about:blank".into() }))
                .map_err(|_| std::io::Error::other("Helper event loop closed"))?;
        }
        event_loop.run_return(|event, _, flow| {
            *flow = if self_test { ControlFlow::WaitUntil(deadline) } else { ControlFlow::Wait };
            match event {
                Event::UserEvent(Command::Message(Message::Open { url })) => {
                    if !self_test && !opened && url != ipc::LOGIN { *flow = ControlFlow::Exit; return; }
                    opened = true;
                    if !(self_test && url == "about:blank" || ipc::allowed_url(&url)) { *flow = ControlFlow::Exit; return; }
                    let headers = ipc::headers(&uuid::Uuid::new_v4().to_string()).into_iter().map(|(k,v)| Ok((wry::http::HeaderName::from_bytes(k.as_bytes())?, wry::http::HeaderValue::from_str(&v)?))).collect::<Result<wry::http::HeaderMap, Box<dyn std::error::Error>>>();
                    let Ok(headers) = headers else { *flow = ControlFlow::Exit; return; };
                    if let Some(view) = &webview {
                        if view.load_url_with_headers(&url, headers).is_err() { *flow = ControlFlow::Exit; }
                        return;
                    }
                    let connection = connection.clone();
                    let proxy_load = proxy.clone();
                    let builder = WebViewBuilder::new_with_web_context(&mut context)
                        .with_url(&url).with_headers(headers).with_incognito(true)
                        .with_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64; MSAppHost/3.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/70.0.3538.102 Safari/537.36 Edge/18.26100")
                        .with_initialization_script("window.external = {notify: window.ipc.postMessage}")
                        .with_navigation_handler(|url: String| ipc::allowed_navigation(&url))
                        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                        .with_ipc_handler(move |request| {
                            if request.body().len() > ipc::LIMIT || !ipc::allowed_url(&request.uri().to_string()) { return; }
                            let Ok(data) = serde_json::from_str::<serde_json::Value>(request.body()) else { return; };
                            if data.get("sDAToken").and_then(|v| v.as_str()).is_some() && ipc::credential_page(&request.uri().to_string()) {
                                // Forward only the DA fields, each a string; never the rest of the page state.
                                let fields: Option<serde_json::Map<_, _>> = ipc::DA_FIELDS.iter().map(|f| Some((f.to_string(), serde_json::Value::String(data.get(*f)?.as_str()?.into())))).collect();
                                if let (Some(stream), Some(fields)) = (&connection, fields) { send(stream, Message::Credentials { data: fields.into() }); }
                            } else if data.get("type").and_then(|v| v.as_str()) == Some("invoke") && data.pointer("/value/name").and_then(|v| v.as_str()) == Some("CloudExperienceHost.getContext")
                                && let Some(context) = data.pointer("/value/context").and_then(|v| v.as_str()) {
                                // Post an internal UI command; never interpolate context into executable JS.
                                let _ = proxy_load.send_event(Command::Context(context.into()));
                            }
                        })
                        .with_on_page_load_handler({ let proxy = proxy.clone(); move |event, url| {
                            if matches!(event, PageLoadEvent::Finished) {
                                let _ = proxy.send_event(Command::Message(if self_test { Message::Complete } else if ipc::credential_origin(&url) { Message::Credentials { data: serde_json::Value::Null } } else { return; }));
                            }
                        }});
                    match builder.build_gtk(window.default_vbox().expect("GTK window box")) {
                        Ok(view) => webview = Some(view),
                        Err(_) => { *flow = ControlFlow::Exit; },
                    }
                },
                Event::UserEvent(Command::Context(context)) => {
                    if let Some(view) = &webview { let _ = view.evaluate_script(&ipc::host_context(&context)); }
                },
                Event::UserEvent(Command::Message(Message::Credentials { data })) if data.is_null() => {
                    if let Some(view) = &webview { let _ = view.evaluate_script(&ipc::read_server_data()); }
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
