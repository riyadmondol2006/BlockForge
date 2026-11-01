//! The player's worlds, settings and servers across game versions, and the backups that make a version change
//! safe. Ported from BedrockOnLinux `bol/saves.py` (MIT), with credit.
//!
//! Everything the player made sits under one `Users` folder per edition inside the profile's prefix, beside
//! caches the game rebuilds by itself:
//!
//! `<prefix>/drive_c/users/steamuser/AppData/Roaming/Minecraft Bedrock[ Preview]/Users/<account>/games/com.mojang/`
//!
//! Every build of an edition reads the same folder: an account's folder is named after the account, never after
//! the build, so a new version opens exactly what the last one saved. A signed-in session reads its account's
//! folder and a signed-out one the `Shared` folder, so a build that fails to sign in appears to have lost the
//! worlds; they are untouched ([`Saves::ran_signed_out`] tells the two apart). Preview is Mojang's exception on
//! purpose: it has a folder of its own so that a world it upgrades can never reach the stable game, and nothing
//! here ever writes Release from Preview or the other way round, except a *copy* the player asked for.
//!
//! Backups sit beside the prefix, not inside it, so a prefix repair or reset cannot take them along:
//! `<backups>/<edition>/<timestamp>/{Users,backup.json}`, five kept per edition.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::Edition;
use super::prefix::registry::OfflineLock;
use super::prefix::{PrefixError, free_bytes, prefix_processes, require_prefix_idle, write_atomic};

const ROAMING: &str = "drive_c/users/steamuser/AppData/Roaming";
/// A version change is rare and worlds are big: five is a few changes back without the backups becoming the
/// largest thing BlockForge keeps.
pub const KEEP: usize = 5;
/// Room left on the drive after a copy, so a backup never becomes the reason the game has nowhere to save the
/// world it just opened.
const HEADROOM: u64 = 512 * 1024 * 1024;
const RECORD: &str = "backup.json";
const PARTIAL: &str = ".partial-";
// Beside the Users folder they replace, so taking it out of the way and putting the new one in are both renames.
const INCOMING: &str = ".Users.bf-incoming";
const OUTGOING: &str = ".Users.bf-outgoing";
const OPTIONS_BACKUP_SUFFIX: &str = ".bf-backup";
const STATE: &str = "state.json";

fn data_folder(edition: Edition) -> &'static str {
    match edition {
        Edition::Release => "Minecraft Bedrock",
        Edition::Preview => "Minecraft Bedrock Preview",
    }
}

fn edition_key(edition: Edition) -> &'static str {
    match edition {
        Edition::Release => "release",
        Edition::Preview => "preview",
    }
}

pub fn edition_name(edition: Edition) -> &'static str {
    match edition {
        Edition::Release => "Minecraft",
        Edition::Preview => "Minecraft Preview",
    }
}

fn io_err(what: impl Into<String>, e: io::Error) -> PrefixError {
    PrefixError::io(what, e)
}

/// One kept backup.
#[derive(Clone, Debug)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub edition: Edition,
    pub reason: String,
    pub version: Option<String>,
    pub previous: Option<String>,
    pub created: f64,
    pub size: u64,
    pub worlds: usize,
}

#[derive(Serialize, Deserialize)]
struct Record {
    schema: u32,
    edition: String,
    reason: String,
    version: Option<String>,
    previous: Option<String>,
    created: f64,
    size: u64,
    worlds: usize,
}

/// Small durable facts about a profile's data, beside its backups.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct SaveState {
    /// The build that last opened each edition's data, by edition ("release"/"preview").
    pub last_played: HashMap<String, String>,
    /// "copied" once Preview was seeded from Release; "separate" when the player chose to keep them apart.
    pub preview_data: Option<String>,
}

/// What [`Saves::before_launch`] did, for the caller to tell the player.
#[derive(Clone, Debug, Default)]
pub struct BeforeLaunch {
    /// A backup was made because the build changed.
    pub backed_up: Option<PathBuf>,
    /// Preview was given a copy of Release's data.
    pub preview_seeded: bool,
    /// An earlier replacement that was cut off was finished or undone.
    pub recovered_interrupted_swap: bool,
    /// The build is older than the one that last opened this data: a world that newer build upgraded may not
    /// load. Only returned when the caller said that was agreed to.
    pub downgrade: bool,
    pub previous: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LaunchPolicy {
    /// Give Preview a copy of the stable game's data the first time, unless it already has a life of its own.
    pub seed_preview_from_release: bool,
    /// The player agreed to open data with an older build than last opened it. Without this, such a launch is
    /// refused (after the backup is made) rather than the world being silently exposed to an older game.
    pub allow_downgrade: bool,
}

/// The saves of one profile: its prefix and where its backups are kept.
#[derive(Clone, Debug)]
pub struct Saves {
    prefix: PathBuf,
    backups: PathBuf,
    headroom: u64,
}

impl Saves {
    pub fn new(prefix: impl Into<PathBuf>, backups: impl Into<PathBuf>) -> Self {
        Self { prefix: prefix.into(), backups: backups.into(), headroom: HEADROOM }
    }

    #[cfg(test)]
    fn with_headroom(mut self, headroom: u64) -> Self {
        self.headroom = headroom;
        self
    }

    pub fn users_dir(&self, edition: Edition) -> PathBuf {
        self.prefix.join(ROAMING).join(data_folder(edition)).join("Users")
    }

    fn edition_backups(&self, edition: Edition) -> PathBuf {
        self.backups.join(edition_key(edition))
    }

    pub fn worlds(users: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(accounts) = fs::read_dir(users) else { return out };
        for account in accounts.flatten() {
            let Ok(worlds) = fs::read_dir(account.path().join("games/com.mojang/minecraftWorlds")) else { continue };
            out.extend(worlds.flatten().filter(|w| w.file_type().is_ok_and(|t| t.is_dir())).map(|w| w.path()));
        }
        out.sort();
        out
    }

