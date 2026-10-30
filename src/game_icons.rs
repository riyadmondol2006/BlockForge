//! GNOME matches native Wayland windows to desktop entries rather than their window icons.

use crate::{config::Settings, paths::Dirs};

/// Best effort: desktop integration must never prevent a game from starting.
pub fn ensure_minecraft(settings: &Settings, dirs: &Dirs, index_id: &str, snapshot: bool) {
    #[cfg(target_os = "linux")]
    linux::ensure(settings, linux::Game::Minecraft, Some((dirs, index_id, snapshot)));
    #[cfg(not(target_os = "linux"))]
    let _ = (settings, dirs, index_id, snapshot);
}

/// Call immediately before starting the Horizon client, with the launcher's settings.
pub fn ensure_horizon(settings: &Settings) {
    #[cfg(target_os = "linux")]
    linux::ensure(settings, linux::Game::Horizon, None);
    #[cfg(not(target_os = "linux"))]
    let _ = settings;
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::desktop_integration::{enabled, exec_quote, run_quietly, system_dirs, write_if_changed};
    use std::{
        ffi::OsStr,
        fs, io,
        path::{Path, PathBuf},
        time::SystemTime,
    };

    const MARKER_KEY: &str = "X-BlockForge-Integration";
    const MARKER_VALUE: &str = "GameIcon";
    const HORIZON_ICONS: &[(u32, &[u8])] = &[
        (16, include_bytes!("../assets/horizon/16x16.png")),
        (24, include_bytes!("../assets/horizon/24x24.png")),
        (32, include_bytes!("../assets/horizon/32x32.png")),
        (48, include_bytes!("../assets/horizon/48x48.png")),
        (64, include_bytes!("../assets/horizon/64x64.png")),
        (128, include_bytes!("../assets/horizon/128x128.png")),
        (256, include_bytes!("../assets/horizon/256x256.png")),
        (512, include_bytes!("../assets/horizon/512x512.png")),
    ];

    #[derive(Clone, Copy)]
    pub(super) enum Game {
        Minecraft,
        Horizon,
    }

    impl Game {
        fn name(self) -> &'static str {
            match self {
                Self::Minecraft => "Minecraft",
                Self::Horizon => "Horizon",
            }
        }
        fn icon(self) -> &'static str {
            match self {
                Self::Minecraft => "blockforge-minecraft",
                Self::Horizon => "blockforge-horizon",
            }
        }
        fn app_id(self) -> &'static str {
            match self {
                Self::Minecraft => "com.mojang.minecraft",
                Self::Horizon => "horizon",
            }
        }
    }

    // Only unlocalized keys in the main group count. Comments and action groups cannot claim ownership.
    fn entry_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
        let mut main = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                main = line == "[Desktop Entry]";
                continue;
            }
            if main
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return Some(v.trim());
            }
        }
        None
    }

    fn is_ours(text: &str) -> bool {
        entry_value(text, MARKER_KEY) == Some(MARKER_VALUE)
    }

    fn claims(text: &str, desktop_id: &str, app_id: &str) -> bool {
        desktop_id == format!("{app_id}.desktop") || entry_value(text, "StartupWMClass") == Some(app_id)
    }

    fn desktop_entry(game: Game, executable: &Path) -> String {
        let comment = match game {
            Game::Minecraft => "Minecraft: Java Edition",
            Game::Horizon => "Minecraft: Java Edition rewritten in Rust, by the Horizon project",
        };
        format!(
            "[Desktop Entry]\nType=Application\nName={}\nComment={comment}\nNoDisplay=true\nStartupWMClass={}\nIcon={}\nExec={}\nTerminal=false\n{MARKER_KEY}={MARKER_VALUE}\n",
            game.name(),
            game.app_id(),
            game.icon(),
            exec_quote(&executable.to_string_lossy())
        )
    }

    fn icon_target(data_home: &Path, icon: &str, size: Option<u32>) -> PathBuf {
        let (dir, extension) = match size {
            Some(n) => (format!("{n}x{n}"), "png"),
            None => ("scalable".into(), "svg"),
        };
        data_home.join("icons/hicolor").join(dir).join("apps").join(format!("{icon}.{extension}"))
    }

    struct Context {
        data_home: PathBuf,
        data_dirs: Vec<PathBuf>,
        executable: PathBuf,
    }

    impl Context {
        fn from_env() -> io::Result<Self> {
            let data_home = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| {
                    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".local/share"))
                })
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no XDG data home"))?;
            let executable = match std::env::var_os("APPIMAGE").filter(|p| !p.is_empty()) {
                Some(p) => {
                    let p = PathBuf::from(p);
                    if p.is_absolute() { p } else { std::env::current_dir()?.join(p) }
                }
                None => std::env::current_exe()?,
            };
            Ok(Self {
                data_home,
                executable,
                data_dirs: system_dirs(&std::env::var("XDG_DATA_DIRS").unwrap_or_default(), None),
            })
        }
    }

    fn foreign_entry(applications: &Path, relative: &Path, game: Game) -> io::Result<bool> {
        let entries = match fs::read_dir(applications.join(relative)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let rel = relative.join(entry.file_name());
            // Do not follow directory symlinks: a desktop folder can contain loops.
            if entry.file_type()?.is_dir() {
                if foreign_entry(applications, &rel, game)? {
                    return Ok(true);
                }
            } else if entry.path().extension() == Some(OsStr::new("desktop")) {
                let text = fs::read_to_string(entry.path())?;
                let id = rel.to_string_lossy().replace('/', "-");
                if !is_ours(&text) && claims(&text, &id, game.app_id()) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn should_install(ctx: &Context, game: Game) -> io::Result<bool> {
        for dir in std::iter::once(&ctx.data_home).chain(&ctx.data_dirs) {
            if foreign_entry(&dir.join("applications"), Path::new(""), game)? {
                return Ok(false);
            }
        }
        let target = ctx.data_home.join("applications").join(format!("{}.desktop", game.icon()));
        match fs::read_to_string(target) {
            Ok(text) => Ok(is_ours(&text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(e),
        }
    }

    fn install(ctx: &Context, game: Game, icons: &[(Option<u32>, Vec<u8>)], refresh: bool) -> io::Result<()> {
        let mut changed = false;
        for (size, bytes) in icons {
            changed |= write_if_changed(&icon_target(&ctx.data_home, game.icon(), *size), bytes)?;
        }
        let applications = ctx.data_home.join("applications");
        let entry_changed = write_if_changed(
            &applications.join(format!("{}.desktop", game.icon())),
            desktop_entry(game, &ctx.executable).as_bytes(),
        )?;
        if refresh {
            if entry_changed {
                run_quietly("update-desktop-database", [applications.as_os_str()]);
            }
            if changed {
                let hicolor = ctx.data_home.join("icons/hicolor");
                // Invalidate directory-based icon discovery even when no GTK cache exists.
                fs::File::open(&hicolor)?.set_modified(SystemTime::now())?;
                if hicolor.join("icon-theme.cache").exists() {
                    for tool in ["gtk-update-icon-cache", "gtk4-update-icon-cache"] {
                        run_quietly(tool, [OsStr::new("-q"), OsStr::new("-t"), OsStr::new("-f"), hicolor.as_os_str()]);
                    }
                }
            }
        }
        Ok(())
    }

    // An index is local input: reject malformed hashes before slicing or constructing paths.
    fn object_path(assets: &Path, hash: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()), "invalid icon asset hash");
        Ok(assets.join("objects").join(&hash[..2]).join(hash))
    }

    fn minecraft_icons(dirs: &Dirs, index_id: &str, snapshot: bool) -> anyhow::Result<Vec<(Option<u32>, Vec<u8>)>> {
        let index = crate::install::read_asset_index(dirs, index_id)?;
        let mut icons = Vec::new();
        for size in [16, 24, 32, 48, 64, 128, 256] {
            let release = format!("icons/icon_{size}x{size}.png");
            let snapshot_key = format!("icons/snapshot/icon_{size}x{size}.png");
            let object =
                if snapshot { index.objects.get(&snapshot_key) } else { None }.or_else(|| index.objects.get(&release));
            if let Some(object) = object {
                let path = object_path(&dirs.assets, &object.hash)?;
                icons.push((Some(size), fs::read(path)?));
            }
        }
        Ok(icons)
    }

    pub(super) fn ensure(settings: &Settings, game: Game, minecraft: Option<(&Dirs, &str, bool)>) {
        if !enabled(settings) {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let ctx = Context::from_env()?;
            if !should_install(&ctx, game)? {
                return Ok(());
            }
            let icons = match minecraft {
                Some((dirs, id, snapshot)) => minecraft_icons(dirs, id, snapshot)?,
                None => HORIZON_ICONS
                    .iter()
                    .map(|(n, b)| (Some(*n), b.to_vec()))
                    .chain(std::iter::once((None, include_bytes!("../assets/horizon/scalable.svg").to_vec())))
                    .collect(),
            };
            // Old versions may not ship these assets; their X11 window icons still work.
            if !icons.is_empty() {
                install(&ctx, game, &icons, true)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            crate::logs::warn(format!("Game icon integration: {e:#}"));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_only_main_unlocalized_keys() {
            let text = "# StartupWMClass=horizon\n[Desktop Entry]\nName[fr]=Horizon\nStartupWMClass[fr]=wrong\n StartupWMClass = horizon \n[Desktop Action Play]\nStartupWMClass=other\nX-BlockForge-Integration=GameIcon\n";
            assert!(claims(text, "other.desktop", "horizon"));
            assert!(!claims(text, "unclaimed.desktop", "other"));
            assert!(!is_ours(text));
            assert!(!is_ours(
                "[Desktop Entry]\n# X-BlockForge-Integration=GameIcon\nX-BlockForge-Integration[fr]=GameIcon\n"
            ));
            assert!(is_ours("[Desktop Entry]\nX-BlockForge-Integration=GameIcon\n"));
            assert!(claims("[Desktop Entry]\n", "com.mojang.minecraft.desktop", "com.mojang.minecraft"));
        }

        #[test]
        fn entries_and_icon_paths() {
            let entry = desktop_entry(Game::Minecraft, Path::new("/my games/BlockForge.AppImage"));
            assert!(entry.contains("\nExec=\"/my games/BlockForge.AppImage\"\n"));
            assert!(entry.contains("\nNoDisplay=true\n"));
            assert!(entry.contains("\nIcon=blockforge-minecraft\n"));
            assert!(claims(&entry, "blockforge-minecraft.desktop", "com.mojang.minecraft"));
            assert!(is_ours(&entry));
            let horizon = desktop_entry(Game::Horizon, Path::new("/bin/blockforge"));
            assert!(horizon.contains("\nName=Horizon\n"));
            assert!(horizon.contains("by the Horizon project\n"));
            assert!(claims(&horizon, "", "horizon"));
            assert_eq!(
                icon_target(Path::new("/data"), "blockforge-minecraft", Some(256)),
                Path::new("/data/icons/hicolor/256x256/apps/blockforge-minecraft.png")
            );
            assert_eq!(
                icon_target(Path::new("/data"), "blockforge-horizon", None),
                Path::new("/data/icons/hicolor/scalable/apps/blockforge-horizon.svg")
            );
        }

        #[test]
        fn installs_idempotently_and_respects_foreign_entries() {
            let root = crate::testutil::scratch("game-icons");
            let ctx = Context {
                data_home: root.join("user"),
                data_dirs: vec![root.join("system")],
                executable: root.join("BlockForge"),
            };
            let game = Game::Horizon;
            assert!(should_install(&ctx, game).unwrap());
            let icons = vec![(Some(32), b"icon".to_vec())];
            install(&ctx, game, &icons, false).unwrap();
            let target = icon_target(&ctx.data_home, game.icon(), Some(32));
            let modified = fs::metadata(&target).unwrap().modified().unwrap();
            assert!(should_install(&ctx, game).unwrap());
            install(&ctx, game, &icons, false).unwrap();
            assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), modified);
            let desktop = ctx.data_home.join("applications/blockforge-horizon.desktop");
            let entry_modified = fs::metadata(&desktop).unwrap().modified().unwrap();
            install(&ctx, game, &icons, false).unwrap();
            assert_eq!(fs::metadata(&desktop).unwrap().modified().unwrap(), entry_modified);
            let moved = Context { executable: root.join("Moved BlockForge.AppImage"), ..ctx };
            install(&moved, game, &icons, false).unwrap();
            assert_eq!(fs::read_to_string(&desktop).unwrap(), desktop_entry(game, &moved.executable));
            let ctx = moved;
            fs::write(&target, b"changed").unwrap();
            install(&ctx, game, &icons, false).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"icon");
            for dir in [&ctx.data_home, &ctx.data_dirs[0]] {
                let apps = dir.join("applications");
                fs::create_dir_all(apps.join("nested")).unwrap();
                for (file, text) in [
                    ("horizon.desktop", "[Desktop Entry]\n"),
                    ("nested/launcher.desktop", "[Desktop Entry]\nStartupWMClass=horizon\n"),
                ] {
                    let entry = apps.join(file);
                    fs::write(&entry, text).unwrap();
                    assert!(!should_install(&ctx, game).unwrap());
                    assert_eq!(fs::read_to_string(&entry).unwrap(), text);
                    fs::remove_file(entry).unwrap();
                }
            }
            fs::write(ctx.data_home.join("applications/blockforge-horizon.desktop"), "[Desktop Entry]\nName=Mine\n")
                .unwrap();
            assert!(!should_install(&ctx, game).unwrap());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn disabled_integration_does_not_read_assets_or_write_files() {
            let root = crate::testutil::scratch("game-icons-disabled");
            let mut settings = Settings::default();
            settings.extras.insert("desktopIntegration".into(), serde_json::Value::Bool(false));
            ensure_minecraft(&settings, &Dirs::at(&root), "missing", false);
            ensure_horizon(&settings);
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn asset_paths_reject_malformed_hashes() {
            let root = crate::testutil::scratch("game-icon-paths");
            for hash in ["", "a", "é", "../../escape", &"z".repeat(40)] {
                assert!(object_path(&root, hash).is_err());
            }
            let hash = "ab".repeat(20);
            assert_eq!(object_path(&root, &hash).unwrap(), root.join("objects/ab").join(hash));
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn selects_icons_from_the_requested_index_and_snapshot() {
            let root = crate::testutil::scratch("game-icon-assets");
            let dirs = Dirs::at(&root);
            fs::create_dir_all(dirs.assets.join("indexes")).unwrap();
            fs::create_dir_all(dirs.assets.join("objects/ab")).unwrap();
            fs::write(dirs.assets.join("objects/ab/abcdabcdabcdabcdabcdabcdabcdabcdabcdabcd"), b"release").unwrap();
            fs::write(dirs.assets.join("objects/ab/abefabefabefabefabefabefabefabefabefabef"), b"snapshot").unwrap();
            fs::write(crate::install::asset_index_file(&dirs, "chosen"), r#"{"objects":{"icons/icon_32x32.png":{"hash":"abcdabcdabcdabcdabcdabcdabcdabcdabcdabcd","size":7},"icons/snapshot/icon_32x32.png":{"hash":"abefabefabefabefabefabefabefabefabefabef","size":8}}}"#).unwrap();
            assert_eq!(minecraft_icons(&dirs, "chosen", false).unwrap(), vec![(Some(32), b"release".to_vec())]);
            assert_eq!(minecraft_icons(&dirs, "chosen", true).unwrap(), vec![(Some(32), b"snapshot".to_vec())]);
            assert!(minecraft_icons(&dirs, "missing", false).is_err());
            fs::write(crate::install::asset_index_file(&dirs, "empty"), r#"{"objects":{}}"#).unwrap();
            assert!(minecraft_icons(&dirs, "empty", false).unwrap().is_empty());
            fs::write(
                crate::install::asset_index_file(&dirs, "bad"),
                r#"{"objects":{"icons/icon_16x16.png":{"hash":"x","size":0}}}"#,
            )
            .unwrap();
            assert!(minecraft_icons(&dirs, "bad", false).unwrap().is_empty());
            fs::remove_dir_all(root).unwrap();
        }
    }
}
