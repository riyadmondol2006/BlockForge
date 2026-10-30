//! Horizon's releases: which one is the newest, checking that a download is the one the Horizon project signed, and
//! unpacking it into `horizon/builds/<tag>/`.
//!
//! Every release carries `SHA256SUMS` (`<sha256>  <file>` per line) and `SHA256SUMS.sig`, base64 Ed25519
//! signatures, one per line, over `horizon release <tag>\n` followed by the list. The tag is part of what is signed,
//! so an old release's list can't be passed off as a newer one. An archive is unpacked only when the list is signed
//! by the release key built in here and the archive's SHA-256 is the one the list gives.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::install::{self, Job, Progress};
use crate::paths::Dirs;

/// The newest stable release, from GitHub's API.
const LATEST_API: &str = "https://api.github.com/repos/horizon-rust/horizon/releases/latest";
/// Where a release's files download from (GitHub's fixed address for them).
pub(crate) const DOWNLOADS: &str = "https://github.com/horizon-rust/horizon/releases/download";
/// The Horizon project's release key (crates/horizon-release-sign/release-keys.txt in their repository).
const RELEASE_KEY: &str = "MU4oiIQcCxkZvbwSjjXs7SySCuM6ywRQ3nGaJYZG0gw=";
pub(crate) const SUMS: &str = "SHA256SUMS";
pub(crate) const SIGNATURE: &str = "SHA256SUMS.sig";

/// How long a look at GitHub is trusted: Horizon releases often, but asking on every Play would be slow and use up
/// GitHub's 60 questions an hour.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How long Play waits for GitHub before going on with the build already here.
const API_TIMEOUT: Duration = Duration::from_secs(8);

/// An Ed25519 public key.
pub(crate) type PublicKey = [u8; 32];

/// The keys a release may be signed with.
pub(crate) fn trusted_keys() -> Vec<PublicKey> {
    base64::engine::general_purpose::STANDARD
        .decode(RELEASE_KEY)
        .ok()
        .and_then(|k| PublicKey::try_from(k.as_slice()).ok())
        .into_iter()
        .collect()
}

/// Everything of Horizon in BlockForge's data folder.
pub fn horizon_dir(dirs: &Dirs) -> PathBuf {
    dirs.base.join("horizon")
}

/// The unpacked releases, one folder per tag.
pub fn builds_dir(dirs: &Dirs) -> PathBuf {
    horizon_dir(dirs).join("builds")
}

pub fn build_dir(dirs: &Dirs, tag: &str) -> PathBuf {
    builds_dir(dirs).join(crate::paths::safe_component(tag))
}

/// The Minecraft server jar of version `mc`, which preparing reads (Mojang's file, downloaded once).
pub fn server_jar(dirs: &Dirs, mc: &str) -> PathBuf {
    horizon_dir(dirs).join("server-jars").join(format!("{}.jar", crate::paths::safe_component(mc)))
}

fn state_file(dirs: &Dirs) -> PathBuf {
    horizon_dir(dirs).join("state.json")
}

/// The release file for this computer, if Horizon makes one.
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("horizon-linux-x86_64.tar.gz"),
        ("macos", "aarch64") => Some("horizon-macos-arm64.zip"),
        ("windows", "x86_64") => Some("horizon-windows-x64.zip"),
        _ => None,
    }
}

