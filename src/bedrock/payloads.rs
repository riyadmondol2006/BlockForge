//! What goes beside the game besides the game itself: the reviewed GDK Xbox-Live DLLs, the OpenSSL XCurl set (which
//! also brings the native RNG stub for the prefix's system32) and a dated Mozilla CA bundle. All are pinned by size
//! and SHA-256 here, downloaded once into `bedrock/payloads/` and never changed afterwards. Ported from
//! BedrockOnLinux bol/config.py and bol/fixups.py (MIT); the files themselves keep their own licences (the GDK
//! dependencies from minecraft-linux/mcpelauncher-gdk-dependencies, the MSYS2/OpenSSL libraries of the XCurl set,
//! the CA bundle from curl.se under the MPL).
use super::{
    BedrockError, Cancel, ErrorKind, Result,
    artifact::{self, invalid, io},
    fixups::DllSources,
    pins::ArtifactPin,
};
use crate::install::Progress;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A single pinned file: `archive_root`, `xz` and `unpack_limit` are unused.
const fn file(revision: &'static str, url: &'static str, sha256: &'static str, size: u64) -> ArtifactPin {
    ArtifactPin { revision, url, sha256, size, archive_root: "", xz: false, unpack_limit: 0 }
}

/// minecraft-linux/mcpelauncher-gdk-dependencies v0.0.0 (PORT-PLAN.md 1.4). The same hashes as
/// [`super::fixups::GDK_DEPS`], which checks them again before copying.
pub const GDK_DEPS: [ArtifactPin; 2] = [
    file(
        "libHttpClient.GDK.dll",
        "https://github.com/minecraft-linux/mcpelauncher-gdk-dependencies/releases/download/v0.0.0/libHttpClient.GDK.dll",
        "718f0c6842c8e35d7cac5c18ec025404c83be03ec035f0d3904e70d6508b9e24",
        306_176,
    ),
    file(
        "XCurl.dll",
        "https://github.com/minecraft-linux/mcpelauncher-gdk-dependencies/releases/download/v0.0.0/XCurl.dll",
        "28443194e9860733f24294219bcb45226677636f9721d4bf145094d977f7161f",
        536_576,
    ),
];

/// BedrockOnLinux's published OpenSSL XCurl set (release `xcurl-504bb166e4e7`): the OpenSSL libcurl and its MSYS2
/// closure, the CA-injecting `xcurl-cashim.dll` and the `cryptbase.dll` RNG stub, built from the sources that release
/// names. Its members sit at the top level of the archive (`./name.dll`).
pub const OPENSSL_SET: ArtifactPin = ArtifactPin {
    revision: "openssl-xcurl-504bb166e4e7",
    url: "https://github.com/Wyze3306/BedrockOnLinux/releases/download/xcurl-504bb166e4e7/openssl-xcurl-set-504bb166e4e7.tar.gz",
    sha256: "504bb166e4e737ad81c3ac8e7a917740b28478f69acd89e538c3bf921c29523f",
    size: 10_657_695,
    archive_root: ".",
    xz: false,
    unpack_limit: 128 * 1024 * 1024,
};

/// Mozilla's CA certificates as curl.se extracted them on 2026-10-20 (a dated, immutable snapshot; its SHA-256 is the
/// one curl.se publishes beside it). The game's XCurl reads it as `cacert.pem`; a newer snapshot is a new pin.
pub const CA_BUNDLE: ArtifactPin = file(
    "cacert-2026-10-20.pem",
    "https://curl.se/ca/cacert-2026-10-20.pem",
    "796f1389bfea13c0ed78f663b765baaff54a508e5483786bd6ece73c5e3e3cbe",
    200_116,
);

/// `bedrock/payloads`.
pub fn dir(base: &Path) -> PathBuf {
    base.join("payloads")
}

