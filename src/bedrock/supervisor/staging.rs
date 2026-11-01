//! The decrypted game image's short life in RAM: where it is staged, the `WINE_DLL_FILE_MAP` that hands it to the
//! loader, and cleaning up after it — only ever our own, and only of sessions that are over.
//!
//! The plan (PORT-PLAN.md 1.7 and 4): plaintext images never touch durable storage. They are written into a private
//! directory on a RAM file system, named in the map as `<absolute path>:<NT name>` entries joined by `|`; Wine's
//! patched loader opens each file and unlinks it at once, so the file is gone milliseconds after the game starts.
//! Ported from BedrockOnLinux `bol/xodus.py` (`staging_dir`, the generated wrapper's `stage`), whose per-file copy
//! is replaced by writing directly into the owned directory.
//!
//! Rules that keep this safe:
//! - Directory `0700`, files `0600`, under `$XDG_RUNTIME_DIR/blockforge-bedrock/<session>/` when that is a RAM file
//!   system the container can see, else `/dev/shm/blockforge-bedrock-<uid>/<session>/`. **No fallback to disk**: with
//!   no RAM file system, staging is refused.
//! - A path in the map can't hold `:` or `|` (the map's separators), nor line breaks or NULs.
//! - A directory is swept only when it is a session of ours (a UUID name, our owner record inside) whose owner
//!   process is gone. Another launcher's files, or anything else in the same place, are never touched.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const NAMESPACE: &str = "blockforge-bedrock";
const OWNER_FILE: &str = ".blockforge-owner.json";

/// Who made a session directory: the process that owns it, with its start time, so a recycled number is no owner.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Owner {
    pub pid: u32,
    pub start: Option<u64>,
    pub boot_id: String,
    pub session: String,
}

impl Owner {
    pub fn this_process(session: &str) -> Owner {
        Owner {
            pid: std::process::id(),
            start: process_start(std::process::id()),
            boot_id: boot_id(),
            session: session.to_string(),
        }
    }

    /// Is the owner still running? Not if it's another boot, if the number is free, or if the number now has another
    /// start time.
    pub fn is_alive(&self) -> bool {
        let boot = boot_id();
        if !boot.is_empty() && !self.boot_id.is_empty() && boot != self.boot_id {
            return false;
        }
        match (process_start(self.pid), self.start) {
            (None, _) => false,
            (Some(now), Some(then)) => now == then,
            // an owner recorded without a start time can only be judged by its number
            (Some(_), None) => true,
        }
    }
}

pub fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map(|s| s.trim().to_string()).unwrap_or_default()
}

pub fn process_start(pid: u32) -> Option<u64> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    super::procs::parse_stat(&text).map(|(_, _, _, start)| start)
}

/// A mount from `/proc/self/mountinfo`: where, and what kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mount {
    pub point: PathBuf,
    pub fstype: String,
}

/// Undo the `\040` style octal escapes the kernel uses for spaces and other awkward characters in mount paths.
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 4 <= b.len() && s.is_char_boundary(i + 1) && s.is_char_boundary(i + 4) {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 4], 8) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            // "36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue"
            let (head, tail) = line.split_once(" - ")?;
            let point = head.split(' ').nth(4)?;
            let fstype = tail.split(' ').next()?;
            Some(Mount { point: PathBuf::from(unescape_mount(point)), fstype: fstype.to_string() })
        })
        .collect()
}

/// The file system type of the mount `path` is on: the mount with the longest matching point.
pub fn fstype_of(mounts: &[Mount], path: &Path) -> Option<String> {
    mounts
        .iter()
        .filter(|m| path.starts_with(&m.point))
        .max_by_key(|m| m.point.components().count())
        .map(|m| m.fstype.clone())
}

/// Where staging may go: a candidate directory (its parent must exist) with the reason it was chosen.
#[derive(Clone, Debug)]
pub struct Choice {
    pub root: PathBuf,
}

