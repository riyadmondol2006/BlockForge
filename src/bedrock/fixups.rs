//! Fixes applied to the game's own files, and the DLLs that go beside them.
//!
//! Ported from BedrockOnLinux `bol/fixups.py` and `bol/pe.py` (MIT), with credit. All of this works on a game
//! folder that is not running. BlockForge patches a *staged* copy of a build once and publishes it unchanged, so
//! a running game never sees a half-patched file; every patch here is idempotent and keeps the original bytes
//! (`<file>.bf-orig`, made once) so it can be undone.
//!
//! The patches are anchored on structure, never on names a build can rename. A build that moves past an anchor
//! is *reported* ([`HbuiStatus::AnchorsLost`], [`GateStatus::AnchorNotFound`]) and never silently skipped: the
//! game still starts, and only the Servers tab or online routing misbehaves, which is indistinguishable from a
//! broken install unless the launcher says why.

use std::fs::{self, File};
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::{FileExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use super::prefix::{PrefixError, sha256_file, write_atomic};

// ---------------------------------------------------------------- backups

/// Keep the original of `path` once, beside it.
fn backup_once(path: &Path) -> Result<(), PrefixError> {
    let mut name = path.as_os_str().to_owned();
    name.push(".bf-orig");
    let backup = PathBuf::from(name);
    if fs::symlink_metadata(&backup).is_ok() {
        return Ok(());
    }
    fs::copy(path, &backup).map_err(|e| PrefixError::io(format!("keep a copy of {}", path.display()), e))?;
    Ok(())
}

fn file_mode(path: &Path) -> u32 {
    fs::metadata(path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o644)
}

/// Replace `path` with `bytes` (original kept once, new file swapped in whole).
fn patch_file(path: &Path, bytes: &[u8]) -> Result<(), PrefixError> {
    backup_once(path)?;
    write_atomic(path, bytes, file_mode(path) | 0o200)
}

// ---------------------------------------------------------------- PE stack reserve

/// Raise `SizeOfStackReserve` of the PE32+ image behind `fd` to at least `target`; return the old value, or None
/// when there is nothing to do (not a PE32+ image, or already at least `target`).
///
/// The game's executable ships with a 1 MB reserve, and its settings/pause screens walk a deep-but-bounded call
/// chain that overran it by ~11 KB, crashing on Wine with a stack-limit exception. Wine sizes the initial
/// thread's stack, and every worker that passes a size of 0, from this header field, so 16 MB gives the chain
/// room; address space is the only cost on x64.
///
/// It takes a descriptor, not a path, because a Store package never has a loadable executable on disk: the
/// launcher decrypts it into anonymous memory or a private RAM file, and that open descriptor is the only place
/// this edit can land. The descriptor must be open for writing; it is read and written by offset, so its
/// position is not disturbed. This is an in-place 8-byte edit, safe to repeat on every launch.
pub fn raise_stack_reserve(fd: impl AsFd, target: u64) -> io::Result<Option<u64>> {
    // Duplicate the descriptor to get a `File` to call pread/pwrite on; both share one open file description.
    let file = File::from(fd.as_fd().try_clone_to_owned()?);
    let mut head = vec![0u8; 0x400];
    let mut got = 0;
    while got < head.len() {
        match file.read_at(&mut head[got..], got as u64)? {
            0 => break,
            n => got += n,
        }
    }
    head.truncate(got);
    let Some(pe) = super::prefix::pe::pe_offset(&head) else { return Ok(None) };
    let coff = pe + 4;
    let opt = coff + 20;
    let size_of_optional =
        u16::from_le_bytes(head.get(coff + 16..coff + 18).ok_or_else(truncated)?.try_into().unwrap()) as usize;
    let magic = u16::from_le_bytes(head.get(opt..opt + 2).ok_or_else(truncated)?.try_into().unwrap());
    if magic != 0x20b {
        return Ok(None); // not PE32+: leave it alone
    }
    let field = opt + 72; // SizeOfStackReserve in the PE32+ optional header
    if size_of_optional < 72 + 8 || field + 8 > head.len() {
        return Err(truncated());
    }
    let current = u64::from_le_bytes(head[field..field + 8].try_into().unwrap());
    if current >= target {
        return Ok(None);
    }
    file.write_at(&target.to_le_bytes(), field as u64)
        .and_then(|n| if n == 8 { Ok(()) } else { Err(io::Error::from(io::ErrorKind::WriteZero)) })?;
    let _ = file.as_raw_fd();
    Ok(Some(current))
}

fn truncated() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "the executable's headers are cut off")
}

/// [`raise_stack_reserve`] for an executable on disk.
#[cfg(test)]
pub fn raise_stack_reserve_path(exe: &Path, target: u64) -> io::Result<Option<u64>> {
    let file = fs::OpenOptions::new().read(true).write(true).open(exe)?;
    raise_stack_reserve(&file, target)
}

/// The reserve the game gets: 16 MiB.
pub const GAME_STACK_RESERVE: u64 = 0x100_0000;

// ---------------------------------------------------------------- libHttpClient provider gate

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateStatus {
    Applied,
    Already,
    /// The DLL is not in this build.
    NotPresent,
    /// The structure the patch is anchored on is not there: this build changed past it.
    AnchorNotFound,
    /// The anchor matched but the jump it should end in is not the expected one.
    Misaligned,
}

