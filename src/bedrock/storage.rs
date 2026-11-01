//! What Bedrock keeps on disk under `<data>/bedrock` (PORT-PLAN.md section 4): where each thing lives, the versioned
//! records of instances, profiles and Store slots, JSON written so a crash never leaves half a file, names checked
//! before they become paths, and file locks between BlockForge processes.
//!
//! Everything here touches the disk, so it runs on the runtime's blocking workers, never on the window's thread.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{
    BedrockError, BedrockInstance, BuildId, BuildSelection, Edition, ErrorKind, InstanceId, ProfileId, Result,
    SessionId, StoreId,
};

/// The layout and record version this build writes. A record of a newer version is left alone (never rewritten in an
/// older shape); an older one is read through [`migrate`].
pub(super) const SCHEMA: u32 = 1;

/// Where everything of Bedrock lives, relative to one data root (`Dirs.base/bedrock`).
#[derive(Clone, Debug)]
pub(super) struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn at(root: impl Into<PathBuf>) -> Paths {
        Paths { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn schema_file(&self) -> PathBuf {
        self.root.join("schema.json")
    }

    pub fn instances(&self) -> PathBuf {
        self.root.join("instances")
    }

    pub fn instance_file(&self, id: &InstanceId) -> PathBuf {
        self.instances().join(format!("{}.json", id.0))
    }

    pub fn profiles(&self) -> PathBuf {
        self.root.join("profiles")
    }

    /// One game profile: its own Wine prefix, Xbox state and backups (one per instance in v1).
    pub fn profile(&self, id: &ProfileId) -> PathBuf {
        self.profiles().join(id.0.to_string())
    }

    /// The profile's launch lock: setup, repair, sign-out and play of its prefix exclude each other, and the game's
    /// supervisor holds it for the whole session (the same file as `linux::Profile::launch_lease`).
    pub fn launch_lease(&self, id: &ProfileId) -> PathBuf {
        self.profile(id).join("locks/launch.lease")
    }

    pub fn profile_file(&self, id: &ProfileId) -> PathBuf {
        self.profile(id).join("profile.json")
    }

    pub fn stores(&self) -> PathBuf {
        self.root.join("stores")
    }

    /// One persistent Store slot (a device identity and the sign-in of the account that owns the game).
    pub fn store(&self, id: &StoreId) -> PathBuf {
        self.stores().join(id.0.to_string())
    }

    pub fn store_file(&self, id: &StoreId) -> PathBuf {
        self.store(id).join("slot.json")
    }

    /// A downloaded build: `builds/<release|preview>/<version>/<package-id>`. The version and package come from the
    /// network or from a record, so both are checked before they are joined.
    pub fn build(&self, id: &BuildId) -> Result<PathBuf> {
        Ok(self
            .root
            .join("builds")
            .join(edition_dir(id.edition))
            .join(component(&id.version)?)
            .join(component(&id.package_id)?))
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn session_logs(&self, id: &SessionId) -> PathBuf {
        self.logs().join(id.0.to_string())
    }

    pub fn locks(&self) -> PathBuf {
        self.root.join("locks")
    }

    /// Copies of an instance's worlds made by Back up worlds.
    pub fn world_backups(&self, profile: &ProfileId, edition: Edition) -> PathBuf {
        self.profile(profile).join("backups").join(edition_dir(edition))
    }
}

pub(super) fn edition_dir(e: Edition) -> &'static str {
    match e {
        Edition::Release => "release",
        Edition::Preview => "preview",
    }
}

/// `s` when it is safe as one folder or file name on every system: not empty, not `.` or `..`, no separator, drive or
/// device name, only letters, digits and `.`, `_`, `-`, and at most 128 characters. Package ids
/// (`Microsoft.MinecraftUWP_1.26.5004.0_x64__8wekyb3d8bbwe`) and versions pass; anything a server or a damaged record
/// could use to reach outside its folder does not.
pub(super) fn component(s: &str) -> Result<&str> {
    let ok_chars = s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if s.is_empty() || s.len() > 128 || !ok_chars || s == "." || s == ".." || s.starts_with('.') || device {
        return Err(BedrockError::new(ErrorKind::InvalidRequest, format!("\"{s}\" can't be used as a folder name")));
    }
    Ok(s)
}

/// A name the player gave an instance: trimmed, 1 to 64 characters, no control characters.
pub(super) fn instance_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BedrockError::new(ErrorKind::InvalidRequest, "The instance needs a name"));
    }
    if name.chars().count() > 64 {
        return Err(BedrockError::new(ErrorKind::InvalidRequest, "Keep the name to 64 characters"));
    }
    if name.chars().any(char::is_control) {
        return Err(BedrockError::new(ErrorKind::InvalidRequest, "The name can't have control characters"));
    }
    Ok(name.to_string())
}

