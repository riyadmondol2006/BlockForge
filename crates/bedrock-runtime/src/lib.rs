//! Native inner runtime. Ported from BedrockOnLinux bol/prefix.py and the
//! native21 Proton init_wine algorithm. See LICENSE-BOL and README.md.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, process::Command};

/// The one SteamRT3 (sniper) build this helper and BlockForge's runtime pin agree on. The helper refuses to run
/// anywhere else, so a missing container can never fall back to the host's own Wine or libraries.
pub const STEAMRT_BUILD: &str = "3.0.20261021.266025";

/// Kept deliberately small: credentials and game arguments stay in a private file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InnerPlan {
    pub schema: u32,
    pub engine: PathBuf,
    pub prefix: PathBuf,
    pub program: Program,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub headless: bool,
    pub ntsync: bool,
    pub nvapi: bool,
    pub helper: PathBuf,
    pub plan_path: PathBuf,
    pub state: PathBuf,
    /// Explicit staged images (for example private tmpfs files) outside HOME.
    #[serde(default)]
    pub mounts: Vec<PathBuf>,
    /// Stable UUID belonging to this BF profile, not an Xbox title identifier.
    pub app_id: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Program {
    Wine,
    Wineboot,
    WineserverWait,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub env_removals: Vec<String>,
    pub cwd: PathBuf,
}
impl CommandSpec {
    /// Clearing inheritance is intentional. Only the validated plan crosses the boundary.
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.env_clear().args(&self.argv).envs(&self.env).current_dir(&self.cwd);
        for key in &self.env_removals {
            c.env_remove(key);
        }
        c
    }
}

