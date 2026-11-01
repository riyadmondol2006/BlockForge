//! The managed WineGDK engine (GDK-Proton native21 with the universal vkd3d-proton build): downloaded once, checked
//! against the reviewed manifest compiled into BlockForge, kept read-only under `bedrock/engine/<revision>/` and shared
//! by every profile. Only its native Wine binaries are ever run; its `proton` script is Python and never is.
//! Ported from BedrockOnLinux bol/winegdk.py and bol/vkd3d.py (MIT).
use super::{
    Cancel, Result,
    artifact::{self, invalid, io},
    pins,
};
use crate::install::Progress;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A verified, installed engine. Only [`open`], [`install`] and [`ensure`] make one, so holding an `Engine` means
/// the tree passed the reviewed-manifest check (at install) and the cheap check (now). Take [`Engine::lease`] for as
/// long as anything runs from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Engine {
    root: PathBuf,
    revision: &'static str,
}
impl Engine {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn revision(&self) -> &'static str {
        self.revision
    }
    /// The WoW64 Wine (`PROTON_USE_WOW64=1` in the script): one 64-bit loader that runs 32-bit code in-process. The
    /// runtime helper picks it from the plan's engine root; this is what the tests check it against.
    #[cfg(test)]
    pub fn wine(&self) -> PathBuf {
        self.root.join("files/bin-wow64/wine")
    }
    #[cfg(test)]
    pub fn wineserver(&self) -> PathBuf {
        self.root.join("files/bin-wow64/wineserver")
    }
    /// A shared hold that keeps installs and pruning off this revision until dropped.
    pub fn lease(&self) -> Result<artifact::Lease> {
        artifact::lease(self.root.parent().ok_or_else(|| invalid("Engine root"))?, self.revision)
    }
    /// An engine at any path, for tests of what uses one.
    #[cfg(test)]
    pub(crate) fn synthetic(root: PathBuf) -> Self {
        Self::reviewed_as(root, pins::ENGINE.revision)
    }
    /// An engine claiming another revision: what a changed pin looks like to the code using it.
    #[cfg(test)]
    pub(crate) fn reviewed_as(root: PathBuf, revision: &'static str) -> Self {
        Self { root, revision }
    }
}

/// Binaries that must be executable when listed among the manifest's critical files.
const EXECUTABLES: &[&str] =
    &["files/bin-wow64/wine", "files/bin-wow64/wineserver", "files/bin/wine64", "files/bin/wineserver"];

/// The full check, run at install and repair: every link stays inside the tree, the shipped manifest is exactly the
/// reviewed one, and every critical file and both graphics variants hash to it.
pub fn verify(root: &Path) -> Result<()> {
    verify_with(root, pins::ENGINE_MANIFEST)
}
fn verify_with(root: &Path, expected: &str) -> Result<()> {
    check_links(root, root)?;
    let actual: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("engine-manifest.json")).map_err(io)?).map_err(io)?;
    let expected: serde_json::Value = serde_json::from_str(expected).map_err(io)?;
    // Exact equality pins schema, source, components, critical paths and both variants; a tampered manifest can't
    // vouch for its own replacement hashes.
    if actual != expected {
        return Err(invalid("Engine manifest differs from reviewed native21 manifest"));
    }
    let malformed = || invalid("Reviewed engine manifest is malformed");
    let critical = actual["critical_files"].as_object().ok_or_else(malformed)?;
    let variants = actual["variants"].as_object().ok_or_else(malformed)?;
    let owned = fs::canonicalize(root).map_err(io)?;
    for section in std::iter::once(critical).chain(variants.values().filter_map(|v| v["files"].as_object())) {
        for (name, hash) in section {
            let path = root.join(name);
            let resolved = fs::canonicalize(&path)
                .map_err(|e| invalid(format!("Engine file or Wine alias missing: {name}: {e}")))?;
            if !resolved.starts_with(&owned) {
                return Err(invalid("Engine critical link escapes"));
            }
            artifact::verify_hash(&path, hash.as_str().ok_or_else(malformed)?)?;
        }
    }
    for name in EXECUTABLES.iter().filter(|n| critical.contains_key(**n)) {
        executable(&root.join(name))?;
    }
    Ok(())
}
fn executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if fs::metadata(path).map_err(io)?.permissions().mode() & 0o111 == 0 {
        return Err(invalid(format!("Not executable: {}", path.display())));
    }
    Ok(())
}
/// What Play runs: the manifest is still the reviewed one and the WoW64 binaries are there. Milliseconds, no hashing
/// of the 2 GB tree; tamper suspicions go through [`ensure`] with a full check.
pub fn quick(root: &Path) -> Result<()> {
    if fs::read(root.join("engine-manifest.json")).map_err(io)? != pins::ENGINE_MANIFEST.as_bytes() {
        return Err(invalid("Engine manifest changed since install"));
    }
    for name in ["files/bin-wow64/wine", "files/bin-wow64/wineserver", "files/bin/wineserver"] {
        executable(&root.join(name))?;
    }
    Ok(())
}
fn check_links(root: &Path, dir: &Path) -> Result<()> {
    let owned = fs::canonicalize(root).map_err(io)?;
    for entry in fs::read_dir(dir).map_err(io)? {
        let entry = entry.map_err(io)?;
        let ty = entry.file_type().map_err(io)?;
        if ty.is_dir() {
            check_links(root, &entry.path())?;
        } else if ty.is_symlink() && !fs::canonicalize(entry.path()).map_err(io)?.starts_with(&owned) {
            return Err(invalid("Engine link escapes root"));
        }
    }
    Ok(())
}