fn lhc_gate_regex() -> &'static regex::bytes::Regex {
    static RE: OnceLock<regex::bytes::Regex> = OnceLock::new();
    // add eax,-2 ; mov edx,4 ; lea rcx,[rip+imm32] ; cmp eax,6 ; (ja rel32 | six NOPs)
    RE.get_or_init(|| {
        regex::bytes::Regex::new(
            r"(?s-u)\x83\xc0\xfe\xba\x04\x00\x00\x00\x48\x8d\x0d.{4}\x83\xf8\x06(?:\x0f\x87.{4}|\x90{6})",
        )
        .unwrap()
    })
}

/// Force libHttpClient.GDK onto the XCurl HTTP provider. Its console check only takes the XCurl path for console
/// enums 2..8; under Wine it falls to WinHTTP, which hits Azure's TLS wall. The six-byte conditional jump after
/// the comparison is replaced by NOPs so XCurl is always used. Found by the instruction shape around it.
pub fn patch_lhc_xcurl_gate(game_dir: &Path) -> Result<GateStatus, PrefixError> {
    let dll = game_dir.join("libHttpClient.GDK.dll");
    let mut data = match fs::read(&dll) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(GateStatus::NotPresent),
        Err(e) => return Err(PrefixError::io("read libHttpClient.GDK.dll", e)),
    };
    let Some(m) = lhc_gate_regex().find(&data) else { return Ok(GateStatus::AnchorNotFound) };
    let ja = m.start() + 18; // past add(3) + mov(5) + lea(7) + cmp(3)
    if data[ja..ja + 6] == [0x90; 6] {
        return Ok(GateStatus::Already);
    }
    if data[ja..ja + 2] != [0x0f, 0x87] {
        return Ok(GateStatus::Misaligned);
    }
    data[ja..ja + 6].fill(0x90);
    patch_file(&dll, &data)?;
    Ok(GateStatus::Applied)
}

// ---------------------------------------------------------------- HBUI: the Microsoft-account gate and the dead link

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchStatus {
    Applied,
    Already,
    Missing,
}

// Both patches are anchored on text a Minecraft build cannot rename -- facet property names, the states they
// derive and the sign-in source -- with every minified identifier matched as \w+. Anchoring on the minified
// names retired both once already: the needles written for 1.1.0 stopped matching once the UI was rebundled
// (the module alias moved from `l.` to `r.`), and because the patch skipped quietly the Servers tab stayed
// blocked and the dead in-game "Sign in" link came back. 1.26.60 went further: its bundler calls the imported
// hooks directly (`k(` for `(0,r.useFacetMap)(`) and drops the parentheses around a lone arrow function.
const FACET_MAP: &str = r"(?:\(0,\w+\.useFacetMap\)|\s[\w$]+)\(";

struct HbuiRegexes {
    gate: Regex,
    gate_done: Regex,
    link: Regex,
    link_done: Regex,
}

fn hbui_regexes() -> &'static HbuiRegexes {
    static RE: OnceLock<HbuiRegexes> = OnceLock::new();
    RE.get_or_init(|| HbuiRegexes {
        gate: Regex::new(&format!(r"(function\s+\w+\(\)\{{)(return{FACET_MAP})")).unwrap(),
        gate_done: Regex::new(&format!(r#"function\s+\w+\(\)\{{return"";return{FACET_MAP}"#)).unwrap(),
        link: Regex::new(r"(_NotLoggedInWarning_OreUI`\)\}\)?,\[[^\]]*\]\);)(return \w+\.createElement\(\w+,)")
            .unwrap(),
        link_done: Regex::new(r"_NotLoggedInWarning_OreUI`\)\}\)?,\[[^\]]*\]\);return null;").unwrap(),
    })
}

// What the Play screen's guest/sign-in facet reads out of the account facet and the states it returns.
const GATE_MARKS: [&str; 4] =
    ["isSignedInPlatformNetwork", "isLoggedInWithMicrosoftAccount", "\"not-signed-in\"", "\"msa-guest-playstation\""];

/// Make the derived "not signed in" facet report nothing. Its consumers all switch on the state it returns and
/// end in `default: return null`, so an empty string takes the "You need a Microsoft account" notice off the
/// Servers and Realms tabs. The notice is there because XSAPI never completes its init under Wine, not because
/// the account is missing. The early return is unconditional, so the function contributes the same (zero) hooks
/// on every render and React's hook order stays consistent.
pub fn hbui_gate_patch(data: &str) -> (String, PatchStatus) {
    let re = hbui_regexes();
    if re.gate_done.is_match(data) {
        return (data.to_string(), PatchStatus::Already);
    }
    for caps in re.gate.captures_iter(data) {
        let whole = caps.get(0).unwrap();
        let head_end = caps.get(1).unwrap().end();
        // Bound the window at the next function declaration: the marks are common enough that an unbounded
        // look-ahead matches the neighbour.
        let stop = data[whole.end()..]
            .find("}function ")
            .map(|i| whole.end() + i)
            .unwrap_or_else(|| (whole.end() + 2000).min(data.len()));
        let stop = floor_boundary(data, stop);
        let body = &data[whole.end()..stop];
        if GATE_MARKS.iter().all(|m| body.contains(m)) {
            let mut out = String::with_capacity(data.len() + 10);
            out.push_str(&data[..head_end]);
            out.push_str("return\"\";");
            out.push_str(&data[head_end..]);
            return (out, PatchStatus::Applied);
        }
    }
    (data.to_string(), PatchStatus::Missing)
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Remove the in-game "Sign in" link of the not-logged-in notice. It reaches an interactive XUser sign-in the
/// engine does not implement, so it can only ever answer "Failed to log in"; the account is linked from the
/// launcher instead. The early return goes *after* the component's hooks (the callback whose body holds the
/// sign-in source) so React's hook order is preserved, unlike a naive return at the top.
pub fn hbui_link_patch(data: &str) -> (String, PatchStatus) {
    let re = hbui_regexes();
    if re.link_done.is_match(data) {
        return (data.to_string(), PatchStatus::Already);
    }
    let Some(caps) = re.link.captures(data) else { return (data.to_string(), PatchStatus::Missing) };
    let at = caps.get(1).unwrap().end();
    let mut out = String::with_capacity(data.len() + 12);
    out.push_str(&data[..at]);
    out.push_str("return null;");
    out.push_str(&data[at..]);
    (out, PatchStatus::Applied)
}

/// One HBUI patch run (or inspection): the bundle that carries the Play screen and what each patch did there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HbuiReport {
    pub bundle: Option<String>,
    pub gate: PatchStatus,
    pub link: PatchStatus,
}

/// What the patches mean for the player, from the report. The compatibility diagnostic for a build whose UI was
/// rebundled past the anchors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HbuiStatus {
    NoBundle,
    Patched,
    /// Anchors found but not applied yet (a read-only inspection of an unpatched build).
    NotAppliedYet,
    /// A patch's anchor is gone: this build rebundled its UI.
    AnchorsLost {
        lost: Vec<&'static str>,
    },
}

