//! Whether the game's files are on a spinning hard disk.
//!
//! A hard disk moves a head to every file it reads, about 10 ms each: starting a game touches thousands of files, so
//! the same start takes seconds on an SSD and 40 s on a hard disk. On one the launcher reads in a single sweep, one
//! file at a time, in the order the files lie on the disk, and skips checks that would visit every file.

use std::path::Path;
use std::sync::OnceLock;

use crate::config::Settings;

/// The setting `spinningDisk` (in the settings file): `true`/`false` to say what kind of disk this is, anything else
/// (or nothing) to find out. For computers where the detection guesses wrong.
pub fn forced(settings: &Settings) -> Option<bool> {
    settings.extras.get("spinningDisk").and_then(|v| v.as_bool())
}

/// Whether `path` is on a spinning disk; `forced` wins when the player has said. Found out once per run.
pub fn is_spinning(settings: &Settings, path: &Path) -> bool {
    if let Some(f) = forced(settings) {
        return f;
    }
    static ANSWER: OnceLock<bool> = OnceLock::new();
    *ANSWER.get_or_init(|| detect(path))
}

#[cfg(target_os = "linux")]
fn detect(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(path) else { return false };
    spinning_from_sys(device_numbers(meta.dev()), Path::new("/sys/dev/block"))
}

/// The major and minor number of a device id, as Linux splits them.
#[cfg(any(target_os = "linux", test))]
fn device_numbers(dev: u64) -> (u64, u64) {
    ((dev >> 8) & 0xfff | (dev >> 32) & !0xfff, dev & 0xff | (dev >> 12) & !0xff)
}

/// Reads `queue/rotational` of the disk with this device number, looking at the whole disk when it is a partition.
#[cfg(any(target_os = "linux", test))]
fn spinning_from_sys((major, minor): (u64, u64), sys_block: &Path) -> bool {
    let dir = sys_block.join(format!("{major}:{minor}"));
    for file in [dir.join("queue/rotational"), dir.join("../queue/rotational")] {
        if let Ok(text) = std::fs::read_to_string(&file) {
            return text.trim() == "1";
        }
    }
    false
}

/// Windows: asks PowerShell which kind the drive is. That takes a second, so the answer is kept for a month.
#[cfg(windows)]
fn detect(path: &Path) -> bool {
    use std::io::Write;
    let drive = path.to_string_lossy().chars().next().filter(|c| c.is_ascii_alphabetic());
    let Some(drive) = drive else { return false };
    let cache = path.join("cache").join("disk-type.txt");
    let stamp = format!("{}", drive.to_ascii_uppercase());
    if let (Ok(text), Ok(meta)) = (std::fs::read_to_string(&cache), std::fs::metadata(&cache)) {
        let fresh = meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|e| e.as_secs() < 30 * 86_400);
        let mut parts = text.split_whitespace();
        if fresh && parts.next() == Some(stamp.as_str()) {
            return parts.next() == Some("HDD");
        }
    }
    let script = format!("(Get-Partition -DriveLetter {drive} | Get-Disk | Get-PhysicalDisk).MediaType");
    let out = crate::platform::command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    // unknown ("Unspecified", nothing at all) counts as fast: the careful reading only helps on a real hard disk
    let kind = if out.eq_ignore_ascii_case("HDD") { "HDD" } else { "OTHER" };
    if let Ok(mut f) = std::fs::File::create(&cache) {
        let _ = write!(f, "{stamp} {kind}");
    }
    kind == "HDD"
}

/// macOS computers with a hard disk are rare and old; everything else is treated as fast too.
#[cfg(not(any(target_os = "linux", windows)))]
fn detect(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_numbers_split_like_linux() {
        // /dev/sda1 is 8:1, /dev/nvme0n1p4 is 259:4
        assert_eq!(device_numbers((8 << 8) | 1), (8, 1));
        assert_eq!(device_numbers((259 << 8) | 4), (259, 4));
    }

    #[test]
    fn rotational_flag_is_read_from_the_disk_or_its_partition() {
        let base = std::env::temp_dir().join(format!("blockforge-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // a partition (8:1) has no queue of its own; the disk above it (8:0) does, and the partition's folder sits
        // inside the disk's, like in /sys/devices
        let disk = base.join("devices/sda");
        std::fs::create_dir_all(disk.join("queue")).unwrap();
        std::fs::create_dir_all(disk.join("sda1")).unwrap();
        std::fs::write(disk.join("queue/rotational"), "1\n").unwrap();
        let blocks = base.join("dev/block");
        std::fs::create_dir_all(&blocks).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(disk.join("sda1"), blocks.join("8:1")).unwrap();
            assert!(spinning_from_sys((8, 1), &blocks));
            std::fs::write(disk.join("queue/rotational"), "0\n").unwrap();
            assert!(!spinning_from_sys((8, 1), &blocks));
        }
        assert!(!spinning_from_sys((9, 9), &blocks), "an unknown device counts as fast");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_setting_wins() {
        let mut s = Settings::default();
        assert_eq!(forced(&s), None);
        s.extras.insert("spinningDisk".into(), serde_json::Value::Bool(true));
        assert!(is_spinning(&s, Path::new("/")));
        s.extras.insert("spinningDisk".into(), serde_json::Value::Bool(false));
        assert!(!is_spinning(&s, Path::new("/")));
    }
}
