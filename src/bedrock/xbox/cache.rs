//! The profile's private Xbox folder: the Microsoft refresh token, the account epoch, the online payload WineGDK
//! reads, and the lock that keeps a sign-out and a sign-in (or a Play) from crossing.
//!
//! Ported from BedrockOnLinux bol/auth.py (`msa_save`, `_account_cache_lock`, `_account_cache_epoch`,
//! `msa_save_for_account_epoch`, `_purge_account_preauth`, `_store_online_preauth`), MIT, with credit.
//!
//! The epoch is what makes a sign-out final. Every flow that may write account credentials reads it first and
//! writes only if it is still the same under the lock; a sign-out rotates it under that lock before deleting
//! anything. So a sign-in answer that arrives after a sign-out, a Play that was refreshing tokens while the player
//! signed out, or a payload a crash left behind can never bring the old account back.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::Secret;
use super::preauth::Payload;

pub(super) const TOKEN_FILE: &str = "msa-token.json";
pub(super) const EPOCH_FILE: &str = ".account-epoch";
pub(super) const LOCK_FILE: &str = ".account-cache.lock";
pub(super) const PAYLOAD_FILE: &str = "device.json";
pub(super) const KEY_FILE: &str = "device-key.pk8";
pub(super) const DEVICE_ID_FILE: &str = "device-id.txt";

/// The account generation: 32 lowercase hex digits, rotated by every sign-out. Not a secret.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Epoch(String);

impl Epoch {
    fn parse(text: &str) -> Option<Epoch> {
        let t = text.trim();
        let ok = t.len() == 32 && t.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then(|| Epoch(t.to_string()))
    }

    fn fresh() -> io::Result<Epoch> {
        use ring::rand::SecureRandom;
        let mut bytes = [0u8; 16];
        ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| io::Error::other("no system randomness"))?;
        Ok(Epoch(hex::encode(bytes)))
    }
}

/// The saved Microsoft (legacy Live) sign-in. Only the refresh token is kept: access tokens are minted per Play.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MsaToken {
    pub refresh_token: Secret,
    pub obtained: i64,
}

/// Make the folder (and only it) private to this user: 0700, created that way rather than tightened afterwards.
pub(super) fn ensure_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    // A symlink here could point the credentials anywhere: refuse it rather than follow it.
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err(io::Error::other(format!("{} is not a folder", dir.display())));
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// Held while the epoch is read and credentials are written or purged. An OS lock, so it also keeps a second
/// BlockForge process out, and it goes away with the process if that crashes.
pub(super) struct Locked {
    _file: File,
}

pub(super) fn lock(dir: &Path) -> io::Result<Locked> {
    ensure_dir(dir)?;
    let file =
        OpenOptions::new().create(true).truncate(false).read(true).write(true).mode(0o600).open(dir.join(LOCK_FILE))?;
    file.lock()?;
    Ok(Locked { _file: file })
}

/// Write a private file atomically: the old one or the whole new one, readable only by its owner from the start.
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    crate::config::write_atomic(path, bytes, true).map_err(|e| io::Error::other(format!("{e:#}")))?;
    sync_dir(path.parent().unwrap_or(Path::new(".")));
    Ok(())
}

/// Make a rename or unlink durable: without it, a power cut can bring back the file a sign-out just removed.
fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

