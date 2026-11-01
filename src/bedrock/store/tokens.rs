//! Ported from BedrockOnLinux bol/xodus.py. Store slots never use HOME or the global keyring.
use super::*;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    sync::Arc,
};
use xodus::tokens::{
    TokenManager,
    backend::MemoryBackend,
    store::{TokenBackend, TokenStoreError},
};

pub struct PathBackend {
    path: PathBuf,
}
impl std::fmt::Debug for PathBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PathBackend([private])")
    }
}
impl PathBackend {
    pub(super) fn read(&self) -> std::result::Result<BTreeMap<String, Vec<u8>>, TokenStoreError> {
        // Missing after slot creation means lost credentials, never a fresh device.
        let mut file = private_open(&self.path, false)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
            return Err(std::io::Error::other("Unsafe Store credentials").into());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub fn replace(&self, entries: &BTreeMap<String, Vec<u8>>) -> Result<()> {
        atomic(&self.path, &serde_json::to_vec(entries).map_err(integrity)?).map_err(io_error)
    }
}
impl TokenBackend for PathBackend {
    fn get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, TokenStoreError> {
        Ok(self.read()?.get(key).cloned())
    }
    fn set(&self, key: &str, value: &[u8]) -> std::result::Result<(), TokenStoreError> {
        let mut entries = self.read()?;
        entries.insert(key.into(), value.into());
        atomic(&self.path, &serde_json::to_vec(&entries)?)?;
        Ok(())
    }
    fn remove(&self, key: &str) -> std::result::Result<(), TokenStoreError> {
        let mut entries = self.read()?;
        entries.remove(key);
        atomic(&self.path, &serde_json::to_vec(&entries)?)?;
        Ok(())
    }
}

/// Hold this lease across every provisioning, login, licensing or token merge operation.
/// The permanent lock inode is never renamed or unlinked.
pub struct Slot {
    pub root: PathBuf,
    pub tokens: TokenManager,
    pub backend: Arc<PathBackend>,
    _lease: File,
}
impl Slot {
    pub fn open(root: PathBuf) -> Result<Self> {
        Self::open_inner(root, false)
    }
    pub fn open_for_reset(root: PathBuf) -> Result<Self> {
        Self::open_inner(root, true)
    }
    fn open_inner(root: PathBuf, explicit_reset: bool) -> Result<Self> {
        private_dir(&root)?;
        let lease = private_open(&root.join("lock"), true).map_err(io_error)?;
        lease.try_lock().map_err(|_| err(ErrorKind::Busy, "Store slot is in use"))?;
        let path = root.join("credentials.json");
        let initialized = root.join("initialized");
        if !initialized.exists() {
            if path.exists() && !explicit_reset {
                return Err(err(ErrorKind::Integrity, "Incomplete Store slot; repair required"));
            }
            // Stamp first: interruption cannot silently spend a second device registration.
            atomic(&initialized, b"1").map_err(io_error)?;
            atomic(&path, b"{}").map_err(io_error)?;
        }
        let backend = Arc::new(PathBackend { path });
        if !explicit_reset {
            backend.read().map_err(|_| {
                err(ErrorKind::SignInRequired, "Store credentials are missing or corrupt; explicit repair required")
            })?;
        }
        Ok(Self {
            root,
            tokens: TokenManager::new(backend.clone(), Arc::new(MemoryBackend::default())),
            backend,
            _lease: lease,
        })
    }
    pub fn sign_out(&self, mode: SignOutMode) -> Result<()> {
        let mut entries = if matches!(mode, SignOutMode::UserAndDevice) {
            BTreeMap::new()
        } else {
            self.backend.read().map_err(store_error)?
        };
        entries.remove("user-tokens");
        entries.remove("user-DA");
        if matches!(mode, SignOutMode::UserAndDevice) {
            entries.clear();
            match fs::remove_file(self.root.join("provisioning")) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(io_error(e)),
            }
        }
        self.backend.replace(&entries)
    }
}

/// Whether the slot's credentials hold a saved device registration (no lock, no network).
pub fn has_device_license(root: &Path) -> bool {
    PathBackend { path: root.join("credentials.json") }.read().is_ok_and(|entries| entries.contains_key("dev_license"))
}

/// A slot's sign-in from its credentials file alone (no lock, no network): linked with the account's name, signed
/// out, or damaged.
pub fn cached_state(root: &Path) -> SignInState {
    let backend = PathBackend { path: root.join("credentials.json") };
    let entries = match backend.read() {
        Ok(entries) => entries,
        Err(_) => {
            return SignInState::NeedsSignIn {
                reason: "This Microsoft Store sign-in record is damaged. Sign out and sign in again.".into(),
            };
        }
    };
    if !entries.contains_key("user-tokens") || !entries.contains_key("user-DA") {
        return SignInState::SignedOut;
    }
    let manager = TokenManager::new(Arc::new(backend), Arc::new(MemoryBackend::default()));
    SignInState::Linked { display_name: manager.get_user().ok().map(|u| u.username).filter(|n| !n.is_empty()) }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(io_error)?;
    let meta = fs::symlink_metadata(path).map_err(io_error)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid() != rustix::process::geteuid().as_raw() {
        return Err(integrity("Unsafe private directory"));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)
}
pub fn private_open(path: &Path, create: bool) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
}
pub fn atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(path.parent().ok_or_else(|| std::io::Error::other("Missing parent"))?)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub fn store_error(cause: impl std::fmt::Display) -> BedrockError {
    let cause = super::redact_cause(&cause.to_string());
    crate::logs::warn(format!("Bedrock Store credentials: {cause}"));
    err(
        ErrorKind::SignInRequired,
        format!("Store credentials are missing, expired or invalid; sign in or repair this slot ({cause})"),
    )
}
