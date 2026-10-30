//! Menu entry and icons for the AppImage.
//!
//! The deb and rpm packages install a menu entry, icons and a MIME type themselves. An AppImage is just a file,
//! so the first time one is run it puts the same things in the user's own folders (`~/.local/share/applications`,
//! `~/.local/share/icons/hicolor`, `~/.local/share/mime`). Every start it checks that the entry still points at the
//! AppImage that is running, so moving the file fixes the entry the next time it is started from its new place.
//!
//! Nothing happens when the program was not started from an AppImage (no `APPIMAGE` variable), when a package
//! already installed an entry, when AppImageLauncher or appimaged manage the file, when the user has an entry of
//! their own named `blockforge.desktop`, or when `desktopIntegration` is `false` in the launcher settings.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Settings;

/// The name the packages use too, so a package entry and this one are the same menu entry.
const DESKTOP_FILE: &str = "blockforge.desktop";
const ICON_NAME: &str = "blockforge";
const MIME_FILE: &str = "blockforge.xml";
/// Marks an entry as written by this program, so it may be rewritten when the AppImage moves.
const MARKER: &str = "X-BlockForge-Integration=AppImage";

const ICONS: &[(u32, &[u8])] = &[
    (16, include_bytes!("../docs/logo/png/blockforge-16.png")),
    (24, include_bytes!("../docs/logo/png/blockforge-24.png")),
    (32, include_bytes!("../docs/logo/png/blockforge-32.png")),
    (48, include_bytes!("../docs/logo/png/blockforge-48.png")),
    (64, include_bytes!("../docs/logo/png/blockforge-64.png")),
    (128, include_bytes!("../docs/logo/png/blockforge-128.png")),
    (256, include_bytes!("../docs/logo/png/blockforge-256.png")),
    (512, include_bytes!("../docs/logo/png/blockforge-512.png")),
];
const ICON_SVG: &[u8] = include_bytes!("../docs/logo/blockforge-icon.svg");
const MIME_XML: &[u8] = include_bytes!("../packaging/linux/blockforge-mime.xml");

/// What the integration found out about the computer it runs on.
#[derive(Clone, Debug)]
pub struct Context {
    /// The AppImage file that is running (absolute).
    pub appimage: PathBuf,
    /// `$XDG_DATA_HOME`, usually `~/.local/share`.
    pub data_home: PathBuf,
    /// `$XDG_DATA_DIRS`, where a package would have put its menu entry.
    pub system_dirs: Vec<PathBuf>,
    /// AppImageLauncher or appimaged started this program and look after the menu entry.
    pub helper_runs_it: bool,
}

/// What `integrate` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A package (deb, rpm, ...) has installed a menu entry already.
    PackageEntry,
    /// AppImageLauncher or appimaged made an entry.
    HelperManagesIt,
    /// The user has a `blockforge.desktop` that this program did not write; it is left alone.
    UserEntry,
    /// Everything is in place and points at this AppImage.
    UpToDate,
    /// The menu entry was created.
    Installed,
    /// The menu entry was there, but pointed somewhere else (the AppImage moved), or an icon was missing.
    Updated,
}

impl Context {
    /// `None` unless this is an AppImage on Linux: no `APPIMAGE` variable, or one that does not name a file.
    pub fn from_env() -> Option<Context> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let appimage = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()).map(PathBuf::from)?;
        let appimage = if appimage.is_absolute() { appimage } else { std::env::current_dir().ok()?.join(appimage) };
        if !appimage.is_file() {
            return None;
        }
        let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local").join("share")))?;
        let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
        let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        Some(Context {
            appimage,
            data_home,
            system_dirs: system_dirs(&dirs, appdir.as_deref()),
            helper_runs_it: std::env::var_os("DESKTOPINTEGRATION").is_some(),
        })
    }
}

/// The folders of `$XDG_DATA_DIRS` (or the standard two when it is empty), without anything inside the AppImage's
/// own mount, because the entry in there is ours and not a package's.
pub(crate) fn system_dirs(xdg_data_dirs: &str, appdir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = xdg_data_dirs.split(':').filter(|d| d.starts_with('/')).map(PathBuf::from).collect();
    if dirs.is_empty() {
        dirs = vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")];
    }
    dirs.retain(|d| appdir.is_none_or(|a| !d.starts_with(a)));
    dirs
}