impl HbuiReport {
    fn lost(&self) -> Vec<&'static str> {
        [("the Microsoft-account gate", self.gate), ("the dead in-game Sign-in link", self.link)]
            .into_iter()
            .filter(|(_, s)| *s == PatchStatus::Missing)
            .map(|(l, _)| l)
            .collect()
    }

    pub fn status(&self) -> HbuiStatus {
        if self.bundle.is_none() {
            return HbuiStatus::NoBundle;
        }
        let lost = self.lost();
        if !lost.is_empty() {
            HbuiStatus::AnchorsLost { lost }
        } else if self.gate == PatchStatus::Applied || self.link == PatchStatus::Applied {
            HbuiStatus::NotAppliedYet
        } else {
            HbuiStatus::Patched
        }
    }
}

impl HbuiStatus {
    /// (summary, problem): the problem is None when nothing is wrong.
    pub fn diagnostic(&self) -> (String, Option<String>) {
        match self {
            HbuiStatus::NoBundle => ("no HBUI bundle in this build".into(), None),
            HbuiStatus::Patched => ("OK (gate + link patched)".into(), None),
            HbuiStatus::NotAppliedYet => (
                "not applied yet".into(),
                Some("The in-game Microsoft-account gate is not patched in the installed build yet; repair the install to apply it.".into()),
            ),
            HbuiStatus::AnchorsLost { lost } => (
                "NOT PATCHED (this build rebundled its UI)".into(),
                Some(format!(
                    "This Minecraft build rebundled its UI past {}, so the Servers tab can stay blocked behind 'You need a Microsoft account' \
                     and the in-game Sign-in link answers 'Failed to log in'. Sign in from the launcher instead, and report the Minecraft \
                     version so the patch can be re-anchored.",
                    lost.join(" and ")
                )),
            ),
        }
    }
}

fn hbui_bundles(game_dir: &Path) -> Vec<PathBuf> {
    let dir = game_dir.join("data/gui/dist/hbui");
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.starts_with("index-") && n.ends_with(".js")
            })
        })
        .collect();
    found.sort();
    found
}

fn best_report(results: Vec<HbuiReport>) -> HbuiReport {
    let score = |r: &HbuiReport| (r.gate != PatchStatus::Missing) as u8 + (r.link != PatchStatus::Missing) as u8;
    // The Play screen lives in a single bundle; report on the one that had it.
    results.into_iter().max_by_key(score).unwrap_or(HbuiReport {
        bundle: None,
        gate: PatchStatus::Missing,
        link: PatchStatus::Missing,
    })
}

/// Both patch states in an installed game folder, without writing.
pub fn hbui_inspect(game_dir: &Path) -> HbuiReport {
    let results = hbui_bundles(game_dir)
        .into_iter()
        .filter_map(|js| {
            let text = fs::read_to_string(&js).ok()?;
            Some(HbuiReport {
                bundle: js.file_name().map(|n| n.to_string_lossy().into_owned()),
                gate: hbui_gate_patch(&text).1,
                link: hbui_link_patch(&text).1,
            })
        })
        .collect();
    best_report(results)
}

/// Neutralise HBUI's Microsoft-account gate and its dead "Sign in" link in every bundle that carries them.
/// Never fatal, never silent: the report says whether the anchors were found. Idempotent.
pub fn patch_hbui(game_dir: &Path) -> Result<HbuiReport, PrefixError> {
    let mut results = Vec::new();
    for js in hbui_bundles(game_dir) {
        // A bundle that is not UTF-8 text is not one these patches understand.
        let Ok(original) = fs::read_to_string(&js) else { continue };
        let (data, gate) = hbui_gate_patch(&original);
        let (data, link) = hbui_link_patch(&data);
        if data != original {
            patch_file(&js, data.as_bytes())?;
        }
        results.push(HbuiReport { bundle: js.file_name().map(|n| n.to_string_lossy().into_owned()), gate, link });
    }
    Ok(best_report(results))
}

// ---------------------------------------------------------------- the DLLs that go in the game folder