/// A build chosen by id is checked the way its folder will be.
pub(super) fn check_selection(selection: &BuildSelection) -> Result<()> {
    match selection {
        BuildSelection::Latest(_) => Ok(()),
        BuildSelection::Installed(id) => check_build(id),
    }
}

pub(super) fn check_build(id: &BuildId) -> Result<()> {
    component(&id.version)?;
    component(&id.package_id)?;
    Ok(())
}

pub(super) fn io_error(what: &str, path: &Path, e: std::io::Error) -> BedrockError {
    let kind = if e.kind() == std::io::ErrorKind::StorageFull { ErrorKind::NoSpace } else { ErrorKind::Internal };
    BedrockError::new(kind, format!("Couldn't {what} {}: {e}", path.display()))
}

/// A folder only this user can read (Store slots and profiles hold credentials and Xbox keys).
pub(super) fn private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| io_error("make", path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| io_error("protect", path, e))?;
    }
    Ok(())
}

/// Write `value` as JSON to `path` so a reader sees the old file or the new one, never part of one: into a temporary
/// file beside it, flushed to the disk, then renamed over it. The file is readable by this user only.
pub(super) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| BedrockError::new(ErrorKind::Internal, format!("Couldn't write {}: {e}", path.display())))?;
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| io_error("make", parent, e))?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        // the rename itself reaches the disk once the folder is flushed (not possible on Windows, where it isn't
        // needed in the same way)
        #[cfg(unix)]
        {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_error("write", path, e)
    })
}

/// Read a JSON record, `None` when there is no file.
pub(super) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error("read", path, e)),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
        BedrockError::new(ErrorKind::Integrity, format!("{} is damaged and can't be read: {e}", path.display()))
    })
}

/// The root's `schema.json`: which layout version made this folder.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct RootSchema {
    pub schema: u32,
}

/// A Bedrock instance as it is kept in `instances/<id>.json`. What the window sees of it is [`BedrockInstance`]; the
/// rest is the backend's own.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct InstanceRecord {
    pub schema: u32,
    pub id: InstanceId,
    pub name: String,
    pub profile: ProfileId,
    pub store: StoreId,
    pub selection: BuildSelection,
    #[serde(default)]
    pub installed: Option<BuildId>,
    /// Seconds since 1970.
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub last_played: Option<i64>,
    /// The build was changed since the last start: the profile's worlds are copied before the new build first runs.
    #[serde(default)]
    pub backup_before_play: bool,
}

impl InstanceRecord {
    pub fn public(&self) -> BedrockInstance {
        BedrockInstance {
            id: self.id.clone(),
            name: self.name.clone(),
            profile: self.profile.clone(),
            store: self.store.clone(),
            selection: self.selection.clone(),
            installed: self.installed.clone(),
        }
    }

