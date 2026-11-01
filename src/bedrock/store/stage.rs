//! Ported from BedrockOnLinux bol/xodus.py: encrypted executables stay encrypted on durable storage.
use super::*;
use msixvc::xvd::{SegmentFile, XvdFile};
use std::fs::File;
use tokio::io::AsyncWriteExt;

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
    let mut metadata = tokio::fs::File::open(build.content.join(download::METADATA))
        .await
        .map_err(|_| err(ErrorKind::Integrity, "Required encrypted MSIXVC metadata is missing"))?;
    let xvd = XvdFile::parse(&mut metadata).await.map_err(integrity)?;
    if xvd.content_id().to_string() != build.receipt.content_id {
        return Err(integrity("Mismatched metadata"));
    }
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
    let needed = build
        .receipt
        .files
        .values()
        .filter(|e| e.encrypted)
        .try_fold(0u64, |n, e| n.checked_add(e.length))
        .ok_or_else(|| integrity("Image size overflow"))?;
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
        if !xvd.encrypted_segment(&segment) {
            return Err(integrity("Missing encrypted image region"));
        }
        interrupt(cancel, async {
            xvd.mount_mem_fd(&mut source, &mut output, &segment, key, |_, _| {}).await.map_err(integrity)
        })
        .await?;
        output.flush().await.map_err(io_error)?;
        let file = output.into_std().await;
        if file.metadata().map_err(io_error)?.len() != entry.length {
            return Err(integrity("Truncated staged image"));
        }
        staged.images.push(StagedImage { relative: name, path, file });
    }
    if staged.images.is_empty() {
        return Err(integrity("Package has no encrypted images"));
    }
    Ok(staged)
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
