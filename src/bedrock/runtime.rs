//! Steam Linux Runtime 3 (sniper) and the way into it, in Rust: what umu/umu_runtime.py and umu_run.py do for BOL,
//! without UMU, Python or the runtime's shell entry scripts. BlockForge runs the pinned `pressure-vessel-wrap` ELF
//! directly with an argv array; inside the container the static `bedrock-runtime` helper reads its private plan and
//! becomes native Wine. See crates/bedrock-runtime/README.md for the whole chain.
use super::{
    BedrockError, Cancel, ErrorKind, Result,
    artifact::{self, invalid, io},
    engine::Engine,
    pins,
};
use crate::install::Progress;
pub use bedrock_runtime::{CommandSpec, InnerPlan, Program};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A verified, installed runtime and its private view. Only [`open`]/[`install`]/[`ensure`] make one. Hold
/// [`Runtime::lease`] for as long as anything runs from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    root: PathBuf,
    view: PathBuf,
    revision: &'static str,
}
impl Runtime {
    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn revision(&self) -> &'static str {
        self.revision
    }
    /// The copy pressure-vessel actually runs from (see [`ensure_view`]).
    #[cfg(test)]
    pub fn view(&self) -> &Path {
        &self.view
    }
    pub fn lease(&self) -> Result<artifact::Lease> {
        artifact::lease(self.root.parent().ok_or_else(|| invalid("Runtime root"))?, self.revision)
    }
    #[cfg(test)]
    pub(crate) fn synthetic(root: PathBuf) -> Self {
        let dir = root.parent().unwrap().to_path_buf();
        Self { view: view_path(&dir), root, revision: pins::RUNTIME.revision }
    }
}

/// `bedrock/runtime/steamrt3`.
pub fn dir(base: &Path) -> PathBuf {
    base.join("runtime/steamrt3")
}
fn view_path(dir: &Path) -> PathBuf {
    dir.join(format!(".{}.view", pins::RUNTIME.revision))
}
fn platform() -> String {
    format!("sniper_platform_{}", pins::RUNTIME.revision)
}