/// Pick the RAM-backed root. `xdg_runtime` is `$XDG_RUNTIME_DIR`, `shm` is `/dev/shm`, `uid` the user. A candidate
/// counts only when its file system is `tmpfs`/`ramfs` (not merely named like one). `None` means: no staging.
pub fn choose_root(mounts: &[Mount], xdg_runtime: Option<&Path>, shm: &Path, uid: u32) -> Option<Choice> {
    let ram = |p: &Path| matches!(fstype_of(mounts, p).as_deref(), Some("tmpfs") | Some("ramfs"));
    if let Some(x) = xdg_runtime.filter(|x| x.is_absolute() && x.is_dir() && ram(x)) {
        return Some(Choice { root: x.join(NAMESPACE) });
    }
    if shm.is_dir() && ram(shm) {
        return Some(Choice { root: shm.join(format!("{NAMESPACE}-{uid}")) });
    }
    None
}

/// The root on this computer, or `None` when it has no usable RAM file system.
#[cfg(target_os = "linux")]
pub fn system_root() -> Option<Choice> {
    let mounts = parse_mountinfo(&std::fs::read_to_string("/proc/self/mountinfo").ok()?);
    let xdg = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    choose_root(&mounts, xdg.as_deref(), Path::new("/dev/shm"), super::procs::current_uid())
}

/// A name that may be used in the map: not empty, no separators, no `:` or `|`, no control characters.
pub fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A private directory for one session's staged files. Removes everything it staged when dropped.
#[derive(Debug)]
pub struct StagingDir {
    dir: PathBuf,
    /// false once the folder was handed to a supervisor, which removes it
    armed: bool,
}

#[derive(Debug)]
pub enum StageError {
    /// no RAM file system to stage in (we never write plaintext to disk)
    NoRam,
    NotEnoughRoom {
        need: u64,
        have: u64,
    },
    BadName(String),
    BadPath(String),
    Io(std::io::Error),
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageError::NoRam => f.write_str("this computer has no RAM file system (tmpfs) to hold the game's decrypted files; BlockForge never writes them to disk"),
            StageError::NotEnoughRoom { need, have } => write!(
                f,
                "not enough free memory for the game's decrypted files: needs {} MiB in the RAM file system, {} MiB free",
                need.div_ceil(1 << 20),
                have >> 20
            ),
            StageError::BadName(n) => write!(f, "can't stage a file called {n:?}"),
            StageError::BadPath(p) => write!(f, "{p:?} can't go in the game's file map (no ':' or '|' allowed)"),
            StageError::Io(e) => write!(f, "staging the game's files: {e}"),
        }
    }
}

impl std::error::Error for StageError {}

impl From<std::io::Error> for StageError {
    fn from(e: std::io::Error) -> Self {
        StageError::Io(e)
    }
}

#[cfg(unix)]
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    // a directory that was there already may have had other modes
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

