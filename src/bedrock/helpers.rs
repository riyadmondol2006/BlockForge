//! The two helper programs BlockForge ships beside its own executable on Linux:
//!
//! - `blockforge-bedrock-auth` (crates/bedrock-auth): the Microsoft Store sign-in window (WebKitGTK). Runs on the host
//!   from where it is installed.
//! - `bedrock-runtime` (crates/bedrock-runtime): the static program that becomes Wine inside the Steam
//!   Linux Runtime container. The container has its own `/usr`, so a copy installed in `/usr/bin` (or inside an
//!   AppImage's mount) is not visible there: it is copied, once per build of it, into `bedrock/helpers/<sha256>/`
//!   in BlockForge's own data folder, which the container mounts.
//!
//! Developers point at freshly built ones with `BF_BEDROCK_AUTH_HELPER` and `BF_BEDROCK_RUNTIME_HELPER`.
use super::{BedrockError, ErrorKind, Result, artifact};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const AUTH_HELPER: &str = "blockforge-bedrock-auth";
pub const RUNTIME_HELPER: &str = "bedrock-runtime";
pub const AUTH_ENV: &str = "BF_BEDROCK_AUTH_HELPER";
pub const RUNTIME_ENV: &str = "BF_BEDROCK_RUNTIME_HELPER";

fn missing(name: &str, env: &str) -> BedrockError {
    BedrockError::new(
        ErrorKind::Runtime,
        format!(
            "BlockForge's helper program {name} isn't installed beside BlockForge. Reinstall BlockForge (when you \
             built it yourself, build the helpers too and set {env} to the helper's path)."
        ),
    )
}

/// The helper `name`: `env` when it names a file, else the file of that name beside the running BlockForge.
fn find(name: &str, env: &str) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(env).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()) {
        return if path.is_absolute() && path.is_file() { Ok(path) } else { Err(missing(name, env)) };
    }
    let exe = std::env::current_exe().map_err(|_| missing(name, env))?;
    let beside = exe.parent().map(|d| d.join(name)).ok_or_else(|| missing(name, env))?;
    if beside.is_file() { Ok(beside) } else { Err(missing(name, env)) }
}

/// The Store sign-in helper (blocking: a stat or two).
pub fn auth_helper() -> Result<PathBuf> {
    find(AUTH_HELPER, AUTH_ENV)
}

/// The runtime helper, copied into `base/helpers/<sha256>/` where the container can see it. Checked to be a static
/// x86-64 ELF program (the container's C library is not the host's). Blocking; the copy happens once per build of
/// the helper.
pub fn runtime_helper(base: &Path) -> Result<PathBuf> {
    let source = find(RUNTIME_HELPER, RUNTIME_ENV)?;
    let mut bytes = Vec::new();
    fs::File::open(&source)
        .and_then(|f| f.take(MAX_HELPER + 1).read_to_end(&mut bytes))
        .map_err(|e| BedrockError::new(ErrorKind::Runtime, format!("Couldn't read {}: {e}", source.display())))?;
    if bytes.len() as u64 > MAX_HELPER {
        return Err(BedrockError::new(ErrorKind::Runtime, format!("{} is far too large", source.display())));
    }
    static_elf(&bytes).map_err(|why| {
        BedrockError::new(ErrorKind::Runtime, format!("{} can't run in the game's container: {why}", source.display()))
    })?;
    let sha = {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(&bytes))
    };
    let dir = base.join("helpers").join(&sha);
    let dest = dir.join(RUNTIME_HELPER);
    if artifact::sha256(&dest).is_ok_and(|s| s == sha) {
        return Ok(dest);
    }
    let io =
        |e: std::io::Error| BedrockError::new(ErrorKind::Runtime, format!("Couldn't install the runtime helper: {e}"));
    fs::create_dir_all(&dir).map_err(io)?;
    let tmp = dir.join(format!(".{RUNTIME_HELPER}.{}.part", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o755).open(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, &dest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(io)?;
    Ok(dest)
}

const MAX_HELPER: u64 = 32 * 1024 * 1024;

/// Whether `elf` is a 64-bit little-endian x86-64 executable without a program interpreter (a static program).
fn static_elf(elf: &[u8]) -> std::result::Result<(), &'static str> {
    let u16_at = |o: usize| elf.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| elf.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap_or_default()));
    let u64_at = |o: usize| elf.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()));
    if elf.get(..6) != Some(b"\x7fELF\x02\x01".as_slice()) {
        return Err("it isn't a 64-bit Linux program");
    }
    if u16_at(18) != Some(62) {
        return Err("it isn't built for x86-64");
    }
    let (Some(phoff), Some(phentsize), Some(phnum)) = (u64_at(32), u16_at(54), u16_at(56)) else {
        return Err("its header is cut off");
    };
    for i in 0..phnum as usize {
        let at = usize::try_from(phoff).map_err(|_| "its header is damaged")? + i * phentsize as usize;
        match u32_at(at) {
            // PT_INTERP: it needs the host's dynamic loader
            Some(3) => return Err("it is dynamically linked (build it for x86_64-unknown-linux-musl)"),
            Some(_) => {}
            None => return Err("its program headers are cut off"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF64 header with the given program header types.
    fn elf(types: &[u32]) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[..6].copy_from_slice(b"\x7fELF\x02\x01");
        b[18..20].copy_from_slice(&62u16.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&(types.len() as u16).to_le_bytes());
        for t in types {
            let mut ph = vec![0u8; 56];
            ph[..4].copy_from_slice(&t.to_le_bytes());
            b.extend(ph);
        }
        b
    }

    #[test]
    fn only_static_x86_64_programs_go_into_the_container() {
        assert!(static_elf(&elf(&[1, 1, 4])).is_ok());
        assert!(static_elf(&elf(&[6, 3, 1])).unwrap_err().contains("dynamically linked"));
        assert!(static_elf(b"#!/bin/sh\n").is_err());
        let mut arm = elf(&[1]);
        arm[18..20].copy_from_slice(&183u16.to_le_bytes());
        assert!(static_elf(&arm).is_err());
        let mut cut = elf(&[1, 1]);
        cut.truncate(100);
        assert!(static_elf(&cut).is_err());
    }

    #[test]
    fn the_runtime_helper_is_copied_once_into_the_data_folder() {
        let dir = crate::testutil::scratch("helpers");
        let built = dir.join("built");
        fs::write(&built, elf(&[1])).unwrap();
        // SAFETY: tests that read this variable run in this test only
        unsafe { std::env::set_var(RUNTIME_ENV, &built) };
        let base = dir.join("bedrock");
        let first = runtime_helper(&base).unwrap();
        let again = runtime_helper(&base).unwrap();
        unsafe { std::env::remove_var(RUNTIME_ENV) };
        assert_eq!(first, again);
        assert!(first.starts_with(base.join("helpers")) && first.ends_with(RUNTIME_HELPER));
        assert_eq!(fs::read(&first).unwrap(), elf(&[1]));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&first).unwrap().permissions().mode() & 0o777, 0o755);
        fs::remove_dir_all(dir).unwrap();
    }
}