/// The full check, after the archive's own SHA-256: pinned entrypoints and mtrees, then every native file
/// pressure-vessel brings along against the pinned mtree, and only then the runtime's own native verifier
/// (`pv-verify`) over the whole platform, file digests included.
pub fn verify(root: &Path) -> Result<()> {
    for (path, hash) in pins::RUNTIME_CRITICAL {
        artifact::verify_hash(&root.join(path), hash)?;
    }
    verify_native_closure(root)?;
    quick(root)?;
    pv_verify(root)
}
fn pv_verify(root: &Path) -> Result<()> {
    let output = std::process::Command::new(root.join("pressure-vessel/bin/pv-verify"))
        .args(["--quiet", "--minimized-runtime"])
        .arg(root.join(platform()).join("files"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .map_err(io)?;
    if !output.status.success() {
        return Err(invalid(format!("SteamRT mtree verification failed: {}", String::from_utf8_lossy(&output.stderr))));
    }
    Ok(())
}
/// What Play runs: the right build, and its native entrypoints are ELF files. A few small reads.
pub fn quick(root: &Path) -> Result<()> {
    let versions = fs::read_to_string(root.join("VERSIONS.txt")).map_err(io)?;
    let has = |name: &str, version: &str| {
        versions.lines().any(|l| {
            let mut f = l.split_whitespace();
            f.next() == Some(name) && f.next() == Some(version)
        })
    };
    if !has("depot", pins::RUNTIME.revision) || !has("pressure-vessel", pins::PRESSURE_VESSEL) {
        return Err(invalid("Wrong SteamRT build"));
    }
    for name in ["pressure-vessel-wrap", "pv-verify"] {
        let mut magic = [0; 4];
        use std::io::Read;
        fs::File::open(root.join("pressure-vessel/bin").join(name))
            .and_then(|mut f| f.read_exact(&mut magic))
            .map_err(io)?;
        if &magic != b"\x7fELF" {
            return Err(invalid("SteamRT native entrypoint missing"));
        }
    }
    Ok(())
}

/// The installed runtime with a complete view, or None. Blocking, a few small reads.
#[cfg(test)]
pub fn open(dir: &Path) -> Result<Option<Runtime>> {
    let Some(root) = artifact::open(dir, pins::RUNTIME, quick)? else { return Ok(None) };
    let view = view_path(dir);
    if !view_complete(&view) {
        return Ok(None);
    }
    Ok(Some(Runtime { root, view, revision: pins::RUNTIME.revision }))
}
/// Installs from an archive already on disk (hashed again here), then builds the view. Blocking; minutes.
#[cfg(test)]
pub fn install(dir: &Path, archive: &Path) -> Result<Runtime> {
    let root = artifact::install(dir, pins::RUNTIME, archive, verify)?;
    ensure_view(dir, &root, false)
}
/// The runtime, ready: like [`super::engine::ensure`], plus its view. `base` is `Dirs.base/bedrock`.
pub async fn ensure(
    base: &Path,
    client: &reqwest::Client,
    progress: &std::sync::Arc<Progress>,
    cancel: &Cancel,
    check: super::engine::Check,
) -> Result<Runtime> {
    let dir = dir(base);
    let full = check == super::engine::Check::Full;
    progress.stage("Checking the Steam Linux Runtime");
    let root = artifact::ensure(
        artifact::Ensure { dir: &dir, cache: &base.join("cache/artifacts"), pin: pins::RUNTIME, full },
        client,
        progress,
        cancel,
        ("the Steam Linux Runtime", quick, verify),
    )
    .await?;
    tokio::task::spawn_blocking(move || ensure_view(&dir, &root, full))
        .await
        .map_err(|e| BedrockError::new(ErrorKind::Internal, e.to_string()))?
}

fn view_complete(view: &Path) -> bool {
    fs::read_to_string(view.join(".bf-pin")).is_ok_and(|s| s == pins::RUNTIME.sha256)
}
/// pressure-vessel opens the platform's `files/.ref` read-write (for its own lock) even when it copies the runtime,
/// and imports its tool directories' modes while building its container. The verified tree stays read-only, so it
/// runs from this private, verified copy beside it, `.<revision>.view`, shared by every profile; each profile's
/// variable dir then hard-links from it. Built once (or again on a full check) under the install lock.
fn ensure_view(dir: &Path, root: &Path, rebuild: bool) -> Result<Runtime> {
    let view = view_path(dir);
    let runtime = Runtime { root: root.to_path_buf(), view: view.clone(), revision: pins::RUNTIME.revision };
    if !rebuild && view_complete(&view) {
        return Ok(runtime);
    }
    let _guard = artifact::exclusive(dir, pins::RUNTIME.revision)?;
    if artifact::in_use(std::slice::from_ref(&view))? {
        return Err(BedrockError::new(ErrorKind::Busy, "A game still runs from the Steam Linux Runtime"));
    }
    let stage = dir.join(format!(".{}.view-stage", pins::RUNTIME.revision));
    artifact::remove(&stage)?;
    let result = (|| {
        copy_runtime_view(root, &stage)?;
        verify_native_closure(&stage)?;
        pv_verify(&stage).map_err(|e| invalid(format!("The runtime's private view: {}", e.message)))?;
        fs::write(stage.join(".bf-pin"), pins::RUNTIME.sha256).map_err(io)?;
        artifact::sync_tree(&stage)?;
        artifact::remove(&view)?;
        fs::rename(&stage, &view).map_err(io)?;
        fs::File::open(dir).and_then(|f| f.sync_all()).map_err(io)
    })();
    if result.is_err() {
        let _ = artifact::remove(&stage);
    }
    result.map(|()| runtime)
}

/// The outer command: pinned `pressure-vessel-wrap` from the view, with the helper and its private plan file as the
/// only arguments it passes on. Environment from scratch: the session and GPU choice (see
/// [`bedrock_runtime::OUTER_ENV`]), UMU's container variables, and BF-owned state dirs; inherited loader, Python and
/// Wine variables are removed. The caller writes the plan with [`write_plan`] and holds both leases while it runs.
pub fn command(runtime: &Runtime, engine: &Engine, plan: &InnerPlan) -> Result<CommandSpec> {
    bedrock_runtime::validate(plan).map_err(|e| BedrockError::new(ErrorKind::InvalidRequest, e))?;
    if plan.engine != engine.root()
        || engine.revision() != pins::ENGINE.revision
        || runtime.revision != pins::RUNTIME.revision
    {
        return Err(invalid("Runtime plan engine/build mismatch"));
    }
    let mut env = std::collections::BTreeMap::new();
    for key in bedrock_runtime::OUTER_ENV {
        if let Some(value) = plan.env.get(*key) {
            env.insert(key.to_string(), value.clone());
        }
    }
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    for (key, dir) in
        [("XDG_CACHE_HOME", "xdg-cache"), ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data")]
    {
        env.insert(key.into(), plan.state.join(dir).display().to_string());
    }
    // Everything the container must see beyond the tools: the helper, its plan, the prefix, the game, BF's state and
    // staged images. Paths were validated free of ':' above.
    let shared: Vec<String> = [&plan.helper, &plan.plan_path, &plan.prefix, &plan.cwd, &plan.state]
        .into_iter()
        .chain(plan.mounts.iter())
        .map(|p| p.display().to_string())
        .collect();
    let mounts = shared.join(":");
    let variable = plan.state.join("pressure-vessel");
    for (key, value) in [
        ("WINEPREFIX", plan.prefix.display().to_string()),
        ("STEAM_COMPAT_DATA_PATH", plan.prefix.display().to_string()),
        ("STEAM_COMPAT_INSTALL_PATH", plan.cwd.display().to_string()),
        ("STEAM_COMPAT_CLIENT_INSTALL_PATH", plan.state.join("steam-client").display().to_string()),
        ("STEAM_COMPAT_SHADER_PATH", plan.state.join("shaders").display().to_string()),
        ("STEAM_COMPAT_APP_ID", plan.app_id.clone()),
        ("STEAM_COMPAT_TOOL_PATHS", format!("{}:{}", engine.root().display(), runtime.view.display())),
        ("STEAM_COMPAT_MOUNTS", mounts),
        ("PRESSURE_VESSEL_VARIABLE_DIR", variable.display().to_string()),
        ("PRESSURE_VESSEL_RUNTIME_BASE", runtime.view.display().to_string()),
        ("PRESSURE_VESSEL_RUNTIME", platform()),
        ("PRESSURE_VESSEL_COPY_RUNTIME", "1".into()),
        ("PRESSURE_VESSEL_ARCHITECTURES", "x86_64-linux-gnu:i386-linux-gnu".into()),
    ] {
        env.insert(key.into(), value);
    }
    Ok(CommandSpec {
        program: runtime.view.join("pressure-vessel/bin/pressure-vessel-wrap"),
        // pressure-vessel-wrap shares the home folder by itself, but not the rest: the staged images live in a RAM
        // folder under /run/user or /dev/shm, which only `--filesystem` brings into the container (STEAM_COMPAT_MOUNTS
        // is read by Steam's entry-point script, which BlockForge doesn't run)
        argv: std::iter::once(format!("--variable-dir={}", variable.display()))
            .chain(shared.iter().map(|p| format!("--filesystem={p}")))
            .chain(["--".into()])
            .chain([plan.helper.display().to_string(), "--plan".into(), plan.plan_path.display().to_string()])
            .collect(),
        env,
        env_removals: [
            "WINEARCH",
            "WINEDLLOVERRIDES",
            "WINEDLLPATH",
            "LD_PRELOAD",
            "LD_AUDIT",
            "LD_LIBRARY_PATH",
            "PYTHONPATH",
            "PYTHONHOME",
            "PRESSURE_VESSEL_RUNTIME_ARCHIVE",
            "PRESSURE_VESSEL_PREFIX",
        ]
        .map(String::from)
        .to_vec(),
        cwd: plan.cwd.clone(),
    })
}
/// Validates `plan` against these exact installs, makes its state dirs and writes it with create-new, 0600. The
/// plan carries what only the helper may read (preauth, file maps); delete it when the session ends.
pub fn write_plan(runtime: &Runtime, engine: &Engine, plan: &InnerPlan) -> Result<()> {
    command(runtime, engine, plan)?;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    for dir in ["steam-client", "shaders", "pressure-vessel", "gstreamer-1.0", "xdg-cache", "xdg-config", "xdg-data"] {
        fs::create_dir_all(plan.state.join(dir)).map_err(io)?;
    }
    let mut f = fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(&plan.plan_path).map_err(io)?;
    f.write_all(&serde_json::to_vec(plan).map_err(io)?).map_err(io)?;
    f.sync_all().map_err(io)
}
fn copy_runtime_view(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dst).map_err(io)?;
    for entry in fs::read_dir(src).map_err(io)? {
        let entry = entry.map_err(io)?;
        let target = dst.join(entry.file_name());
        let ty = entry.file_type().map_err(io)?;
        if ty.is_dir() {
            copy_runtime_view(&entry.path(), &target)?;
        } else if ty.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(entry.path()).map_err(io)?, target).map_err(io)?;
        } else {
            fs::copy(entry.path(), &target).map_err(io)?;
            if entry.file_name() == ".ref" {
                fs::set_permissions(target, fs::Permissions::from_mode(0o600)).map_err(io)?;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn plan(base: &Path) -> InnerPlan {
        InnerPlan {
            schema: 1,
            engine: base.join("engine/wow64-archs-native21"),
            prefix: base.join("profiles/p/compat/pfx"),
            program: Program::Wineboot,
            args: vec![],
            env: std::collections::BTreeMap::from([
                ("HOME".into(), "/home/player".into()),
                ("LANG".into(), "en_US.UTF-8".into()),
            ]),
            cwd: base.join("profiles/p/compat"),
            headless: true,
            ntsync: false,
            nvapi: false,
            helper: base.join("helpers/bedrock-runtime"),
            plan_path: base.join("sessions/s/plan.json"),
            state: base.join("profiles/p/compat/state"),
            app_id: "00000000-0000-4000-8000-000000000001".into(),
            mounts: vec![],
        }
    }
    fn installs(base: &Path) -> (Runtime, Engine) {
        (
            Runtime::synthetic(dir(base).join(pins::RUNTIME.revision)),
            Engine::synthetic(base.join("engine/wow64-archs-native21")),
        )
    }
    /// The exact outer argv/env for this engine and runtime build (`fixtures/runtime-command.json`). Regenerate with
    /// BF_BLESS=1 only after reviewing the difference.
    #[test]
    fn explicit_native_commands_and_environment() {
        let base = Path::new("/owned/bedrock");
        let p = plan(base);
        let (r, e) = installs(base);
        let outer = command(&r, &e, &p).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bedrock/fixtures/runtime-command.json");
        if std::env::var_os("BF_BLESS").is_some() {
            fs::write(&fixture, serde_json::to_string_pretty(&outer).unwrap() + "\n").unwrap();
        }
        let golden: CommandSpec = serde_json::from_str(include_str!("fixtures/runtime-command.json")).unwrap();
        assert_eq!(outer, golden);
        assert_eq!(
            outer.program,
            Path::new(
                "/owned/bedrock/runtime/steamrt3/.3.0.20261021.266025.view/pressure-vessel/bin/pressure-vessel-wrap"
            )
        );
        // every folder the game needs is shared with the container, then the helper runs
        assert!(outer.argv.contains(&"--filesystem=/owned/bedrock/profiles/p/compat/pfx".to_string()));
        let tail = outer.argv.iter().position(|a| a == "--").unwrap();
        assert!(outer.argv[1..tail].iter().all(|a| a.starts_with("--filesystem=/")));
        assert_eq!(
            outer.argv[tail..],
            ["--", "/owned/bedrock/helpers/bedrock-runtime", "--plan", "/owned/bedrock/sessions/s/plan.json"]
        );
        assert_eq!(outer.env["PRESSURE_VESSEL_RUNTIME"], "sniper_platform_3.0.20261021.266025");
        for removed in ["LD_PRELOAD", "LD_LIBRARY_PATH", "WINEARCH", "PYTHONPATH", "PYTHONHOME"] {
            assert!(outer.env_removals.contains(&removed.into()), "{removed}");
            assert!(!outer.env.contains_key(removed));
        }
        // The child sees exactly this environment, nothing inherited from BlockForge's own.
        let probe = CommandSpec { program: "/usr/bin/env".into(), argv: vec![], cwd: "/".into(), ..outer.clone() };
        let seen = String::from_utf8(probe.command().output().unwrap().stdout).unwrap();
        let seen: std::collections::BTreeMap<String, String> =
            seen.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.into(), v.into())).collect();
        assert_eq!(seen, outer.env);
    }
    #[test]
    fn inner_command_selects_wow64_wine_and_graphics_overrides() {
        let p = plan(Path::new("/owned/bedrock"));
        let setup = bedrock_runtime::inner_command(&p).unwrap();
        assert_eq!(setup.program, Engine::synthetic(p.engine.clone()).wine());
        assert_eq!(setup.argv, vec!["wineboot.exe", "-u"]);
        assert!(setup.env["WINEDLLOVERRIDES"].contains("winevulkan=d"));
        let mut game = p.clone();
        game.headless = false;
        game.program = Program::Wine;
        game.args = vec!["C:\\game\\Minecraft.Windows.exe".into()];
        let game = bedrock_runtime::inner_command(&game).unwrap();
        assert_eq!(game.argv, vec!["C:\\game\\Minecraft.Windows.exe"]);
        assert!(!game.env["WINEDLLOVERRIDES"].contains("winevulkan=d"));
        assert!(game.env["WINEDLLOVERRIDES"].contains("d3d11,d3d10core,d3d9,dxgi,d3d12,d3d12core=n"));
        assert!(
            game.env["WINEDLLOVERRIDES"].contains("winex11.drv=b;")
                || game.env["WINEDLLOVERRIDES"].contains("winex11.drv=b")
        );
        assert!(game.env["WINEDLLOVERRIDES"].contains("winewayland.drv=d"));
        assert_eq!(game.env["WINEDLLPATH"], format!("{0}/files/lib/vkd3d:{0}/files/lib/wine", p.engine.display()));
        assert_eq!(game.env["LANG"], "en_US.UTF-8");
        assert_eq!(game.env["WINEFSYNC"], "0");
        // The exact in-container command for Play with this engine (`fixtures/inner-command.json`).
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bedrock/fixtures/inner-command.json");
        if std::env::var_os("BF_BLESS").is_some() {
            fs::write(&fixture, serde_json::to_string_pretty(&game).unwrap() + "\n").unwrap();
        }
        let golden: CommandSpec = serde_json::from_str(include_str!("fixtures/inner-command.json")).unwrap();
        assert_eq!(game, golden);
        let mut wait = p.clone();
        wait.program = Program::WineserverWait;
        let wait = bedrock_runtime::inner_command(&wait).unwrap();
        assert_eq!(wait.program, Engine::synthetic(p.engine.clone()).wineserver());
        assert_eq!(wait.argv, vec!["-w"]);
        let mut poisoned = p;
        poisoned.env.insert("LD_PRELOAD".into(), "untrusted".into());
        assert!(bedrock_runtime::inner_command(&poisoned).is_err());
    }
    #[test]
    fn gpu_and_session_reach_pressure_vessel_but_credentials_stay_in_the_plan() {
        let base = Path::new("/owned/bedrock");
        let mut p = plan(base);
        p.env.insert("VK_DRIVER_FILES".into(), "/usr/share/vulkan/icd.d/nvidia_icd.json".into());
        p.env.insert("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into());
        p.env.insert("WAYLAND_DISPLAY".into(), "wayland-0".into());
        p.env.insert("WINE_XBOX_PREAUTH_FILE".into(), "/run/user/1000/preauth".into());
        p.env.insert("WINEGDK_PREAUTH_DEVICE".into(), "device".into());
        let (r, e) = installs(base);
        let outer = command(&r, &e, &p).unwrap();
        assert_eq!(outer.env["VK_DRIVER_FILES"], "/usr/share/vulkan/icd.d/nvidia_icd.json");
        assert_eq!(outer.env["__NV_PRIME_RENDER_OFFLOAD"], "1");
        assert_eq!(outer.env["WAYLAND_DISPLAY"], "wayland-0");
        assert!(!outer.env.keys().any(|k| k.contains("PREAUTH")));
        assert!(!outer.argv.iter().any(|a| a.contains("preauth")));
    }
    #[test]
    fn a_plan_for_another_engine_or_unsafe_paths_is_refused() {
        let base = Path::new("/owned/bedrock");
        let (r, e) = installs(base);
        let mut p = plan(base);
        p.engine = "/elsewhere/engine".into();
        assert!(command(&r, &e, &p).is_err());
        let mut p = plan(base);
        p.mounts.push("/run/user/1000/a:b".into());
        assert!(matches!(command(&r, &e, &p).unwrap_err().kind, ErrorKind::InvalidRequest));
    }
    #[test]
    fn plans_are_private_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::testutil::scratch("b-plan");
        let p = plan(&dir);
        let (r, e) = installs(&dir);
        fs::create_dir_all(p.plan_path.parent().unwrap()).unwrap();
        write_plan(&r, &e, &p).unwrap();
        assert_eq!(fs::metadata(&p.plan_path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(write_plan(&r, &e, &p).is_err());
        let back: InnerPlan = serde_json::from_slice(&fs::read(&p.plan_path).unwrap()).unwrap();
        assert_eq!(back, p);
        assert!(p.state.join("pressure-vessel").is_dir());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn runtime_build_or_mtree_missing_is_failure() {
        let dir = crate::testutil::scratch("b-runtime");
        fs::write(dir.join("VERSIONS.txt"), "depot\t3.0.20261021.266024\npressure-vessel\t0.20261007.0\n").unwrap();
        assert!(quick(&dir).unwrap_err().message.contains("Wrong SteamRT build"));
        assert!(verify(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    /// The real check, no account needed: installs the pinned engine and runtime from the research archives (hashed
    /// again) into BF's layout under `~/.cache/bf-bedrock-pkgb/bedrock`, prepares a prefix, runs headless
    /// `wineboot -u` through pressure-vessel and the static helper under `strace -f -e execve`, waits for
    /// `wineserver -w`, then records hives, xgameruntime files, fonts and the exec audit under
    /// `~/bf-ui/bedrock/evidence/pkgb`. Needs BF_BEDROCK_RUNTIME_HELPER (static musl build) and strace (BF_STRACE).
    #[test]
    #[ignore = "native engine + SteamRT smoke, ~5 GiB and strace required"]
    fn real_native_wineboot() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let scratch = home.join(".cache/bf-bedrock-pkgb");
        let base = scratch.join("bedrock");
        let evidence = home.join("bf-ui/bedrock/evidence/pkgb");
        fs::create_dir_all(&evidence).unwrap();
        let cache = home.join(".cache/bedrock-research");
        let started = std::time::Instant::now();
        let e = match super::super::engine::open(&super::super::engine::dir(&base)).unwrap() {
            Some(e) => e,
            None => {
                super::super::engine::install(&super::super::engine::dir(&base), &cache.join("engine-native21.tar.gz"))
                    .unwrap()
            }
        };
        let r = match open(&dir(&base)).unwrap() {
            Some(r) => r,
            None => install(&dir(&base), &cache.join("steamrt3-3.0.20261021.266025.tar.xz")).unwrap(),
        };
        let installed = started.elapsed();
        let warm = std::time::Instant::now();
        assert_eq!(super::super::engine::open(&super::super::engine::dir(&base)).unwrap().as_ref(), Some(&e));
        assert_eq!(open(&dir(&base)).unwrap().as_ref(), Some(&r));
        let warm = warm.elapsed();
        let _elease = e.lease().unwrap();
        let _rlease = r.lease().unwrap();
        let mut p = plan(&base);
        p.engine = e.root().to_path_buf();
        if p.prefix.exists() {
            artifact::remove(&p.prefix).unwrap();
        }
        let _ = fs::remove_dir_all(p.plan_path.parent().unwrap());
        fs::create_dir_all(p.plan_path.parent().unwrap()).unwrap();
        for key in ["DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"] {
            if let Ok(v) = std::env::var(key) {
                p.env.insert(key.into(), v);
            }
        }
        p.env.insert("HOME".into(), home.display().to_string());
        p.helper = PathBuf::from(
            std::env::var_os("BF_BEDROCK_RUNTIME_HELPER").expect("BF_BEDROCK_RUNTIME_HELPER static binary"),
        );
        super::super::proton::prepare(&e, &p.prefix, false).unwrap();
        write_plan(&r, &e, &p).unwrap();
        let spec = command(&r, &e, &p).unwrap();
        fs::write(evidence.join("outer-command.json"), serde_json::to_vec_pretty(&spec).unwrap()).unwrap();
        let trace = evidence.join("wineboot-execve.log");
        let mut c = std::process::Command::new(std::env::var_os("BF_STRACE").unwrap_or_else(|| "strace".into()));
        c.args(["-f", "-e", "trace=execve", "-e", "signal=none", "-s", "4096", "-o"])
            .arg(&trace)
            .arg(&spec.program)
            .args(&spec.argv)
            .env_clear()
            .envs(&spec.env)
            .current_dir(&spec.cwd);
        let boot = std::time::Instant::now();
        let out = c.output().unwrap();
        fs::write(evidence.join("wineboot.stdout"), &out.stdout).unwrap();
        fs::write(evidence.join("wineboot.stderr"), &out.stderr).unwrap();
        assert!(out.status.success(), "wineboot failed: {}", String::from_utf8_lossy(&out.stderr));
        fs::remove_file(&p.plan_path).unwrap();
        p.program = Program::WineserverWait;
        write_plan(&r, &e, &p).unwrap();
        let out = command(&r, &e, &p).unwrap().command().output().unwrap();
        fs::write(evidence.join("wineserver.stderr"), &out.stderr).unwrap();
        assert!(out.status.success());
        fs::remove_file(&p.plan_path).unwrap();
        let boot = boot.elapsed();
        // Nothing of this prefix may still run once wineserver -w returns.
        let marker = format!("WINEPREFIX={}", p.prefix.display());
        for entry in fs::read_dir("/proc").unwrap().flatten() {
            if let Ok(env) = fs::read(entry.path().join("environ")) {
                assert!(!env.split(|b| *b == 0).any(|v| v == marker.as_bytes()), "prefix process left: {entry:?}");
            }
        }

        let mut report = format!(
            "engine {} at {}\nruntime {} at {} (view {})\ninstall-or-open {:.1}s, warm open {} ms, wineboot+wait {:.1}s\n",
            e.revision(),
            e.root().display(),
            r.revision(),
            r.root().display(),
            r.view().display(),
            installed.as_secs_f64(),
            warm.as_millis(),
            boot.as_secs_f64()
        );
        for path in [
            "system.reg",
            "user.reg",
            "userdef.reg",
            "drive_c/windows/system32/xgameruntime.dll",
            "drive_c/windows/system32/xgameruntime.dll.threading",
            "drive_c/windows/system32/d3d12core.dll",
            "drive_c/windows/syswow64/d3d12core.dll",
            "drive_c/windows/system32/cryptbase.dll",
        ] {
            let f = p.prefix.join(path);
            report.push_str(&format!(
                "{path}: {} bytes SHA256={}\n",
                fs::metadata(&f).unwrap().len(),
                artifact::sha256(&f).unwrap()
            ));
        }
        let system = fs::read_to_string(p.prefix.join("system.reg")).unwrap();
        assert!(system.starts_with("WINE REGISTRY Version 2"));
        report.push_str(&format!("system.reg has MachineGuid: {}\n", system.contains("\"MachineGuid\"=")));
        let mut fonts: Vec<_> = fs::read_dir(p.prefix.join("drive_c/windows/Fonts")).unwrap().flatten().collect();
        fonts.sort_by_key(|f| f.file_name());
        for f in fonts {
            report.push_str(&format!(
                "font {:?} -> {:?}, readable={}\n",
                f.file_name(),
                fs::read_link(f.path()).ok(),
                fs::read(f.path()).is_ok()
            ));
        }
        fs::write(evidence.join("prefix-files.txt"), report).unwrap();

        // The audit: every program the whole chain started, and none of them Python, a shell, UMU, the proton
        // script or BOL.
        let log = fs::read_to_string(&trace).unwrap();
        let mut paths = std::collections::BTreeSet::new();
        for line in log.lines() {
            if let Some(rest) = line.split("execve(\"").nth(1)
                && let Some(path) = rest.split('"').next()
            {
                paths.insert(path.to_string());
            }
        }
        let banned = |p: &str| {
            let name = p.rsplit('/').next().unwrap_or(p);
            name.starts_with("python")
                || p.contains("umu")
                || name == "proton"
                || p.contains("bedrock-on-linux")
                || p.contains("protonfixes")
                || p.ends_with(".pyz")
        };
        let shell = |p: &str| {
            matches!(p.rsplit('/').next().unwrap_or(p), "sh" | "bash" | "dash" | "zsh" | "ash" | "busybox" | "env")
        };
        let python: Vec<_> = paths.iter().filter(|p| banned(p)).collect();
        let shells: Vec<_> = paths.iter().filter(|p| shell(p)).collect();
        fs::write(evidence.join("executable-paths.txt"), paths.iter().map(|p| format!("{p}\n")).collect::<String>())
            .unwrap();
        fs::write(
            evidence.join("exec-audit.txt"),
            format!(
                "Distinct execve paths (attempts included): {}\nPython/UMU/proton-script/BOL exec paths: {}\nShell interpreter exec paths: {}\n{python:?}{shells:?}\n",
                paths.len(),
                python.len(),
                shells.len()
            ),
        )
        .unwrap();
        assert!(python.is_empty() && shells.is_empty(), "{python:?} {shells:?}");
        assert!(paths.iter().any(|p| p.ends_with("/files/bin-wow64/wine")));
        assert!(paths.contains(&p.helper.display().to_string()));
        super::super::engine::verify(e.root()).unwrap();
    }
}

/// Authenticate the verifier's bundled dynamic libraries and subprocesses before
/// executing it. The pinned root mtree is data; it cannot authorize new hashes.
fn verify_native_closure(root: &Path) -> Result<()> {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::GzDecoder::new(fs::File::open(root.join("mtree.txt.gz")).map_err(io)?)
        .take(32 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(io)?;
    if text.len() > 32 * 1024 * 1024 {
        return Err(invalid("SteamRT mtree too large"));
    }
    check_native_manifest(root, &text)
}
fn check_native_manifest(root: &Path, text: &str) -> Result<()> {
    let owned = fs::canonicalize(root).map_err(io)?;
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.is_empty() || !fields[0].starts_with("./pressure-vessel/") || !fields.contains(&"type=file") {
            continue;
        }
        let decoded = mtree_path(fields[0])?;
        let name = Path::new(&decoded);
        if name.is_absolute() || name.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(invalid("Unexpected SteamRT native manifest path"));
        }
        let path = root.join(name);
        if !fs::canonicalize(&path).map_err(io)?.starts_with(&owned) {
            return Err(invalid("SteamRT native file escapes runtime"));
        }
        let hash = fields
            .iter()
            .find_map(|f| f.strip_prefix("sha256="))
            .ok_or_else(|| invalid("Native mtree digest missing"))?;
        artifact::verify_hash(&path, hash)?;
    }
    Ok(())
}
fn mtree_path(encoded: &str) -> Result<std::ffi::OsString> {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = vec![];
    let mut input = encoded.as_bytes().iter().copied();
    while let Some(b) = input.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        let digits = [input.next(), input.next(), input.next()];
        let mut value = 0u16;
        for d in digits {
            let d = d.filter(|b| (b'0'..=b'7').contains(b)).ok_or_else(|| invalid("Invalid mtree path escape"))?;
            value = value * 8 + u16::from(d - b'0');
        }
        if value == 0 || value > 255 {
            return Err(invalid("Invalid mtree path byte"));
        }
        bytes.push(value as u8);
    }
    Ok(std::ffi::OsString::from_vec(bytes))
}

#[cfg(test)]
mod mtree_tests {
    use super::*;
    #[test]
    fn native_library_tamper_or_mtree_digest_failure_prevents_execution() {
        let dir = crate::testutil::scratch("b-mtree");
        let lib = dir.join("pressure-vessel/lib/libtest.so");
        fs::create_dir_all(lib.parent().unwrap()).unwrap();
        fs::write(&lib, b"native library").unwrap();
        let manifest =
            format!("#mtree\n./pressure-vessel/lib/libtest.so type=file sha256={}\n", artifact::sha256(&lib).unwrap());
        check_native_manifest(&dir, &manifest).unwrap();
        fs::write(&lib, b"tampered").unwrap();
        assert!(check_native_manifest(&dir, &manifest).is_err());
        assert!(check_native_manifest(&dir, "./pressure-vessel/../../escape type=file sha256=bad").is_err());
        fs::write(dir.join("mtree.txt.gz"), b"corrupt checksum manifest").unwrap();
        assert!(verify(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    #[test]
    fn mutable_ref_does_not_change_the_verified_platform() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::testutil::scratch("b-view");
        let source = dir.join("source");
        fs::create_dir_all(source.join("files")).unwrap();
        fs::write(source.join("files/.ref"), b"").unwrap();
        fs::set_permissions(source.join("files/.ref"), fs::Permissions::from_mode(0o444)).unwrap();
        fs::write(source.join("files/lib.so"), b"runtime bytes").unwrap();
        let view = dir.join("view");
        copy_runtime_view(&source, &view).unwrap();
        fs::write(view.join("files/.ref"), b"mutable metadata").unwrap();
        assert!(fs::read(source.join("files/.ref")).unwrap().is_empty());
        assert_eq!(fs::read(view.join("files/lib.so")).unwrap(), b"runtime bytes");
        fs::remove_dir_all(dir).unwrap();
    }
}