impl StagingDir {
    /// Make `<root>/<session>` (0700) and record who owns it. `need` is the room wanted, checked against what the RAM
    /// file system has free.
    pub fn create(root: &Path, session: &str, need: u64) -> Result<StagingDir, StageError> {
        if !safe_component(session) {
            return Err(StageError::BadName(session.to_string()));
        }
        let dir = root.join(session);
        #[cfg(unix)]
        {
            private_dir(root)?;
            if need > 0 {
                #[cfg(target_os = "linux")]
                if let Some((free, _)) = super::sys::space(root) {
                    if free < need {
                        return Err(StageError::NotEnoughRoom { need, have: free });
                    }
                }
            }
            private_dir(&dir)?;
        }
        let owner = Owner::this_process(session);
        write_private(&dir.join(OWNER_FILE), &serde_json::to_vec(&owner).unwrap_or_default())?;
        Ok(StagingDir { dir, armed: true })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// The path of a file to stage called `name`, checked to be safe in the map.
    #[cfg(test)]
    pub fn file_path(&self, name: &str) -> Result<PathBuf, StageError> {
        if !safe_component(name) || name == OWNER_FILE {
            return Err(StageError::BadName(name.to_string()));
        }
        let p = self.dir.join(name);
        check_map_path(&p)?;
        Ok(p)
    }

    /// Write `bytes` as a staged file (0600, not shared with anyone) and return its path.
    #[cfg(test)]
    pub fn stage_bytes(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, StageError> {
        let p = self.file_path(name)?;
        write_private(&p, bytes)?;
        Ok(p)
    }

    /// Remove the staged files but keep the directory and the owner record (the loader unlinks its own copies as it
    /// opens them; whatever is still here after the game is up was never opened).
    #[cfg(test)]
    pub fn release_files(&self) -> usize {
        release_files_in(&self.dir)
    }

    /// Files left (besides the owner record).
    #[cfg(test)]
    pub fn remaining(&self) -> usize {
        std::fs::read_dir(&self.dir).map(|rd| rd.flatten().filter(|e| e.file_name() != OWNER_FILE).count()).unwrap_or(0)
    }

    /// Give the folder to the session supervisor (which names it in its plan and removes it at the end): this value
    /// no longer removes it. Returns the folder.
    pub fn hand_over(mut self) -> PathBuf {
        self.armed = false;
        self.dir.clone()
    }
}

/// The owner record of a session folder, if it has a readable one.
pub fn read_owner(dir: &Path) -> Option<Owner> {
    std::fs::read(dir.join(OWNER_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn write_owner(dir: &Path, owner: &Owner) {
    let _ = write_private(&dir.join(OWNER_FILE), &serde_json::to_vec(owner).unwrap_or_default());
}

/// Remove everything in `dir` but the owner record. Returns how many entries went.
pub fn release_files_in(dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name() == OWNER_FILE {
                continue;
            }
            let p = e.path();
            let is_dir = e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false);
            let removed = if is_dir { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            if removed.is_ok() {
                n += 1;
            }
        }
    }
    n
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // only ever our own directory, and std's remove_dir_all doesn't follow links out of it
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(bytes)?;
    f.flush()
}

/// A path that can be in a map entry.
pub fn check_map_path(p: &Path) -> Result<(), StageError> {
    let s = p.to_str().ok_or_else(|| StageError::BadPath(p.display().to_string()))?;
    if !p.is_absolute() || s.chars().any(|c| matches!(c, ':' | '|' | '\0' | '\n' | '\r')) {
        return Err(StageError::BadPath(s.to_string()));
    }
    Ok(())
}

/// `WINE_DLL_FILE_MAP`: `<absolute path>:<NT name>` entries joined by `|`. A leading `/` is what makes Wine's loader
/// treat the first part as a file to open and unlink (and not as an inherited descriptor number).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileMap {
    entries: Vec<(PathBuf, String)>,
}

impl FileMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// `nt_name` is the NT path of the image the game's loader asks for, like `\??\Z:\games\mc\Minecraft.Windows.exe`.
    pub fn add(&mut self, staged: &Path, nt_name: &str) -> Result<(), StageError> {
        check_map_path(staged)?;
        if nt_name.is_empty() || nt_name.contains(['|', '\0', '\n', '\r']) {
            return Err(StageError::BadPath(nt_name.to_string()));
        }
        self.entries.push((staged.to_path_buf(), nt_name.to_string()));
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn value(&self) -> String {
        self.entries.iter().map(|(p, n)| format!("{}:{}", p.display(), n)).collect::<Vec<_>>().join("|")
    }

    /// Read a map back the way Wine's loader does (split at `|`, then at the first `:`).
    #[cfg(test)]
    pub fn parse(value: &str) -> Vec<(String, String)> {
        value
            .split('|')
            .filter_map(|e| e.split_once(':'))
            .filter(|(p, n)| !p.is_empty() && !n.is_empty())
            .map(|(p, n)| (p.to_string(), n.to_string()))
            .collect()
    }
}

/// What a sweep did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SweepReport {
    pub removed: Vec<PathBuf>,
    pub kept_alive: Vec<PathBuf>,
    pub not_ours: Vec<PathBuf>,
}

/// Remove the leftovers of BlockForge sessions that are over: directories in `root` named like a session whose owner
/// record says the process is gone. Live sessions, other launchers' files and anything unrecognised stay.
pub fn sweep_stale(root: &Path) -> SweepReport {
    let mut report = SweepReport::default();
    let Ok(rd) = std::fs::read_dir(root) else { return report };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        // a real directory only: a link in here is not ours to follow
        let is_dir = e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false);
        if !is_dir || uuid::Uuid::parse_str(&name).is_err() {
            report.not_ours.push(path);
            continue;
        }
        let owner: Option<Owner> =
            std::fs::read(path.join(OWNER_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok());
        match owner {
            Some(o) if o.session == name => {
                if o.is_alive() {
                    report.kept_alive.push(path);
                } else if std::fs::remove_dir_all(&path).is_ok() {
                    report.removed.push(path);
                }
            }
            // no record or a record of another session: not something we made
            _ => report.not_ours.push(path),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    const MOUNTS: &str = "\
22 28 0:21 / /proc rw,nosuid,nodev,noexec,relatime shared:13 - proc proc rw
30 22 0:26 / /dev/shm rw,nosuid,nodev shared:4 - tmpfs tmpfs rw,seclabel
41 28 0:35 / /run/user/1000 rw,nosuid,nodev,relatime shared:90 - tmpfs tmpfs rw,size=3257856k,mode=700,uid=1000
50 28 8:2 / / rw,relatime - ext4 /dev/sda2 rw
51 50 8:5 /home /home rw,relatime - btrfs /dev/sda5 rw
60 28 0:50 / /mnt/with\\040space rw - tmpfs tmpfs rw
";

    #[test]
    fn mountinfo_is_parsed_with_escapes() {
        let m = parse_mountinfo(MOUNTS);
        assert_eq!(fstype_of(&m, Path::new("/dev/shm/x")).as_deref(), Some("tmpfs"));
        assert_eq!(fstype_of(&m, Path::new("/home/me")).as_deref(), Some("btrfs"));
        assert_eq!(fstype_of(&m, Path::new("/etc")).as_deref(), Some("ext4"));
        assert_eq!(fstype_of(&m, Path::new("/mnt/with space/a")).as_deref(), Some("tmpfs"));
    }

    #[test]
    fn staging_is_only_ever_in_ram() {
        let m = parse_mountinfo(MOUNTS);
        let xdg = scratch("stage-xdg");
        // the runtime dir is a real directory but not in the mount table as RAM: /dev/shm is used instead
        let c = choose_root(&m, Some(&xdg), Path::new("/dev/shm"), 1000).unwrap();
        assert_eq!(c.root, Path::new("/dev/shm/blockforge-bedrock-1000"));
        // no RAM file system at all: no staging, no fall back to a disk
        let disk_only = parse_mountinfo("50 28 8:2 / / rw - ext4 /dev/sda2 rw\n");
        assert!(choose_root(&disk_only, Some(&xdg), Path::new("/dev/shm"), 1000).is_none());
        // a runtime dir that is RAM is preferred
        let m2 = parse_mountinfo(&format!("{MOUNTS}70 28 0:70 / {} rw - tmpfs tmpfs rw\n", xdg.display()));
        assert_eq!(
            choose_root(&m2, Some(&xdg), Path::new("/dev/shm"), 1000).unwrap().root,
            xdg.join("blockforge-bedrock")
        );
        std::fs::remove_dir_all(xdg).unwrap();
    }

    #[test]
    fn the_map_has_wines_format_and_refuses_separators() {
        let mut map = FileMap::new();
        map.add(Path::new("/dev/shm/blockforge-bedrock-1000/aaaa/exe.img"), r"\??\Z:\games\mc\Minecraft.Windows.exe")
            .unwrap();
        map.add(Path::new("/dev/shm/blockforge-bedrock-1000/aaaa/dll.img"), r"\??\Z:\games\mc\Other.dll").unwrap();
        assert_eq!(
            map.value(),
            r"/dev/shm/blockforge-bedrock-1000/aaaa/exe.img:\??\Z:\games\mc\Minecraft.Windows.exe|/dev/shm/blockforge-bedrock-1000/aaaa/dll.img:\??\Z:\games\mc\Other.dll"
        );
        // read back as the loader does: split at '|', then at the first ':' (the NT name has its own ':')
        let back = FileMap::parse(&map.value());
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].1, r"\??\Z:\games\mc\Minecraft.Windows.exe");
        assert!(back[0].0.starts_with('/'), "a leading slash is how the loader tells a path from a descriptor");
        for bad in ["/dev/shm/a:b/x", "/dev/shm/a|b/x", "relative/x", "/dev/shm/a\nb"] {
            assert!(matches!(map.clone().add(Path::new(bad), r"\??\Z:\x.exe"), Err(StageError::BadPath(_))), "{bad}");
        }
        assert!(map.clone().add(Path::new("/dev/shm/ok"), "a|b").is_err());
    }