/// The current epoch, under the lock. A missing marker is a new profile and gets a fresh one; an unreadable or
/// damaged one is `None`, which every caller refuses (signing out writes a good one again).
fn read_epoch(dir: &Path, _held: &Locked) -> io::Result<Option<Epoch>> {
    let path = dir.join(EPOCH_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(Epoch::parse(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let epoch = Epoch::fresh()?;
            write_private(&path, format!("{}\n", epoch.0).as_bytes())?;
            Ok(Some(epoch))
        }
        Err(_) => Ok(None),
    }
}

pub(super) fn current_epoch(dir: &Path) -> io::Result<Option<Epoch>> {
    let held = lock(dir)?;
    read_epoch(dir, &held)
}

fn load_token(dir: &Path) -> Option<MsaToken> {
    let bytes = fs::read(dir.join(TOKEN_FILE)).ok()?;
    let token: MsaToken = serde_json::from_slice(&bytes).ok()?;
    (!token.refresh_token.expose().is_empty()).then_some(token)
}

/// The refresh token and the epoch it belongs to, read together so neither can change in between.
pub(super) fn session_snapshot(dir: &Path) -> io::Result<(Option<MsaToken>, Option<Epoch>)> {
    let held = lock(dir)?;
    let epoch = read_epoch(dir, &held)?;
    Ok((load_token(dir), epoch))
}

/// Save a sign-in (or a rotated refresh token) only if no sign-out happened since `expected` was read. `false`
/// means it was discarded.
pub(super) fn save_token_for_epoch(dir: &Path, token: &MsaToken, expected: &Epoch) -> io::Result<bool> {
    let held = lock(dir)?;
    if read_epoch(dir, &held)?.as_ref() != Some(expected) {
        return Ok(false);
    }
    let json = serde_json::to_vec_pretty(token).map_err(io::Error::other)?;
    write_private(&dir.join(TOKEN_FILE), &json)?;
    Ok(true)
}

pub(super) fn epoch_is_current(dir: &Path, expected: &Epoch) -> io::Result<bool> {
    let held = lock(dir)?;
    Ok(read_epoch(dir, &held)?.as_ref() == Some(expected))
}

/// Sign out: rotate the epoch first (so a crash half way still leaves every old file refused), then delete the
/// refresh token and the online payload, including the temporary files a crash may have left. The device key and
/// ID stay: Xbox Live expects one stable device, and they hold nothing of the account.
pub(super) fn purge(dir: &Path) -> io::Result<()> {
    let _held = lock(dir)?;
    let epoch = Epoch::fresh()?;
    write_private(&dir.join(EPOCH_FILE), format!("{}\n", epoch.0).as_bytes())?;
    for name in [PAYLOAD_FILE, TOKEN_FILE] {
        match fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    // `config::write_atomic` stages as `.<name>.<pid>-<n>.tmp`.
    for entry in fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let staged = [PAYLOAD_FILE, TOKEN_FILE].iter().any(|f| name.starts_with(&format!(".{f}.")));
        if staged && name.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
    sync_dir(dir);
    Ok(())
}

pub(super) fn load_payload(dir: &Path) -> Option<Payload> {
    let bytes = fs::read(dir.join(PAYLOAD_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write the online payload if it belongs to `expected` and that is still the current account. The caller has
/// already checked it is complete; this only refuses a payload that crossed a sign-out.
pub(super) fn store_payload(dir: &Path, payload: &Payload, expected: &Epoch) -> io::Result<bool> {
    let held = lock(dir)?;
    if payload.account_epoch.as_ref() != Some(expected) || read_epoch(dir, &held)?.as_ref() != Some(expected) {
        return Ok(false);
    }
    let json = serde_json::to_vec_pretty(payload).map_err(io::Error::other)?;
    write_private(&dir.join(PAYLOAD_FILE), &json)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_folder_and_its_files_are_private() {
        let dir = scratch("xbox-cache").join("xbox");
        let (token, epoch) = session_snapshot(&dir).unwrap();
        assert!(token.is_none());
        let epoch = epoch.unwrap();
        assert_eq!(mode(&dir), 0o700);
        let saved = MsaToken { refresh_token: Secret::new("fake-refresh"), obtained: 1 };
        assert!(save_token_for_epoch(&dir, &saved, &epoch).unwrap());
        assert_eq!(mode(&dir.join(TOKEN_FILE)), 0o600);
        assert_eq!(mode(&dir.join(EPOCH_FILE)), 0o600);
        assert_eq!(mode(&dir.join(LOCK_FILE)), 0o600);
        assert_eq!(load_token(&dir).unwrap().refresh_token.expose(), "fake-refresh");
    }

    #[test]
    fn a_sign_out_rotates_the_epoch_and_refuses_what_was_read_before_it() {
        let dir = scratch("xbox-cache").join("xbox");
        let before = current_epoch(&dir).unwrap().unwrap();
        purge(&dir).unwrap();
        let after = current_epoch(&dir).unwrap().unwrap();
        assert_ne!(before, after);
        assert!(!epoch_is_current(&dir, &before).unwrap());
        let late = MsaToken { refresh_token: Secret::new("late"), obtained: 1 };
        assert!(!save_token_for_epoch(&dir, &late, &before).unwrap());
        assert!(!dir.join(TOKEN_FILE).exists());
        assert!(save_token_for_epoch(&dir, &late, &after).unwrap());
    }

    #[test]
    fn a_sign_out_keeps_the_device_and_removes_the_account() {
        let dir = scratch("xbox-cache").join("xbox");
        ensure_dir(&dir).unwrap();
        for name in
            [KEY_FILE, DEVICE_ID_FILE, PAYLOAD_FILE, TOKEN_FILE, ".device.json.1-0.tmp", ".msa-token.json.7-3.tmp"]
        {
            fs::write(dir.join(name), b"x").unwrap();
        }
        purge(&dir).unwrap();
        assert!(dir.join(KEY_FILE).exists() && dir.join(DEVICE_ID_FILE).exists());
        for name in [PAYLOAD_FILE, TOKEN_FILE, ".device.json.1-0.tmp", ".msa-token.json.7-3.tmp"] {
            assert!(!dir.join(name).exists(), "{name} survived the sign-out");
        }
    }

    #[test]
    fn a_damaged_epoch_refuses_everything_until_a_sign_out() {
        let dir = scratch("xbox-cache").join("xbox");
        ensure_dir(&dir).unwrap();
        fs::write(dir.join(EPOCH_FILE), "not an epoch\n").unwrap();
        assert!(current_epoch(&dir).unwrap().is_none());
        purge(&dir).unwrap();
        assert!(current_epoch(&dir).unwrap().is_some());
    }

    #[test]
    fn a_symlinked_folder_is_refused() {
        let root = scratch("xbox-cache");
        fs::create_dir_all(root.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(root.join("elsewhere"), root.join("xbox")).unwrap();
        assert!(ensure_dir(&root.join("xbox")).is_err());
    }
}