/// Where each payload is once present (whether it is there yet or not).
pub fn sources(base: &Path) -> DllSources {
    let root = dir(base);
    DllSources {
        gdk_deps: root.join("gdk-v0.0.0"),
        openssl_set: root.join(OPENSSL_SET.revision),
        cacert: root.join(CA_BUNDLE.revision),
    }
}

/// The RNG stub from the OpenSSL set, for the prefix's system32.
pub fn cryptbase(sources: &DllSources) -> PathBuf {
    sources.openssl_set.join("cryptbase.dll")
}

/// One ensure at a time in this process: two would download the same files beside each other.
static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Every payload, present and checked: a file is hashed again each time (they are small), the unpacked set is trusted
/// by its completion stamp. Missing ones are downloaded. Disk work runs on blocking workers.
pub async fn ensure(
    base: &Path,
    client: &reqwest::Client,
    progress: &Arc<Progress>,
    cancel: &Cancel,
) -> Result<DllSources> {
    let _one = ONE.lock().await;
    let sources = sources(base);
    for pin in GDK_DEPS {
        ensure_file(client, pin, &sources.gdk_deps.join(pin.revision), progress, cancel).await?;
    }
    ensure_file(client, CA_BUNDLE, &sources.cacert, progress, cancel).await?;
    let set = sources.openssl_set.clone();
    if !blocking(move || Ok(set_complete(&set))).await? {
        progress.stage("Downloading the OpenSSL XCurl set");
        let archive = base.join("cache/artifacts").join(format!("{}.tar.gz", OPENSSL_SET.revision));
        if blocking({
            let a = archive.clone();
            move || Ok(artifact::verify_hash(&a, OPENSSL_SET.sha256).is_err())
        })
        .await?
        {
            artifact::download(client, OPENSSL_SET, &archive, progress, cancel).await?;
        }
        let set = sources.openssl_set.clone();
        let a = archive.clone();
        blocking(move || unpack_set(&a, &set)).await?;
        let _ = tokio::fs::remove_file(&archive).await;
    }
    Ok(sources)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| BedrockError::new(ErrorKind::Internal, format!("A Bedrock job stopped unexpectedly: {e}")))?
}

async fn ensure_file(
    client: &reqwest::Client,
    pin: ArtifactPin,
    dest: &Path,
    progress: &Arc<Progress>,
    cancel: &Cancel,
) -> Result<()> {
    let d = dest.to_path_buf();
    if blocking(move || Ok(artifact::verify_hash(&d, pin.sha256).is_ok())).await? {
        return Ok(());
    }
    progress.stage(format!("Downloading {}", pin.revision));
    // download() writes through a .part file and renames only exact, pinned bytes over whatever was there
    artifact::download(client, pin, dest, progress, cancel).await
}

const COMPLETE: &str = ".complete";

fn set_complete(dir: &Path) -> bool {
    fs::read_to_string(dir.join(COMPLETE)).is_ok_and(|s| s.trim() == OPENSSL_SET.sha256)
        && ["libcurl-4.dll", "xcurl-cashim.dll", "cryptbase.dll"].iter().all(|n| dir.join(n).is_file())
}

/// A member name the set may have: one plain file name, nothing that could be a path.
fn member_name(raw: &Path) -> Option<String> {
    let name = raw.to_str()?.strip_prefix("./").unwrap_or(raw.to_str()?);
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'));
    ok.then(|| name.to_string())
}