    #[test]
    fn a_staging_directory_is_private_and_removes_its_files() {
        let root = scratch("stage-dir");
        let session = uuid::Uuid::new_v4().to_string();
        let staging = StagingDir::create(&root.join("ns"), &session, 0).unwrap();
        let file = staging.stage_bytes("exe.img", b"MZ plaintext").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(staging.path()).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        for bad in ["a:b", "a|b", "../x", "", ".hidden", OWNER_FILE, "a/b"] {
            assert!(staging.file_path(bad).is_err(), "{bad}");
        }
        // the loader opened and unlinked it: nothing left, and releasing is fine
        std::fs::remove_file(&file).unwrap();
        assert_eq!(staging.remaining(), 0);
        staging.stage_bytes("never-opened.img", b"x").unwrap();
        assert_eq!(staging.remaining(), 1);
        assert_eq!(staging.release_files(), 1);
        assert!(staging.path().join(OWNER_FILE).exists());
        staging.stage_bytes("again.img", b"x").unwrap();
        let dir = staging.path().to_path_buf();
        drop(staging);
        assert!(!dir.exists(), "dropping removes the directory and what's in it");
        assert!(StagingDir::create(&root, "not a session/..", 0).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn room_is_checked_in_the_ram_file_system() {
        let root = scratch("stage-room");
        let r = StagingDir::create(&root.join("ns"), &uuid::Uuid::new_v4().to_string(), u64::MAX / 2);
        assert!(matches!(r, Err(StageError::NotEnoughRoom { .. })), "{r:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_dead_sessions_of_ours_are_swept() {
        let root = scratch("stage-sweep");
        let ns = root.join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        // a live session: this process
        let live_id = uuid::Uuid::new_v4().to_string();
        let live = StagingDir::create(&ns, &live_id, 0).unwrap();
        live.stage_bytes("img", b"x").unwrap();
        // a dead one: the owner is a process that has ended
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let (dead_pid, dead_start) = (child.id(), process_start(child.id()));
        child.kill().unwrap();
        child.wait().unwrap();
        let dead_id = uuid::Uuid::new_v4().to_string();
        let dead_dir = ns.join(&dead_id);
        std::fs::create_dir_all(&dead_dir).unwrap();
        std::fs::write(dead_dir.join("img"), b"plaintext").unwrap();
        let dead_owner = Owner { pid: dead_pid, start: dead_start, boot_id: boot_id(), session: dead_id.clone() };
        std::fs::write(dead_dir.join(OWNER_FILE), serde_json::to_vec(&dead_owner).unwrap()).unwrap();
        // a recycled number: alive process, but a different start time
        let recycled_id = uuid::Uuid::new_v4().to_string();
        let recycled = ns.join(&recycled_id);
        std::fs::create_dir_all(&recycled).unwrap();
        let o = Owner { pid: std::process::id(), start: Some(1), boot_id: boot_id(), session: recycled_id.clone() };
        std::fs::write(recycled.join(OWNER_FILE), serde_json::to_vec(&o).unwrap()).unwrap();
        // not ours: a UUID name without our record, a record of another session, a plain name, a file, a link
        let alien = ns.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&alien).unwrap();
        std::fs::write(alien.join("theirs.dat"), b"other launcher").unwrap();
        let mislabelled = ns.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&mislabelled).unwrap();
        std::fs::write(mislabelled.join(OWNER_FILE), serde_json::to_vec(&dead_owner).unwrap()).unwrap();
        let named = ns.join("steam-shader-cache");
        std::fs::create_dir_all(&named).unwrap();
        std::fs::write(ns.join("file.txt"), b"x").unwrap();
        let outside = scratch("stage-sweep-outside");
        std::fs::write(outside.join("precious"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, ns.join(uuid::Uuid::new_v4().to_string())).unwrap();

        let report = sweep_stale(&ns);
        assert_eq!(report.removed.len(), 2, "{report:?}");
        assert!(report.removed.contains(&dead_dir) && report.removed.contains(&recycled));
        assert_eq!(report.kept_alive, vec![live.path().to_path_buf()]);
        assert!(live.path().join("img").exists());
        assert!(alien.join("theirs.dat").exists() && mislabelled.exists() && named.exists());
        assert!(ns.join("file.txt").exists() && outside.join("precious").exists());
        assert!(!dead_dir.exists());
        drop(live);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
}
