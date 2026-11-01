//! Microsoft GameInput, installed from the downloaded game's own `Installers/GameInputRedist.msi`.
//!
//! Ported from BedrockOnLinux `bol/gameinput.py` (MIT), with credit. Windows Installer is never run: its
//! RtlGenRandom custom action hangs msiexec for minutes on some hosts, and starting Wine only to unpack files
//! would wake the graphics driver before the game does. The MSI is an OLE compound file holding one MSZip
//! cabinet; both are read here with checked offsets (see `cfb.rs` and `cab.rs`), the PE images in it are
//! sorted by what they are (DLL or EXE, 64-bit or 32-bit) rather than by guessing from file names, and the
//! files and registry values the MSI would have written are written directly into the stopped prefix.
//!
//! Wine's builtin `gameinput.dll` being in system32 says nothing: wineboot seeds every prefix with it, and its
//! mouse and controller readers need HID devices Wine never offers (the "keyboard works but the mouse is dead"
//! bug). The game loads `GameInputRedist.dll` straight from the `RedistDir` registry value, but only when this
//! tree exists, so this tree is what "ready" means.

use std::fs;
use std::path::{Path, PathBuf};

use super::pe::{self, Arch, PeInfo};
use super::registry::{RegChange, update_prefix_registry};
use super::{PrefixError, cab, cfb, prefix_ready, require_prefix_idle, safe_dir, write_atomic};

pub const REDIST_DIR_REL: &str = "drive_c/Program Files/Microsoft GameInput";
const REDIST_DIR_WIN: &str = r"C:\Program Files\Microsoft GameInput\x64";
/// An installer larger than this is not the redistributable.
const MAX_MSI_BYTES: u64 = 256 * 1024 * 1024;

/// The files of the redistributable, sorted by what each is.
pub struct RedistPayload {
    pub x64_redist: Vec<u8>,
    pub x64_service: Vec<u8>,
    pub x64_bridge: Option<Vec<u8>>,
    pub x64_raw_input_proxy: Option<Vec<u8>>,
    pub x86_redist: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Already installed and not older than the game's; only the registry values were checked.
    AlreadyCurrent,
    Installed,
    /// An older redistributable was replaced by the game's.
    Updated,
    /// The game package has no `GameInputRedist.msi` and the prefix has no redistributable: the in-game mouse and
    /// controller will not work. A diagnostic for the caller to show, not a launch failure.
    MsiMissing,
}

/// The embedded cabinet of an MSI: the first stream that starts with the cabinet signature.
pub fn embedded_cabinet(msi: &[u8]) -> Result<Vec<u8>, PrefixError> {
    let file = cfb::Cfb::open(msi).map_err(|e| PrefixError::Malformed(format!("GameInputRedist.msi: {e}")))?;
    let mut first_problem: Option<String> = None;
    for entry in file.entries().iter().filter(|e| e.kind == 2 && e.size >= 4) {
        // Only the first bytes: most streams are the installer's tables, which are not worth reading whole.
        match file.read_prefix(entry, 4) {
            Ok(head) if head == b"MSCF" => {
                return file
                    .read_stream(entry)
                    .map_err(|e| PrefixError::Malformed(format!("GameInputRedist.msi: {e}")));
            }
            Ok(_) => {}
            Err(e) => {
                first_problem.get_or_insert(e.to_string());
            }
        }
    }
    let streams: Vec<&str> = file.entries().iter().filter(|e| e.kind == 2).map(|e| e.name.as_str()).collect();
    Err(PrefixError::Malformed(match first_problem {
        Some(p) => format!("GameInputRedist.msi holds no readable cabinet ({p})"),
        None => format!("GameInputRedist.msi holds no cabinet (streams: {})", streams.join(", ")),
    }))
}