    fn account_options(users: &Path) -> Vec<PathBuf> {
        let Ok(accounts) = fs::read_dir(users) else { return Vec::new() };
        let mut out: Vec<PathBuf> = accounts
            .flatten()
            .map(|a| a.path().join("games/com.mojang/minecraftpe/options.txt"))
            .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file()))
            .collect();
        out.sort();
        out
    }

    /// Whether the game has saved anything worth keeping in `users`: a world, or settings (it writes its options
    /// file the first time it runs, so a folder without one has only held what was seeded into it).
    pub fn has_player_data(users: &Path) -> bool {
        !Self::worlds(users).is_empty() || !Self::account_options(users).is_empty()
    }

    // ------------------------------------------------------------ state

    pub fn state(&self) -> SaveState {
        fs::read(self.backups.join(STATE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save_state(&self, state: &SaveState) -> Result<(), PrefixError> {
        fs::create_dir_all(&self.backups).map_err(|e| io_err("create the backups folder", e))?;
        let json = serde_json::to_vec_pretty(state).expect("state serializes");
        write_atomic(&self.backups.join(STATE), &json, 0o600)
    }

    // ------------------------------------------------------------ backups

    pub fn list_backups(&self, edition: Option<Edition>) -> Vec<BackupInfo> {
        let editions = match edition {
            Some(e) => vec![e],
            None => vec![Edition::Release, Edition::Preview],
        };
        let mut out = Vec::new();
        for e in editions {
            let Ok(read) = fs::read_dir(self.edition_backups(e)) else { continue };
            for entry in read.flatten() {
                let path = entry.path();
                if entry.file_name().to_string_lossy().starts_with('.')
                    || !fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                    || !path.join("Users").is_dir()
                {
                    continue;
                }
                let Some(record) =
                    fs::read(path.join(RECORD)).ok().and_then(|b| serde_json::from_slice::<Record>(&b).ok())
                else {
                    continue;
                };
                if record.edition != edition_key(e) {
                    continue;
                }
                out.push(BackupInfo {
                    path,
                    edition: e,
                    reason: record.reason,
                    version: record.version,
                    previous: record.previous,
                    created: record.created,
                    size: record.size,
                    worlds: record.worlds,
                });
            }
        }
        out.sort_by(|a, b| b.created.total_cmp(&a.created).then_with(|| b.path.cmp(&a.path)));
        out
    }

    /// Keep the newest [`KEEP`] backups of an edition, and drop what a failed copy left.
    fn prune(&self, edition: Edition, protect: &[PathBuf]) {
        if let Ok(read) = fs::read_dir(self.edition_backups(edition)) {
            for e in read.flatten() {
                if e.file_name().to_string_lossy().starts_with(PARTIAL) {
                    let _ = fs::remove_dir_all(e.path());
                }
            }
        }
        for old in self.list_backups(Some(edition)).into_iter().skip(KEEP) {
            if protect.iter().any(|p| p == &old.path) {
                continue;
            }
            let _ = fs::remove_dir_all(&old.path);
        }
    }

    /// Copy an edition's data into a new backup and return its folder; None when there is nothing to keep.
    /// Errors when there is something and it could not be copied, so that nothing about to overwrite that data
    /// goes ahead on the strength of a backup that does not exist.
    pub fn back_up(
        &self,
        edition: Edition,
        reason: &str,
        version: Option<&str>,
        previous: Option<&str>,
        protect: &[PathBuf],
    ) -> Result<Option<PathBuf>, PrefixError> {
        let source = self.users_dir(edition);
        if !Self::has_player_data(&source) {
            return Ok(None);
        }
        let size = dir_size(&source);
        let dir = self.edition_backups(edition);
        fs::create_dir_all(&dir).map_err(|e| io_err(format!("create {}", dir.display()), e))?;
        let free = free_bytes(&dir).map_err(|e| io_err("check the free space", e))?;
        if free < size.saturating_add(self.headroom) {
            return Err(PrefixError::NoSpace(format!(
                "Not enough free space to back up your {} worlds and settings ({} MB, {} MB free on the drive holding {}).",
                edition_name(edition),
                size / 1024 / 1024,
                free / 1024 / 1024,
                dir.display()
            )));
        }
        // To the microsecond: it is what orders the backups, and two can be made within the same second.
        let created = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        let staged = dir.join(format!("{PARTIAL}{}", uuid::Uuid::new_v4().simple()));
        let result = (|| -> io::Result<PathBuf> {
            fs::create_dir(&staged)?;
            copy_tree(&source, &staged.join("Users"))?;
            let record = Record {
                schema: 1,
                edition: edition_key(edition).into(),
                reason: reason.into(),
                version: version.map(String::from),
                previous: previous.map(String::from),
                created,
                size,
                worlds: Self::worlds(&staged.join("Users")).len(),
            };
            fs::write(staged.join(RECORD), serde_json::to_vec(&record).expect("record serializes"))?;
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.6f").to_string();
            let mut final_path = dir.join(&stamp);
            for index in 2..100 {
                if !final_path.exists() {
                    break;
                }
                final_path = dir.join(format!("{stamp}-{index}"));
            }
            fs::rename(&staged, &final_path)?;
            Ok(final_path)
        })();
        match result {
            Ok(path) => {
                let mut protect = protect.to_vec();
                protect.push(path.clone());
                self.prune(edition, &protect);
                Ok(Some(path))
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&staged);
                Err(io_err(format!("back up your {} worlds and settings", edition_name(edition)), e))
            }
        }
    }

    /// Finish, or undo, a replacement BlockForge was stopped in.
    ///
    /// A replacement copies the new data in beside the old, then makes two renames: the old data goes out, the
    /// new copy goes in. Stopped between the two, the data that went out is the player's and the copy never went
    /// in -- which the copy still being there says. A game started before anything looked has meanwhile made a
    /// fresh Users folder where the data was; that is no reason to throw the player's data away: whatever the
    /// game made is backed up if it holds anything, and the player's data goes back. Returns whether it acted.
    pub fn recover_interrupted_swap(&self, edition: Edition) -> Result<bool, PrefixError> {
        let target = self.users_dir(edition);
        let outgoing = target.with_file_name(OUTGOING);
        let incoming = target.with_file_name(INCOMING);
        let mut acted = false;
        if outgoing.exists() && (incoming.exists() || !target.exists()) {
            if target.exists() {
                // Errors before anything is moved when there is no room for it.
                self.back_up(edition, "after-interrupted-swap", None, None, &[])?;
                fs::remove_dir_all(&target).map_err(|e| io_err("remove the Users folder the game made", e))?;
            }
            fs::rename(&outgoing, &target).map_err(|e| io_err("put your worlds back", e))?;
            acted = true;
        }
        // Past this point anything that went out was replaced by a completed swap, and anything that was coming
        // in never made it.
        for leftover in [&outgoing, &incoming] {
            if leftover.exists() {
                acted |= fs::remove_dir_all(leftover).is_ok();
            }
        }
        Ok(acted)
    }

    /// Make a copy of the Users tree `source` what `edition` reads. The copy is complete before anything moves,
    /// and the swap is two renames inside one folder, so a failure at any point leaves the player with one whole
    /// set of data: the old one or the new one.
    fn replace_users(&self, source: &Path, edition: Edition) -> Result<(), PrefixError> {
        let target = self.users_dir(edition);
        let parent = target.parent().expect("users dir has a parent");
        fs::create_dir_all(parent).map_err(|e| io_err("create the data folder", e))?;
        self.recover_interrupted_swap(edition)?;
        let incoming = target.with_file_name(INCOMING);
        let outgoing = target.with_file_name(OUTGOING);
        copy_tree(source, &incoming).map_err(|e| {
            let _ = fs::remove_dir_all(&incoming);
            io_err("copy the data", e)
        })?;
        if target.exists() {
            fs::rename(&target, &outgoing).map_err(|e| {
                let _ = fs::remove_dir_all(&incoming);
                io_err("move the old data aside", e)
            })?;
        }
        if let Err(e) = fs::rename(&incoming, &target) {
            if outgoing.exists() {
                let _ = fs::rename(&outgoing, &target);
            }
            return Err(io_err("put the new data in", e));
        }
        let _ = fs::remove_dir_all(&outgoing);
        Ok(())
    }

    /// Put the data of one backup back. Returns the backup made of the data it replaced (None when there was
    /// none): what is there now is the player's too, and restoring is a choice they may want back.
    pub fn restore_backup(&self, backup: &Path) -> Result<Option<PathBuf>, PrefixError> {
        let info =
            self.list_backups(None).into_iter().find(|b| b.path == backup).ok_or_else(|| {
                PrefixError::Invalid(format!("{} is not a backup BlockForge made.", backup.display()))
            })?;
        require_prefix_idle(&self.prefix, "restore a backup")?;
        let _lock = OfflineLock::take(&self.prefix)?;
        let kept = self.back_up(info.edition, "before-restore", None, None, std::slice::from_ref(&info.path))?;
        self.replace_users(&info.path.join("Users"), info.edition).map_err(|e| {
            PrefixError::Integrity(format!("Could not restore the backup: {e}. Your data was left as it was."))
        })?;
        Ok(kept)
    }

    // ------------------------------------------------------------ preview

    /// Whether Preview would start empty while the stable game has data. Asked once: after a copy, or after the
    /// player chose to keep Preview apart, Preview's data is its own and nothing writes over it.
    pub fn preview_copy_pending(&self) -> bool {
        self.state().preview_data.is_none()
            && Self::worlds(&self.users_dir(Edition::Preview)).is_empty()
            && Self::has_player_data(&self.users_dir(Edition::Release))
    }

    pub fn keep_preview_separate(&self) -> Result<(), PrefixError> {
        let mut state = self.state();
        state.preview_data = Some("separate".into());
        self.save_state(&state)
    }

    /// Give Preview a copy of the stable game's worlds, settings and servers. A copy, never a link: a world
    /// Preview opens is upgraded to a version the stable game does not have yet, which is the reason Mojang keeps
    /// the two apart. Release is only read. Whatever Preview already had is backed up first.
    pub fn copy_to_preview(&self) -> Result<usize, PrefixError> {
        let source = self.users_dir(Edition::Release);
        if !Self::has_player_data(&source) {
            return Err(PrefixError::Invalid("Minecraft has no worlds or settings to copy yet.".into()));
        }
        self.back_up(Edition::Preview, "before-preview-copy", None, None, &[])?;
        let worlds = Self::worlds(&source).len();
        self.replace_users(&source, Edition::Preview)
            .map_err(|e| PrefixError::Integrity(format!("Could not copy your Minecraft data into Preview: {e}")))?;
        let mut state = self.state();
        state.preview_data = Some("copied".into());
        self.save_state(&state)?;
        Ok(worlds)
    }

    /// [`Saves::copy_to_preview`] for a request made outside a launch.
    pub fn copy_to_preview_when_idle(&self) -> Result<usize, PrefixError> {
        require_prefix_idle(&self.prefix, "copy your worlds into Preview")?;
        let _lock = OfflineLock::take(&self.prefix)?;
        self.copy_to_preview()
    }

    // ------------------------------------------------------------ at PLAY

    /// Make sure what the build about to start opens can be had back. Runs with the prefix idle and the launch
    /// lock held. If this build is not the one that last opened the edition's data, that data is backed up -- a
    /// world a newer version opens is upgraded for good, and one an older version opens may not load. A backup
    /// that cannot be made stops the launch (errors), because the data is about to be exposed to a new version.
    pub fn before_launch(
        &self,
        edition: Edition,
        version: &str,
        policy: LaunchPolicy,
    ) -> Result<BeforeLaunch, PrefixError> {
        require_prefix_idle(&self.prefix, "prepare your saved data")?;
        // Before the game opens the folder: a replacement stopped half-way has the player's data beside it.
        let recovered = self.recover_interrupted_swap(edition).map_err(|e| {
            PrefixError::Integrity(format!(
                "An earlier restore of your {} worlds was interrupted, and putting them back failed: {e}. Nothing was deleted; free some space and press Play again.",
                edition_name(edition)
            ))
        })?;
        let mut out = BeforeLaunch { recovered_interrupted_swap: recovered, ..Default::default() };
        let mut state = self.state();
        let mut copied = false;
        if edition == Edition::Preview && state.preview_data.is_none() {
            if policy.seed_preview_from_release && self.preview_copy_pending() {
                self.copy_to_preview()?;
                state = self.state();
                copied = true;
                out.preview_seeded = true;
            } else if Self::has_player_data(&self.users_dir(Edition::Preview)) {
                // Preview already has a life of its own: never copy over it.
                self.keep_preview_separate()?;
                state = self.state();
            }
        }
        let key = edition_key(edition);
        let previous = state.last_played.get(key).cloned();
        out.previous = previous.clone();
        if previous.as_deref() != Some(version) {
            out.downgrade = previous.as_deref().is_some_and(|p| version_older(version, p));
            // A copy made a moment ago has its original right beside it.
            if !copied {
                out.backed_up = self.back_up(edition, "version-change", Some(version), previous.as_deref(), &[])?;
            }
            if out.downgrade && !policy.allow_downgrade {
                return Err(PrefixError::NeedsConfirmation(format!(
                    "{} {version} is older than {}, which last opened your worlds. A world a newer version opened may not load in an older \
                     one. A backup was made first; restore it, or confirm to open them with {version} anyway.",
                    edition_name(edition),
                    previous.as_deref().unwrap_or("the last version")
                )));
            }
            state.last_played.insert(key.into(), version.into());
            self.save_state(&state)?;
        }
        Ok(out)
    }

    /// Whether the session that started at `since` played signed out while an account's data was there to be
    /// missed. The game keeps its settings beside the profile it plays as and writes them while it runs; a
    /// signed-in session never writes the signed-out profile's file, so that one being written while no
    /// account's file was is the session having run without the account.
    pub fn ran_signed_out(&self, edition: Edition, since: SystemTime) -> bool {
        let users = self.users_dir(edition);
        let written = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| t >= since);
        if !written(&users.join("Shared/games/com.mojang/minecraftpe/options.txt")) {
            return false;
        }
        let accounts: Vec<PathBuf> = Self::account_options(&users)
            .into_iter()
            .filter(|p| {
                p.strip_prefix(&users)
                    .ok()
                    .and_then(|r| r.components().next())
                    .is_some_and(|c| c.as_os_str() != "Shared")
            })
            .collect();
        !accounts.is_empty() && !accounts.iter().any(|p| written(p))
    }

    // ------------------------------------------------------------ options.txt torn by a crash

    fn all_options(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = [Edition::Release, Edition::Preview]
            .into_iter()
            .flat_map(|e| Self::account_options(&self.users_dir(e)))
            .collect();
        out.sort();
        out
    }

    /// Keep a copy of each intact `options.txt` before the game rewrites it. Minecraft saves it by truncating
    /// and streaming the whole thing back; a crash in that window leaves it cut off and every setting past the
    /// cut silently reads as its default next time. The write is the game's, so it cannot be made atomic from
    /// here: keep the last intact copy instead. Only an intact file replaces a kept copy.
    pub fn snapshot_options(&self) -> usize {
        let mut kept = 0;
        for path in self.all_options() {
            let Ok(data) = fs::read(&path) else { continue };
            if !options_intact(&data) {
                continue;
            }
            let backup = options_backup(&path);
            if fs::read(&backup).ok().as_deref() == Some(data.as_slice()) || write_atomic(&backup, &data, 0o644).is_ok()
            {
                kept += 1;
            }
        }
        kept
    }

    /// Put back any `options.txt` the game left cut off mid-write. Deliberately narrow: only from a copy taken
    /// here, and only over a file that cannot be a finished save. `prefix_idle` is the caller's answer when it
    /// already waited for the prefix to settle; None asks. Returns the files restored.
    pub fn restore_truncated_options(&self, prefix_idle: Option<bool>) -> Vec<PathBuf> {
        let mut restored = Vec::new();
        for path in self.all_options() {
            if fs::read(&path).is_ok_and(|d| options_intact(&d)) {
                continue;
            }
            let Ok(kept) = fs::read(options_backup(&path)) else { continue };
            if !options_intact(&kept) {
                continue;
            }
            let idle = prefix_idle.unwrap_or_else(|| prefix_processes(&self.prefix).is_ok_and(|p| p.is_empty()));
            if !idle {
                // The game is still shutting down and may yet finish its save.
                return restored;
            }
            if write_atomic(&path, &kept, 0o644).is_ok() {
                restored.push(path);
            }
        }
        restored
    }
}

