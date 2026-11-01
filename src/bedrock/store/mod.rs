//! Ported from BedrockOnLinux bol/xodus.py (MIT); acquisition uses the patched Xodus libraries.
//! Call on backend workers. No startup work, GUI imports or process-global credential state.
use super::{
    BedrockError, BuildId, BuildSelection, Cancel, Edition, ErrorKind, Result, SignInState, SignOutMode, StoreId,
};
use crate::install::Progress;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
mod catalog;
mod device;
mod download;
pub mod ipc;
mod license;
mod login;
mod stage;
mod tokens;
pub use download::InstalledBuild;
pub use stage::StagedImages;
pub use tokens::Slot;

#[derive(Clone)]
pub struct Context {
    /// Dirs.base/bedrock, never another launcher's data.
    pub root: PathBuf,
    pub client: reqwest::Client,
    /// Packaged alongside blockforge; supplied by the integrator, never PATH searched.
    pub auth_helper: PathBuf,
}
impl Context {
    pub async fn slot(&self, id: &StoreId) -> Result<Slot> {
        let path = self.root.join("stores").join(id.0.to_string());
        tokio::task::spawn_blocking(move || Slot::open(path)).await.map_err(internal)?
    }
}
pub async fn sign_in(ctx: &Context, id: StoreId, progress: Arc<Progress>, cancel: Cancel) -> Result<()> {
    let slot = ctx.slot(&id).await?;
    progress.stage("Signing in to the Microsoft Store");
    device::ensure(&ctx.client, &slot, &cancel).await?;
    login::run(ctx, &slot, &cancel).await
}
pub async fn sign_out(ctx: &Context, id: StoreId, mode: SignOutMode) -> Result<()> {
    let slot = if matches!(mode, SignOutMode::UserAndDevice) {
        let root = ctx.root.join("stores").join(id.0.to_string());
        tokio::task::spawn_blocking(move || Slot::open_for_reset(root)).await.map_err(internal)??
    } else {
        ctx.slot(&id).await?
    };
    tokio::task::spawn_blocking(move || slot.sign_out(mode)).await.map_err(internal)?
}
pub async fn install(
    ctx: &Context,
    id: StoreId,
    selection: BuildSelection,
    progress: Arc<Progress>,
    cancel: Cancel,
) -> Result<InstalledBuild> {
    let slot = ctx.slot(&id).await?;
    match selection {
        BuildSelection::Installed(build) => download::load(ctx, &build).await,
        BuildSelection::Latest(edition) => {
            slot.tokens.get_user().map_err(tokens::store_error)?;
            license::legacy(slot.tokens.get_user_sts_token())?;
            device::ensure(&ctx.client, &slot, &cancel).await?;
            let package = catalog::latest(&ctx.client, &slot.tokens, edition, &cancel).await?;
            download::install(ctx, &slot, package, progress, &cancel).await
        }
    }
}
/// The newest build of `edition` the Store offers this slot's account, and its download size. Network; nothing is
/// downloaded or licensed.
pub async fn latest(ctx: &Context, id: StoreId, edition: Edition, cancel: Cancel) -> Result<(BuildId, u64)> {
    let slot = ctx.slot(&id).await?;
    slot.tokens.get_user().map_err(tokens::store_error)?;
    license::legacy(slot.tokens.get_user_sts_token())?;
    let package = catalog::latest(&ctx.client, &slot.tokens, edition, &cancel).await?;
    Ok((package.build, package.bytes))
}
/// Decrypt the build's encrypted images into `dir`, a private folder on a RAM file system that the caller owns (the
/// session supervisor's staging folder): the images are left there for it, and it removes them. A fresh content
/// licence is fetched for this: files on disk alone never authorise a start.
pub async fn stage_into(
    ctx: &Context,
    id: StoreId,
    installed: &InstalledBuild,
    dir: &Path,
    cancel: Cancel,
) -> Result<StagedImages> {
    let slot = ctx.slot(&id).await?;
    slot.tokens.get_user().map_err(tokens::store_error)?;
    license::legacy(slot.tokens.get_user_sts_token())?;
    device::ensure(&ctx.client, &slot, &cancel).await.map_err(license::unavailable)?;
    stage::stage(ctx, &slot, installed, Some(dir), &cancel).await
}
/// What is known of a slot's sign-in without the network or its lock: a hint for the window (a valid licence is
/// only proven by a download or a start). Blocking: a few small reads.
pub fn cached_state(root: &Path, id: &StoreId) -> SignInState {
    let dir = root.join("stores").join(id.0.to_string());
    if !dir.join("initialized").exists() {
        return SignInState::SignedOut;
    }
    // The marker means a registration may have been spent without being saved; once one is saved (a slot from
    // before the marker was cleared on success has both), it is not interrupted.
    if dir.join("provisioning").exists() && !tokens::has_device_license(&dir) {
        return SignInState::NeedsSignIn {
            reason: "Registering this computer with the Microsoft Store was interrupted. Sign out with \"forget this \
                     computer\" and sign in again."
                .into(),
        };
    }
    tokens::cached_state(&dir)
}
fn err(kind: ErrorKind, message: impl Into<String>) -> BedrockError {
    BedrockError::new(kind, message)
}
/// A Store error with its cause: the cause goes into BlockForge's log and, cut short and with anything that looks
/// like a token or key taken out, after the message, so a failure can be told apart from the others that share it.
fn with_cause(kind: ErrorKind, message: &str, cause: impl std::fmt::Display) -> BedrockError {
    let cause = redact_cause(&cause.to_string());
    crate::logs::warn(format!("Bedrock Store: {message}: {cause}"));
    err(kind, format!("{message} ({cause})"))
}

