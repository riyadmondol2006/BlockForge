//! Ported from Xodus commands/login.rs and BedrockOnLinux bol/xodus.py.
use super::*;
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
pub async fn run(ctx: &Context, slot: &Slot, cancel: &Cancel) -> Result<()> {
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
    let result = interrupt(cancel, async {
        let (mut stream, _) = tokio::select! {
            accepted = tokio::time::timeout(std::time::Duration::from_secs(30), listener.accept()) => accepted.map_err(internal)?.map_err(io_error)?,
            _ = child.wait() => return Err(err(ErrorKind::Runtime, "Store sign-in helper exited before connecting")),
        };
        let peer = stream.peer_cred().map_err(io_error)?;
        if peer.uid() != rustix::process::geteuid().as_raw() || peer.pid().map(|p| p as u32) != child.id() { return Err(integrity("Unexpected helper peer")); }
        send(&mut stream, Message::Open { url: ipc::LOGIN.into() }).await?;
        let mut second_leg = false;
        loop {
            match receive(&mut stream).await? {
                Message::Credentials { data } => {
                    let data: DAProperty = serde_json::from_value(data).map_err(integrity)?;
                    let (outcome, user) = exchange(&ctx.client, &slot.tokens, data, second_leg).await?;
                    match outcome {
                        ExchangeUserTokenOutcome::Fault(Some(fault)) if !second_leg => {
                            let url = fault.inline_auth_url.filter(|u| ipc::allowed_url(u)).ok_or_else(|| store_error("Missing inline URL"))?;
                            second_leg = true;
                            send(&mut stream, Message::Open { url }).await?;
                        },
                        ExchangeUserTokenOutcome::Issued(body) => {
                            // Prepare all entries in memory, publish the account in one durable transaction.
                            let memory = Arc::new(xodus::tokens::backend::MemoryBackend::default());
                            let manager = TokenManager::new(memory.clone(), Arc::new(xodus::tokens::backend::MemoryBackend::default()));
                            for response in issued(body)? {
                                let address = response.applies_to.endpoint_reference.address.clone();
                                let token = convert(response)?;
                                let address = if let Token::Legacy(ref token) = token { token.key_name.clone().unwrap_or(address) } else { address };
                                manager.save_user_token(address, token).map_err(store_error)?;
                            }
                            manager.get_user_sts_token().map_err(store_error)?;
                            manager.save_user(&user).map_err(store_error)?;
                            // Same owner/mode checks as every other read; device entries are kept as they are.
                            let mut entries = slot.backend.read().map_err(store_error)?;
                            for key in ["user-tokens", "user-DA"] { entries.insert(key.into(), memory.get(key).map_err(store_error)?.ok_or_else(|| store_error("Missing user"))?); }
                            slot.backend.replace(&entries)?;
                            send(&mut stream, Message::Complete).await?;
                            return Ok(());
                        },
                        _ => return Err(store_error("Store exchange refused")),
                    }
                },
                Message::Cancelled => return Err(err(ErrorKind::Cancelled, "Store window closed without signing in")),
                _ => return Err(integrity("Unexpected helper message")),
            }
        }
    }).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
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
