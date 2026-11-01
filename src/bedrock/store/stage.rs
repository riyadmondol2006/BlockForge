//! Ported from BedrockOnLinux bol/xodus.py: encrypted executables stay encrypted on durable storage.
use super::*;
use msixvc::xvd::{SegmentEncryption, SegmentFile, XvdFile};
use std::fs::File;
use tokio::io::AsyncWriteExt;

/// Whether these first bytes of a file are a PE image's: "MZ", and the "PE\0\0" signature the DOS header points at.
/// A build may ship a file its licence lists as encrypted in the clear (Minecraft 1.26.60's own program is, outside
/// both its encrypted sections); BOL tells the two apart by the "MZ" (bol/xodus.py `exe_is_encrypted`).
pub(super) fn is_plain_pe(head: &[u8]) -> bool {
    if head.len() < 0x40 || !head.starts_with(b"MZ") {
        return false;
    }
    let at = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
    head.get(at..at.saturating_add(4)) == Some(b"PE\0\0".as_slice())
}

/// Whether the file is a loadable image as it lies on disk, so it runs from there and is not decrypted.
pub(super) async fn plain_on_disk(path: &Path) -> bool {
    use tokio::io::AsyncReadExt;
    let Ok(file) = tokio::fs::File::open(path).await else { return false };
    let mut head = Vec::with_capacity(4096);
    file.take(4096).read_to_end(&mut head).await.is_ok() && is_plain_pe(&head)
}

pub struct StagedImage {
    pub relative: String,
    pub path: PathBuf,
    pub file: File,
}
pub struct StagedImages {
    pub images: Vec<StagedImage>,
    /// The folder this value made and removes when dropped; None when the images went into a folder the caller owns
    /// (the session supervisor's), which removes them itself.
    root: Option<PathBuf>,
}
impl std::fmt::Debug for StagedImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StagedImages([private])")
    }
}
impl Drop for StagedImages {
    fn drop(&mut self) {
        if let Some(root) = &self.root {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
pub async fn stage(
    ctx: &Context,
    slot: &tokens::Slot,
    build: &InstalledBuild,
    into: Option<&Path>,
    cancel: &Cancel,
) -> Result<StagedImages> {
    if !build.verified() {
        return Err(unverified(&build.id));
    }
    let mut metadata = tokio::fs::File::open(build.content.join(download::METADATA))
        .await
        .map_err(|_| err(ErrorKind::Integrity, "Required encrypted MSIXVC metadata is missing"))?;
    let mut xvd = XvdFile::parse(&mut metadata).await.map_err(verify::package_error)?;
    if xvd.content_id().to_string() != build.receipt.content_id {
        return Err(integrity("Mismatched metadata"));
    }
    // the images' ciphertext is checked page by page against the tree its download was verified with, as it is
    // decrypted: a copy changed on disk since is refused, not decrypted into garbage
    xvd.require_integrity(download::local_tree(&build.content, &build.receipt).await?).map_err(verify::refused)?;
    let key = license::acquire(&ctx.client, slot, xvd.content_id().to_string(), cancel).await?;
    let base = match into {
        Some(dir) => dir.to_path_buf(),
        None => {
            let base = PathBuf::from(format!("/dev/shm/blockforge-bedrock-{}", rustix::process::geteuid().as_raw()));
            tokens::private_dir(&base)?;
            base
        }
    };
    let stat = rustix::fs::statfs(&base).map_err(integrity)?;
    // tmpfs or ramfs only: the plaintext never goes to a disk
    if !matches!(stat.f_type as u64, 0x01021994 | 0x858458f6) {
        return Err(err(ErrorKind::Runtime, "Executable staging requires private tmpfs"));
    }
    let needed = build.decrypted_bytes().await?;
    let space = rustix::fs::statvfs(&base).map_err(integrity)?;
    let ram = memory_available(&tokio::fs::read_to_string("/proc/meminfo").await.map_err(io_error)?)?;
    if space.f_bavail.saturating_mul(space.f_frsize).min(ram) < needed {
        return Err(err(ErrorKind::NoSpace, "Not enough tmpfs space to stage executable images"));
    }
    let (root, owned) = match into {
        Some(dir) => (dir.to_path_buf(), None),
        None => {
            let root = base.join(uuid::Uuid::new_v4().simple().to_string());
            tokens::private_dir(&root)?;
            (root.clone(), Some(root))
        }
    };
    let mut staged = StagedImages { images: vec![], root: owned };
    for (name, entry) in &build.receipt.files {
        if !entry.encrypted {
            continue;
        }
        cancelled(cancel)?;
        let name = download::safe_name(name)?;
        let encrypted = build.content.join(&name);
        // shipped in the clear: it runs from disk, as BOL runs a plain program without Xodus
        if plain_on_disk(&encrypted).await {
            continue;
        }
        if download::disk_hash(&encrypted).await? != entry.disk_sha256 {
            return Err(integrity("Encrypted file was modified"));
        }
        let path = root.join(format!("image-{}", staged.images.len()));
        let mut output = tokio::fs::File::from_std(tokens::private_open(&path, true).map_err(io_error)?);
        let mut source = tokio::fs::File::open(encrypted).await.map_err(io_error)?;
        let segment = SegmentFile {
            offset: entry.offset,
            length: entry.length,
            keep_encrypted: true,
            data_hashs: entry.hashes.clone(),
        };
        // A file the licence keeps encrypted is usually in an encrypted section and decrypted here; one that is not a
        // program yet lies outside the sections is copied as it is, like Xodus does. One that only partly overlaps a
        // section would come out half encrypted: refused.
        let plain = match xvd.segment_encryption(&segment) {
            SegmentEncryption::Encrypted => false,
            SegmentEncryption::Plain => true,
            SegmentEncryption::Straddles => return Err(integrity("Encrypted image straddles an encrypted region")),
        };
        interrupt(cancel, async {
            xvd.mount_mem_fd(&mut source, &mut output, &segment, key, |_, _| {}).await.map_err(verify::package_error)
        })
        .await?;
        output.flush().await.map_err(io_error)?;
        let file = output.into_std().await;
        // pages are copied whole: the file is cut back to its own length
        let written = file.metadata().map_err(io_error)?.len();
        if written > entry.length {
            file.set_len(entry.length).map_err(io_error)?;
        }
        if file.metadata().map_err(io_error)?.len() != entry.length {
            return Err(integrity("Truncated staged image"));
        }
        if plain {
            crate::logs::info(format!("Bedrock: {name} is not encrypted in this build; staged as it is"));
        }
        staged.images.push(StagedImage { relative: name, path, file });
    }
    if staged.images.is_empty() {
        return Err(integrity("Package has no encrypted images"));
    }
    Ok(staged)
}

/// The refusal for a build downloaded before BlockForge checked Store downloads against the package's hash tree.
pub fn unverified(id: &BuildId) -> BedrockError {
    err(
        ErrorKind::Integrity,
        format!(
            "Minecraft {} was downloaded before BlockForge verified Store downloads, so its files can't be trusted. \
             Repair the instance to download it again, verified; your worlds are kept.",
            id.version
        ),
    )
}

pub(super) fn memory_available(info: &str) -> Result<u64> {
    let kb = info
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|line| {
            let mut parts = line.split_whitespace();
            let kb = parts.next()?.parse::<u64>().ok()?;
            (parts.next()? == "kB").then_some(kb)
        })
        .and_then(|kb| kb.checked_mul(1024));
    kb.ok_or_else(|| err(ErrorKind::Runtime, "Available RAM could not be determined for executable staging"))
}
