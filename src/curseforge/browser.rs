//! Files the author does not let launchers download (CurseForge gives them no `downloadUrl`). The player has to
//! fetch them in a browser, and the launcher watches the downloads folders until the file turns up. This is the
//! Java edition's `BrowserDownloadDialog`, `RestrictedFileMatcher` and the single-file loop in
//! `Instance.addFileFromCurseForge`, as logic that a window (or the command line) drives through a [`BrowserHub`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::paths::Dirs;

/// One file the player has to download by hand.
#[derive(Clone, Debug)]
pub struct BrowserRequest {
    /// What to call it on screen (the mod's name).
    pub label: String,
    /// The name the file should have once downloaded.
    pub file_name: String,
    /// The CurseForge page that starts the download.
    pub url: String,
    /// Its size in bytes, 0 when unknown.
    pub size: u64,
    pub md5: Option<String>,
    pub sha1: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemState {
    Waiting,
    /// Found, checked and moved to this path.
    Found(PathBuf),
    /// The player chose to carry on without it.
    Skipped,
}

#[derive(Clone, Debug)]
pub struct BrowserItem {
    pub id: u64,
    pub request: BrowserRequest,
    pub state: ItemState,
}

struct Inner {
    items: Vec<BrowserItem>,
    next_id: u64,
    cancelled: bool,
}

/// The shared state between an install running in the background and the dialog that shows it.
pub struct BrowserHub {
    inner: Mutex<Inner>,
    /// Where finished downloads are collected (`<data>/downloads`, the Java edition's `FileSystem.DOWNLOADS`).
    pub collect_dir: PathBuf,
    /// The folders watched: the collect folder and the player's own Downloads folder.
    pub watch_dirs: Vec<PathBuf>,
}

impl BrowserHub {
    pub fn new(dirs: &Dirs, custom_downloads: Option<&Path>) -> Arc<BrowserHub> {
        let collect_dir = dirs.base.join("downloads");
        let user = super::user_downloads_dir(custom_downloads);
        let mut watch_dirs = vec![collect_dir.clone()];
        if user != collect_dir {
            watch_dirs.push(user);
        }
        Arc::new(BrowserHub {
            inner: Mutex::new(Inner { items: Vec::new(), next_id: 1, cancelled: false }),
            collect_dir,
            watch_dirs,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What the dialog shows right now.
    pub fn items(&self) -> Vec<BrowserItem> {
        self.lock().items.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// Whether any file is still waited for.
    pub fn waiting(&self) -> bool {
        self.lock().items.iter().any(|i| i.state == ItemState::Waiting)
    }

    /// Whether the dialog has anything to show (so the window knows to open it).
    pub fn has_items(&self) -> bool {
        !self.lock().items.is_empty()
    }

    pub fn skip(&self, id: u64) {
        for i in self.lock().items.iter_mut().filter(|i| i.id == id && i.state == ItemState::Waiting) {
            i.state = ItemState::Skipped;
        }
    }

    /// "Skip Remaining": carry on without the files that have not arrived.
    pub fn skip_remaining(&self) {
        for i in self.lock().items.iter_mut().filter(|i| i.state == ItemState::Waiting) {
            i.state = ItemState::Skipped;
        }
    }

    /// "Cancel Install": stop the whole install.
    pub fn cancel(&self) {
        self.lock().cancelled = true;
    }

    /// The pages of the files still waited for, for "Open All In Browser".
    pub fn waiting_urls(&self) -> Vec<String> {
        self.lock().items.iter().filter(|i| i.state == ItemState::Waiting).map(|i| i.request.url.clone()).collect()
    }

    fn add(&self, requests: Vec<BrowserRequest>) -> Vec<u64> {
        let mut g = self.lock();
        let mut ids = Vec::new();
        for r in requests {
            let id = g.next_id;
            g.next_id += 1;
            g.items.push(BrowserItem { id, request: r, state: ItemState::Waiting });
            ids.push(id);
        }
        ids
    }

    fn set(&self, id: u64, state: ItemState) {
        if let Some(i) = self.lock().items.iter_mut().find(|i| i.id == id) {
            i.state = state;
        }
    }

    fn state_of(&self, id: u64) -> Option<ItemState> {
        self.lock().items.iter().find(|i| i.id == id).map(|i| i.state.clone())
    }

    /// Forget finished files so the hub can be used for the next batch.
    pub fn clear(&self) {
        let mut g = self.lock();
        g.items.clear();
        g.cancelled = false;
    }
}

// ---------------------------------------------------------------------------------------------------------
// recognising the downloaded file

const PARTIAL: [&str; 6] = [".crdownload", ".part", ".tmp", ".download", ".opdownload", ".partial"];

/// The files in a folder that are finished downloads (not a browser's half-written file).
pub fn finished_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && !is_partial(p)).collect()
}

fn is_partial(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    PARTIAL.iter().any(|s| name.ends_with(s))
}

fn extension(name: &str) -> &str {
    name.rfind('.').map(|i| &name[i..]).unwrap_or("")
}

/// Equal ignoring case, with spaces turned to `+` or `_`, with a browser's ` (1)` / `-1` / `_1` added, or with
/// `.zip` added to the end.
pub fn same_name(actual: &str, expected: &str) -> bool {
    let a = actual.to_lowercase();
    let e = expected.to_lowercase();
    if a == e || a == e.replace(' ', "+") || a == e.replace(' ', "_") || a == format!("{e}.zip") {
        return true;
    }
    let ext = extension(&e);
    let stem = &e[..e.len() - ext.len()];
    if !a.starts_with(stem) || !a.ends_with(ext) || a.len() < stem.len() + ext.len() {
        return false;
    }
    let mid = &a[stem.len()..a.len() - ext.len()];
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(n) = mid.strip_prefix(" (").and_then(|m| m.strip_suffix(')')) {
        return digits(n);
    }
    mid.strip_prefix('-').or_else(|| mid.strip_prefix('_')).is_some_and(digits)
}

fn size_matches(p: &Path, size: u64) -> bool {
    size == 0 || std::fs::metadata(p).map(|m| m.len() == size).unwrap_or(false)
}

/// Find the downloaded copy of a file among `files`: first a file with the right name (or the name a browser
/// would give it), then any file of exactly the right size and the same ending. A candidate only counts when
/// `verified` accepts its contents.
pub fn find_download(
    files: &[PathBuf],
    file_name: &str,
    size: u64,
    verified: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if same_name(&name, file_name) && size_matches(f, size) && verified(f) {
            return Some(f.clone());
        }
    }
    if size > 0 {
        let want = extension(&file_name.to_lowercase()).to_string();
        for f in files {
            let name = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if size_matches(f, size) && extension(&name) == want && verified(f) {
                return Some(f.clone());
            }
        }
    }
    None
}

/// Whether a file's contents are the ones expected, by the md5 or sha1 CurseForge lists (no hash: accepted).
pub fn contents_match(path: &Path, md5: Option<&str>, sha1: Option<&str>) -> bool {
    let Ok(data) = std::fs::read(path) else { return false };
    if let Some(m) = md5.filter(|m| !m.is_empty()) {
        use md5::{Digest, Md5};
        let mut h = Md5::new();
        h.update(&data);
        return hex::encode(h.finalize()).eq_ignore_ascii_case(m);
    }
    if let Some(s) = sha1.filter(|s| !s.is_empty()) {
        return crate::http::sha1_hex(&data).eq_ignore_ascii_case(s);
    }
    true
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if to.exists() {
        std::fs::remove_file(to)?;
    }
    if std::fs::rename(from, to).is_err() {
        // another disk: copy, then remove the original
        std::fs::copy(from, to)?;
        let _ = std::fs::remove_file(from);
    }
    Ok(())
}

/// Look once for every waiting item; items found are moved into the collect folder.
pub fn scan_once(hub: &BrowserHub) {
    let waiting: Vec<BrowserItem> = hub.items().into_iter().filter(|i| i.state == ItemState::Waiting).collect();
    if waiting.is_empty() {
        return;
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for d in &hub.watch_dirs {
        files.extend(finished_files(d));
    }
    for item in waiting {
        let r = &item.request;
        let verify = |p: &Path| contents_match(p, r.md5.as_deref(), r.sha1.as_deref());
        // already in place under its own name
        let target = hub.collect_dir.join(&r.file_name);
        if target.is_file() && size_matches(&target, r.size) && verify(&target) {
            hub.set(item.id, ItemState::Found(target));
            continue;
        }
        if let Some(found) = find_download(&files, &r.file_name, r.size, &verify) {
            if found == target || move_file(&found, &target).is_ok() {
                hub.set(item.id, ItemState::Found(target));
            }
        }
    }
}

/// Wait for every file in `requests`. The browser is opened for each of them first when `open_browser` is set
/// (the single-mod flow); a modpack leaves that to the dialog's buttons. The answer has one entry per request:
/// the file's place in the collect folder, or `None` when the player skipped it. Fails when the install is
/// cancelled, or nothing happens for `give_up_after`.
pub async fn fetch_by_hand(
    hub: &Arc<BrowserHub>,
    requests: Vec<BrowserRequest>,
    open_browser: bool,
    give_up_after: Duration,
) -> Result<Vec<Option<PathBuf>>> {
    if open_browser {
        for r in &requests {
            let _ = open::that(&r.url);
        }
    }
    let ids = hub.add(requests);
    let started = Instant::now();
    loop {
        if hub.is_cancelled() {
            bail!("Installation cancelled from the browser downloads window");
        }
        let h = hub.clone();
        let _ = tokio::task::spawn_blocking(move || scan_once(&h)).await;
        if !ids.iter().any(|id| hub.state_of(*id) == Some(ItemState::Waiting)) {
            break;
        }
        if started.elapsed() > give_up_after {
            bail!("Waited too long for the files to be downloaded in the browser");
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    Ok(ids
        .iter()
        .map(|id| match hub.state_of(*id) {
            Some(ItemState::Found(p)) => Some(p),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsers_rename_files_in_known_ways() {
        assert!(same_name("My Mod-1.0.jar", "my mod-1.0.jar"));
        assert!(same_name("My+Mod-1.0.jar", "My Mod-1.0.jar"));
        assert!(same_name("My_Mod-1.0.jar", "My Mod-1.0.jar"));
        assert!(same_name("mod-1.0 (1).jar", "mod-1.0.jar"));
        assert!(same_name("mod-1.0-2.jar", "mod-1.0.jar"));
        assert!(same_name("mod-1.0.jar.zip", "mod-1.0.jar"));
        assert!(!same_name("other-1.0.jar", "mod-1.0.jar"));
        assert!(!same_name("mod-1.0 (x).jar", "mod-1.0.jar"));
    }

    #[test]
    fn partial_downloads_are_ignored() {
        let dir = tempdir("partial");
        std::fs::write(dir.join("a.jar"), b"x").unwrap();
        std::fs::write(dir.join("b.jar.crdownload"), b"x").unwrap();
        std::fs::write(dir.join("c.part"), b"x").unwrap();
        let names: Vec<String> =
            finished_files(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, vec!["a.jar"]);
    }

    #[test]
    fn finds_by_name_then_by_size_and_checks_contents() {
        let dir = tempdir("find");
        std::fs::write(dir.join("Some Mod 1.0.jar"), b"hello").unwrap();
        std::fs::write(dir.join("download (3).jar"), b"world!").unwrap();
        let files = finished_files(&dir);
        let all = |_: &Path| true;
        assert!(find_download(&files, "some mod 1.0.jar", 5, &all).is_some());
        // renamed by the browser, but the size and ending are right
        let by_size = find_download(&files, "mod-9.jar", 6, &all).unwrap();
        assert!(by_size.ends_with("download (3).jar"));
        // wrong contents are refused
        assert!(find_download(&files, "mod-9.jar", 6, &|_| false).is_none());
        assert!(find_download(&files, "mod-9.jar", 7, &all).is_none());
    }

    #[test]
    fn hash_check_uses_md5_then_sha1() {
        let dir = tempdir("hash");
        let p = dir.join("f.jar");
        std::fs::write(&p, b"hello").unwrap();
        assert!(contents_match(&p, Some("5d41402abc4b2a76b9719d911017c592"), None));
        assert!(!contents_match(
            &p,
            Some("00000000000000000000000000000000"),
            Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")
        ));
        assert!(contents_match(&p, None, Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")));
        assert!(contents_match(&p, None, None));
    }

    #[tokio::test]
    async fn a_download_appearing_later_is_picked_up_and_skipping_works() {
        let base = tempdir("hub");
        let user = tempdir("hubuser");
        let dirs = Dirs::at(&base);
        let hub = BrowserHub::new(&dirs, Some(&user));
        let req = |name: &str, body: &[u8]| BrowserRequest {
            label: name.into(),
            file_name: name.into(),
            url: "https://example.invalid/".into(),
            size: body.len() as u64,
            md5: None,
            sha1: Some(crate::http::sha1_hex(body)),
        };
        let reqs = vec![req("one.jar", b"first"), req("two.jar", b"second")];
        let h2 = hub.clone();
        let u2 = user.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            std::fs::write(u2.join("one (1).jar"), b"first").unwrap();
            // two.jar never arrives; the player skips it
            tokio::time::sleep(Duration::from_millis(2500)).await;
            h2.skip_remaining();
        });
        let got = fetch_by_hand(&hub, reqs, false, Duration::from_secs(30)).await.unwrap();
        writer.await.unwrap();
        assert_eq!(got[0].as_deref(), Some(dirs.base.join("downloads").join("one.jar").as_path()));
        assert!(got[0].as_ref().unwrap().is_file());
        assert!(got[1].is_none());
    }

    #[tokio::test]
    async fn cancelling_stops_the_wait() {
        let base = tempdir("cancel");
        let hub = BrowserHub::new(&Dirs::at(&base), Some(&base));
        let h2 = hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            h2.cancel();
        });
        let req = BrowserRequest {
            label: "x".into(),
            file_name: "x.jar".into(),
            url: String::new(),
            size: 0,
            md5: None,
            sha1: Some("00".into()),
        };
        assert!(fetch_by_hand(&hub, vec![req], false, Duration::from_secs(30)).await.is_err());
    }

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bf-cf-{tag}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