/// The programs of a build, with `.exe` on Windows.
pub fn program(build: &Path, name: &str) -> PathBuf {
    build.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// The numbers of a stable release's tag (`v0.2.97` is `[0, 2, 97]`). Nightlies (`nightly…`) and the Horizon
/// launcher's own releases (`launcher@…`) aren't game releases and give `None`.
pub fn tag_version(tag: &str) -> Option<Vec<u64>> {
    let numbers = tag.strip_prefix('v')?;
    numbers.split('.').map(|n| n.parse::<u64>().ok().filter(|_| !n.is_empty())).collect::<Option<Vec<_>>>()
}

/// What signatures cover: the tag, then the checksum list.
pub(crate) fn signed_message(tag: &str, sums: &[u8]) -> Vec<u8> {
    let mut message = format!("horizon release {tag}\n").into_bytes();
    message.extend_from_slice(sums);
    message
}

/// The `(file name, SHA-256 in lower-case hex)` pairs of a checksum list (`<hex>  <name>` or `<hex> *<name>`).
pub fn parse_sums(text: &str) -> Result<Vec<(String, String)>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let line = line.trim_end_matches('\r');
            let bad = || anyhow::anyhow!("{SUMS}: {line:?} is not `<sha256>  <file>`");
            let (hex, name) = line.split_once(' ').ok_or_else(bad)?;
            let name = name.strip_prefix([' ', '*']).unwrap_or(name);
            if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
                return Err(bad());
            }
            Ok((name.to_string(), hex.to_ascii_lowercase()))
        })
        .collect()
}

/// The SHA-256 the signed checksum list of release `tag` gives for `asset`, or why the download must be refused.
pub fn signed_sha256(tag: &str, asset: &str, sums: &[u8], signature: &str, keys: &[PublicKey]) -> Result<String> {
    use ring::signature::{ED25519, UnparsedPublicKey};
    let message = signed_message(tag, sums);
    let signed = signature
        .lines()
        .filter_map(|line| base64::engine::general_purpose::STANDARD.decode(line.trim()).ok())
        .any(|sig| keys.iter().any(|key| UnparsedPublicKey::new(&ED25519, key).verify(&message, &sig).is_ok()));
    if !signed {
        bail!(
            "Horizon {tag} isn't signed by the Horizon project's release key, so it was not installed. The download \
             may have been tampered with."
        );
    }
    let text = std::str::from_utf8(sums).map_err(|_| anyhow::anyhow!("{SUMS} of Horizon {tag} is not text"))?;
    parse_sums(text)?
        .into_iter()
        .find(|(name, _)| name == asset)
        .map(|(_, hex)| hex)
        .with_context(|| format!("{asset} is not in the signed checksums of Horizon {tag}, so it was not installed"))
}

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The installed builds, newest first: folders of `builds/` named after a release, holding its research-lock.json
/// and the game. Half-unpacked ones (`<tag>.partial`) aren't builds.
pub fn installed(dirs: &Dirs) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(builds_dir(dirs)) else { return Vec::new() };
    let mut found: Vec<(Vec<u64>, String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let tag = e.file_name().to_string_lossy().to_string();
            let version = tag_version(&tag)?;
            let dir = e.path();
            (dir.join("research-lock.json").is_file() && program(&dir, "horizon-client").is_file())
                .then_some((version, tag, dir))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, tag, dir)| (tag, dir)).collect()
}

/// When GitHub was last asked, and what it said.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
struct State {
    /// Seconds since 1970.
    #[serde(default)]
    checked: i64,
    #[serde(default)]
    latest: Option<String>,
}

