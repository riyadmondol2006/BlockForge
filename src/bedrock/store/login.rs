//! Ported from Xodus commands/login.rs and BedrockOnLinux bol/xodus.py.
use super::*;
use crate::bedrock::xbox::Secret;
use ipc::Message;
use tokens::{Slot, store_error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xodus::{
    models::{
        live::{DAProperty, ExchangeUserTokenOutcome},
        secrets::{LegacyToken, Token, User},
        soap,
    },
    tokens::{TokenManager, store::TokenBackend},
};

async fn send(stream: &mut tokio::net::UnixStream, message: Message) -> Result<()> {
    stream.write_all(&ipc::encode(message).map_err(integrity)?).await.map_err(io_error)
}
pub(super) async fn receive(stream: &mut tokio::net::UnixStream) -> Result<Message> {
    let size = stream.read_u32().await.map_err(ipc_error)? as usize;
    if size > ipc::LIMIT {
        return Err(integrity("Oversized IPC frame"));
    }
    let mut data = vec![0; size];
    stream.read_exact(&mut data).await.map_err(ipc_error)?;
    ipc::decode(&data).map_err(integrity)
}
struct SocketRoot(PathBuf);
impl Drop for SocketRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// What one sign-in window is asked to do: the Store sign-in (unless `slot` is None: it is already signed in),
/// then `xbox_codes` Xbox authorisations in the same session, one per game profile to link.
pub struct Window {
    /// The BlockForge account's sign-in name, prefilled when it is an e-mail address.
    pub login_hint: Option<String>,
    pub xbox_codes: usize,
}
/// What the window's Xbox step gave for one code asked for.
pub enum XboxStep {
    /// The redirect's authorisation code, for `xbox::sign_in_with_code`. Single use, short-lived.
    Code(Secret),
    /// None came (the reason is a short code, never shown as is): the device code is the way left.
    Unavailable(String),
}
/// How long one Xbox authorisation may wait for the player (an "allow" click, or signing in again).
const XBOX_STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

pub async fn run(ctx: &Context, slot: Option<&Slot>, window: &Window, cancel: &Cancel) -> Result<Vec<XboxStep>> {
    use std::os::unix::fs::PermissionsExt;
    let socket_dir = PathBuf::from("/dev/shm").join(format!("bf-store-auth-{}", uuid::Uuid::new_v4().simple()));
    tokens::private_dir(&socket_dir)?;
    let _cleanup = SocketRoot(socket_dir.clone());
    let socket = socket_dir.join("ipc");
    let listener = tokio::net::UnixListener::bind(&socket).map_err(io_error)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).map_err(io_error)?;
    let mut child = tokio::process::Command::new(&ctx.auth_helper)
        .arg("--socket")
        .arg(&socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| {
            err(ErrorKind::Runtime, "Store sign-in helper could not start; check WebKitGTK 4.1 installation")
        })?;
    let result = async {
        let child_id = child.id();
        let mut stream = interrupt(cancel, async {
            let (stream, _) = tokio::select! {
                accepted = tokio::time::timeout(std::time::Duration::from_secs(30), listener.accept()) => accepted.map_err(internal)?.map_err(io_error)?,
                _ = child.wait() => return Err(err(ErrorKind::Runtime, "Store sign-in helper exited before connecting")),
            };
            let peer = stream.peer_cred().map_err(io_error)?;
            if peer.uid() != rustix::process::geteuid().as_raw() || peer.pid().map(|p| p as u32) != child_id { return Err(integrity("Unexpected helper peer")); }
            Ok(stream)
        }).await?;
        if let Some(slot) = slot {
            interrupt(cancel, store_legs(ctx, slot, &mut stream, window.login_hint.as_deref())).await?;
        }
        // The Store sign-in is kept whatever happens from here: the Xbox step only adds to it.
        let steps = xbox_steps(&mut stream, window.login_hint.as_deref(), window.xbox_codes, cancel).await;
        let _ = send(&mut stream, Message::Complete).await;
        Ok(steps)
    }.await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

/// The Store's hosted sign-in, both legs, until its account is published in the slot.
async fn store_legs(
    ctx: &Context,
    slot: &Slot,
    stream: &mut tokio::net::UnixStream,
    login_hint: Option<&str>,
) -> Result<()> {
    {
        send(stream, Message::Open { url: ipc::login_url(login_hint) }).await?;
        let mut second_leg = false;
        loop {
            match receive(stream).await? {
                Message::Credentials { data } => {
                    let data: DAProperty = serde_json::from_value(data).map_err(integrity)?;
                    let (outcome, user) = exchange(&ctx.client, &slot.tokens, data, second_leg).await?;
                    match outcome {
                        ExchangeUserTokenOutcome::Fault(Some(fault)) if !second_leg => {
                            let url = fault
                                .inline_auth_url
                                .filter(|u| ipc::allowed_url(u))
                                .ok_or_else(|| store_error("Missing inline URL"))?;
                            second_leg = true;
                            send(stream, Message::Open { url }).await?;
                        }
                        ExchangeUserTokenOutcome::Issued(body) => {
                            // Prepare all entries in memory, publish the account in one durable transaction.
                            let memory = Arc::new(xodus::tokens::backend::MemoryBackend::default());
                            let manager = TokenManager::new(
                                memory.clone(),
                                Arc::new(xodus::tokens::backend::MemoryBackend::default()),
                            );
                            for response in issued(body)? {
                                let address = response.applies_to.endpoint_reference.address.clone();
                                let token = convert(response)?;
                                let address = if let Token::Legacy(ref token) = token {
                                    token.key_name.clone().unwrap_or(address)
                                } else {
                                    address
                                };
                                manager.save_user_token(address, token).map_err(store_error)?;
                            }
                            manager.get_user_sts_token().map_err(store_error)?;
                            manager.save_user(&user).map_err(store_error)?;
                            // Same owner/mode checks as every other read; device entries are kept as they are.
                            let mut entries = slot.backend.read().map_err(store_error)?;
                            for key in ["user-tokens", "user-DA"] {
                                entries.insert(
                                    key.into(),
                                    memory.get(key).map_err(store_error)?.ok_or_else(|| store_error("Missing user"))?,
                                );
                            }
                            slot.backend.replace(&entries)?;
                            return Ok(());
                        }
                        _ => return Err(store_error("Store exchange refused")),
                    }
                }
                Message::Cancelled => return Err(err(ErrorKind::Cancelled, "Store window closed without signing in")),
                _ => return Err(integrity("Unexpected helper message")),
            }
        }
    }
}

/// Ask the window for `count` Xbox authorisation codes, one at a time, each with a fresh `state` that the answer
/// must carry. Never an error: whatever goes wrong (the window closed, a refusal, a cancel, a page that never
/// answers) leaves that code, and the ones after it, `Unavailable`, and the caller falls back to the device code.
pub(super) async fn xbox_steps(
    stream: &mut tokio::net::UnixStream,
    login_hint: Option<&str>,
    count: usize,
    cancel: &Cancel,
) -> Vec<XboxStep> {
    let mut steps = Vec::with_capacity(count);
    // Once the window is gone (or can no longer be trusted to be in step), nothing more is asked of it.
    let mut ended: Option<&'static str> = None;
    for _ in 0..count {
        if let Some(why) = ended {
            steps.push(XboxStep::Unavailable(why.into()));
            continue;
        }
        let state = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let ask = Message::XboxAuthorize { state: state.clone(), login_hint: login_hint.map(str::to_string) };
        if send(stream, ask).await.is_err() {
            ended = Some("closed");
            steps.push(XboxStep::Unavailable("closed".into()));
            continue;
        }
        let answer = async {
            loop {
                match receive(stream).await {
                    Ok(Message::XboxCode { code, state: s }) if s == state => {
                        return Ok(XboxStep::Code(Secret::new(code)));
                    }
                    Ok(Message::XboxCode { .. }) => return Ok(XboxStep::Unavailable("state_mismatch".into())),
                    Ok(Message::XboxUnavailable { error }) => return Ok(XboxStep::Unavailable(error)),
                    // A page of the Xbox step may look like a Store leg's end to the helper; the Store is done.
                    Ok(Message::Credentials { .. }) => continue,
                    Ok(Message::Cancelled) | Err(_) => return Err("closed"),
                    Ok(_) => return Err("unexpected"),
                }
            }
        };
        let step = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err("cancelled"),
            _ = tokio::time::sleep(XBOX_STEP_TIMEOUT) => Err("timeout"),
            step = answer => step,
        };
        match step {
            Ok(step) => steps.push(step),
            Err(why) => {
                ended = Some(why);
                steps.push(XboxStep::Unavailable(why.into()));
            }
        }
    }
    steps
}
pub fn issued(body: soap::BodyContent) -> Result<Vec<soap::RequestSecurityTokenResponse>> {
    match body {
        soap::BodyContent::RequestSecurityTokenResponseCollection(c) if !c.security_tokens.is_empty() => {
            Ok(c.security_tokens)
        }
        soap::BodyContent::RequestSecurityTokenResponse(t) => Ok(vec![*t]),
        _ => Err(store_error("Empty token response")),
    }
}
async fn exchange(
    client: &reqwest::Client,
    tokens: &TokenManager,
    data: DAProperty,
    second_leg: bool,
) -> Result<(ExchangeUserTokenOutcome, User)> {
    let Token::Legacy(device) = tokens.get_device_sts_token().map_err(store_error)? else {
        return Err(store_error("Invalid device token"));
    };
    let user = User { puid: data.puid, username: data.username.clone() };
    let token = LegacyToken {
        key_name: None,
        token: data.da_token,
        binary_secret: None,
        tpm_key: None,
        lifetime: soap::Timestamp { id: None, created: data.da_start_time, expires: data.da_expires },
    };
    let mut scopes = vec![(
        "scope=service::user.auth.xboxlive.com::MBI_SSL&api-version=2.0".into(),
        Some(soap::PolicyReference::token_broker()),
    )];
    if second_leg {
        scopes.push(("http://Passport.NET/tb".into(), None));
    }
    let response = xodus::api::live::exchange_user_token(
        client,
        token,
        data.username,
        device,
        Some(data.sts_inline_flow_token),
        None,
        "000000004424da1f".into(),
        &scopes,
    )
    .await
    .map_err(network)?;
    Ok((response, user))
}

pub fn convert(response: soap::RequestSecurityTokenResponse) -> Result<Token> {
    match response.token_type.as_str() {
        "urn:passport:legacy" if response.requested_security_token.encrypted_data.is_some() => (),
        "urn:passport:compact" | "urn:passport:delegationcompact"
            if response.requested_security_token.binary_security_token.is_some() => {}
        _ => return Err(integrity("Malformed broker token")),
    }
    Ok(response.into())
}

fn ipc_error(error: std::io::Error) -> BedrockError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
    ) {
        err(ErrorKind::Cancelled, "Store sign-in window disconnected without completing sign-in")
    } else {
        io_error(error)
    }
}