/// `cause` without its long unbroken runs of letters and digits (tokens, keys, licence blobs), at most 200 characters.
pub(super) fn redact_cause(cause: &str) -> String {
    let mut out = String::new();
    for word in cause.split_inclusive(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '"' | '\'' | '=')) {
        let token_like =
            word.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_')).count();
        if token_like >= 32 { out.push_str("[…] ") } else { out.push_str(word) }
    }
    let mut out: String = out.trim().chars().take(200).collect();
    if out.is_empty() {
        out.push_str("no detail");
    }
    out
}
fn integrity(cause: impl std::fmt::Display) -> BedrockError {
    with_cause(ErrorKind::Integrity, "Invalid or incomplete Store package data", cause)
}
fn internal(cause: impl std::fmt::Display) -> BedrockError {
    with_cause(ErrorKind::Internal, "Store worker failed", cause)
}
fn io_error(e: std::io::Error) -> BedrockError {
    let kind = if e.raw_os_error() == Some(28) { ErrorKind::NoSpace } else { ErrorKind::Integrity };
    with_cause(kind, "Store storage operation failed", e)
}
fn network(cause: impl std::fmt::Display) -> BedrockError {
    with_cause(ErrorKind::Network, "Microsoft Store request failed", cause)
}
fn cancelled(cancel: &Cancel) -> Result<()> {
    if cancel.is_cancelled() { Err(err(ErrorKind::Cancelled, "Store operation cancelled")) } else { Ok(()) }
}
async fn interrupt<T>(cancel: &Cancel, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::select! { biased; _ = cancel.cancelled() => Err(err(ErrorKind::Cancelled, "Store operation cancelled")), result = future => result }
}
#[cfg(test)]
pub async fn stage(ctx: &Context, id: StoreId, installed: &InstalledBuild, cancel: Cancel) -> Result<StagedImages> {
    let slot = ctx.slot(&id).await?;
    slot.tokens.get_user().map_err(tokens::store_error)?;
    license::legacy(slot.tokens.get_user_sts_token())?;
    device::ensure(&ctx.client, &slot, &cancel).await.map_err(license::unavailable)?;
    stage::stage(ctx, &slot, installed, None, &cancel).await
}
#[cfg(test)]
mod tests;
