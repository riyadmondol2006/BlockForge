//! Ported from BedrockOnLinux bol/xodus.py (MIT); acquisition uses the patched Xodus libraries.
//! Call on backend workers. No startup work, GUI imports or process-global credential state.
use super::{BedrockError, BuildId, BuildSelection, Cancel, Edition, ErrorKind, Result, SignOutMode, StoreId};
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
pub use stage::{StagedImage, StagedImages};
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
pub async fn stage(ctx: &Context, id: StoreId, installed: &InstalledBuild, cancel: Cancel) -> Result<StagedImages> {
    let slot = ctx.slot(&id).await?;
    slot.tokens.get_user().map_err(tokens::store_error)?;
    license::legacy(slot.tokens.get_user_sts_token())?;
    device::ensure(&ctx.client, &slot, &cancel).await.map_err(license::unavailable)?;
    stage::stage(ctx, &slot, installed, &cancel).await
}
fn err(kind: ErrorKind, message: impl Into<String>) -> BedrockError {
    BedrockError::new(kind, message)
}
fn integrity(_: impl std::fmt::Display) -> BedrockError {
    err(ErrorKind::Integrity, "Invalid or incomplete Store package data")
}
fn internal(_: impl std::fmt::Display) -> BedrockError {
    err(ErrorKind::Internal, "Store worker failed")
}
fn io_error(e: std::io::Error) -> BedrockError {
    let kind = if e.raw_os_error() == Some(28) { ErrorKind::NoSpace } else { ErrorKind::Integrity };
    err(kind, "Store storage operation failed")
}
fn network(_: impl std::fmt::Display) -> BedrockError {
    err(ErrorKind::Network, "Microsoft Store request failed")
}
fn cancelled(cancel: &Cancel) -> Result<()> {
    if cancel.is_cancelled() { Err(err(ErrorKind::Cancelled, "Store operation cancelled")) } else { Ok(()) }
}
async fn interrupt<T>(cancel: &Cancel, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::select! { biased; _ = cancel.cancelled() => Err(err(ErrorKind::Cancelled, "Store operation cancelled")), result = future => result }
}
#[cfg(test)]
mod tests;