/// Whether `Settings` allows it: `desktopIntegration` in the settings file, on unless it is `false`.
pub fn enabled(settings: &Settings) -> bool {
    settings.extras.get("desktopIntegration").and_then(|v| v.as_bool()).unwrap_or(true)
}

/// Quotes a program path for the `Exec` key. Plain paths stay as they are; anything with a space or a character the
/// desktop entry format treats specially is put in double quotes with the required escapes (which are escaped once
/// more because the key is a string value).
pub fn exec_quote(path: &str) -> String {
    let plain = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ',' | ':' | '@' | '='));
    if plain {
        return path.to_string();
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '\\' => out.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text of the menu entry for an AppImage at `appimage`.
pub fn desktop_entry(appimage: &Path) -> String {
    let path = appimage.to_string_lossy();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=BlockForge\n\
         GenericName=Minecraft Launcher\n\
         Comment=A fast, modern Minecraft launcher\n\
         Exec={exec} %F\n\
         TryExec={path}\n\
         Icon={ICON_NAME}\n\
         Terminal=false\n\
         Categories=Game;ActionGame;AdventureGame;Simulation;\n\
         Keywords=Minecraft;Launcher;Mods;Modpacks;Modrinth;Java;\n\
         MimeType=application/x-modrinth-modpack+zip;\n\
         StartupWMClass=blockforge\n\
         {MARKER}\n",
        exec = exec_quote(&path),
    )
}

/// Whether `file` is a menu entry that AppImageLauncher or appimaged wrote for BlockForge (they name theirs
/// `appimagekit_<hash>-<AppName>.desktop`).
fn is_helper_entry(file_name: &OsStr) -> bool {
    let name = file_name.to_string_lossy().to_ascii_lowercase();
    name.starts_with("appimagekit_") && name.contains("blockforge") && name.ends_with(".desktop")
}

fn helper_entry_exists(applications: &Path) -> bool {
    fs::read_dir(applications).map(|rd| rd.flatten().any(|e| is_helper_entry(&e.file_name()))).unwrap_or(false)
}

/// Writes `data` to `path` through a temporary file, unless the file already holds exactly that. `true` if it wrote.
pub(crate) fn write_if_changed(path: &Path, data: &[u8]) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|old| old == data) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(true)
}

/// Puts the menu entry, icons and MIME type in place for `ctx`. Returns what it did; `refresh` runs the
/// `update-desktop-database`, `gtk-update-icon-cache` and `update-mime-database` programs afterwards, when there
/// are any. Those are only a courtesy: a failure there never fails the integration.
pub fn integrate(ctx: &Context, refresh: bool) -> io::Result<Outcome> {
    if ctx.helper_runs_it {
        return Ok(Outcome::HelperManagesIt);
    }
    if ctx.system_dirs.iter().any(|d| d.join("applications").join(DESKTOP_FILE).is_file()) {
        return Ok(Outcome::PackageEntry);
    }
    let applications = ctx.data_home.join("applications");
    if helper_entry_exists(&applications) {
        return Ok(Outcome::HelperManagesIt);
    }
    let entry = applications.join(DESKTOP_FILE);
    let want = desktop_entry(&ctx.appimage);
    let existing = fs::read_to_string(&entry).ok();
    if let Some(old) = &existing
        && !old.contains(MARKER)
    {
        return Ok(Outcome::UserEntry);
    }

    let mut entry_changed = false;
    let mut icons_changed = false;
    let mut mime_changed = false;
    entry_changed |= write_if_changed(&entry, want.as_bytes())?;
    let hicolor = ctx.data_home.join("icons").join("hicolor");
    for (size, data) in ICONS {
        let file = hicolor.join(format!("{size}x{size}")).join("apps").join(format!("{ICON_NAME}.png"));
        icons_changed |= write_if_changed(&file, data)?;
    }
    icons_changed |=
        write_if_changed(&hicolor.join("scalable").join("apps").join(format!("{ICON_NAME}.svg")), ICON_SVG)?;
    let mime_dir = ctx.data_home.join("mime");
    mime_changed |= write_if_changed(&mime_dir.join("packages").join(MIME_FILE), MIME_XML)?;

    if refresh {
        if entry_changed {
            run_quietly("update-desktop-database", [applications.as_os_str()]);
        }
        if icons_changed {
            // GTK only needs the cache refreshed when there is one; without it the icons are found by looking
            if hicolor.join("icon-theme.cache").exists() {
                for tool in ["gtk-update-icon-cache", "gtk4-update-icon-cache"] {
                    run_quietly(tool, [OsStr::new("-q"), OsStr::new("-t"), OsStr::new("-f"), hicolor.as_os_str()]);
                }
            }
        }
        if mime_changed {
            run_quietly("update-mime-database", [mime_dir.as_os_str()]);
        }
    }

    Ok(match (existing.is_some(), entry_changed || icons_changed || mime_changed) {
        (false, _) => Outcome::Installed,
        (true, true) => Outcome::Updated,
        (true, false) => Outcome::UpToDate,
    })
}