/// Unpack the (already pinned) set archive into `dest` through a stage beside it. Only regular files with plain
/// names are taken; links, folders below the top, devices and anything over the size limit are refused. The
/// archive is hashed again from the same open file it is unpacked from.
fn unpack_set(archive: &Path, dest: &Path) -> Result<()> {
    use sha2::Digest;
    use std::io::Seek;
    let mut file = fs::File::open(archive).map_err(io)?;
    let mut hash = sha2::Sha256::new();
    let mut buf = vec![0; 256 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    if hex::encode(hash.finalize()) != OPENSSL_SET.sha256 {
        return Err(invalid("The OpenSSL XCurl set's SHA-256 is not the pinned one"));
    }
    file.rewind().map_err(io)?;
    let parent = dest.parent().ok_or_else(|| invalid("Payload folder"))?;
    fs::create_dir_all(parent).map_err(io)?;
    let stage = parent.join(format!(".{}.stage", OPENSSL_SET.revision));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir(&stage).map_err(io)?;
    let result = (|| {
        let mut total = 0u64;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries().map_err(io)? {
            let mut entry = entry.map_err(io)?;
            let path = entry.path().map_err(io)?.into_owned();
            let kind = entry.header().entry_type();
            if kind.is_dir() && matches!(path.to_str(), Some("./" | "." | "")) {
                continue;
            }
            let name = member_name(&path).ok_or_else(|| invalid("Unexpected member in the OpenSSL XCurl set"))?;
            if !kind.is_file() {
                return Err(invalid("The OpenSSL XCurl set may hold plain files only"));
            }
            total = total.saturating_add(entry.size());
            if total > OPENSSL_SET.unpack_limit {
                return Err(invalid("The OpenSSL XCurl set is larger than allowed"));
            }
            let target = stage.join(&name);
            let mut out = fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(io)?;
            std::io::copy(&mut (&mut entry).take(OPENSSL_SET.unpack_limit), &mut out).map_err(io)?;
            out.sync_all().map_err(io)?;
        }
        fs::write(stage.join(COMPLETE), OPENSSL_SET.sha256).map_err(io)?;
        if !set_complete(&stage) {
            return Err(invalid("The OpenSSL XCurl set is incomplete"));
        }
        let _ = fs::remove_dir_all(dest);
        fs::rename(&stage, dest).map_err(io)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_archive(dir: &Path, members: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("set.tar.gz");
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        ));
        for (name, bytes) in members {
            let mut h = tar::Header::new_gnu();
            h.set_size(bytes.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, *bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        path
    }

    #[test]
    fn member_names_are_plain_file_names() {
        assert_eq!(member_name(Path::new("./libcurl-4.dll")).as_deref(), Some("libcurl-4.dll"));
        assert_eq!(member_name(Path::new("libstdc++-6.dll")).as_deref(), Some("libstdc++-6.dll"));
        for bad in ["./../x.dll", "./a/b.dll", "./.hidden", "", "./x:y.dll"] {
            assert!(member_name(Path::new(bad)).is_none(), "{bad}");
        }
    }

    #[test]
    fn an_archive_that_is_not_the_pinned_one_is_never_unpacked() {
        let dir = crate::testutil::scratch("payload-set");
        let archive = set_archive(
            &dir,
            &[("libcurl-4.dll", b"MZ curl"), ("xcurl-cashim.dll", b"MZ shim"), ("cryptbase.dll", b"MZ rng")],
        );
        let dest = dir.join("payloads").join(OPENSSL_SET.revision);
        assert!(unpack_set(&archive, &dest).unwrap_err().message.contains("SHA-256"));
        assert!(!dest.exists() && !set_complete(&dest));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_layout_is_fixed_by_the_pins() {
        let s = sources(Path::new("/b"));
        assert_eq!(s.gdk_deps, Path::new("/b/payloads/gdk-v0.0.0"));
        assert_eq!(s.openssl_set, Path::new("/b/payloads/openssl-xcurl-504bb166e4e7"));
        assert_eq!(s.cacert, Path::new("/b/payloads/cacert-2026-10-20.pem"));
        assert_eq!(cryptbase(&s), Path::new("/b/payloads/openssl-xcurl-504bb166e4e7/cryptbase.dll"));
        for (pin, (name, hash)) in GDK_DEPS.iter().zip(super::super::fixups::GDK_DEPS) {
            assert_eq!((pin.revision, pin.sha256), (name, hash));
        }
    }
}