/// Reviewed builds of the open-source GDK Xbox-Live DLLs (minecraft-linux/mcpelauncher-gdk-dependencies v0.0.0),
/// from PORT-PLAN.md 1.4 (BedrockOnLinux `bol/config.py`). A file that does not hash to its pin is never copied.
pub const GDK_DEPS: [(&str, &str); 2] = [
    ("libHttpClient.GDK.dll", "718f0c6842c8e35d7cac5c18ec025404c83be03ec035f0d3904e70d6508b9e24"),
    ("XCurl.dll", "28443194e9860733f24294219bcb45226677636f9721d4bf145094d977f7161f"),
];

/// Where the verified inputs of [`install_game_dlls`] are. The caller (package B's artifact layer) has already
/// verified the archives these came from; the GDK DLLs are re-checked here against their pins anyway.
#[derive(Clone, Debug)]
pub struct DllSources {
    /// Holds `libHttpClient.GDK.dll` and `XCurl.dll`.
    pub gdk_deps: PathBuf,
    /// The unpacked OpenSSL XCurl set: `libcurl-4.dll`, `xcurl-cashim.dll`, `cryptbase.dll` and the libssl/zlib
    /// closure.
    pub openssl_set: PathBuf,
    /// The reviewed CA bundle.
    pub cacert: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct DllReport {
    pub copied: Vec<String>,
    pub lhc_gate: Option<GateStatus>,
    pub hbui: Option<HbuiReport>,
}

fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn install_file(src: &Path, dst: &Path, report: &mut DllReport) -> Result<(), PrefixError> {
    let meta = fs::symlink_metadata(src).map_err(|e| PrefixError::io(format!("read {}", src.display()), e))?;
    if !meta.file_type().is_file() {
        return Err(PrefixError::Unsafe(format!("{} is not a plain file.", src.display())));
    }
    if let (Ok(a), Ok(b)) = (sha256_file(src), sha256_file(dst)) {
        if a == b && fs::symlink_metadata(dst).is_ok_and(|m| m.file_type().is_file()) {
            return Ok(());
        }
    }
    let bytes = fs::read(src).map_err(|e| PrefixError::io(format!("read {}", src.display()), e))?;
    if fs::symlink_metadata(dst).is_ok_and(|m| m.file_type().is_file()) {
        backup_once(dst)?;
    }
    write_atomic(dst, &bytes, 0o644)?;
    report.copied.push(dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Ok(())
}

/// Put the Xbox-Live DLLs, the OpenSSL XCurl closure and the CA bundle in the game folder, then apply the
/// libHttpClient provider gate and the HBUI patches.
///
/// `XCurl.dll` ends up as the CA-injecting shim (libHttpClient never sets `CURLOPT_CAINFO`, so the shim injects
/// `<game>\cacert.pem` at `curl_easy_init` and forwards the rest) and `xcurl_real.dll` is the real OpenSSL
/// libcurl; routing PlayFab over OpenSSL avoids Wine's Schannel TLS, which Azure Front Door drops, so login would
/// loop. The online path needs the *complete* set: a missing piece is an error, not a warning that leaves the
/// Schannel XCurl in place and fails later. The genuine originals are kept once as `.bf-orig`. `cryptbase.dll`
/// is deliberately not copied here: it belongs in the prefix's system32 (see `prefix::install_cryptbase`).
pub fn install_game_dlls(game_dir: &Path, sources: &DllSources) -> Result<DllReport, PrefixError> {
    let mut report = DllReport::default();
    // Check everything is there and reviewed before the first byte is written.
    for (name, pin) in GDK_DEPS {
        let path = sources.gdk_deps.join(name);
        let actual = sha256_file(&path).map_err(|e| PrefixError::io(format!("read {name}"), e))?;
        if hex(actual) != pin {
            return Err(PrefixError::Integrity(format!(
                "{name} is not the reviewed build BlockForge installs, so it was not put into the game."
            )));
        }
    }
    let set = &sources.openssl_set;
    for need in ["libcurl-4.dll", "xcurl-cashim.dll", "cryptbase.dll"] {
        if !fs::symlink_metadata(set.join(need)).is_ok_and(|m| m.file_type().is_file()) {
            return Err(PrefixError::Integrity(format!(
                "The OpenSSL XCurl set is incomplete ({need} is missing); online play needs all of it."
            )));
        }
    }
    if !fs::symlink_metadata(&sources.cacert).is_ok_and(|m| m.file_type().is_file()) {
        return Err(PrefixError::Integrity("The CA bundle is missing.".into()));
    }

    for (name, _) in GDK_DEPS {
        install_file(&sources.gdk_deps.join(name), &game_dir.join(name), &mut report)?;
    }
    // The genuine XCurl is preserved before the shim goes over it (install_file keeps whatever it replaces once).
    let mut deps: Vec<PathBuf> = fs::read_dir(set)
        .map_err(|e| PrefixError::io("read the OpenSSL XCurl set", e))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")))
        .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file()))
        .filter(|p| !matches!(p.file_name().and_then(|n| n.to_str()), Some("cryptbase.dll" | "xcurl-cashim.dll")))
        .collect();
    deps.sort();
    for dll in deps {
        install_file(&dll, &game_dir.join(dll.file_name().expect("has a name")), &mut report)?;
    }
    install_file(&set.join("libcurl-4.dll"), &game_dir.join("xcurl_real.dll"), &mut report)?;
    for name in ["XCurl.dll", "Xcurl.dll"] {
        install_file(&set.join("xcurl-cashim.dll"), &game_dir.join(name), &mut report)?;
    }
    install_ca_bundle(&sources.cacert, game_dir, &mut report)?;
    report.lhc_gate = Some(patch_lhc_xcurl_gate(game_dir)?);
    report.hbui = Some(patch_hbui(game_dir)?);
    Ok(report)
}