/// Pick the redistributable's files out of an MSI.
///
/// The MSI's own file names are unreliable (the bridge reports itself as GameInputRedist), so images are told
/// apart by structure: DLL or EXE, 64-bit or 32-bit, then by size. The 64-bit DLL that is largest is the
/// redistributable, the next the bridge; the 64-bit EXEs are the service, then the raw-input proxy; the largest
/// 32-bit DLL is the 32-bit redistributable.
pub fn payload_from_msi(msi: &[u8]) -> Result<RedistPayload, PrefixError> {
    let cab_bytes = embedded_cabinet(msi)?;
    let files =
        cab::extract_all(&cab_bytes).map_err(|e| PrefixError::Malformed(format!("GameInputRedist.msi: {e}")))?;
    let mut x64_dlls: Vec<Vec<u8>> = Vec::new();
    let mut x64_exes: Vec<Vec<u8>> = Vec::new();
    let mut x86_dlls: Vec<Vec<u8>> = Vec::new();
    let mut seen = 0usize;
    let names: Vec<String> = files.iter().map(|f| String::from_utf8_lossy(&f.name).into_owned()).collect();
    for f in files {
        let Some(PeInfo { is_dll, .. }) = pe::parse(&f.data) else { continue };
        let info = pe::parse(&f.data).expect("just parsed");
        seen += 1;
        match (info.arch(), is_dll) {
            (Arch::X64, true) => x64_dlls.push(f.data),
            (Arch::X64, false) => x64_exes.push(f.data),
            (Arch::X86, true) => x86_dlls.push(f.data),
            _ => {}
        }
    }
    let largest_first = |v: &mut Vec<Vec<u8>>| v.sort_by_key(|d| std::cmp::Reverse(d.len()));
    largest_first(&mut x64_dlls);
    largest_first(&mut x64_exes);
    largest_first(&mut x86_dlls);
    let mut dlls = x64_dlls.into_iter();
    let mut exes = x64_exes.into_iter();
    let (Some(x64_redist), Some(x64_service)) = (dlls.next(), exes.next()) else {
        return Err(PrefixError::Malformed(format!(
            "GameInputRedist.msi does not look like the GameInput redistributable ({seen} Windows image(s) found, \
             but not a 64-bit DLL together with a 64-bit service; files: {})",
            names.join(", ")
        )));
    };
    Ok(RedistPayload {
        x64_redist,
        x64_service,
        x64_bridge: dlls.next(),
        x64_raw_input_proxy: exes.next(),
        x86_redist: x86_dlls.into_iter().next(),
    })
}

fn x64_dir(prefix: &Path) -> PathBuf {
    prefix.join(REDIST_DIR_REL).join("x64")
}

/// Whether the native redistributable is installed. Not the builtin `gameinput.dll`: see the module docs.
pub fn redist_ok(prefix: &Path) -> bool {
    let x64 = x64_dir(prefix);
    x64.join("GameInputRedist.dll").is_file() && x64.join("GameInputRedistService.exe").is_file()
}

/// The registry values the MSI writes: where the game finds the redistributable (both registry views) and the
/// demand-start service entry. The service lives under the concrete control set (the editor resolves
/// `CurrentControlSet` itself).
pub fn registry_changes() -> Vec<RegChange> {
    let service = r"System\CurrentControlSet\Services\GameInputRedistService";
    vec![
        RegChange::sz(r"Software\Microsoft\GameInput", "RedistDir", REDIST_DIR_WIN),
        RegChange::sz(r"Software\Wow6432Node\Microsoft\GameInput", "RedistDir", REDIST_DIR_WIN),
        RegChange::sz(service, "DisplayName", "GameInput Redist Service"),
        RegChange::sz(service, "Description", "GameInput Redist Service"),
        RegChange::sz(service, "ImagePath", &format!(r"{REDIST_DIR_WIN}\GameInputRedistService.exe")),
        RegChange::sz(service, "ObjectName", "LocalSystem"),
        RegChange::dword(service, "ErrorControl", 0),
        RegChange::dword(service, "Start", 3),
        RegChange::dword(service, "Type", 0x10),
    ]
}

/// Whether the game ships a newer redistributable than the prefix holds. A prefix keeps the one it was first
/// given across every build; a build made against a newer GameInput then found the old one answering, and the
/// devices it reads through GameInput (the camera's mouse movement, every controller) never reached it. A newer
/// redistributable than the game's is left alone: GameInput keeps older callers working.
fn outdated(prefix: &Path, msi: &Path) -> bool {
    let Ok(len) = fs::metadata(msi).map(|m| m.len()) else { return false };
    if len > MAX_MSI_BYTES {
        return false;
    }
    let Ok(bytes) = fs::read(msi) else { return false };
    let Ok(payload) = payload_from_msi(&bytes) else { return false };
    let Some(bundled) = pe::file_version(&payload.x64_redist) else { return false };
    let installed = fs::read(x64_dir(prefix).join("GameInputRedist.dll")).ok().and_then(|d| pe::file_version(&d));
    installed.is_none_or(|i| bundled > i)
}