fn read_state(dirs: &Dirs) -> State {
    std::fs::read(state_file(dirs)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_state(dirs: &Dirs, state: &State) {
    let file = state_file(dirs);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::to_vec_pretty(state).unwrap_or_default());
}

#[derive(Deserialize)]
struct GithubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The tag of the newest stable release, asking GitHub at `api`.
async fn latest_tag(api: &str) -> Result<String> {
    let r = crate::http::client()
        .get(api)
        .timeout(API_TIMEOUT)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("couldn't reach GitHub")?;
    if !r.status().is_success() {
        bail!("GitHub answered {}", r.status());
    }
    let release: GithubRelease = r.json().await.context("couldn't read GitHub's answer")?;
    if release.draft || release.prerelease || tag_version(&release.tag_name).is_none() {
        bail!("the newest release on GitHub ({}) is not a stable game release", release.tag_name);
    }
    Ok(release.tag_name)
}

/// The newest release as far as is known: from the last look at GitHub while it is recent, else from asking again
/// (and remembering the answer). An error when GitHub can't be asked.
async fn newest_release(dirs: &Dirs, api: &str, now: i64) -> Result<String> {
    let state = read_state(dirs);
    if let Some(tag) = state.latest.as_ref().filter(|_| fresh(state.checked, now)) {
        return Ok(tag.clone());
    }
    let tag = latest_tag(api).await?;
    write_state(dirs, &State { checked: now, latest: Some(tag.clone()) });
    Ok(tag)
}

/// Whether a look at GitHub at `checked` still holds at `now` (seconds since 1970). A clock that went back counts
/// as stale.
fn fresh(checked: i64, now: i64) -> bool {
    (0..CHECK_EVERY.as_secs() as i64).contains(&(now - checked))
}

/// The build to play: the newest release, downloaded when it isn't here yet. When GitHub can't be reached or the
/// download fails, the newest build already installed is used (and `notes` says why); without one, the error.
pub async fn ensure_newest(dirs: &Dirs, progress: &Arc<Progress>, notes: &mut Vec<String>) -> Result<PathBuf> {
    ensure_newest_from(dirs, LATEST_API, DOWNLOADS, &trusted_keys(), progress, notes).await
}

pub(crate) async fn ensure_newest_from(
    dirs: &Dirs,
    api: &str,
    downloads: &str,
    keys: &[PublicKey],
    progress: &Arc<Progress>,
    notes: &mut Vec<String>,
) -> Result<PathBuf> {
    if asset_name().is_none() {
        bail!("{} makes no build for this computer", super::NAME);
    }
    let here = installed(dirs);
    progress.stage(format!("Looking for the newest {}", super::NAME));
    let newest = newest_release(dirs, api, chrono::Utc::now().timestamp()).await;
    let wanted = match newest {
        Ok(tag) => tag,
        Err(e) => {
            return match here.into_iter().next() {
                Some((tag, dir)) => {
                    notes.push(format!("Couldn't look for a newer {} ({e:#}); playing {tag}", super::NAME));
                    Ok(dir)
                }
                None => Err(e.context(format!(
                    "{} isn't installed yet and couldn't be downloaded. Check your internet connection",
                    super::NAME
                ))),
            };
        }
    };
    // the build here is already as new (a release taken back on GitHub leaves an older "latest")
    if let Some((tag, dir)) = here.first() {
        if tag_version(tag) >= tag_version(&wanted) {
            return Ok(dir.clone());
        }
    }
    match install_from(dirs, &wanted, downloads, keys, progress).await {
        Ok(dir) => Ok(dir),
        Err(e) if progress.cancelled.load(Ordering::Relaxed) => Err(e),
        Err(e) => match here.into_iter().next() {
            Some((tag, dir)) => {
                notes.push(format!("Couldn't install {} {wanted} ({e:#}); playing {tag}", super::NAME));
                Ok(dir)
            }
            None => Err(e),
        },
    }
}

/// Download release `tag` from `downloads`, check it, and unpack it into its build folder.
async fn install_from(
    dirs: &Dirs,
    tag: &str,
    downloads: &str,
    keys: &[PublicKey],
    progress: &Arc<Progress>,
) -> Result<PathBuf> {
    let asset = asset_name().context("Horizon makes no build for this computer")?;
    if tag_version(tag).is_none() {
        bail!("{tag} is not a Horizon release");
    }
    progress.stage(format!("Checking {} {tag}", super::NAME));
    let sums = crate::http::get_bytes(&format!("{downloads}/{tag}/{SUMS}"))
        .await
        .with_context(|| format!("Horizon {tag} has no checksum list ({SUMS}), so it can't be installed"))?;
    let signature = crate::http::get_bytes(&format!("{downloads}/{tag}/{SIGNATURE}"))
        .await
        .with_context(|| format!("Horizon {tag} isn't signed ({SIGNATURE}), so it can't be installed"))?;
    let expected = signed_sha256(tag, asset, &sums, &String::from_utf8_lossy(&signature), keys)?;

    let archive = horizon_dir(dirs).join("downloads").join(format!("{}-{asset}", crate::paths::safe_component(tag)));
    // a download of an earlier try that isn't the signed file is fetched again
    if archive.exists() {
        let (a, e) = (archive.clone(), expected.clone());
        let good = tokio::task::spawn_blocking(move || sha256_file(&a).is_ok_and(|got| got == e)).await?;
        if !good {
            let _ = std::fs::remove_file(&archive);
        }
    }
    progress.stage(format!("Downloading {} {tag}", super::NAME));
    install::download_all(vec![Job::new(format!("{downloads}/{tag}/{asset}"), &archive)], progress, false).await?;

    progress.stage(format!("Unpacking {} {tag}", super::NAME));
    let (d, t, a) = (dirs.clone(), tag.to_string(), archive.clone());
    let result = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        let got = sha256_file(&a)?;
        if got != expected {
            bail!(
                "the downloaded {} doesn't match its signed checksum (damaged or tampered with), so it was not \
                 installed. Try again",
                a.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            );
        }
        unpack(&a, &builds_dir(&d), &t)
    })
    .await?;
    // kept only while it may still be needed: a checked archive is unpacked, a bad one is fetched again next time
    let _ = std::fs::remove_file(&archive);
    result
}