    /// The edition this instance plays: the one it was asked for, or that of the build it was given.
    pub fn edition(&self) -> Edition {
        match &self.selection {
            BuildSelection::Latest(e) => *e,
            BuildSelection::Installed(b) => b.edition,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ProfileRecord {
    pub schema: u32,
    pub id: ProfileId,
    pub instance: InstanceId,
    #[serde(default)]
    pub created: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct StoreRecord {
    pub schema: u32,
    pub id: StoreId,
    #[serde(default)]
    pub created: i64,
}

/// An older record brought up to [`SCHEMA`], or why it can't be read. Version 0 is a record from before the field
/// existed; it has the same shape.
pub(super) fn migrate(mut value: serde_json::Value) -> std::result::Result<serde_json::Value, String> {
    let version = value.get("schema").and_then(|v| v.as_u64()).unwrap_or(0);
    if version > u64::from(SCHEMA) {
        return Err(format!("it was written by a newer BlockForge (layout {version}); it is left as it is"));
    }
    if let Some(map) = value.as_object_mut() {
        map.insert("schema".into(), SCHEMA.into());
    }
    Ok(value)
}

/// Everything of Bedrock read from the disk, with what couldn't be read said in `problems`.
#[derive(Debug, Default)]
pub(super) struct Loaded {
    pub instances: Vec<InstanceRecord>,
    pub stores: Vec<StoreId>,
    pub problems: Vec<String>,
    /// The folder was made by a newer BlockForge: it is read but not written.
    pub read_only: Option<String>,
}

/// Read every record under the root. Missing folders are just empty; a record that can't be read, or that names a
/// profile other than its own file's, is left out and said.
pub(super) fn load_all(paths: &Paths) -> Loaded {
    let mut out = Loaded::default();
    match read_json::<RootSchema>(&paths.schema_file()) {
        Ok(Some(s)) if s.schema > SCHEMA => {
            out.read_only = Some(format!(
                "Bedrock's data folder was written by a newer BlockForge (layout {}). Update BlockForge to change it.",
                s.schema
            ));
        }
        Ok(_) => {}
        Err(e) => out.problems.push(e.message),
    }
    for path in json_files(&paths.stores(), true) {
        let file = path.join("slot.json");
        match read_record::<StoreRecord>(&file) {
            Ok(Some(r)) if path.file_name().is_some_and(|n| n.to_string_lossy() == r.id.0.to_string()) => {
                out.stores.push(r.id)
            }
            Ok(Some(_)) => out.problems.push(format!("{} names another Store slot; left out", file.display())),
            Ok(None) => {}
            Err(e) => out.problems.push(e),
        }
    }
    for file in json_files(&paths.instances(), false) {
        let mut rec = match read_record::<InstanceRecord>(&file) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => {
                out.problems.push(e);
                continue;
            }
        };
        if file.file_stem().is_none_or(|s| s.to_string_lossy() != rec.id.0.to_string()) {
            out.problems.push(format!("{} names another instance; left out", file.display()));
            continue;
        }
        // a build named in a damaged record is never used as a path
        if rec.installed.as_ref().is_some_and(|b| check_build(b).is_err()) {
            out.problems
                .push(format!("{}: the installed build's name isn't valid; it will be installed again", rec.name));
            rec.installed = None;
        }
        if check_selection(&rec.selection).is_err() {
            rec.selection = BuildSelection::Latest(rec.edition());
        }
        if !out.stores.contains(&rec.store) {
            // the slot's folder went missing: it is made again on the next sign-in
            out.stores.push(rec.store.clone());
        }
        out.instances.push(rec);
    }
    // the oldest first, as they were made
    out.instances.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.name.cmp(&b.name)));
    out
}

fn read_record<T: DeserializeOwned>(path: &Path) -> std::result::Result<Option<T>, String> {
    let value = match read_json::<serde_json::Value>(path) {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e.message),
    };
    let value = migrate(value).map_err(|why| format!("{}: {why}", path.display()))?;
    serde_json::from_value(value).map(Some).map_err(|e| format!("{} can't be read: {e}", path.display()))
}

/// The `.json` files of a folder (or with `dirs`, its folders), skipping temporary files.
fn json_files(dir: &Path, dirs: bool) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') {
                return false;
            }
            if dirs { p.is_dir() } else { p.extension().is_some_and(|x| x == "json") && p.is_file() }
        })
        .collect()
}