/// `bedrock/engine`, beside which nothing else lives.
pub fn dir(base: &Path) -> PathBuf {
    base.join("engine")
}
/// The installed engine if its receipt and the cheap check agree; blocking, but only a few small reads.
pub fn open(dir: &Path) -> Result<Option<Engine>> {
    Ok(artifact::open(dir, pins::ENGINE, quick)?.map(|root| Engine { root, revision: pins::ENGINE.revision }))
}
/// Installs from an archive already on disk (it's hashed again here). Blocking; minutes. Play and Repair go through
/// [`ensure`], which finds a pre-seeded archive in `cache/artifacts` by itself.
#[cfg(test)]
pub fn install(dir: &Path, archive: &Path) -> Result<Engine> {
    let root = artifact::install(dir, pins::ENGINE, archive, verify)?;
    Ok(Engine { root, revision: pins::ENGINE.revision })
}

/// What [`ensure`] checks before trusting an installed tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// Receipt + metadata: what Play uses.
    Quick,
    /// Rehash everything against the reviewed manifest: Repair, or after a tamper hint.
    Full,
}

/// The engine, ready: opened, revalidated, or downloaded into `bedrock/cache/artifacts` and installed (the archive is
/// deleted afterwards, and any older revision nothing uses is pruned). `base` is `Dirs.base/bedrock`. Runs its disk
/// work on blocking workers; safe to await from any task, never the window's thread.
pub async fn ensure(
    base: &Path,
    client: &reqwest::Client,
    progress: &std::sync::Arc<Progress>,
    cancel: &Cancel,
    check: Check,
) -> Result<Engine> {
    let dir = dir(base);
    progress.stage("Checking the Bedrock engine");
    let root = artifact::ensure(
        artifact::Ensure {
            dir: &dir,
            cache: &base.join("cache/artifacts"),
            pin: pins::ENGINE,
            full: check == Check::Full,
        },
        client,
        progress,
        cancel,
        ("the Bedrock engine", quick, verify),
    )
    .await?;
    Ok(Engine { root, revision: pins::ENGINE.revision })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_source_components_and_revision_are_rejected() {
        for pointer in
            ["/build_rev", "/provenance/winegdk/commit", "/components/dxvk", "/critical_files/files~1bin-wow64~1wine"]
        {
            let dir = crate::testutil::scratch("b-manifest");
            let mut manifest: serde_json::Value = serde_json::from_str(pins::ENGINE_MANIFEST).unwrap();
            *manifest.pointer_mut(pointer).unwrap() = "tampered".into();
            fs::write(dir.join("engine-manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert!(verify(&dir).unwrap_err().message.contains("manifest differs"));
            assert!(quick(&dir).is_err());
            fs::remove_dir_all(dir).unwrap();
        }
    }

    /// A tiny engine laid out like native21: a real binary, an alias link to it, and a manifest naming both.
    fn tiny_engine(dir: &Path) -> String {
        use sha2::Digest;
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("files/bin-wow64");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("wine"), b"\x7fELF wine").unwrap();
        fs::set_permissions(bin.join("wine"), fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("wine", bin.join("wine-preloader")).unwrap();
        let hash = hex::encode(sha2::Sha256::digest(b"\x7fELF wine"));
        let manifest = serde_json::json!({
            "schema": 1,
            "critical_files": {"files/bin-wow64/wine": hash, "files/bin-wow64/wine-preloader": hash},
            "variants": {"ext-dgc": {"files": {}}},
        })
        .to_string();
        fs::write(dir.join("engine-manifest.json"), &manifest).unwrap();
        manifest
    }
    #[test]
    fn missing_wine_alias_or_changed_binary_is_not_an_install() {
        let dir = crate::testutil::scratch("b-missing-wine");
        let manifest = tiny_engine(&dir);
        verify_with(&dir, &manifest).unwrap();
        fs::remove_file(dir.join("files/bin-wow64/wine-preloader")).unwrap();
        assert!(verify_with(&dir, &manifest).unwrap_err().message.contains("Wine alias missing"));
        std::os::unix::fs::symlink("../../../outside", dir.join("files/bin-wow64/wine-preloader")).unwrap();
        assert!(verify_with(&dir, &manifest).is_err());
        fs::remove_file(dir.join("files/bin-wow64/wine-preloader")).unwrap();
        std::os::unix::fs::symlink("wine", dir.join("files/bin-wow64/wine-preloader")).unwrap();
        fs::write(dir.join("files/bin-wow64/wine"), b"\x7fELF other").unwrap();
        assert!(verify_with(&dir, &manifest).unwrap_err().message.contains("SHA-256"));
        // The real pin refuses the tiny tree outright.
        assert!(verify(&dir).is_err() && quick(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn wow64_binaries_are_the_ones_selected() {
        let e = Engine::synthetic("/owned/engine/wow64-archs-native21".into());
        assert_eq!(e.wine(), Path::new("/owned/engine/wow64-archs-native21/files/bin-wow64/wine"));
        assert_eq!(e.wineserver(), Path::new("/owned/engine/wow64-archs-native21/files/bin-wow64/wineserver"));
        assert_eq!(dir(Path::new("/b")), Path::new("/b/engine"));
    }
}