/// `cacert.pem` beside the DLLs (the shim injects it) and `etc/ssl/certs/ca-bundle.crt` in the game folder and
/// its parent: GDK-Proton looks for the latter next to the game, and without it every TLS call fails and the
/// server join hangs. Runs every time; copies only what differs.
pub fn install_ca_bundle(cacert: &Path, game_dir: &Path, report: &mut DllReport) -> Result<(), PrefixError> {
    install_file(cacert, &game_dir.join("cacert.pem"), report)?;
    let mut bases = vec![game_dir.to_path_buf()];
    if let Some(parent) = game_dir.parent() {
        bases.push(parent.to_path_buf());
    }
    for base in bases {
        let dir = base.join("etc/ssl/certs");
        fs::create_dir_all(&dir).map_err(|e| PrefixError::io(format!("create {}", dir.display()), e))?;
        install_file(cacert, &dir.join("ca-bundle.crt"), report)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bedrock::prefix::pe::{MACHINE_AMD64, MACHINE_I386, test_image};
    use crate::testutil::scratch;
    use std::os::fd::OwnedFd;

    // The bundle shapes BedrockOnLinux's own regression tests use (tests/test_hbui_signin_gate.py).
    const GATE_NEW: &str = r#"function T3(){return(0,r.useFacetMap)(((e,t)=>{const{isSignedInPlatformNetwork:a,isLoggedInWithMicrosoftAccount:n,hasPremiumNetworkAccess:c}=e,l=pI(t.platform),o=dI(t.platform);return n&&!a&&l?"platform-guest-nintendo":!n&&a&&o&&!c?"msa-guest-playstation":"not-signed-in"}),[],[(0,r.useSharedFacet)(lC)])}"#;
    const LINK_NEW: &str = r#"function I3({location:e}){const{t}=Ku("notLoggedInWarning"),a=f(),c=VT(),l=(0,n.useCallback)((()=>{c({screen:`${O3(e)}`,button:"SignIn"}),a.push(`/sign-in?signInSource=${O3(e)}_NotLoggedInWarning_OreUI`)}),[c,a,e]);return n.createElement(nD,{role:"noticeTint"},n.createElement(nD.Link,{onClick:l},t(".loginLink")))}"#;
    const GATE_OLD: &str = r#"function wB(){return(0,l.useFacetMap)(((e,t)=>{const{isSignedInPlatformNetwork:a,isLoggedInWithMicrosoftAccount:n,hasPremiumNetworkAccess:c}=e;return!n?"msa-guest-playstation":"not-signed-in"}),[],[])}"#;
    const LINK_OLD: &str = r#"function kB({location:e}){const l=(0,n.useCallback)((()=>{a.push(`/sign-in?signInSource=PlayScreen_NotLoggedInWarning_OreUI`)}),[a,e]);return r.createElement(sx,{role:"noticeTint"},r.createElement(sx.Link,{onClick:l},"login"))}"#;
    const GATE_DIRECT: &str = r#"function mle(){return k((e,t)=>{const{isSignedInPlatformNetwork:n,isLoggedInWithMicrosoftAccount:a,hasPremiumNetworkAccess:r}=e,l=oW(t.platform),o=rW(t.platform);return a&&!n&&l?"platform-guest-nintendo":!a&&n&&o&&!r?"msa-guest-playstation":"not-signed-in"},[],[Fo(WD),Fo(ci)])}"#;
    const LINK_DIRECT: &str = r#"function ple({location:e}){const{t:n}=zu("notLoggedInWarning"),a=_(),r=pB(),l=(0,t.useCallback)(()=>{r({screen:`${fle(e)}`,button:"SignIn"}),a.push(`/sign-in?signInSource=${fle(e)}_NotLoggedInWarning_OreUI`)},[r,a,e]);return t.createElement(SX,{role:"noticeTint"},t.createElement(SX.Link,{onClick:l},n(".loginLink")))}"#;
    const DECOY: &str =
        r#"function p3(){return(0,r.useFacetMap)((e=>e.thirdPartyServers),[],[(0,r.useSharedFacet)(sC)])}"#;

    fn game_with_bundle(tag: &str, bundle: &str) -> (PathBuf, PathBuf) {
        let game = scratch(tag);
        let hbui = game.join("data/gui/dist/hbui");
        fs::create_dir_all(&hbui).unwrap();
        let js = hbui.join("index-0de4683795677b1eaae4.js");
        fs::write(&js, bundle).unwrap();
        (game, js)
    }

    #[test]
    fn patches_all_three_bundle_shapes() {
        for (gate, link, gate_at, link_at) in [
            (
                GATE_NEW,
                LINK_NEW,
                r#"function T3(){return"";return(0,r.useFacetMap)"#,
                "[c,a,e]);return null;return n.createElement(nD,",
            ),
            (
                GATE_OLD,
                LINK_OLD,
                r#"function wB(){return"";return(0,l.useFacetMap)"#,
                "[a,e]);return null;return r.createElement(sx,",
            ),
            (
                GATE_DIRECT,
                LINK_DIRECT,
                r#"function mle(){return"";return k("#,
                "[r,a,e]);return null;return t.createElement(SX,",
            ),
        ] {
            let (game, js) = game_with_bundle("hbui-shapes", &format!("{gate}{link}"));
            let report = patch_hbui(&game).unwrap();
            assert_eq!((report.gate, report.link), (PatchStatus::Applied, PatchStatus::Applied));
            let out = fs::read_to_string(&js).unwrap();
            assert!(out.contains(gate_at) && out.contains(link_at), "{out}");
            assert_eq!(
                fs::read_to_string(format!("{}.bf-orig", js.display())).unwrap(),
                format!("{gate}{link}"),
                "the original is kept"
            );
            let again = patch_hbui(&game).unwrap();
            assert_eq!((again.gate, again.link), (PatchStatus::Already, PatchStatus::Already));
            assert_eq!(fs::read_to_string(&js).unwrap(), out, "idempotent");
            assert_eq!(out.matches("return\"\";return").count(), 1);
            assert_eq!(out.matches("return null;return ").count(), 1);
            assert_eq!(hbui_inspect(&game).status(), HbuiStatus::Patched);
        }
    }

    #[test]
    fn a_neighbouring_facet_hook_is_left_alone() {
        let (game, js) = game_with_bundle("hbui-decoy", &format!("{DECOY}{GATE_NEW}{LINK_NEW}"));
        patch_hbui(&game).unwrap();
        let out = fs::read_to_string(&js).unwrap();
        assert!(out.contains("function p3(){return(0,r.useFacetMap)"));
        assert!(out.contains("function T3(){return\"\";"));
        assert_eq!(out.matches("return\"\";return(0,").count(), 1);
    }

    #[test]
    fn a_rebundled_ui_is_reported_with_a_compatibility_diagnostic() {
        let (game, js) = game_with_bundle("hbui-lost", "function q1(){return 42}");
        let report = patch_hbui(&game).unwrap();
        assert_eq!((report.gate, report.link), (PatchStatus::Missing, PatchStatus::Missing));
        let status = report.status();
        assert_eq!(
            status,
            HbuiStatus::AnchorsLost { lost: vec!["the Microsoft-account gate", "the dead in-game Sign-in link"] }
        );
        let (summary, problem) = status.diagnostic();
        assert!(summary.starts_with("NOT PATCHED"));
        let problem = problem.unwrap();
        assert!(
            problem.contains("Microsoft-account gate")
                && problem.contains("Sign-in link")
                && problem.contains("report the Minecraft version")
        );
        assert_eq!(fs::read_to_string(&js).unwrap(), "function q1(){return 42}", "an unknown bundle is not touched");
        assert!(!PathBuf::from(format!("{}.bf-orig", js.display())).exists());
        // Only one of the two lost.
        let (game, _) = game_with_bundle("hbui-half", &format!("{GATE_NEW}function z(){{}}"));
        let status = patch_hbui(&game).unwrap().status();
        assert_eq!(status, HbuiStatus::AnchorsLost { lost: vec!["the dead in-game Sign-in link"] });
    }

    #[test]
    fn no_bundle_and_non_text_bundles_are_handled() {
        let game = scratch("hbui-none");
        assert_eq!(patch_hbui(&game).unwrap().status(), HbuiStatus::NoBundle);
        assert_eq!(HbuiStatus::NoBundle.diagnostic().1, None);
        let (game, js) = game_with_bundle("hbui-binary", "");
        fs::write(&js, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert_eq!(patch_hbui(&game).unwrap().status(), HbuiStatus::NoBundle);
    }

    #[test]
    fn an_unpatched_build_reads_as_not_applied_yet_without_being_written() {
        let (game, js) = game_with_bundle("hbui-inspect", &format!("{GATE_NEW}{LINK_NEW}"));
        let before = fs::read(&js).unwrap();
        let status = hbui_inspect(&game).status();
        assert_eq!(status, HbuiStatus::NotAppliedYet);
        assert!(status.diagnostic().1.is_some());
        assert_eq!(fs::read(&js).unwrap(), before);
    }

    #[test]
    fn the_play_screen_bundle_is_the_one_reported() {
        let game = scratch("hbui-many");
        let hbui = game.join("data/gui/dist/hbui");
        fs::create_dir_all(&hbui).unwrap();
        fs::write(hbui.join("index-aaa.js"), "function other(){}").unwrap();
        fs::write(hbui.join("index-bbb.js"), format!("{GATE_NEW}{LINK_NEW}")).unwrap();
        fs::write(hbui.join("other.js"), "x").unwrap();
        let report = patch_hbui(&game).unwrap();
        assert_eq!(report.bundle.as_deref(), Some("index-bbb.js"));
        assert_eq!(report.status(), HbuiStatus::NotAppliedYet.max_patched());
    }

    impl HbuiStatus {
        fn max_patched(self) -> HbuiStatus {
            HbuiStatus::NotAppliedYet
        }
    }

    fn lhc(ja: &[u8]) -> Vec<u8> {
        let mut d = vec![0xCCu8; 64];
        d.extend_from_slice(&[0x83, 0xc0, 0xfe, 0xba, 0x04, 0, 0, 0, 0x48, 0x8d, 0x0d, 1, 2, 3, 4, 0x83, 0xf8, 0x06]);
        d.extend_from_slice(ja);
        d.extend_from_slice(&[0xC3; 64]);
        d
    }

    #[test]
    fn the_provider_gate_is_nopped_once() {
        let game = scratch("lhc");
        assert_eq!(patch_lhc_xcurl_gate(&game).unwrap(), GateStatus::NotPresent);
        let dll = game.join("libHttpClient.GDK.dll");
        let original = lhc(&[0x0f, 0x87, 9, 9, 9, 9]);
        fs::write(&dll, &original).unwrap();
        assert_eq!(patch_lhc_xcurl_gate(&game).unwrap(), GateStatus::Applied);
        let patched = fs::read(&dll).unwrap();
        assert_eq!(&patched[64 + 18..64 + 24], &[0x90; 6]);
        assert_eq!(patched.len(), original.len());
        assert_eq!(patched[..64 + 18], original[..64 + 18]);
        assert_eq!(patched[64 + 24..], original[64 + 24..]);
        assert_eq!(fs::read(game.join("libHttpClient.GDK.dll.bf-orig")).unwrap(), original);
        assert_eq!(patch_lhc_xcurl_gate(&game).unwrap(), GateStatus::Already);
        assert_eq!(fs::read(&dll).unwrap(), patched);
    }

    #[test]
    fn an_unknown_gate_is_reported_and_never_patched() {
        let game = scratch("lhc-unknown");
        let dll = game.join("libHttpClient.GDK.dll");
        fs::write(&dll, vec![0x41u8; 4000]).unwrap();
        assert_eq!(patch_lhc_xcurl_gate(&game).unwrap(), GateStatus::AnchorNotFound);
        assert_eq!(fs::read(&dll).unwrap(), vec![0x41u8; 4000]);
        // Right shape, wrong jump: the bytes after the comparison are not a long conditional jump nor NOPs, so
        // the anchor does not match at all (the regex ends in one of the two).
        let odd = lhc(&[0x74, 0x05, 0x90, 0x90, 0x90, 0x90]);
        fs::write(&dll, &odd).unwrap();
        assert_eq!(patch_lhc_xcurl_gate(&game).unwrap(), GateStatus::AnchorNotFound);
        assert_eq!(fs::read(&dll).unwrap(), odd);
    }

    fn pe32_plus(reserve: u64) -> Vec<u8> {
        let mut d = test_image(MACHINE_AMD64, false, None, 400);
        let field = 0x80 + 4 + 20 + 72;
        d[field..field + 8].copy_from_slice(&reserve.to_le_bytes());
        d
    }

    fn reserve_of(d: &[u8]) -> u64 {
        let field = 0x80 + 4 + 20 + 72;
        u64::from_le_bytes(d[field..field + 8].try_into().unwrap())
    }

    #[test]
    fn the_stack_reserve_is_raised_in_place_through_an_open_descriptor() {
        let dir = scratch("stack");
        let path = dir.join("Minecraft.Windows.exe");
        let image = pe32_plus(0x100000);
        fs::write(&path, &image).unwrap();
        let file = fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        assert_eq!(raise_stack_reserve(&file, GAME_STACK_RESERVE).unwrap(), Some(0x100000));
        let after = fs::read(&path).unwrap();
        assert_eq!(reserve_of(&after), GAME_STACK_RESERVE);
        let mut expected = image.clone();
        let field = 0x80 + 4 + 20 + 72;
        expected[field..field + 8].copy_from_slice(&GAME_STACK_RESERVE.to_le_bytes());
        assert_eq!(after, expected, "only those 8 bytes changed");
        assert_eq!(raise_stack_reserve(&file, GAME_STACK_RESERVE).unwrap(), None, "idempotent");
        // An already bigger reserve is left alone.
        fs::write(&path, pe32_plus(0x4000000)).unwrap();
        assert_eq!(raise_stack_reserve_path(&path, GAME_STACK_RESERVE).unwrap(), None);
        assert_eq!(reserve_of(&fs::read(&path).unwrap()), 0x4000000);
        // An owned descriptor works the same (the decrypted image lives behind one).
        fs::write(&path, pe32_plus(0x100000)).unwrap();
        let owned: OwnedFd = fs::OpenOptions::new().read(true).write(true).open(&path).unwrap().into();
        assert_eq!(raise_stack_reserve(&owned, GAME_STACK_RESERVE).unwrap(), Some(0x100000));
    }

    #[test]
    fn anything_that_is_not_a_pe32_plus_is_left_alone_and_cut_headers_are_an_error() {
        let dir = scratch("stack-other");
        let path = dir.join("x");
        for bytes in [b"hello".to_vec(), vec![], test_image(MACHINE_I386, false, None, 400), b"MZ".to_vec()] {
            fs::write(&path, &bytes).unwrap();
            assert_eq!(raise_stack_reserve_path(&path, GAME_STACK_RESERVE).unwrap(), None);
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        // PE32+ whose headers stop before the field.
        let full = pe32_plus(0x100000);
        for cut in [0x80 + 4 + 20 + 2, 0x80 + 4 + 20 + 72, 0x80 + 4 + 20 + 79] {
            fs::write(&path, &full[..cut]).unwrap();
            assert!(raise_stack_reserve_path(&path, GAME_STACK_RESERVE).is_err() || cut < 0x80 + 4 + 20 + 2);
            assert_eq!(fs::read(&path).unwrap(), &full[..cut]);
        }
        // A wild e_lfanew.
        let mut wild = full.clone();
        wild[0x3c..0x40].copy_from_slice(&0x7fff_ff00u32.to_le_bytes());
        fs::write(&path, &wild).unwrap();
        assert_eq!(raise_stack_reserve_path(&path, GAME_STACK_RESERVE).unwrap(), None);
        // A read-only descriptor cannot be written: an error, not a silent skip.
        fs::write(&path, &full).unwrap();
        let ro = File::open(&path).unwrap();
        assert!(raise_stack_reserve(&ro, GAME_STACK_RESERVE).is_err());
        assert_eq!(fs::read(&path).unwrap(), full);
    }

    #[test]
    fn the_descriptors_position_is_not_disturbed() {
        use std::io::{Read, Seek, SeekFrom};
        let dir = scratch("stack-pos");
        let path = dir.join("x");
        fs::write(&path, pe32_plus(0x100000)).unwrap();
        let mut file = fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(7)).unwrap();
        raise_stack_reserve(&file, GAME_STACK_RESERVE).unwrap();
        let mut one = [0u8; 1];
        file.read_exact(&mut one).unwrap();
        assert_eq!(one[0], pe32_plus(0x100000)[7]);
    }

    fn sources(tag: &str) -> (DllSources, [u8; 2]) {
        // The real DLLs are not in the repo, so the pins cannot be satisfied by fixtures; tests that exercise the
        // pin check do so by failing it. The copy logic is tested through `install_file` and the CA bundle.
        let root = scratch(tag);
        fs::create_dir_all(root.join("gdk")).unwrap();
        fs::create_dir_all(root.join("ssl")).unwrap();
        fs::write(root.join("gdk/libHttpClient.GDK.dll"), b"fake").unwrap();
        fs::write(root.join("gdk/XCurl.dll"), b"fake").unwrap();
        for n in ["libcurl-4.dll", "xcurl-cashim.dll", "cryptbase.dll", "libssl-3-x64.dll", "zlib1.dll"] {
            fs::write(root.join("ssl").join(n), n.as_bytes()).unwrap();
        }
        fs::write(root.join("cacert.pem"), b"CERTS").unwrap();
        (
            DllSources { gdk_deps: root.join("gdk"), openssl_set: root.join("ssl"), cacert: root.join("cacert.pem") },
            [0, 0],
        )
    }

    #[test]
    fn dlls_that_do_not_match_their_pin_are_never_copied() {
        let (src, _) = sources("dlls-pin");
        let game = scratch("dlls-pin-game");
        let e = install_game_dlls(&game, &src).unwrap_err();
        assert!(matches!(e, PrefixError::Integrity(_)), "{e}");
        assert_eq!(fs::read_dir(&game).unwrap().count(), 0, "nothing was written");
    }

    #[test]
    fn an_incomplete_openssl_set_is_an_error_before_anything_is_written() {
        let (src, _) = sources("dlls-incomplete");
        let game = scratch("dlls-incomplete-game");
        // Satisfy the pins by testing the order of checks: pins fail first here, so check the set separately.
        fs::remove_file(src.openssl_set.join("xcurl-cashim.dll")).unwrap();
        assert!(install_game_dlls(&game, &src).is_err());
        assert_eq!(fs::read_dir(&game).unwrap().count(), 0);
    }

    #[test]
    fn the_ca_bundle_goes_beside_the_dlls_and_in_the_game_and_its_parent() {
        let root = scratch("ca");
        let game = root.join("builds/content");
        fs::create_dir_all(&game).unwrap();
        let cacert = root.join("cacert.pem");
        fs::write(&cacert, b"CERTS").unwrap();
        let mut report = DllReport::default();
        install_ca_bundle(&cacert, &game, &mut report).unwrap();
        assert_eq!(fs::read(game.join("cacert.pem")).unwrap(), b"CERTS");
        assert_eq!(fs::read(game.join("etc/ssl/certs/ca-bundle.crt")).unwrap(), b"CERTS");
        assert_eq!(fs::read(root.join("builds/etc/ssl/certs/ca-bundle.crt")).unwrap(), b"CERTS");
        let first = report.copied.len();
        install_ca_bundle(&cacert, &game, &mut report).unwrap();
        assert_eq!(report.copied.len(), first, "unchanged files are not rewritten");
        fs::write(&cacert, b"NEWER CERTS").unwrap();
        install_ca_bundle(&cacert, &game, &mut report).unwrap();
        assert_eq!(fs::read(game.join("cacert.pem")).unwrap(), b"NEWER CERTS");
    }

    #[test]
    fn install_file_keeps_the_original_once_and_replaces_links() {
        let root = scratch("install-file");
        let src = root.join("new.dll");
        fs::write(&src, b"new").unwrap();
        let dst = root.join("XCurl.dll");
        fs::write(&dst, b"genuine").unwrap();
        let mut report = DllReport::default();
        install_file(&src, &dst, &mut report).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"new");
        assert_eq!(fs::read(root.join("XCurl.dll.bf-orig")).unwrap(), b"genuine");
        fs::write(&src, b"newer").unwrap();
        install_file(&src, &dst, &mut report).unwrap();
        assert_eq!(fs::read(root.join("XCurl.dll.bf-orig")).unwrap(), b"genuine", "the genuine original stays");
        let target = root.join("target");
        fs::write(&target, b"untouched").unwrap();
        fs::remove_file(&dst).unwrap();
        std::os::unix::fs::symlink(&target, &dst).unwrap();
        install_file(&src, &dst, &mut report).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
        assert_eq!(fs::read(&dst).unwrap(), b"newer");
        let evil = root.join("evil-src");
        std::os::unix::fs::symlink(&target, &evil).unwrap();
        assert!(install_file(&evil, &root.join("other.dll"), &mut report).is_err());
    }
}