/// Make a new instance on disk: the Store slot first when it is new, then the profile, and the instance's own record
/// last, so an instance is never there without its profile. When a step fails, what this call made is removed.
pub(super) fn create(paths: &Paths, rec: &InstanceRecord, new_store: bool) -> Result<()> {
    if read_json::<RootSchema>(&paths.schema_file())?.is_none() {
        write_json(&paths.schema_file(), &RootSchema { schema: SCHEMA })?;
    }
    let store_dir = paths.store(&rec.store);
    let profile_dir = paths.profile(&rec.profile);
    let made = (|| {
        if new_store {
            private_dir(&store_dir)?;
            write_json(
                &paths.store_file(&rec.store),
                &StoreRecord { schema: SCHEMA, id: rec.store.clone(), created: rec.created },
            )?;
        }
        private_dir(&profile_dir)?;
        write_json(
            &paths.profile_file(&rec.profile),
            &ProfileRecord { schema: SCHEMA, id: rec.profile.clone(), instance: rec.id.clone(), created: rec.created },
        )?;
        write_json(&paths.instance_file(&rec.id), rec)
    })();
    if made.is_err() {
        let _ = std::fs::remove_dir_all(&profile_dir);
        if new_store {
            let _ = std::fs::remove_dir_all(&store_dir);
        }
    }
    made
}

/// Save an instance's record again (after a rename, a new build, a game played).
pub(super) fn save(paths: &Paths, rec: &InstanceRecord) -> Result<()> {
    write_json(&paths.instance_file(&rec.id), rec)
}

/// Remove an instance: its record first (from then on it is gone, even if the rest fails), then its profile with the
/// Wine prefix and the worlds in it. The Store slot stays: other instances, or a later one, sign in with it.
///
/// The profile's launch lease is taken first and kept until the profile is gone: a game this process doesn't know of
/// (one a closed window left running) holds it through its supervisor, and its Wine prefix is not to be deleted under
/// it. `Busy` then, with nothing removed.
pub(super) fn remove(paths: &Paths, rec: &InstanceRecord) -> Result<()> {
    let profile = paths.profile(&rec.profile);
    #[cfg(unix)]
    let _lease = if profile.is_dir() {
        use super::supervisor::lease::{Kind, Lease, LeaseError};
        match Lease::try_take(&paths.launch_lease(&rec.profile), Kind::Exclusive) {
            Ok(lease) => Some(lease),
            Err(LeaseError::Busy(_)) => {
                return Err(BedrockError::new(
                    ErrorKind::Busy,
                    "This instance's game or another job is using its Wine prefix. Close the game first.",
                ));
            }
            Err(e) => return Err(BedrockError::new(ErrorKind::Internal, e.to_string())),
        }
    } else {
        None
    };
    let file = paths.instance_file(&rec.id);
    match std::fs::remove_file(&file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_error("remove", &file, e)),
    }
    match std::fs::remove_dir_all(&profile) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error("remove", &profile, e)),
    }
}

/// An OS file lock held until this is dropped. Kept off by another BlockForge process holding the same lock; the
/// system lets go of it when a process ends, however it ends.
#[derive(Debug)]
pub(super) struct FileLock {
    _file: File,
}

/// Take `locks/<name>.lock` without waiting: `Busy` when another process holds it.
pub(super) fn try_lock(paths: &Paths, name: &str) -> Result<FileLock> {
    let dir = paths.locks();
    std::fs::create_dir_all(&dir).map_err(|e| io_error("make", &dir, e))?;
    let path = dir.join(format!("{}.lock", component(name)?));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| io_error("open", &path, e))?;
    match file.try_lock() {
        Ok(()) => Ok(FileLock { _file: file }),
        Err(TryLockError::WouldBlock) => Err(BedrockError::new(
            ErrorKind::Busy,
            "Another BlockForge window is working on this Bedrock instance. Try again when it is done.",
        )),
        Err(TryLockError::Error(e)) => Err(io_error("lock", &path, e)),
    }
}

/// Copy the folder `from` into `to` (made new), files and folders only: links are not followed, so nothing outside
/// is copied. Returns how many files were copied.
pub(super) fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    std::fs::create_dir_all(to).map_err(|e| io_error("make", to, e))?;
    let mut files = 0;
    let rd = std::fs::read_dir(from).map_err(|e| io_error("read", from, e))?;
    for entry in rd {
        let entry = entry.map_err(|e| io_error("read", from, e))?;
        let kind = entry.file_type().map_err(|e| io_error("read", &entry.path(), e))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            files += copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map_err(|e| io_error("copy", &entry.path(), e))?;
            files += 1;
        }
    }
    Ok(files)
}