/// Runs a helper program, ignoring a missing program, a failure and anything it prints.
pub(crate) fn run_quietly<'a>(program: &str, args: impl IntoIterator<Item = &'a OsStr>) {
    let _ = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Called once when the window is created: does the integration in the background when this is an AppImage and
/// the user has not turned it off.
pub fn setup(settings: &Settings) {
    if !enabled(settings) {
        return;
    }
    let Some(ctx) = Context::from_env() else { return };
    let _ = std::thread::Builder::new().name("desktop-integration".into()).spawn(move || {
        if let Err(e) = integrate(&ctx, true) {
            eprintln!("desktop integration: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("blockforge-di-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn path(&self, p: &str) -> PathBuf {
            self.0.join(p)
        }
        fn ctx(&self) -> Context {
            let appimage = self.path("apps/BlockForge-4.0.0-linux-x86_64.AppImage");
            fs::create_dir_all(appimage.parent().unwrap()).unwrap();
            fs::write(&appimage, b"x").unwrap();
            Context {
                appimage,
                data_home: self.path("home/.local/share"),
                system_dirs: vec![self.path("usr/local/share"), self.path("usr/share")],
                helper_runs_it: false,
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn plain_paths_are_not_quoted() {
        assert_eq!(exec_quote("/home/me/Apps/BlockForge-4.0.0.AppImage"), "/home/me/Apps/BlockForge-4.0.0.AppImage");
    }

    #[test]
    fn paths_with_spaces_and_specials_are_quoted_and_escaped() {
        assert_eq!(exec_quote("/home/me/My Games/bf.AppImage"), "\"/home/me/My Games/bf.AppImage\"");
        assert_eq!(exec_quote("/a/b$c"), "\"/a/b\\\\$c\"");
        assert_eq!(exec_quote("/a/100%/b"), "\"/a/100%%/b\"");
        assert_eq!(exec_quote("/a/\"q\""), "\"/a/\\\\\"q\\\\\"\"");
        assert_eq!(exec_quote("/a/b\\c"), "\"/a/b\\\\\\\\c\"");
    }

    #[test]
    fn desktop_entry_points_at_the_appimage() {
        let text = desktop_entry(Path::new("/home/me/Apps/BlockForge.AppImage"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("\nExec=/home/me/Apps/BlockForge.AppImage %F\n"));
        assert!(text.contains("\nTryExec=/home/me/Apps/BlockForge.AppImage\n"));
        assert!(text.contains("\nIcon=blockforge\n"));
        assert!(text.contains("\nStartupWMClass=blockforge\n"));
        assert!(text.contains("\nTerminal=false\n"));
        assert!(text.contains(MARKER));
        let spaced = desktop_entry(Path::new("/home/me/My Apps/BlockForge.AppImage"));
        assert!(spaced.contains("\nExec=\"/home/me/My Apps/BlockForge.AppImage\" %F\n"));
        assert!(spaced.contains("\nTryExec=/home/me/My Apps/BlockForge.AppImage\n"));
    }

    #[test]
    fn the_setting_defaults_to_on() {
        let mut s = Settings::default();
        assert!(enabled(&s));
        s.extras.insert("desktopIntegration".into(), serde_json::Value::Bool(false));
        assert!(!enabled(&s));
        s.extras.insert("desktopIntegration".into(), serde_json::Value::Bool(true));
        assert!(enabled(&s));
    }

    #[test]
    fn system_dirs_skip_the_appimage_mount_and_relative_entries() {
        let dirs = system_dirs(
            "/tmp/.mount_x/usr/share:/usr/local/share::rel/share:/usr/share",
            Some(Path::new("/tmp/.mount_x")),
        );
        assert_eq!(dirs, vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]);
        assert_eq!(system_dirs("", None), vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]);
    }

    #[test]
    fn first_run_writes_entry_icons_and_mime() {
        let s = Scratch::new("first");
        let ctx = s.ctx();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::Installed);
        let entry = fs::read_to_string(ctx.data_home.join("applications/blockforge.desktop")).unwrap();
        assert_eq!(entry, desktop_entry(&ctx.appimage));
        for (size, data) in ICONS {
            let icon = ctx.data_home.join(format!("icons/hicolor/{size}x{size}/apps/blockforge.png"));
            assert_eq!(fs::read(icon).unwrap(), *data);
        }
        assert!(ctx.data_home.join("icons/hicolor/scalable/apps/blockforge.svg").is_file());
        assert!(ctx.data_home.join("mime/packages/blockforge.xml").is_file());
    }

    #[test]
    fn running_again_changes_nothing() {
        let s = Scratch::new("again");
        let ctx = s.ctx();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::Installed);
        let entry = ctx.data_home.join("applications/blockforge.desktop");
        let before = fs::metadata(&entry).unwrap().modified().unwrap();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::UpToDate);
        assert_eq!(fs::metadata(&entry).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn moving_the_appimage_updates_the_exec_path() {
        let s = Scratch::new("moved");
        let mut ctx = s.ctx();
        integrate(&ctx, false).unwrap();
        let moved = s.path("elsewhere/BlockForge.AppImage");
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(&ctx.appimage, &moved).unwrap();
        ctx.appimage = moved.clone();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::Updated);
        let entry = fs::read_to_string(ctx.data_home.join("applications/blockforge.desktop")).unwrap();
        assert!(entry.contains(&format!("Exec={} %F", moved.display())));
        assert!(!entry.contains("apps/BlockForge-4.0.0"));
    }

    #[test]
    fn a_missing_icon_is_put_back() {
        let s = Scratch::new("icon");
        let ctx = s.ctx();
        integrate(&ctx, false).unwrap();
        fs::remove_file(ctx.data_home.join("icons/hicolor/48x48/apps/blockforge.png")).unwrap();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::Updated);
        assert!(ctx.data_home.join("icons/hicolor/48x48/apps/blockforge.png").is_file());
    }

    #[test]
    fn a_package_entry_means_nothing_is_written() {
        let s = Scratch::new("pkg");
        let ctx = s.ctx();
        fs::create_dir_all(s.path("usr/share/applications")).unwrap();
        fs::write(s.path("usr/share/applications/blockforge.desktop"), "[Desktop Entry]\n").unwrap();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::PackageEntry);
        assert!(!ctx.data_home.exists());
    }

    #[test]
    fn appimage_helpers_are_left_to_do_their_job() {
        let s = Scratch::new("helper");
        let mut ctx = s.ctx();
        fs::create_dir_all(ctx.data_home.join("applications")).unwrap();
        fs::write(ctx.data_home.join("applications/appimagekit_0a1b2c-BlockForge.desktop"), "[Desktop Entry]\n")
            .unwrap();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::HelperManagesIt);
        assert!(!ctx.data_home.join("applications/blockforge.desktop").exists());
        assert!(!ctx.data_home.join("icons").exists());

        let other = Scratch::new("helper-env");
        ctx = other.ctx();
        ctx.helper_runs_it = true;
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::HelperManagesIt);
        assert!(!ctx.data_home.exists());
    }

    #[test]
    fn other_appimages_from_a_helper_do_not_count() {
        assert!(is_helper_entry(OsStr::new("appimagekit_0a1b2c-BlockForge.desktop")));
        assert!(!is_helper_entry(OsStr::new("appimagekit_0a1b2c-Other.desktop")));
        assert!(!is_helper_entry(OsStr::new("blockforge.desktop")));
    }

    #[test]
    fn an_entry_the_user_made_is_not_touched() {
        let s = Scratch::new("user");
        let ctx = s.ctx();
        fs::create_dir_all(ctx.data_home.join("applications")).unwrap();
        let mine = ctx.data_home.join("applications/blockforge.desktop");
        fs::write(&mine, "[Desktop Entry]\nName=Mine\nExec=/opt/mine\n").unwrap();
        assert_eq!(integrate(&ctx, false).unwrap(), Outcome::UserEntry);
        assert_eq!(fs::read_to_string(&mine).unwrap(), "[Desktop Entry]\nName=Mine\nExec=/opt/mine\n");
        assert!(!ctx.data_home.join("icons").exists());
    }

    #[test]
    fn no_appimage_variable_means_no_context() {
        // the test program is not started from an AppImage
        if std::env::var_os("APPIMAGE").is_none() {
            assert!(Context::from_env().is_none());
        }
    }
}