/// Unpack `archive` (a release's `.tar.gz` or `.zip`, its files in one folder) into `<builds>/<tag>`: first into
/// `<tag>.partial`, moved into place only when whole, so a build folder is never half there.
pub(crate) fn unpack(archive: &Path, builds: &Path, tag: &str) -> Result<PathBuf> {
    let name = crate::paths::safe_component(tag);
    let target = builds.join(&name);
    let partial = builds.join(format!("{name}.partial"));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).with_context(|| format!("writing in {}", builds.display()))?;
    let result = (|| -> Result<PathBuf> {
        let file = std::fs::File::open(archive).with_context(|| format!("reading {}", archive.display()))?;
        if archive.extension().is_some_and(|e| e == "zip") {
            zip::ZipArchive::new(file)?.extract(&partial).context("unpacking the Horizon archive")?;
        } else {
            tar::Archive::new(flate2::read::GzDecoder::new(file))
                .unpack(&partial)
                .context("unpacking the Horizon archive")?;
        }
        // the archive holds one folder (`horizon-linux-x86_64/`) with the build in it
        let inner = std::fs::read_dir(&partial)?.flatten().map(|e| e.path()).collect::<Vec<_>>();
        let root = match inner.as_slice() {
            [one] if one.is_dir() => one.clone(),
            _ => partial.clone(),
        };
        if !root.join("research-lock.json").is_file() || !program(&root, "horizon-client").is_file() {
            bail!("the Horizon archive doesn't hold a build (no research-lock.json or horizon-client)");
        }
        // a zip made elsewhere doesn't keep the programs' permission to run
        #[cfg(unix)]
        for p in ["horizon-client", "horizon-prepare", "horizon-ctl", "horizon-server"] {
            use std::os::unix::fs::PermissionsExt;
            let path = program(&root, p);
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
        }
        if target.exists() {
            // another start finished the same build first
            if target.join("research-lock.json").is_file() {
                return Ok(target.clone());
            }
            std::fs::remove_dir_all(&target)?;
        }
        std::fs::rename(&root, &target).with_context(|| format!("moving Horizon into {}", target.display()))?;
        Ok(target.clone())
    })();
    let _ = std::fs::remove_dir_all(&partial);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Reply, Server, runtime, scratch};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn key() -> Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn public(k: &Ed25519KeyPair) -> PublicKey {
        PublicKey::try_from(k.public_key().as_ref()).unwrap()
    }

    fn sign(k: &Ed25519KeyPair, tag: &str, sums: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(k.sign(&signed_message(tag, sums)).as_ref())
    }

    #[test]
    fn the_built_in_release_key_reads() {
        assert_eq!(trusted_keys().len(), 1);
    }

    #[test]
    fn a_checksum_list_reads_like_sha256sum_writes_it() {
        let hex = "eb5d611d2b6e9295a35ab914e4a095053e23978b0ecc8195a643a3c3eebff68d";
        let text = format!("{hex}  horizon-linux-x86_64.tar.gz\n{} *horizon-windows-x64.zip\r\n\n", hex.to_uppercase());
        let sums = parse_sums(&text).unwrap();
        assert_eq!(sums[0], ("horizon-linux-x86_64.tar.gz".to_string(), hex.to_string()));
        // a binary-mode star and upper-case hex are read the same
        assert_eq!(sums[1], ("horizon-windows-x64.zip".to_string(), hex.to_string()));
        assert!(parse_sums("abc  short.zip").is_err());
        assert!(parse_sums(hex).is_err());
    }

    #[test]
    fn only_a_list_signed_for_its_own_tag_by_the_key_passes() {
        let k = key();
        let keys = [public(&k)];
        let hex = "a".repeat(64);
        let sums = format!("{hex}  horizon.zip\n").into_bytes();
        let sig = sign(&k, "v0.2.97", &sums);
        assert_eq!(signed_sha256("v0.2.97", "horizon.zip", &sums, &sig, &keys).unwrap(), hex);
        // any line of the signature file may be the one that verifies
        let two = format!("bm90IGEgc2lnbmF0dXJl\n{sig}\n");
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, &two, &keys).is_ok());
        // another tag (an old release's list passed off as new), another list, another key, another file: refused
        assert!(signed_sha256("v0.2.98", "horizon.zip", &sums, &sig, &keys).is_err());
        let changed = format!("{}  horizon.zip\n", "b".repeat(64)).into_bytes();
        assert!(signed_sha256("v0.2.97", "horizon.zip", &changed, &sig, &keys).is_err());
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, &sig, &[public(&key())]).is_err());
        assert!(signed_sha256("v0.2.97", "other.zip", &sums, &sig, &keys).is_err());
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, "", &keys).is_err());
        // a nightly's list is signed for the tag `nightly` and is never a release's
        let nightly = sign(&k, "nightly", &sums);
        assert!(signed_sha256("v0.2.97", "horizon.zip", &sums, &nightly, &keys).is_err());
    }

    #[test]
    fn the_real_v0_2_97_list_verifies_with_the_built_in_key_and_a_nightlys_doesnt_pass_for_a_release() {
        // SHA256SUMS and SHA256SUMS.sig of horizon-rust/horizon v0.2.97, as GitHub serves them
        let sums = "16e62f7a9ac95d6be788e92cd5fb50a569926d2378531a4e0ea3a07337b79c11  horizon-linux-x86_64.tar.gz\n\
                    29d3cef224aecdafea314cf6a3acb5caf8228edef9a3c5d73a0b4fe3b2696fca  horizon-windows-x64.zip\n\
                    587fb8a653324e6f1b3bf59f1f2227ad99201ccaaf3f31024db25ccd4f37c63e  horizon-macos-arm64.zip\n\
                    57422db46533e077277ef1505dcc03805d52d15f3a213200569118c36c5f7cc9  horizon-server-linux-x86_64.tar.gz\n";
        let sig = "XDSckSAve+YAwmmuQ1wU8Wpr8rpN3U17OknKPGg3IK+mOUbFquPtS5pYd9vPA9GuxdxXmRkcD4/vAI2xrFr8Bg==\n";
        let keys = trusted_keys();
        let got = signed_sha256("v0.2.97", "horizon-linux-x86_64.tar.gz", sums.as_bytes(), sig, &keys);
        assert_eq!(got.unwrap(), "16e62f7a9ac95d6be788e92cd5fb50a569926d2378531a4e0ea3a07337b79c11");
        assert!(signed_sha256("v0.2.98", "horizon-linux-x86_64.tar.gz", sums.as_bytes(), sig, &keys).is_err());

        // the Nightly of 2026-10-30, signed for the nightly channel: genuine, but not a release
        let nightly = "eb5d611d2b6e9295a35ab914e4a095053e23978b0ecc8195a643a3c3eebff68d  horizon-linux-x86_64.tar.gz\n\
                       169b744d4f7c764d8197e6d164ef8b4b3e390f4d44f5f45cb7410f234f9500b9  horizon-windows-x64.zip\n\
                       3c77932ad4d286592f74a15006da73c78e3eaf12f0aea336aa6ec8672b36510d  horizon-macos-arm64.zip\n\
                       86108fa6477d1f645199b2cb1cff36239a58354fcca98a1ca3c6bd1875c76e9b  \
                       horizon-server-linux-x86_64.tar.gz\n\
                       a8af7058fd7c34671087f6724c44f53c9ebbb14c86f0aaf177fe2c205507b98d  horizon-source.tar.gz\n";
        let nightly_sig = "5ucgwR4JJt9mXINbxWbvbySy9okgLdLwfh7M9CHhJez3pWU/kxbYlL/FSlHRQ/Kp4amLEeO9iJWlwFvf4gkvCA==\n";
        let asset = "horizon-linux-x86_64.tar.gz";
        assert!(signed_sha256("nightly", asset, nightly.as_bytes(), nightly_sig, &keys).is_ok());
        assert!(signed_sha256("v0.2.97", asset, nightly.as_bytes(), nightly_sig, &keys).is_err());
    }

    #[test]
    fn only_stable_release_tags_count_and_they_sort_by_number() {
        assert_eq!(tag_version("v0.2.97"), Some(vec![0, 2, 97]));
        assert!(tag_version("v0.2.100") > tag_version("v0.2.97"));
        for tag in ["nightly", "nightly-2026-10-28", "launcher@0.3.1", "0.2.97", "v", "v0..1", "v1.x"] {
            assert_eq!(tag_version(tag), None, "{tag}");
        }
    }

    #[test]
    fn a_look_at_github_holds_for_six_hours() {
        assert!(fresh(1000, 1000));
        assert!(fresh(1000, 1000 + 6 * 3600 - 1));
        assert!(!fresh(1000, 1000 + 6 * 3600));
        // the clock went back: look again
        assert!(!fresh(1000, 999));
    }

    /// A build folder as a release archive holds it.
    fn fake_build(dir: &Path, mc: &str) {
        std::fs::create_dir_all(dir.join("crates/horizon-prepare/src")).unwrap();
        std::fs::write(
            dir.join("research-lock.json"),
            format!(r#"{{"minecraft":{{"version":"{mc}","client_sha1":"c","server_sha1":"s","java_major":25}}}}"#),
        )
        .unwrap();
        std::fs::write(dir.join("crates/horizon-prepare/src/lib.rs"), "pub const OUTPUT_VERSION: u32 = 2;\n").unwrap();
        for p in ["horizon-client", "horizon-prepare"] {
            std::fs::write(program(dir, p), b"#!/bin/sh\n").unwrap();
        }
    }

    fn tar_gz_of(dir: &Path, folder: &str) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            tar.append_dir_all(folder, dir).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        out
    }

    #[test]
    fn builds_are_listed_newest_first_and_half_unpacked_ones_left_out() {
        let base = scratch("hz-builds");
        let dirs = Dirs::at(&base);
        for tag in ["v0.2.9", "v0.2.97", "v0.2.10"] {
            fake_build(&build_dir(&dirs, tag), "26.3");
        }
        fake_build(&builds_dir(&dirs).join("v0.3.0.partial"), "26.3");
        fake_build(&builds_dir(&dirs).join("nightly"), "26.3");
        std::fs::create_dir_all(build_dir(&dirs, "v0.4.0")).unwrap();
        let tags: Vec<String> = installed(&dirs).into_iter().map(|(t, _)| t).collect();
        assert_eq!(tags, ["v0.2.97", "v0.2.10", "v0.2.9"]);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_signed_release_is_downloaded_checked_and_unpacked_and_a_tampered_one_refused() {
        let base = scratch("hz-install");
        let dirs = Dirs::at(base.join("data"));
        let src = base.join("src");
        fake_build(&src, "26.3");
        let asset = asset_name().unwrap_or("horizon-linux-x86_64.tar.gz");
        // the zip formats would need their own archive; the check and the unpacking are the same
        if !asset.ends_with(".tar.gz") {
            return;
        }
        let archive = tar_gz_of(&src, "horizon-linux-x86_64");
        let k = key();
        let rt = runtime();
        rt.block_on(async {
            let server = Server::start().await;
            let sha = {
                use sha2::Digest;
                hex::encode(sha2::Sha256::digest(&archive))
            };
            let sums = format!("{sha}  {asset}\n");
            server.route(&format!("/dl/v0.2.97/{SUMS}"), Reply::ok(sums.clone()));
            server.route(&format!("/dl/v0.2.97/{SIGNATURE}"), Reply::ok(sign(&k, "v0.2.97", sums.as_bytes())));
            server.route(&format!("/dl/v0.2.97/{asset}"), Reply::ok(archive.clone()));
            server.route("/latest", Reply::ok(r#"{"tag_name":"v0.2.97","draft":false,"prerelease":false}"#));
            let downloads = format!("{}/dl", server.base);
            let api = format!("{}/latest", server.base);
            let progress = Progress::new();
            let mut notes = Vec::new();

            // nothing installed, and a key that didn't sign it: refused, nothing unpacked
            let wrong = [public(&key())];
            let refused = ensure_newest_from(&dirs, &api, &downloads, &wrong, &progress, &mut notes).await;
            assert!(format!("{:#}", refused.unwrap_err()).contains("isn't signed"));
            assert!(installed(&dirs).is_empty());

            let keys = [public(&k)];
            let dir = ensure_newest_from(&dirs, &api, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(dir, build_dir(&dirs, "v0.2.97"));
            assert!(dir.join("research-lock.json").is_file());
            assert!(!builds_dir(&dirs).join("v0.2.97.partial").exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(program(&dir, "horizon-client")).unwrap().permissions().mode();
                assert_eq!(mode & 0o111, 0o111);
            }
            // the answer is remembered: the next start doesn't ask GitHub
            let asked = server.hits("/latest");
            ensure_newest_from(&dirs, &api, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(server.hits("/latest"), asked);

            // a newer release whose archive was changed after signing: refused, the build here is played
            write_state(&dirs, &State::default());
            server.route("/latest", Reply::ok(r#"{"tag_name":"v0.2.98","draft":false,"prerelease":false}"#));
            let sums = format!("{}  {asset}\n", "0".repeat(64));
            server.route(&format!("/dl/v0.2.98/{SUMS}"), Reply::ok(sums.clone()));
            server.route(&format!("/dl/v0.2.98/{SIGNATURE}"), Reply::ok(sign(&k, "v0.2.98", sums.as_bytes())));
            server.route(&format!("/dl/v0.2.98/{asset}"), Reply::ok(archive.clone()));
            notes.clear();
            let played = ensure_newest_from(&dirs, &api, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(played, build_dir(&dirs, "v0.2.97"));
            assert!(notes[0].contains("doesn't match its signed checksum"), "{notes:?}");
            assert!(!build_dir(&dirs, "v0.2.98").exists());

            // GitHub can't be reached: the build here is played
            write_state(&dirs, &State::default());
            notes.clear();
            let offline = format!("{}/nothing-here", server.base);
            let played = ensure_newest_from(&dirs, &offline, &downloads, &keys, &progress, &mut notes).await.unwrap();
            assert_eq!(played, build_dir(&dirs, "v0.2.97"));
            assert_eq!(notes.len(), 1);
        });
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn nothing_installed_and_no_github_is_an_error() {
        let base = scratch("hz-offline");
        let dirs = Dirs::at(&base);
        let rt = runtime();
        rt.block_on(async {
            let server = Server::start().await;
            let mut notes = Vec::new();
            let api = format!("{}/latest", server.base);
            let r = ensure_newest_from(&dirs, &api, &server.base, &[], &Progress::new(), &mut notes).await;
            assert!(format!("{:#}", r.unwrap_err()).contains("isn't installed yet"));
        });
        let _ = std::fs::remove_dir_all(base);
    }
}