fn options_backup(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(OPTIONS_BACKUP_SUFFIX);
    PathBuf::from(name)
}

/// Whether `data` is a finished options.txt rather than a cut-off write. Minecraft terminates every line it
/// writes, so a file that stops without a line ending stopped in the middle of being written; at least one
/// `key:value` on top keeps an empty or shredded file from being mistaken for a good copy.
fn options_intact(data: &[u8]) -> bool {
    !data.is_empty() && data.ends_with(b"\n") && data.split(|&b| b == b'\n').any(|l| l.contains(&b':'))
}

/// Whether `version` is older than `than`, comparing dotted numbers ("1.26.60.0"). Anything that is not a plain
/// dotted number is never called older: an unknown ordering must not block a launch.
pub fn version_older(version: &str, than: &str) -> bool {
    fn parse(v: &str) -> Option<Vec<u64>> {
        v.split('.').map(|p| p.parse::<u64>().ok()).collect()
    }
    let (Some(a), Some(b)) = (parse(version), parse(than)) else { return false };
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0)))
        .find(|(x, y)| x != y)
        .is_some_and(|(x, y)| x < y)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(read) = fs::read_dir(path) {
        for e in read.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                total += dir_size(&e.path());
            } else if t.is_file() {
                total += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

/// Copy a folder tree keeping links as links (a world may hold one; following it could copy a lot more than the
/// world, or loop).
fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        let t = entry.file_type()?;
        if t.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(entry.path())?, &to)?;
        } else if t.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else if t.is_file() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bedrock::prefix::registry::update_prefix_registry;
    use crate::testutil::scratch;
    use std::os::unix::fs::PermissionsExt;

    fn setup(tag: &str) -> (Saves, PathBuf) {
        let root = scratch(tag);
        let prefix = root.join("compat/pfx");
        fs::create_dir_all(&prefix).unwrap();
        (Saves::new(&prefix, root.join("backups")), root)
    }

    fn add_world(saves: &Saves, edition: Edition, account: &str, world: &str, content: &str) {
        let w = saves.users_dir(edition).join(account).join("games/com.mojang/minecraftWorlds").join(world);
        fs::create_dir_all(&w).unwrap();
        fs::write(w.join("level.dat"), content).unwrap();
        fs::write(w.join("db.ldb"), content.repeat(100)).unwrap();
    }

    fn add_options(saves: &Saves, edition: Edition, account: &str, text: &str) -> PathBuf {
        let p = saves.users_dir(edition).join(account).join("games/com.mojang/minecraftpe/options.txt");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, text).unwrap();
        p
    }

    fn tree(path: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(base: &Path, p: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            let Ok(read) = fs::read_dir(p) else { return };
            for e in read.flatten() {
                let q = e.path();
                let rel = q.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                if e.file_type().unwrap().is_dir() {
                    out.push((format!("{rel}/"), Vec::new()));
                    walk(base, &q, out);
                } else {
                    out.push((rel, fs::read(&q).unwrap_or_default()));
                }
            }
        }
        let mut out = Vec::new();
        walk(path, path, &mut out);
        out.sort();
        out
    }

    const V1: &str = "1.26.50.0";
    const V2: &str = "1.26.60.0";

    #[test]
    fn a_backup_holds_signed_in_and_shared_worlds() {
        let (saves, _) = setup("save-backup");
        add_world(&saves, Edition::Release, "1234567890", "mine", "signed-in world");
        add_world(&saves, Edition::Release, "Shared", "offline", "shared world");
        add_options(&saves, Edition::Release, "1234567890", "gfx:1\n");
        let path = saves.back_up(Edition::Release, "version-change", Some(V2), Some(V1), &[]).unwrap().unwrap();
        assert_eq!(
            fs::read(path.join("Users/1234567890/games/com.mojang/minecraftWorlds/mine/level.dat")).unwrap(),
            b"signed-in world"
        );
        assert_eq!(
            fs::read(path.join("Users/Shared/games/com.mojang/minecraftWorlds/offline/level.dat")).unwrap(),
            b"shared world"
        );
        let listed = saves.list_backups(None);
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].worlds, listed[0].reason.as_str(), listed[0].version.as_deref(), listed[0].previous.as_deref()),
            (2, "version-change", Some(V2), Some(V1))
        );
        assert_eq!(tree(&saves.users_dir(Edition::Release)), tree(&path.join("Users")));
    }

    #[test]
    fn nothing_to_keep_makes_no_backup_and_a_seeded_empty_folder_is_not_player_data() {
        let (saves, _) = setup("save-empty");
        assert!(saves.back_up(Edition::Release, "x", None, None, &[]).unwrap().is_none());
        fs::create_dir_all(saves.users_dir(Edition::Release).join("Shared/games/com.mojang/minecraftpe")).unwrap();
        assert!(saves.back_up(Edition::Release, "x", None, None, &[]).unwrap().is_none());
    }

    #[test]
    fn five_backups_are_kept_per_edition_newest_first() {
        let (saves, _) = setup("save-keep");
        add_world(&saves, Edition::Release, "A", "w", "r");
        add_world(&saves, Edition::Preview, "A", "w", "p");
        let preview = saves.back_up(Edition::Preview, "once", None, None, &[]).unwrap().unwrap();
        let mut made = Vec::new();
        for i in 0..8 {
            made.push(saves.back_up(Edition::Release, &format!("n{i}"), None, None, &[]).unwrap().unwrap());
        }
        let listed = saves.list_backups(Some(Edition::Release));
        assert_eq!(listed.len(), KEEP);
        assert_eq!(listed[0].reason, "n7", "newest first");
        assert!(!made[0].exists() && made[7].exists());
        assert!(preview.exists(), "Release backups never push out Preview's");
    }

    #[test]
    fn a_leftover_from_a_failed_copy_is_swept_and_never_listed() {
        let (saves, _) = setup("save-partial");
        add_world(&saves, Edition::Release, "A", "w", "r");
        let dir = saves.edition_backups(Edition::Release);
        fs::create_dir_all(dir.join(format!("{PARTIAL}dead/Users"))).unwrap();
        assert!(saves.list_backups(None).is_empty());
        saves.back_up(Edition::Release, "x", None, None, &[]).unwrap();
        assert!(!dir.join(format!("{PARTIAL}dead")).exists());
    }

    #[test]
    fn a_backup_that_cannot_be_made_is_an_error_and_changes_nothing() {
        let (saves, _) = setup("save-nospace");
        add_world(&saves, Edition::Release, "A", "w", "precious");
        let before = tree(&saves.users_dir(Edition::Release));
        let tight = saves.clone().with_headroom(u64::MAX / 2);
        assert!(matches!(tight.back_up(Edition::Release, "x", None, None, &[]), Err(PrefixError::NoSpace(_))));
        assert_eq!(tree(&saves.users_dir(Edition::Release)), before);
        assert!(saves.list_backups(None).is_empty());
        // And a launch that needs that backup does not go ahead.
        let launch = tight.before_launch(Edition::Release, V2, LaunchPolicy::default());
        assert!(matches!(launch, Err(PrefixError::NoSpace(_))), "{launch:?}");
        assert!(tight.state().last_played.is_empty(), "and the build is not recorded as having opened the data");
        assert_eq!(tree(&saves.users_dir(Edition::Release)), before);
    }

    #[test]
    fn a_changed_build_is_backed_up_once_and_the_same_build_is_not() {
        let (saves, _) = setup("save-version");
        add_world(&saves, Edition::Release, "A", "w", "world");
        let first = saves.before_launch(Edition::Release, V1, LaunchPolicy::default()).unwrap();
        assert!(first.backed_up.is_some() && first.previous.is_none());
        let again = saves.before_launch(Edition::Release, V1, LaunchPolicy::default()).unwrap();
        assert!(again.backed_up.is_none());
        let update = saves.before_launch(Edition::Release, V2, LaunchPolicy::default()).unwrap();
        assert!(update.backed_up.is_some() && !update.downgrade);
        assert_eq!(update.previous.as_deref(), Some(V1));
        assert_eq!(saves.list_backups(Some(Edition::Release)).len(), 2);
        assert_eq!(saves.state().last_played.get("release").map(String::as_str), Some(V2));
    }

    #[test]
    fn a_downgrade_needs_the_players_say_so_and_never_restores_anything_itself() {
        let (saves, _) = setup("save-downgrade");
        add_world(&saves, Edition::Release, "A", "w", "upgraded by the new build");
        saves.before_launch(Edition::Release, V2, LaunchPolicy::default()).unwrap();
        let before = tree(&saves.users_dir(Edition::Release));
        let refused = saves.before_launch(Edition::Release, V1, LaunchPolicy::default());
        assert!(matches!(refused, Err(PrefixError::NeedsConfirmation(_))), "{refused:?}");
        assert_eq!(tree(&saves.users_dir(Edition::Release)), before, "the data is untouched");
        assert!(!saves.list_backups(Some(Edition::Release)).is_empty());
        assert_eq!(
            saves.state().last_played.get("release").map(String::as_str),
            Some(V2),
            "and the record is not moved back"
        );
        let allowed = saves
            .before_launch(Edition::Release, V1, LaunchPolicy { allow_downgrade: true, ..Default::default() })
            .unwrap();
        assert!(allowed.downgrade && allowed.backed_up.is_some());
        assert_eq!(tree(&saves.users_dir(Edition::Release)), before);
        assert!(
            version_older("1.9.0", "1.10.0") && !version_older("1.10.0", "1.9.0") && version_older("1.26", "1.26.1.0")
        );
        assert!(
            !version_older("1.26.60.0", "1.26.60") && !version_older("preview", "1.0") && !version_older("1.0", "x")
        );
    }

    #[test]
    fn restoring_keeps_a_backup_of_what_it_replaces() {
        let (saves, _) = setup("save-restore");
        add_world(&saves, Edition::Release, "A", "old", "old world");
        let backup = saves.back_up(Edition::Release, "manual", None, None, &[]).unwrap().unwrap();
        fs::remove_dir_all(saves.users_dir(Edition::Release)).unwrap();
        add_world(&saves, Edition::Release, "A", "new", "new world");
        let kept = saves.restore_backup(&backup).unwrap().unwrap();
        let users = saves.users_dir(Edition::Release);
        assert_eq!(fs::read(users.join("A/games/com.mojang/minecraftWorlds/old/level.dat")).unwrap(), b"old world");
        assert!(!users.join("A/games/com.mojang/minecraftWorlds/new").exists());
        assert_eq!(
            fs::read(kept.join("Users/A/games/com.mojang/minecraftWorlds/new/level.dat")).unwrap(),
            b"new world"
        );
        assert!(backup.exists(), "the backup restored from is not consumed");
        assert!(saves.restore_backup(&saves.backups.join("nope")).is_err());
        let leftovers: Vec<_> = fs::read_dir(users.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["Users"]);
    }

    #[test]
    fn a_busy_prefix_refuses_restore_launch_prep_and_copy() {
        let (saves, _) = setup("save-busy");
        add_world(&saves, Edition::Release, "A", "w", "x");
        let backup = saves.back_up(Edition::Release, "m", None, None, &[]).unwrap().unwrap();
        let mut child = std::process::Command::new("sleep").arg("30").env("WINEPREFIX", &saves.prefix).spawn().unwrap();
        let r1 = saves.restore_backup(&backup);
        let r2 = saves.before_launch(Edition::Release, V1, LaunchPolicy::default());
        let r3 = saves.copy_to_preview_when_idle();
        child.kill().ok();
        child.wait().ok();
        assert!(
            matches!(r1, Err(PrefixError::Busy(_)))
                && matches!(r2, Err(PrefixError::Busy(_)))
                && matches!(r3, Err(PrefixError::Busy(_)))
        );
    }

    #[test]
    fn preview_is_seeded_by_copy_and_release_is_never_touched() {
        let (saves, _) = setup("save-preview");
        add_world(&saves, Edition::Release, "A", "w", "release world");
        add_options(&saves, Edition::Release, "A", "gfx_fancy:1\n");
        let release_before = tree(&saves.users_dir(Edition::Release));
        assert!(saves.preview_copy_pending());
        let out = saves
            .before_launch(
                Edition::Preview,
                "1.26.70.20",
                LaunchPolicy { seed_preview_from_release: true, ..Default::default() },
            )
            .unwrap();
        assert!(out.preview_seeded);
        assert_eq!(tree(&saves.users_dir(Edition::Preview)), release_before, "Preview got a copy");
        assert_eq!(tree(&saves.users_dir(Edition::Release)), release_before);
        assert!(!saves.preview_copy_pending());
        // Preview now upgrades its world; Release must not see it.
        fs::write(
            saves.users_dir(Edition::Preview).join("A/games/com.mojang/minecraftWorlds/w/level.dat"),
            "upgraded by preview",
        )
        .unwrap();
        add_world(&saves, Edition::Preview, "A", "preview-only", "p");
        saves.before_launch(Edition::Preview, "1.26.70.21", LaunchPolicy::default()).unwrap();
        saves.back_up(Edition::Preview, "manual", None, None, &[]).unwrap();
        assert_eq!(tree(&saves.users_dir(Edition::Release)), release_before, "Preview never mutates Release");
        assert!(saves.list_backups(Some(Edition::Release)).is_empty());
        // Copying again would overwrite Preview's life: it backs that up first, and Release is still only read.
        saves.copy_to_preview().unwrap();
        assert_eq!(tree(&saves.users_dir(Edition::Release)), release_before);
        assert!(saves.list_backups(Some(Edition::Preview)).iter().any(|b| b.reason == "before-preview-copy"));
        let kept =
            saves.list_backups(Some(Edition::Preview)).into_iter().find(|b| b.reason == "before-preview-copy").unwrap();
        assert_eq!(
            fs::read(kept.path.join("Users/A/games/com.mojang/minecraftWorlds/preview-only/level.dat")).unwrap(),
            b"p"
        );
    }

    #[test]
    fn preview_with_its_own_data_is_never_overwritten_by_a_launch() {
        let (saves, _) = setup("save-preview-own");
        add_world(&saves, Edition::Release, "A", "r", "release");
        add_world(&saves, Edition::Preview, "A", "p", "preview life");
        let preview_before = tree(&saves.users_dir(Edition::Preview));
        let out = saves
            .before_launch(
                Edition::Preview,
                "1.26.70.20",
                LaunchPolicy { seed_preview_from_release: true, ..Default::default() },
            )
            .unwrap();
        assert!(!out.preview_seeded);
        assert_eq!(saves.state().preview_data.as_deref(), Some("separate"));
        assert_eq!(tree(&saves.users_dir(Edition::Preview)), preview_before);
        // A launch without the option never seeds either.
        let (other, _) = setup("save-preview-optional");
        add_world(&other, Edition::Release, "A", "r", "release");
        other.before_launch(Edition::Preview, "1.26.70.20", LaunchPolicy::default()).unwrap();
        assert!(Saves::worlds(&other.users_dir(Edition::Preview)).is_empty());
        other.keep_preview_separate().unwrap();
        assert!(!other.preview_copy_pending());
    }

    #[test]
    fn an_interrupted_replacement_is_finished_or_undone_without_losing_data() {
        // State 1: stopped after the old data went out, before the copy came in; the game made a fresh folder.
        let (saves, _) = setup("save-swap1");
        let users = saves.users_dir(Edition::Release);
        fs::create_dir_all(users.parent().unwrap()).unwrap();
        add_world(&saves, Edition::Release, "A", "players", "the player's world");
        fs::rename(&users, users.with_file_name(OUTGOING)).unwrap();
        fs::create_dir_all(users.with_file_name(INCOMING).join("A")).unwrap();
        add_world(&saves, Edition::Release, "B", "fresh", "made by a game started in between");
        assert!(saves.recover_interrupted_swap(Edition::Release).unwrap());
        assert_eq!(
            fs::read(users.join("A/games/com.mojang/minecraftWorlds/players/level.dat")).unwrap(),
            b"the player's world"
        );
        assert!(!users.with_file_name(OUTGOING).exists() && !users.with_file_name(INCOMING).exists());
        let kept = saves.list_backups(Some(Edition::Release));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].reason, "after-interrupted-swap");
        assert_eq!(
            fs::read(kept[0].path.join("Users/B/games/com.mojang/minecraftWorlds/fresh/level.dat")).unwrap(),
            b"made by a game started in between"
        );
        // State 2: the swap completed, only the old copy's delete was cut off.
        let (saves, _) = setup("save-swap2");
        add_world(&saves, Edition::Release, "A", "new", "the new data");
        let users = saves.users_dir(Edition::Release);
        fs::create_dir_all(users.with_file_name(OUTGOING).join("A")).unwrap();
        assert!(saves.recover_interrupted_swap(Edition::Release).unwrap());
        assert_eq!(fs::read(users.join("A/games/com.mojang/minecraftWorlds/new/level.dat")).unwrap(), b"the new data");
        assert!(!users.with_file_name(OUTGOING).exists());
        assert!(saves.list_backups(None).is_empty());
        // State 3: the old data went out and nothing is at the target at all.
        let (saves, _) = setup("save-swap3");
        add_world(&saves, Edition::Release, "A", "w", "only copy");
        let users = saves.users_dir(Edition::Release);
        fs::rename(&users, users.with_file_name(OUTGOING)).unwrap();
        assert!(saves.recover_interrupted_swap(Edition::Release).unwrap());
        assert_eq!(fs::read(users.join("A/games/com.mojang/minecraftWorlds/w/level.dat")).unwrap(), b"only copy");
        assert!(!saves.recover_interrupted_swap(Edition::Release).unwrap(), "and then there is nothing to do");
    }

    #[test]
    fn a_launch_finishes_an_interrupted_swap_before_the_game_opens_the_folder() {
        let (saves, _) = setup("save-swap-launch");
        add_world(&saves, Edition::Release, "A", "w", "mine");
        let users = saves.users_dir(Edition::Release);
        fs::rename(&users, users.with_file_name(OUTGOING)).unwrap();
        let out = saves.before_launch(Edition::Release, V1, LaunchPolicy::default()).unwrap();
        assert!(out.recovered_interrupted_swap);
        assert_eq!(fs::read(users.join("A/games/com.mojang/minecraftWorlds/w/level.dat")).unwrap(), b"mine");
    }

    #[test]
    fn backups_live_outside_the_prefix_and_survive_a_prefix_repair() {
        let (saves, root) = setup("save-repair");
        add_world(&saves, Edition::Release, "A", "w", "survivor");
        add_world(&saves, Edition::Release, "Shared", "s", "shared survivor");
        saves.before_launch(Edition::Release, V1, LaunchPolicy::default()).unwrap();
        fs::write(saves.prefix.join("system.reg"), "wine").unwrap();
        let compat = root.join("compat");
        let kept = crate::bedrock::prefix::reset_prefix_keeping_game_data(&compat).unwrap();
        assert_eq!(kept.len(), 1);
        let users = saves.users_dir(Edition::Release);
        assert_eq!(fs::read(users.join("A/games/com.mojang/minecraftWorlds/w/level.dat")).unwrap(), b"survivor");
        assert_eq!(
            fs::read(users.join("Shared/games/com.mojang/minecraftWorlds/s/level.dat")).unwrap(),
            b"shared survivor"
        );
        assert!(!saves.prefix.join("system.reg").exists());
        assert_eq!(saves.list_backups(None).len(), 1, "the backup was never inside the prefix");
        assert_eq!(saves.state().last_played.len(), 1);
    }

    #[test]
    fn engine_refresh_and_registry_edits_leave_the_worlds_alone() {
        let (saves, _) = setup("save-engine");
        let pfx = &saves.prefix;
        fs::create_dir_all(pfx.join("drive_c/windows/system32")).unwrap();
        fs::write(pfx.join("system.reg"), "WINE REGISTRY Version 2\n\n").unwrap();
        fs::write(pfx.join("user.reg"), "WINE REGISTRY Version 2\n\n").unwrap();
        add_world(&saves, Edition::Release, "A", "w", "world");
        add_world(&saves, Edition::Preview, "A", "w", "preview world");
        let before = (tree(&saves.users_dir(Edition::Release)), tree(&saves.users_dir(Edition::Preview)));
        crate::bedrock::prefix::apply_game_registry(pfx, &Default::default()).unwrap();
        update_prefix_registry(pfx, &[], &[]).unwrap();
        assert_eq!((tree(&saves.users_dir(Edition::Release)), tree(&saves.users_dir(Edition::Preview))), before);
    }

    #[test]
    fn a_torn_options_file_is_restored_from_the_last_intact_copy() {
        let (saves, _) = setup("save-options");
        let p = add_options(&saves, Edition::Release, "A", "gfx_fancy:1\nkeyboard_a:65\n");
        assert_eq!(saves.snapshot_options(), 1);
        fs::write(&p, "gfx_fancy:1\nkeybo").unwrap();
        // The next snapshot must not replace the good copy with a torn file.
        assert_eq!(saves.snapshot_options(), 0);
        assert!(saves.restore_truncated_options(Some(false)).is_empty(), "not while the game may still be saving");
        assert_eq!(saves.restore_truncated_options(Some(true)), vec![p.clone()]);
        assert_eq!(fs::read_to_string(&p).unwrap(), "gfx_fancy:1\nkeyboard_a:65\n");
        // A finished save that changed is never touched.
        fs::write(&p, "gfx_fancy:0\n").unwrap();
        assert!(saves.restore_truncated_options(Some(true)).is_empty());
        assert_eq!(fs::read_to_string(&p).unwrap(), "gfx_fancy:0\n");
        // Garbage is not a good copy.
        assert!(
            options_intact(b"a:1\n")
                && !options_intact(b"")
                && !options_intact(b"no newline:1")
                && !options_intact(b"\n\n")
        );
    }

    #[test]
    fn a_signed_out_session_is_told_apart_from_a_signed_in_one() {
        let (saves, _) = setup("save-signedout");
        let account = add_options(&saves, Edition::Release, "123", "a:1\n");
        let shared = add_options(&saves, Edition::Release, "Shared", "a:1\n");
        let long_ago = SystemTime::now() - std::time::Duration::from_secs(3600);
        for p in [&account, &shared] {
            let f = fs::File::options().write(true).open(p).unwrap();
            f.set_modified(long_ago).unwrap();
        }
        let since = SystemTime::now() - std::time::Duration::from_secs(60);
        assert!(!saves.ran_signed_out(Edition::Release, since), "nothing written this session");
        fs::write(&shared, "a:2\n").unwrap();
        assert!(saves.ran_signed_out(Edition::Release, since), "only the signed-out profile was written");
        fs::write(&account, "a:2\n").unwrap();
        assert!(!saves.ran_signed_out(Edition::Release, since), "the account was written too: it signed in");
        assert!(!saves.ran_signed_out(Edition::Preview, since));
    }

    #[test]
    fn copying_keeps_links_as_links_and_never_follows_them() {
        let (saves, root) = setup("save-links");
        add_world(&saves, Edition::Release, "A", "w", "x");
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("big"), vec![1u8; 100_000]).unwrap();
        let w = saves.users_dir(Edition::Release).join("A/games/com.mojang/minecraftWorlds/w");
        std::os::unix::fs::symlink(&outside, w.join("link")).unwrap();
        let path = saves.back_up(Edition::Release, "x", None, None, &[]).unwrap().unwrap();
        let copied = path.join("Users/A/games/com.mojang/minecraftWorlds/w/link");
        assert!(fs::symlink_metadata(&copied).unwrap().file_type().is_symlink());
        assert_eq!(fs::metadata(path.join(RECORD)).unwrap().permissions().mode() & 0o400, 0o400);
    }
}