fn write_payload(prefix: &Path, payload: &RedistPayload) -> Result<(), PrefixError> {
    let x64 = safe_dir(prefix, "drive_c/Program Files/Microsoft GameInput/x64")?;
    let sys32 = safe_dir(prefix, "drive_c/windows/system32")?;
    let mut written: Vec<(PathBuf, &[u8])> = Vec::new();
    written.push((x64.join("GameInputRedist.dll"), &payload.x64_redist));
    written.push((sys32.join("GameInputRedist.dll"), &payload.x64_redist));
    written.push((x64.join("GameInputRedistService.exe"), &payload.x64_service));
    if let Some(b) = &payload.x64_bridge {
        written.push((x64.join("GameInputBridge.dll"), b));
    }
    if let Some(p) = &payload.x64_raw_input_proxy {
        written.push((x64.join("GameInputRawInputProxy.exe"), p));
    }
    if let Some(x) = &payload.x86_redist {
        let x86 = safe_dir(prefix, "drive_c/Program Files/Microsoft GameInput/x86")?;
        written.push((x86.join("GameInputRedist.dll"), x));
    }
    for (path, bytes) in &written {
        write_atomic(path, bytes, 0o644)?;
    }
    // Every file has to read back as what was written before the install counts.
    for (path, bytes) in &written {
        let back = fs::read(path).map_err(|e| PrefixError::io(format!("check {}", path.display()), e))?;
        if back != *bytes {
            return Err(PrefixError::Integrity(format!("{} did not read back as written", path.display())));
        }
    }
    Ok(())
}