pub fn validate(p: &InnerPlan) -> Result<(), String> {
    if p.schema != 1
        || p.app_id.len() != 36
        || !p
            .app_id
            .bytes()
            .enumerate()
            .all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
    {
        return Err("Invalid runtime plan identity".into());
    }
    for path in [&p.engine, &p.prefix, &p.cwd, &p.helper, &p.plan_path, &p.state].into_iter().chain(p.mounts.iter()) {
        if !path.is_absolute()
            || path.components().any(|c| matches!(c, std::path::Component::ParentDir))
            || path.to_str().is_none_or(|s| s.contains([':', '\n', '\0']))
        {
            return Err("Runtime paths must be absolute UTF-8 paths without colon, newline or parent traversal".into());
        }
    }
    if p.args.iter().any(|s| s.contains('\0')) {
        return Err("NUL in argument".into());
    }
    for (k, v) in &p.env {
        if !allowed_env(k) || v.contains('\0') {
            return Err(format!("Unapproved runtime variable: {k}"));
        }
    }
    Ok(())
}
/// Explicit transport allowlist: do not inherit Wine, loader, Python or Steam control variables.
pub fn allowed_env(k: &str) -> bool {
    if let Some(suffix) = k.strip_prefix("WINEGDK_XSTS_RP_") {
        return !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    }
    matches!(
        k,
        "HOME"
            | "USER"
            | "LOGNAME"
            | "LANG"
            | "LC_ALL"
            | "LC_CTYPE"
            | "DISPLAY"
            | "XAUTHORITY"
            | "WAYLAND_DISPLAY"
            | "XDG_RUNTIME_DIR"
            | "DBUS_SESSION_BUS_ADDRESS"
            | "PULSE_SERVER"
            | "PIPEWIRE_REMOTE"
            | "WINEDEBUG"
            | "VKD3D_DEBUG"
            | "WINE_DLL_FILE_MAP"
            | "WINE_XBOX_PREAUTH"
            | "WINE_XBOX_PREAUTH_FILE"
            | "WINEGDK_PREAUTH_DEVICE"
            | "MICROSOFT_WINDOWSAPPRUNTIME_BOOTSTRAP_INITIALIZE_SHOWUI"
            | "MICROSOFT_WINDOWSAPPRUNTIME_BOOTSTRAP_INITIALIZE_FAILFAST"
            | "MICROSOFT_WINDOWSAPPRUNTIME_DEPLOYMENT_INITIALIZE_ONERRORSHOWUI"
            | "XCURL_CA_BUNDLE"
            | "SSL_CERT_FILE"
            | "XCURL_LOG"
            | "__NV_PRIME_RENDER_OFFLOAD"
            | "__EGL_VENDOR_LIBRARY_FILENAMES"
            | "__GLX_VENDOR_LIBRARY_NAME"
            | "__VK_LAYER_NV_optimus"
            | "DRI_PRIME"
            | "MESA_VK_DEVICE_SELECT"
            | "VK_DRIVER_FILES"
            | "VK_ICD_FILENAMES"
            | "VK_LOADER_DRIVERS_SELECT"
            | "VK_LOADER_DRIVERS_DISABLE"
            | "VKD3D_FILTER_DEVICE_NAME"
            | "DXVK_FILTER_DEVICE_NAME"
            | "WINE_DISABLE_VULKAN_OPWR"
            | "VKD3D_CONFIG"
    )
}
/// What the launcher hands pressure-vessel itself, outside the container: the session (display, audio, locale) and
/// the GPU choice, which pressure-vessel needs to import the right host driver. Credentials and game paths in the
/// plan (preauth, file maps) never go here; only the helper reads them, from the private plan file.
pub const OUTER_ENV: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "DISPLAY",
    "XAUTHORITY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "PULSE_SERVER",
    "PIPEWIRE_REMOTE",
    "__NV_PRIME_RENDER_OFFLOAD",
    "__EGL_VENDOR_LIBRARY_FILENAMES",
    "__GLX_VENDOR_LIBRARY_NAME",
    "__VK_LAYER_NV_optimus",
    "DRI_PRIME",
    "MESA_VK_DEVICE_SELECT",
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_LOADER_DRIVERS_SELECT",
    "VK_LOADER_DRIVERS_DISABLE",
];
pub const CONTAINER_ENV: &[&str] = &[
    "LD_LIBRARY_PATH",
    "ORIG_LD_LIBRARY_PATH",
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_LAYER_PATH",
    "VK_ADD_LAYER_PATH",
    "LIBGL_DRIVERS_PATH",
    "__EGL_VENDOR_LIBRARY_FILENAMES",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "GIO_EXTRA_MODULES",
    "GI_TYPELIB_PATH",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "PULSE_SERVER",
    "PIPEWIRE_REMOTE",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XAUTHORITY",
    "FONTCONFIG_PATH",
    "FONTCONFIG_FILE",
];
pub fn inner_command(p: &InnerPlan) -> Result<CommandSpec, String> {
    inner_command_with_container(p, &BTreeMap::new())
}
/// Only pressure-vessel's rewritten loader/display/driver values cross this
/// boundary. Inheriting the complete host environment would re-enable injection.
pub fn inner_command_with_container(
    p: &InnerPlan,
    container: &BTreeMap<String, String>,
) -> Result<CommandSpec, String> {
    validate(p)?;
    let lib = p.engine.join("files/lib");
    let mut env = p.env.clone();
    for (key, value) in container {
        if !CONTAINER_ENV.contains(&key.as_str()) || value.contains('\0') {
            return Err("Unexpected container variable".into());
        }
        env.insert(key.clone(), value.clone());
    }
    let ld = ["x86_64-linux-gnu", "aarch64-linux-gnu", "i386-linux-gnu"].map(|a| lib.join(a).display().to_string());
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    for (key, dir) in
        [("XDG_CACHE_HOME", "xdg-cache"), ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data")]
    {
        env.insert(key.into(), p.state.join(dir).display().to_string());
    }
    env.insert(
        "ORIG_LD_LIBRARY_PATH".into(),
        container.get("ORIG_LD_LIBRARY_PATH").or_else(|| container.get("LD_LIBRARY_PATH")).cloned().unwrap_or_default(),
    );
    let mut library_path = ld.join(":");
    if let Some(inherited) = container.get("LD_LIBRARY_PATH").filter(|v| !v.is_empty()) {
        library_path.push(':');
        library_path.push_str(inherited);
    }
    env.insert("LD_LIBRARY_PATH".into(), library_path);
    env.insert("WINEDLLPATH".into(), format!("{}/vkd3d:{}/wine", lib.display(), lib.display()));
    env.insert(
        "GST_PLUGIN_SYSTEM_PATH_1_0".into(),
        ld.iter().map(|v| format!("{v}/gstreamer-1.0")).collect::<Vec<_>>().join(":"),
    );
    env.insert("WINE_GST_REGISTRY_DIR".into(), p.state.join("gstreamer-1.0").display().to_string());
    env.insert("WINEPREFIX".into(), p.prefix.display().to_string());
    env.insert("WINEUSERNAME".into(), "steamuser".into());
    env.insert("WINENTSYNC".into(), if p.ntsync { "1" } else { "0" }.into());
    env.insert("WINEESYNC".into(), "0".into());
    env.insert("WINEFSYNC".into(), "0".into());
    env.insert("DXVK_ENABLE_NVAPI".into(), if p.nvapi { "1" } else { "0" }.into());
    for k in ["DXVK_SHADER_CACHE_PATH", "VKD3D_SHADER_CACHE_PATH"] {
        env.insert(k.into(), p.state.join("shaders").display().to_string());
    }
    env.entry("WINEDEBUG".into()).or_insert_with(|| "-all".into());
    env.entry("VKD3D_DEBUG".into()).or_insert_with(|| "info".into());
    let mut overrides = "winebth.sys=d;winewayland.drv=d;d3d8=b;cryptbase=n,b;vrclient=d;vrclient_x64=d;openvr_api=d;wineopenxr=d;amd_ags_x64=d".to_string();
    if p.headless {
        overrides.push_str(";winex11.drv=d;winevulkan=d;dxgi=d;d3d11=d;d3d12=d;d3d12core=d");
        env.remove("DISPLAY");
        env.remove("WAYLAND_DISPLAY");
    } else {
        overrides.push_str(";winex11.drv=b;d3d11,d3d10core,d3d9,dxgi,d3d12,d3d12core=n");
        if p.nvapi {
            overrides.push_str(";nvapi,nvapi64,nvofapi64=n;nvcuda=b");
        } else {
            overrides.push_str(";nvapi,nvapi64,nvofapi64=d");
        }
    }
    env.insert("WINEDLLOVERRIDES".into(), overrides);
    // BOL requires this for the menu's indirect root-CBV updates (bol/launch.py:863-864); a player's own options
    // (`nodxr` to keep ray tracing off, say) stay, in their order.
    let mut vkd3d: Vec<String> = env
        .get("VKD3D_CONFIG")
        .map(|v| v.split([',', ';']).map(str::trim).filter(|o| !o.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    if !vkd3d.iter().any(|o| o == "force_raw_va_cbv") {
        vkd3d.push("force_raw_va_cbv".into());
    }
    env.insert("VKD3D_CONFIG".into(), vkd3d.join(","));
    let (bin, mut argv) = match p.program {
        Program::Wine => ("wine", vec![]),
        Program::Wineboot => ("wine", vec!["wineboot.exe".into(), "-u".into()]),
        Program::WineserverWait => ("wineserver", vec!["-w".into()]),
    };
    argv.extend(p.args.clone());
    Ok(CommandSpec {
        program: p.engine.join("files/bin-wow64").join(bin),
        argv,
        env,
        env_removals: [
            "WINEARCH",
            "LD_PRELOAD",
            "LD_AUDIT",
            "PYTHONPATH",
            "PYTHONHOME",
            "GNUTLS_SYSTEM_PRIORITY_FILE",
            "GNUTLS_SYSTEM_PRIORITY_FAIL_ON_INVALID",
        ]
        .map(String::from)
        .to_vec(),
        cwd: p.cwd.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> InnerPlan {
        InnerPlan {
            schema: 1,
            engine: "/owned/engine".into(),
            prefix: "/owned/prefix".into(),
            program: Program::Wine,
            args: vec!["C:\\game path\\Minecraft.Windows.exe".into()],
            env: BTreeMap::from([("HOME".into(), "/home/player".into()), ("LANG".into(), "bn_BD.UTF-8".into())]),
            cwd: "/owned/game path".into(),
            headless: false,
            ntsync: false,
            nvapi: false,
            helper: "/owned/helper".into(),
            plan_path: "/owned/plan".into(),
            state: "/owned/state".into(),
            app_id: "00000000-0000-4000-8000-000000000001".into(),
            mounts: vec![],
        }
    }
    #[test]
    fn environment_is_explicit_and_headless_never_leaks_to_play() {
        let mut p = plan();
        let game = inner_command(&p).unwrap();
        assert_eq!(game.argv, p.args);
        assert!(game.program.ends_with("bin-wow64/wine"));
        assert_eq!(game.env["LANG"], "bn_BD.UTF-8");
        assert!(game.env["WINEDLLOVERRIDES"].contains("d3d12core=n"));
        assert!(!game.env["WINEDLLOVERRIDES"].contains("winevulkan=d"));
        assert_eq!(game.env["WINENTSYNC"], "0");
        assert_eq!(game.env["DXVK_ENABLE_NVAPI"], "0");
        p.headless = true;
        p.env.insert("DISPLAY".into(), ":1".into());
        let setup = inner_command(&p).unwrap();
        assert!(!setup.env.contains_key("DISPLAY"));
        assert!(setup.env["WINEDLLOVERRIDES"].contains("winevulkan=d"));
        p.headless = false;
        p.nvapi = true;
        let nvidia = inner_command(&p).unwrap();
        assert_eq!(nvidia.env["DXVK_ENABLE_NVAPI"], "1");
        assert!(nvidia.env["WINEDLLOVERRIDES"].contains("nvcuda=b"));
        assert_eq!(nvidia.env["VKD3D_CONFIG"], "force_raw_va_cbv");
        p.env.insert("VKD3D_CONFIG".into(), "nodxr;force_raw_va_cbv".into());
        assert_eq!(inner_command(&p).unwrap().env["VKD3D_CONFIG"], "nodxr,force_raw_va_cbv");
    }
    #[test]
    fn only_validated_identity_and_environment_cross_the_boundary() {
        let mut p = plan();
        p.engine = "/owned/../escape".into();
        assert!(validate(&p).is_err());
        let mut p = plan();
        p.env.insert("PYTHONPATH".into(), "/bad".into());
        assert!(validate(&p).is_err());
        assert!(allowed_env("WINEGDK_PREAUTH_DEVICE"));
        assert!(allowed_env("WINEGDK_XSTS_RP_HOST_EXAMPLE_COM"));
        assert!(!allowed_env("WINEGDK_XSTS_RP_BAD=KEY"));
        let p = plan();
        assert_eq!(serde_json::from_slice::<InnerPlan>(&serde_json::to_vec(&p).unwrap()).unwrap(), p);
    }
}

#[cfg(test)]
mod container_tests {
    use super::*;
    #[test]
    fn container_driver_rewrites_and_library_search_paths_survive_inner_exec() {
        let p: InnerPlan = serde_json::from_value(serde_json::json!({
            "schema":1,"engine":"/owned/engine","prefix":"/owned/prefix","program":"Wine","args":["C:\\game.exe"],
            "env":{"HOME":"/home/player","LANG":"bn_BD.UTF-8","VK_DRIVER_FILES":"/host/driver.json","DRI_PRIME":"1"},
            "cwd":"/owned/game","headless":false,"ntsync":false,"nvapi":false,"helper":"/owned/helper","plan_path":"/owned/plan","state":"/owned/state","mounts":[],"app_id":"00000000-0000-4000-8000-000000000001"
        })).unwrap();
        let mut inherited = BTreeMap::from([
            ("LD_LIBRARY_PATH".into(), "/runtime/overrides/lib".into()),
            ("VK_DRIVER_FILES".into(), "/runtime/driver.json".into()),
        ]);
        let c = inner_command_with_container(&p, &inherited).unwrap();
        assert!(c.env["LD_LIBRARY_PATH"].starts_with("/owned/engine/files/lib/"));
        assert!(c.env["LD_LIBRARY_PATH"].ends_with(":/runtime/overrides/lib"));
        assert_eq!(c.env["ORIG_LD_LIBRARY_PATH"], "/runtime/overrides/lib");
        assert_eq!(c.env["VK_DRIVER_FILES"], "/runtime/driver.json");
        assert_eq!(c.env["DRI_PRIME"], "1");
        inherited.insert("LD_PRELOAD".into(), "unapproved".into());
        assert!(inner_command_with_container(&p, &inherited).is_err());
    }
}