pub(super) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(paths: &Paths) -> InstanceRecord {
        let _ = paths;
        InstanceRecord {
            schema: SCHEMA,
            id: InstanceId(uuid::Uuid::new_v4()),
            name: "Bedrock".into(),
            profile: ProfileId(uuid::Uuid::new_v4()),
            store: StoreId(uuid::Uuid::new_v4()),
            selection: BuildSelection::Latest(Edition::Release),
            installed: None,
            created: 1,
            last_played: None,
            backup_before_play: false,
        }
    }

    #[test]
    fn names_that_could_leave_their_folder_are_refused() {
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "C:", "CON", "nul.txt", "COM1", "lpt9", ".hidden", "a b"] {
            assert!(component(bad).is_err(), "{bad:?}");
        }
        for good in ["1.26.5004.0", "Microsoft.MinecraftUWP_1.26.5004.0_x64__8wekyb3d8bbwe", "COMPUTER", "release"] {
            assert_eq!(component(good).unwrap(), good);
        }
        assert!(component(&"a".repeat(129)).is_err());
        let paths = Paths::at("/data/bedrock");
        let bad = BuildId { edition: Edition::Release, version: "..".into(), package_id: "x".into() };
        assert!(paths.build(&bad).is_err());
        let good = BuildId { edition: Edition::Preview, version: "1.26.70.22".into(), package_id: "p".into() };
        assert_eq!(paths.build(&good).unwrap(), PathBuf::from("/data/bedrock/builds/preview/1.26.70.22/p"));
    }

    #[test]
    fn an_instance_name_is_trimmed_and_kept_sensible() {
        assert_eq!(instance_name("  My world ").unwrap(), "My world");
        assert!(instance_name("   ").is_err());
        assert!(instance_name(&"x".repeat(65)).is_err());
        assert!(instance_name("a\nb").is_err());
    }

    #[test]
    fn records_are_written_whole_and_read_back() {
        let root = crate::testutil::scratch("bedrock-storage");
        let paths = Paths::at(&root);
        let rec = record(&paths);
        create(&paths, &rec, true).unwrap();
        assert!(paths.store_file(&rec.store).is_file());
        assert!(paths.profile_file(&rec.profile).is_file());
        assert_eq!(read_json::<RootSchema>(&paths.schema_file()).unwrap(), Some(RootSchema { schema: SCHEMA }));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&paths.profile(&rec.profile)), 0o700);
            assert_eq!(mode(&paths.store(&rec.store)), 0o700);
            assert_eq!(mode(&paths.instance_file(&rec.id)), 0o600);
        }
        // no temporary file is left beside it
        let left: Vec<_> = std::fs::read_dir(paths.instances()).unwrap().flatten().collect();
        assert_eq!(left.len(), 1);
        let loaded = load_all(&paths);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert_eq!(loaded.instances.len(), 1);
        assert_eq!(loaded.instances[0].id, rec.id);
        assert_eq!(loaded.stores, vec![rec.store.clone()]);
        remove(&paths, &rec).unwrap();
        assert!(!paths.profile(&rec.profile).exists());
        assert!(paths.store(&rec.store).exists(), "the Store slot outlives the instance");
        assert!(load_all(&paths).instances.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    /// A game a closed window left running holds the prefix's launch lease: deleting its instance is refused and
    /// nothing is removed; once the lease is free the instance goes.
    #[cfg(unix)]
    #[test]
    fn removing_an_instance_waits_for_the_prefixs_launch_lease() {
        use super::super::supervisor::lease::{Kind, Lease};
        let root = crate::testutil::scratch("bedrock-remove-lease");
        let paths = Paths::at(&root);
        let rec = record(&paths);
        create(&paths, &rec, true).unwrap();
        std::fs::create_dir_all(paths.profile(&rec.profile).join("compat/pfx")).unwrap();
        let held = Lease::try_take(&paths.launch_lease(&rec.profile), Kind::Exclusive).unwrap();
        let refused = remove(&paths, &rec).unwrap_err();
        assert!(matches!(refused.kind, ErrorKind::Busy), "{refused}");
        assert!(paths.instance_file(&rec.id).exists(), "the record stays");
        assert!(paths.profile(&rec.profile).join("compat/pfx").exists(), "the prefix stays");
        drop(held);
        remove(&paths, &rec).unwrap();
        assert!(!paths.profile(&rec.profile).exists() && !paths.instance_file(&rec.id).exists());
        // removing what is already gone is not an error, and makes nothing
        remove(&paths, &rec).unwrap();
        assert!(!paths.profile(&rec.profile).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn damaged_newer_and_misnamed_records_are_left_out_and_said() {
        let root = crate::testutil::scratch("bedrock-load");
        let paths = Paths::at(&root);
        let good = record(&paths);
        create(&paths, &good, true).unwrap();
        std::fs::write(paths.instances().join("broken.json"), b"{ not json").unwrap();
        let mut newer = serde_json::to_value(record(&paths)).unwrap();
        newer["schema"] = 9.into();
        let id = newer["id"].as_str().unwrap().to_string();
        std::fs::write(paths.instances().join(format!("{id}.json")), newer.to_string()).unwrap();
        let other = record(&paths);
        std::fs::write(paths.instances().join("not-its-id.json"), serde_json::to_vec(&other).unwrap()).unwrap();
        // a record from before the schema field, with a build whose name would leave its folder
        let mut old = serde_json::to_value(record(&paths)).unwrap();
        old.as_object_mut().unwrap().remove("schema");
        old["installed"] = serde_json::json!({"edition": "Release", "version": "../..", "package_id": "x"});
        let old_id = old["id"].as_str().unwrap().to_string();
        std::fs::write(paths.instances().join(format!("{old_id}.json")), old.to_string()).unwrap();
        let loaded = load_all(&paths);
        let ids: Vec<String> = loaded.instances.iter().map(|r| r.id.0.to_string()).collect();
        assert!(ids.contains(&good.id.0.to_string()) && ids.contains(&old_id), "{ids:?}");
        assert_eq!(loaded.instances.len(), 2);
        let migrated = loaded.instances.iter().find(|r| r.id.0.to_string() == old_id).unwrap();
        assert_eq!(migrated.schema, SCHEMA);
        assert!(migrated.installed.is_none());
        assert_eq!(loaded.problems.len(), 4, "{:?}", loaded.problems);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_from_a_newer_blockforge_is_read_only() {
        let root = crate::testutil::scratch("bedrock-newer");
        let paths = Paths::at(&root);
        write_json(&paths.schema_file(), &RootSchema { schema: SCHEMA + 1 }).unwrap();
        assert!(load_all(&paths).read_only.is_some());
        assert!(migrate(serde_json::json!({"schema": SCHEMA + 1})).is_err());
        assert_eq!(migrate(serde_json::json!({})).unwrap()["schema"], SCHEMA);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_lock_keeps_a_second_holder_off_until_it_is_dropped() {
        let root = crate::testutil::scratch("bedrock-lock");
        let paths = Paths::at(&root);
        let held = try_lock(&paths, "instance-a").unwrap();
        // a second open file description is what another process would have
        let again = try_lock(&paths, "instance-a").unwrap_err();
        assert!(matches!(again.kind, ErrorKind::Busy));
        drop(held);
        assert!(try_lock(&paths, "instance-a").is_ok());
        assert!(try_lock(&paths, "../x").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn copying_worlds_takes_files_and_folders() {
        let root = crate::testutil::scratch("bedrock-copy");
        let from = root.join("Users/Shared/games/com.mojang/minecraftWorlds/abc");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("levelname.txt"), b"My world").unwrap();
        std::fs::write(from.join("level.dat"), b"x").unwrap();
        let n = copy_tree(&root.join("Users"), &root.join("backup")).unwrap();
        assert_eq!(n, 2);
        assert!(root.join("backup/Shared/games/com.mojang/minecraftWorlds/abc/levelname.txt").is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