/// Install (or bring up to date) the native GameInput redistributable in a stopped, booted prefix.
pub fn install(prefix: &Path, game_dir: &Path) -> Result<Outcome, PrefixError> {
    if !prefix_ready(prefix) {
        return Err(PrefixError::Invalid(
            "Cannot install Microsoft GameInput: the Wine prefix is incomplete (system.reg, user.reg or system32 is missing). \
             Repair the Bedrock install to set it up first."
                .into(),
        ));
    }
    require_prefix_idle(prefix, "install Microsoft GameInput")?;
    let msi = game_dir.join("Installers").join("GameInputRedist.msi");
    let updating = if redist_ok(prefix) {
        if !outdated(prefix, &msi) {
            update_prefix_registry(prefix, &registry_changes(), &[])?;
            return Ok(Outcome::AlreadyCurrent);
        }
        true
    } else if !msi.is_file() {
        return Ok(Outcome::MsiMissing);
    } else {
        false
    };
    let len = fs::metadata(&msi).map_err(|e| PrefixError::io("read GameInputRedist.msi", e))?.len();
    if len > MAX_MSI_BYTES {
        return Err(PrefixError::Malformed("GameInputRedist.msi is far larger than the redistributable".into()));
    }
    let bytes = fs::read(&msi).map_err(|e| PrefixError::io("read GameInputRedist.msi", e))?;
    let payload = payload_from_msi(&bytes)?;
    write_payload(prefix, &payload)?;
    update_prefix_registry(prefix, &registry_changes(), &[])?;
    if !redist_ok(prefix) {
        return Err(PrefixError::Integrity("GameInput was written but the prefix does not see it".into()));
    }
    Ok(if updating { Outcome::Updated } else { Outcome::Installed })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::bedrock::prefix::cab::build::{cabinet, mszip};
    use crate::bedrock::prefix::cfb::build::{Stream, compound_file};
    use pe::{MACHINE_AMD64, MACHINE_I386, test_image};

    /// A synthetic MSI shaped like the redistributable's: one MSZip cabinet (several blocks) in a compound file,
    /// with distinguishable 64-bit and 32-bit images and a catalogue that is not a PE. No game bytes.
    pub fn msi(version: (u16, u16, u16, u16)) -> Vec<u8> {
        let images: Vec<(&str, Vec<u8>)> = vec![
            ("redist.x86", test_image(MACHINE_I386, true, Some(version), 3_000)),
            ("proxy.exe", test_image(MACHINE_AMD64, false, None, 2_000)),
            ("redist.x64", test_image(MACHINE_AMD64, true, Some(version), 40_000)),
            ("redist.cat", b"catalog bytes, not a PE".to_vec()),
            ("service.exe", test_image(MACHINE_AMD64, false, None, 9_000)),
            ("bridge.dll", test_image(MACHINE_AMD64, true, None, 12_000)),
        ];
        let mut all = Vec::new();
        let mut index = Vec::new();
        for (name, data) in &images {
            index.push((*name, all.len(), data.len()));
            all.extend_from_slice(data);
        }
        let blocks: Vec<_> = all.chunks(32_768).map(mszip).collect();
        let cab = cabinet(1, &blocks, &index);
        compound_file(&[
            Stream { name: "\u{4840}Property", data: vec![1, 2, 3, 4, 5, 6] },
            Stream { name: "redist.cab", data: cab },
            Stream { name: "\u{4840}File", data: vec![9; 100] },
        ])
    }

    pub fn image_parts(version: (u16, u16, u16, u16)) -> (usize, usize) {
        (
            test_image(MACHINE_AMD64, true, Some(version), 40_000).len(),
            test_image(MACHINE_AMD64, false, None, 9_000).len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::testutil::scratch;
    use pe::{MACHINE_AMD64, MACHINE_I386};

    fn prefix_with_hives() -> PathBuf {
        let pfx = scratch("gameinput");
        fs::create_dir_all(pfx.join("drive_c/windows/system32")).unwrap();
        fs::write(
            pfx.join("system.reg"),
            "WINE REGISTRY Version 2\n\n[System\\\\Select] 1\n\"Current\"=dword:00000001\n\n",
        )
        .unwrap();
        fs::write(pfx.join("user.reg"), "WINE REGISTRY Version 2\n\n").unwrap();
        pfx
    }

    fn game_with_msi(version: (u16, u16, u16, u16)) -> PathBuf {
        let game = scratch("gameinput-game");
        fs::create_dir_all(game.join("Installers")).unwrap();
        fs::write(game.join("Installers/GameInputRedist.msi"), msi(version)).unwrap();
        game
    }

    #[test]
    fn sorts_the_images_by_architecture_kind_and_size() {
        let payload = payload_from_msi(&msi((2, 2, 26100, 6106))).unwrap();
        assert_eq!(pe::parse(&payload.x64_redist).unwrap().arch(), Arch::X64);
        assert_eq!(payload.x64_redist.len(), image_parts((2, 2, 26100, 6106)).0);
        assert_eq!(payload.x64_service.len(), image_parts((2, 2, 26100, 6106)).1);
        assert!(
            payload
                .x64_bridge
                .as_ref()
                .is_some_and(|b| pe::parse(b).unwrap().is_dll && b.len() < payload.x64_redist.len())
        );
        assert!(payload.x64_raw_input_proxy.is_some());
        let x86 = payload.x86_redist.unwrap();
        assert_eq!(pe::parse(&x86).unwrap().machine, MACHINE_I386);
        assert_eq!(pe::file_version(&payload.x64_redist), Some((2, 2, 26100, 6106)));
        let _ = MACHINE_AMD64;
    }

    #[test]
    fn installs_every_file_and_the_registry_and_is_idempotent() {
        let pfx = prefix_with_hives();
        let game = game_with_msi((2, 2, 26100, 6106));
        assert_eq!(install(&pfx, &game).unwrap(), Outcome::Installed);
        let x64 = x64_dir(&pfx);
        for f in
            ["GameInputRedist.dll", "GameInputRedistService.exe", "GameInputBridge.dll", "GameInputRawInputProxy.exe"]
        {
            assert!(x64.join(f).is_file(), "{f}");
        }
        assert!(pfx.join(REDIST_DIR_REL).join("x86/GameInputRedist.dll").is_file());
        assert_eq!(
            fs::read(pfx.join("drive_c/windows/system32/GameInputRedist.dll")).unwrap(),
            fs::read(x64.join("GameInputRedist.dll")).unwrap()
        );
        assert!(redist_ok(&pfx));
        let system = fs::read_to_string(pfx.join("system.reg")).unwrap();
        assert!(
            system.contains(r"[Software\\Microsoft\\GameInput]")
                && system.contains(r"[Software\\Wow6432Node\\Microsoft\\GameInput]")
        );
        assert!(system.contains(r#""RedistDir"="C:\\Program Files\\Microsoft GameInput\\x64""#));
        assert!(system.contains(r"[System\\ControlSet001\\Services\\GameInputRedistService]"), "{system}");
        assert!(!system.contains("CurrentControlSet"));
        assert!(
            system.contains("\"Start\"=dword:00000003")
                && system.contains("\"Type\"=dword:00000010")
                && system.contains("\"ErrorControl\"=dword:00000000")
        );
        assert!(
            system.contains(r#""ImagePath"="C:\\Program Files\\Microsoft GameInput\\x64\\GameInputRedistService.exe""#)
        );
        let before = (fs::read(pfx.join("system.reg")).unwrap(), fs::read(x64.join("GameInputRedist.dll")).unwrap());
        assert_eq!(install(&pfx, &game).unwrap(), Outcome::AlreadyCurrent);
        assert_eq!(
            (fs::read(pfx.join("system.reg")).unwrap(), fs::read(x64.join("GameInputRedist.dll")).unwrap()),
            before
        );
    }

    #[test]
    fn an_older_redist_is_updated_and_a_newer_one_is_kept() {
        let pfx = prefix_with_hives();
        install(&pfx, &game_with_msi((1, 0, 22621, 2))).unwrap();
        assert_eq!(install(&pfx, &game_with_msi((2, 2, 26100, 6106))).unwrap(), Outcome::Updated);
        let version = |p: &Path| pe::file_version(&fs::read(x64_dir(p).join("GameInputRedist.dll")).unwrap());
        assert_eq!(version(&pfx), Some((2, 2, 26100, 6106)));
        assert_eq!(install(&pfx, &game_with_msi((1, 0, 22621, 2))).unwrap(), Outcome::AlreadyCurrent);
        assert_eq!(version(&pfx), Some((2, 2, 26100, 6106)), "a newer redist is not taken away");
    }

    #[test]
    fn a_missing_msi_is_reported_not_fatal_and_a_broken_one_is_an_error() {
        let pfx = prefix_with_hives();
        let game = scratch("gameinput-nomsi");
        assert_eq!(install(&pfx, &game).unwrap(), Outcome::MsiMissing);
        assert!(!redist_ok(&pfx));
        fs::create_dir_all(game.join("Installers")).unwrap();
        let good = msi((2, 2, 26100, 6106));
        // Truncated anywhere: an error, and nothing installed.
        for cut in [10, 600, good.len() / 3, good.len() / 2, good.len() - 700] {
            fs::write(game.join("Installers/GameInputRedist.msi"), &good[..cut]).unwrap();
            assert!(install(&pfx, &game).is_err(), "cut {cut}");
            assert!(!redist_ok(&pfx));
        }
        let hives = fs::read(pfx.join("system.reg")).unwrap();
        assert!(!String::from_utf8_lossy(&hives).contains("GameInput"), "a failed install writes no registry values");
    }

    #[test]
    fn an_msi_without_the_expected_images_is_refused() {
        use crate::bedrock::prefix::cab::build::{cabinet, mszip};
        use crate::bedrock::prefix::cfb::build::{Stream, compound_file};
        let only_exe = pe::test_image(MACHINE_AMD64, false, None, 100);
        let cab = cabinet(1, &[mszip(&only_exe)], &[("a.exe", 0, only_exe.len())]);
        let msi = compound_file(&[Stream { name: "c", data: cab }]);
        let e = payload_from_msi(&msi).err().unwrap();
        assert!(e.to_string().contains("does not look like"), "{e}");
        let no_cab = compound_file(&[Stream { name: "t", data: vec![1; 50] }]);
        assert!(payload_from_msi(&no_cab).is_err());
        assert!(payload_from_msi(b"not an msi").is_err());
    }

    #[test]
    fn an_incomplete_prefix_is_refused_and_a_busy_one_too() {
        let game = game_with_msi((2, 2, 26100, 6106));
        let empty = scratch("gameinput-empty");
        assert!(install(&empty, &game).is_err());
        let pfx = prefix_with_hives();
        let mut child = std::process::Command::new("sleep").arg("30").env("WINEPREFIX", &pfx).spawn().unwrap();
        let result = install(&pfx, &game);
        child.kill().ok();
        child.wait().ok();
        assert!(matches!(result, Err(PrefixError::Busy(_))));
        assert!(!redist_ok(&pfx));
    }

    #[test]
    fn wines_builtin_gameinput_is_not_readiness() {
        let pfx = prefix_with_hives();
        fs::write(pfx.join("drive_c/windows/system32/gameinput.dll"), b"builtin").unwrap();
        assert!(!redist_ok(&pfx));
    }

    #[test]
    fn a_symlinked_install_dir_is_not_followed() {
        let pfx = prefix_with_hives();
        let game = game_with_msi((2, 2, 26100, 6106));
        let outside = scratch("gameinput-outside");
        fs::create_dir_all(pfx.join("drive_c/Program Files")).unwrap();
        std::os::unix::fs::symlink(&outside, pfx.join("drive_c/Program Files/Microsoft GameInput")).unwrap();
        assert!(install(&pfx, &game).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }
}
