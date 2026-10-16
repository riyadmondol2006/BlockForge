//! Putting the game's command line together: the class path, the Java options, the main class and the game's own
//! arguments. A port of the Java edition's `MCLauncher.getArguments`, written as a plain function of its inputs so the
//! order of everything can be tested without a game on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::instances::Instance;
use crate::launch::jvmargs;
use crate::launch::legacy;
use crate::launch::settings::{LaunchSettings, QuickPlay, QuickPlayOption, is_quick_play_supported};
use crate::model::{Arg, Features, Library, VersionJson, rules_allow};
use crate::paths::Dirs;
use crate::platform::{self, Os};

/// Arguments in Mojang's lists that the Java edition leaves out (they belong to Mojang's own launcher).
pub const IGNORED_ARGUMENTS: [&str; 4] = ["--clientId", "${clientid}", "--xuid", "${auth_xuid}"];

/// Who is playing.
#[derive(Clone, Debug)]
pub struct Player {
    pub name: String,
    /// The account's id without dashes.
    pub uuid: String,
    pub access_token: String,
    /// `msa`, or `legacy` for an offline player.
    pub user_type: String,
}

/// Everything the command line depends on.
pub struct Input<'a> {
    pub dirs: &'a Dirs,
    pub ls: &'a LaunchSettings,
    pub instance: &'a Instance,
    /// The game's version, with any library replacements already applied.
    pub version: &'a VersionJson,
    pub player: &'a Player,
    pub java_major: Option<u32>,
    pub natives_dir: &'a Path,
    pub lwjgl_natives_dir: Option<&'a Path>,
    pub game_assets: PathBuf,
    pub heap_mb: u32,
    pub window: (u32, u32),
    pub quick_play: QuickPlay,
    pub legacy: bool,
    pub os: Os,
    pub cores: usize,
    pub total_ram_mb: i64,
    /// The memory free right now (0 when unknown), for choosing how to start Java.
    pub available_ram_mb: i64,
    pub logging_config_exists: bool,
}

/// The command line, in pieces.
#[derive(Debug, Clone)]
pub struct Assembled {
    /// Java's own options (everything between the program and the main class).
    pub options: Vec<String>,
    pub main_class: String,
    /// What follows the main class.
    pub game: Vec<String>,
    pub classpath: String,
    pub notes: Vec<String>,
}

/// Java's version comparison for libraries: leading numbers of each dotted part. Unreadable versions count as equal.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Option<Vec<u64>> {
        v.split('.')
            .map(|p| {
                let d: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
                if d.is_empty() { None } else { d.parse().ok() }
            })
            .collect()
    };
    let (Some(x), Some(y)) = (parts(a), parts(b)) else { return std::cmp::Ordering::Equal };
    for i in 0..x.len().max(y.len()) {
        let o = x.get(i).copied().unwrap_or(0).cmp(&y.get(i).copied().unwrap_or(0));
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

/// `group:artifact[:classifier]` and the version of a library name; names that are not Maven names stand for
/// themselves with version 0 (the Java edition's `convertMavenIdentifierToNameAndVersion`).
pub fn name_and_version(identifier: &str) -> (String, String) {
    let parts: Vec<&str> = identifier.splitn(3, ':').collect();
    if parts.len() < 3 {
        return (identifier.to_string(), "0".to_string());
    }
    let mut version = parts[2].to_string();
    let mut classifier = String::new();
    if let Some(at) = version.find('@') {
        version.truncate(at);
    }
    if let Some(colon) = version.find(':') {
        classifier = format!(":{}", &version[colon + 1..]);
        version.truncate(colon);
    }
    (format!("{}:{}{}", parts[0], parts[1], classifier), version)
}

/// The class path of a game, and whether it has jar mods.
pub fn classpath(i: &Input, notes: &mut Vec<String>) -> (Vec<String>, bool) {
    let mut cp: Vec<String> = Vec::new();
    let mut has_jar_mods = false;
    let root = &i.instance.root;
    let features = Features::default();

    // jar mods go first so they replace the game's own classes
    if let Ok(rd) = std::fs::read_dir(root.join("jarmods")) {
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            has_jar_mods = true;
            cp.push(f.to_string_lossy().to_string());
        }
    }

    // libraries without natives, one per name, the newest version winning (in the place of the first)
    let mut order: Vec<String> = Vec::new();
    let mut chosen: HashMap<String, (&Library, String)> = HashMap::new();
    for lib in i.version.libraries.iter().filter(|l| rules_allow(&l.rules, &features)) {
        if lib.native().is_some() {
            continue;
        }
        let Some(artifact) = lib.artifact() else { continue };
        if artifact.path.is_none() {
            continue;
        }
        let (name, version) = name_and_version(&lib.name);
        match chosen.get(&name) {
            Some((_, existing)) => {
                if compare_versions(&version, existing).is_gt() {
                    chosen.insert(name, (lib, version));
                }
            }
            None => {
                order.push(name.clone());
                chosen.insert(name, (lib, version));
            }
        }
    }
    for name in &order {
        if let Some((lib, _)) = chosen.get(name) {
            if let Some(path) = lib.artifact().and_then(|a| a.path) {
                let full = i.dirs.libraries.join(path).to_string_lossy().to_string();
                if !cp.contains(&full) {
                    cp.push(full);
                }
            }
        }
    }
    // the jars of old-style natives libraries (the natives themselves are unpacked separately)
    for lib in i.version.libraries.iter().filter(|l| rules_allow(&l.rules, &features)) {
        if let Some(path) = lib.native().and_then(|n| n.path) {
            let full = i.dirs.libraries.join(path).to_string_lossy().to_string();
            if !cp.contains(&full) {
                cp.push(full);
            }
        }
    }

    // custom libraries the player or a modpack put in bin
    if let Ok(rd) = std::fs::read_dir(root.join("bin")) {
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let lower = n.to_lowercase();
                p.is_file()
                    && lower != "minecraft.jar"
                    && lower != "modpack.jar"
                    && (n.ends_with(".jar") || n.ends_with(".zip"))
            })
            .collect();
        files.sort();
        for f in files {
            notes.push(format!(
                "Added in custom library {}",
                f.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            ));
            cp.push(f.to_string_lossy().to_string());
        }
    }

    // the game's own jar goes last: the modpack's combined one when there is one. Forge and NeoForge (1.17 and newer)
    // start through a module bootstrap with their own patched game jar, so the vanilla one stays off the path
    if root.join("bin").join("modpack.jar").exists() {
        cp.push(root.join("bin").join("minecraft.jar").to_string_lossy().to_string());
    } else if !i.version.uses_module_bootstrap() {
        cp.push(crate::install::client_jar(i.dirs, i.version.game_version()).to_string_lossy().to_string());
    }
    if i.legacy {
        cp.push(legacy::jar_path(i.dirs).to_string_lossy().to_string());
    }
    (cp, has_jar_mods)
}

fn variables(i: &Input, classpath: &str) -> HashMap<&'static str, String> {
    let version = i.version;
    let p = i.player;
    let mut v: HashMap<&'static str, String> = HashMap::new();
    v.insert("auth_player_name", p.name.clone());
    v.insert("profile_name", i.instance.name().to_string());
    v.insert("user_properties", "[]".into());
    v.insert("version_name", i.instance.mc_version().to_string());
    v.insert("game_directory", i.instance.root.to_string_lossy().to_string());
    v.insert("game_assets", i.game_assets.to_string_lossy().to_string());
    v.insert("assets_root", i.dirs.assets.to_string_lossy().to_string());
    v.insert(
        "assets_index_name",
        version.assets.clone().or_else(|| version.asset_index.as_ref().map(|a| a.id.clone())).unwrap_or_default(),
    );
    v.insert("auth_uuid", p.uuid.clone());
    v.insert("auth_access_token", p.access_token.clone());
    v.insert("auth_session", p.access_token.clone());
    v.insert("user_type", p.user_type.clone());
    v.insert("version_type", if version.kind.is_empty() { "release".into() } else { version.kind.clone() });
    v.insert("launcher_name", crate::NAME.to_string());
    v.insert("launcher_version", crate::VERSION.to_string());
    v.insert("natives_directory", i.natives_dir.to_string_lossy().to_string());
    v.insert("library_directory", i.dirs.libraries.to_string_lossy().to_string());
    v.insert("classpath", classpath.to_string());
    v.insert("classpath_separator", platform::path_separator().to_string());
    v
}

pub fn substitute(arg: &str, vars: &HashMap<&'static str, String>) -> String {
    if !arg.contains("${") {
        return arg.to_string();
    }
    let mut out = arg.to_string();
    for (k, v) in vars {
        let token = format!("${{{k}}}");
        if out.contains(&token) {
            out = out.replace(&token, v);
        }
    }
    out
}

/// A game's argument lists with the rules applied. The Java edition never switches a feature on, so arguments that
/// need one (a custom resolution, a demo, Quick Play) are not in them; the launcher adds those itself.
pub fn expand(list: &[Arg]) -> Vec<String> {
    let features = Features::default();
    let mut out = Vec::new();
    for a in list {
        match a {
            Arg::Plain(s) => out.push(s.clone()),
            Arg::Ruled { rules, value } if rules_allow(rules, &features) => out.extend(value.to_vec()),
            Arg::Ruled { .. } => {}
        }
    }
    out
}

fn mem_metaspace_choice(i: &Input) -> (u32, bool) {
    // the player's setting, unless the pack needs more and the computer is big enough that it is not just a fraction
    // of its memory (the Java edition's rule, in its words: permGen < pack's, and RAM/8 < pack's)
    let perm = i.ls.meta_space_mb;
    let required = i.instance.file.launcher.required_perm_gen.max(0) as u32;
    if i.total_ram_mb != 0 && perm < required && (i.total_ram_mb / 8) < required as i64 {
        (required, true)
    } else {
        (perm, false)
    }
}

/// Builds the command line (without the program itself, the class cache and the wrapper).
pub fn assemble(i: &Input) -> Assembled {
    let mut notes = Vec::new();
    let (cp, has_jar_mods) = classpath(i, &mut notes);
    let classpath = cp.join(platform::path_separator());
    let vars = variables(i, &classpath);

    let mut options: Vec<String> = Vec::new();
    // no -Xms: the game starts small and grows as needed, which matters on computers with little memory
    options.push(format!("-Xmx{}M", i.heap_mb));
    let (meta, _) = mem_metaspace_choice(i);
    // if the version is unknown, assume Java 8 or newer since it's far more likely these days
    if i.java_major.is_none_or(|m| m >= 8) {
        options.push(format!("-XX:MetaspaceSize={meta}M"));
    } else {
        options.push(format!("-XX:PermSize={meta}M"));
    }
    options.push("-Duser.language=en".into());
    options.push("-Duser.country=US".into());
    if has_jar_mods {
        options.push("-Dfml.ignorePatchDiscrepancies=true".into());
        options.push("-Dfml.ignoreInvalidMinecraftCertificates=true".into());
    }
    if let Some(client) = i.version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        if i.logging_config_exists && !client.argument.is_empty() {
            let file = i.dirs.assets.join("log_configs").join(&client.file.id);
            options.push(client.argument.replace("${path}", &file.to_string_lossy()));
        }
    }
    if i.os == Os::Mac {
        options.push("-Dapple.laf.useScreenMenuBar=true".into());
        // no quotes: there's no shell in between, so they'd end up in the Dock as part of the name
        options.push(format!("-Xdock:name={}", i.instance.name()));
        let icon = i.game_assets.join("icons/minecraft.icns");
        if icon.exists() {
            options.push(format!("-Xdock:icon={}", icon.display()));
        }
    }
    if jvmargs::is_custom(Some(&i.ls.java_arguments)) {
        // the person's own arguments are used exactly as they are
        options.extend(jvmargs::split(&i.ls.java_arguments));
    } else {
        // nothing set, or BlockForge's defaults: the well known garbage collector settings, sized for the memory
        // this game gets
        jvmargs::add_missing(&mut options, &jvmargs::performance_flags_for(i.heap_mb, i.cores, i.java_major));
        // how Java's compiler is set up for the start, from what this computer has (never over the person's own)
        let startup =
            jvmargs::startup_flags(jvmargs::StartupMode::parse(&i.ls.startup_mode), i.cores, i.available_ram_mb);
        jvmargs::add_missing(&mut options, &startup.flags);
        notes.push(startup.note);
    }

    // the version's own Java options (the legacy string form has none)
    let (version_jvm, mut game_args): (Vec<String>, Vec<String>) = match &i.version.arguments {
        Some(a) => (expand(&a.jvm), expand(&a.game)),
        None => (Vec::new(), crate::launch::split_args(i.version.minecraft_arguments.as_deref().unwrap_or(""))),
    };
    if i.version.arguments.is_some() {
        if let Some(old) = i.version.minecraft_arguments.as_deref() {
            game_args.extend(crate::launch::split_args(old));
        }
    }
    for a in version_jvm.iter().filter(|a| !IGNORED_ARGUMENTS.contains(&a.as_str())) {
        options.push(substitute(a, &vars));
    }
    if i.os == Os::Windows
        && !options
            .iter()
            .any(|a| a == "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump")
    {
        options.push("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump".into());
    }
    // if there's no -Djava.library.path already, then add it (for older versions)
    if !options.iter().any(|a| a.starts_with("-Djava.library.path=")) {
        options.push(format!("-Djava.library.path={}", i.natives_dir.display()));
    }
    // the replacement LWJGL 2 natives for ARM
    if let Some(dir) = i.lwjgl_natives_dir {
        options.push(format!("-Dorg.lwjgl.librarypath={}", dir.display()));
    }
    // if there's no class path already, then add it (for older versions)
    if !options.iter().any(|a| a == "-cp" || a == "-classpath") {
        options.push("-cp".into());
        options.push(classpath.clone());
    }

    let mut game: Vec<String> = Vec::new();
    let main_class;
    if i.legacy {
        main_class = legacy::MAIN_CLASS.to_string();
        game.extend(legacy::wrapper_arguments(
            i.instance,
            &i.player.name,
            &i.player.access_token,
            i.window.0,
            i.window.1,
            i.ls.maximise_minecraft,
        ));
    } else {
        main_class = i.version.main_class.clone();
        for a in game_args.iter().filter(|a| !IGNORED_ARGUMENTS.contains(&a.as_str())) {
            game.push(substitute(a, &vars));
        }
        game.push(format!("--width={}", i.window.0));
        game.push(format!("--height={}", i.window.1));
    }

    // Quick Play: join a server, a world or a realm straight away
    let q = &i.quick_play;
    if let Some(address) = q.server_address.as_deref().filter(|s| !s.is_empty()) {
        if is_quick_play_supported(i.instance, QuickPlayOption::MultiPlayer) {
            // Minecraft 23w14a and newer versions
            game.extend(["--quickPlayMultiplayer".to_string(), address.to_string()]);
        } else {
            // Minecraft 23w13a and older versions
            let mut parts = address.split(':');
            let host = parts.next().unwrap_or(address);
            let port = parts.next().unwrap_or("25565");
            game.extend(["--server".to_string(), host.to_string(), "--port".to_string(), port.to_string()]);
        }
    }
    if let Some(world) = q.world_name.as_deref().filter(|s| !s.is_empty()) {
        if is_quick_play_supported(i.instance, QuickPlayOption::SinglePlayer) {
            // only works for Minecraft 23w14a and newer versions
            game.extend(["--quickPlaySingleplayer".to_string(), world.to_string()]);
        } else {
            notes.push("Warning: this Minecraft version can't open a world from the launcher (Quick Play needs 1.20 or newer), so it starts at the title screen".to_string());
        }
    }
    if let Some(realm) = q.realm_id.as_deref().filter(|s| !s.is_empty()) {
        if is_quick_play_supported(i.instance, QuickPlayOption::Realm) {
            game.extend(["--quickPlayRealms".to_string(), realm.to_string()]);
        } else {
            notes.push(
                "Warning: this Minecraft version can't join a realm from the launcher (Quick Play needs 1.20 or newer)"
                    .to_string(),
            );
        }
    }

    Assembled { options, main_class, game, classpath, notes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::instances::InstanceFile;

    fn player() -> Player {
        Player {
            name: "Steve".into(),
            uuid: "0123456789abcdef0123456789abcdef".into(),
            access_token: "TOKEN".into(),
            user_type: "msa".into(),
        }
    }

    fn instance(root: &Path, version_json: &str, launcher_json: &str) -> Instance {
        let mut file = InstanceFile::default();
        file.launcher = serde_json::from_str(launcher_json).unwrap();
        file.version = serde_json::from_str(version_json).unwrap();
        Instance { folder: "Pack".into(), mod_count: 0, root: root.to_path_buf(), file }
    }

    const MODERN: &str = r#"{"id":"1.21.1","type":"release","mainClass":"net.minecraft.client.main.Main","assets":"17",
        "assetIndex":{"id":"17","sha1":"x","size":1,"url":"u"},
        "logging":{"client":{"argument":"-Dlog4j.configurationFile=${path}","file":{"id":"client-1.12.xml","sha1":"x","size":1,"url":"u"},"type":"log4j2-xml"}},
        "libraries":[
          {"name":"org.lwjgl:lwjgl:3.3.3","downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar","sha1":"a","size":1,"url":"u"}}},
          {"name":"org.ow2.asm:asm:9.6","downloads":{"artifact":{"path":"org/ow2/asm/asm/9.6/asm-9.6.jar","sha1":"a","size":1,"url":"u"}}},
          {"name":"org.ow2.asm:asm:9.7","downloads":{"artifact":{"path":"org/ow2/asm/asm/9.7/asm-9.7.jar","sha1":"a","size":1,"url":"u"}}},
          {"name":"org.lwjgl:lwjgl:3.3.3:natives-linux","rules":[{"action":"allow","os":{"name":"linux"}}],"downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar","sha1":"a","size":1,"url":"u"}}},
          {"name":"org.lwjgl:lwjgl:3.3.3:natives-windows","rules":[{"action":"allow","os":{"name":"windows"}}],"downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar","sha1":"a","size":1,"url":"u"}}}
        ],
        "arguments":{"game":["--username","${auth_player_name}","--version","${version_name}","--gameDir","${game_directory}","--assetsDir","${assets_root}","--assetIndex","${assets_index_name}","--uuid","${auth_uuid}","--accessToken","${auth_access_token}","--clientId","${clientid}","--xuid","${auth_xuid}","--userType","${user_type}","--versionType","${version_type}",
          {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}","--height","${resolution_height}"]},
          {"rules":[{"action":"allow","features":{"is_quick_play_singleplayer":true}}],"value":["--quickPlaySingleplayer","${quickPlaySingleplayer}"]},
          {"rules":[{"action":"allow","features":{"is_quick_play_multiplayer":true}}],"value":["--quickPlayMultiplayer","${quickPlayMultiplayer}"]}],
          "jvm":[{"rules":[{"action":"allow","os":{"name":"osx"}}],"value":["-XstartOnFirstThread"]},"-Djava.library.path=${natives_directory}","-Dminecraft.launcher.brand=${launcher_name}","-cp","${classpath}"]}}"#;

    fn run(inst: &Instance, tweak: impl FnOnce(&mut Input)) -> Assembled {
        let dirs = Dirs::at("/data");
        let settings = Settings::default();
        let ls = LaunchSettings::resolve(&settings, inst);
        let p = player();
        let natives = PathBuf::from("/tmp/natives-1");
        let mut input = Input {
            dirs: &dirs,
            ls: &ls,
            instance: inst,
            version: &inst.file.version,
            player: &p,
            java_major: Some(21),
            natives_dir: &natives,
            lwjgl_natives_dir: None,
            game_assets: PathBuf::from("/data/assets/virtual/17"),
            heap_mb: 4096,
            window: (854, 480),
            quick_play: QuickPlay::default(),
            legacy: false,
            os: Os::Linux,
            cores: 8,
            total_ram_mb: 16000,
            available_ram_mb: 8000,
            logging_config_exists: true,
        };
        tweak(&mut input);
        assemble(&input)
    }

    #[test]
    fn a_modern_start_is_laid_out_like_the_java_edition() {
        let i = instance(Path::new("/data/instances/Pack"), MODERN, r#"{"name":"Pack"}"#);
        let a = run(&i, |_| {});
        assert_eq!(a.options[0], "-Xmx4096M");
        assert_eq!(a.options[1], "-XX:MetaspaceSize=256M");
        assert_eq!(&a.options[2..4], ["-Duser.language=en", "-Duser.country=US"]);
        assert_eq!(a.options[4], "-Dlog4j.configurationFile=/data/assets/log_configs/client-1.12.xml");
        // the default Java arguments are replaced with flags sized for the heap
        assert!(a.options.contains(&"-XX:+UseG1GC".to_string()));
        assert!(a.options.contains(&"-XX:G1HeapRegionSize=32M".to_string()));
        assert!(a.options.contains(&"-Dminecraft.launcher.brand=BlockForge".to_string()));
        assert!(!a.options.contains(&"-XstartOnFirstThread".to_string()));
        assert_eq!(a.options.iter().filter(|o| *o == "-cp").count(), 1);
        assert_eq!(a.main_class, "net.minecraft.client.main.Main");
        // Mojang's launcher-only arguments are gone and the resolution comes at the end as --width=
        let g = a.game.join(" ");
        assert!(g.starts_with("--username Steve --version 1.21.1 --gameDir /data/instances/Pack --assetsDir /data/assets --assetIndex 17 --uuid 0123456789abcdef0123456789abcdef --accessToken TOKEN --userType msa --versionType release"));
        assert!(!g.contains("clientId") && !g.contains("xuid") && !g.contains("quickPlay"));
        assert!(g.ends_with("--width=854 --height=480"));
        // newest asm only, in the place of the first; the natives of the other systems stay out
        let cp: Vec<&str> = a.classpath.split(':').collect();
        assert_eq!(cp[0], "/data/libraries/org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar");
        assert_eq!(cp[1], "/data/libraries/org/ow2/asm/asm/9.7/asm-9.7.jar");
        assert!(
            cp.iter().any(|p| p.ends_with("natives-linux.jar"))
                && !cp.iter().any(|p| p.ends_with("natives-windows.jar"))
        );
        assert!(!cp.iter().any(|p| p.ends_with("asm-9.6.jar")));
        assert_eq!(*cp.last().unwrap(), "/data/libraries/net/minecraft/client/1.21.1/client-1.21.1.jar");
    }

    #[test]
    fn the_players_own_java_arguments_are_used_as_they_are() {
        let i = instance(
            Path::new("/data/instances/Pack"),
            MODERN,
            r#"{"name":"Pack","javaArguments":"-Xss1M -Dname=\"two words\""}"#,
        );
        let a = run(&i, |_| {});
        assert_eq!(&a.options[5..7], ["-Xss1M", "-Dname=two words"]);
        assert!(!a.options.iter().any(|o| o.contains("G1")));
        // one core: G1 as well (Java's own choice there froze the game while worlds loaded)
        let d = instance(Path::new("/data/instances/Pack"), MODERN, r#"{"name":"Pack"}"#);
        assert!(run(&d, |i| i.cores = 1).options.contains(&"-XX:+UseG1GC".to_string()));
        assert!(run(&d, |_| {}).options.contains(&"-XX:+ExplicitGCInvokesConcurrent".to_string()));
        // the player's own arguments get nothing added
        assert!(!run(&i, |_| {}).options.iter().any(|o| o.contains("ExplicitGC")));
    }

    #[test]
    fn old_versions_get_a_class_path_and_natives_folder_added() {
        let old = r#"{"id":"1.8.9","type":"release","mainClass":"net.minecraft.client.main.Main","minecraftArguments":"--username ${auth_player_name} --session ${auth_session} --gameDir ${game_directory} --userProperties ${user_properties}","libraries":[]}"#;
        let i = instance(Path::new("/data/instances/Pack"), old, r#"{"name":"Pack"}"#);
        let a = run(&i, |i| i.java_major = Some(8));
        assert!(a.options.contains(&"-XX:MetaspaceSize=256M".to_string()));
        assert!(a.options.contains(&"-Djava.library.path=/tmp/natives-1".to_string()));
        let at = a.options.iter().position(|o| o == "-cp").unwrap();
        assert_eq!(a.options[at + 1], a.classpath);
        // Java 7 and older has a permanent generation instead
        let a7 = run(&i, |i| i.java_major = Some(7));
        assert!(a7.options.contains(&"-XX:PermSize=256M".to_string()));
        let g = a.game.join(" ");
        assert!(g.starts_with("--username Steve --session TOKEN --gameDir /data/instances/Pack --userProperties []"));
    }

    #[test]
    fn legacy_starts_go_through_the_wrapper() {
        let old = r#"{"id":"1.5.2","type":"release","mainClass":"net.minecraft.client.Minecraft","minecraftArguments":"${auth_player_name} ${auth_session}","libraries":[]}"#;
        let i = instance(Path::new("/data/instances/Pack"), old, r#"{"name":"Pack"}"#);
        let a = run(&i, |i| i.legacy = true);
        assert_eq!(a.main_class, "com.blockforge.launcher.mclauncher.legacy.LegacyMCLauncher");
        assert_eq!(a.game, vec!["/data/instances/Pack", "Steve", "TOKEN", "BlockForge - Pack", "854", "480", "false"]);
        assert!(a.classpath.ends_with("/data/libraries/launcher/legacy-launch.jar"));
        let maximised = run(&i, |i| {
            i.legacy = true;
            i.window = (1920, 1080);
        });
        assert_eq!(&maximised.game[4..6], ["1920", "1080"]);
    }

    #[test]
    fn quick_play_uses_the_new_arguments_where_the_version_has_them() {
        let i = instance(Path::new("/data/instances/Pack"), MODERN, r#"{"name":"Pack"}"#);
        let a = run(&i, |i| {
            i.quick_play = QuickPlay { server_address: Some("play.example.com:25566".into()), ..Default::default() }
        });
        assert_eq!(&a.game[a.game.len() - 2..], ["--quickPlayMultiplayer", "play.example.com:25566"]);
        let w = run(&i, |i| i.quick_play = QuickPlay { world_name: Some("My World".into()), ..Default::default() });
        assert_eq!(&w.game[w.game.len() - 2..], ["--quickPlaySingleplayer", "My World"]);
        // the realm is not in this version's list
        let r = run(&i, |i| i.quick_play = QuickPlay { realm_id: Some("1234".into()), ..Default::default() });
        assert!(!r.game.contains(&"--quickPlayRealms".to_string()));
        assert!(r.notes.iter().any(|n| n.contains("realm")));
        // an older version joins a server the old way, with the default port
        let old = r#"{"id":"1.8.9","type":"release","mainClass":"M","minecraftArguments":"--username ${auth_player_name}","libraries":[]}"#;
        let o = instance(Path::new("/data/instances/Pack"), old, r#"{"name":"Pack"}"#);
        let j = run(&o, |i| {
            i.quick_play = QuickPlay { server_address: Some("mc.example.com".into()), ..Default::default() }
        });
        assert_eq!(&j.game[j.game.len() - 4..], ["--server", "mc.example.com", "--port", "25565"]);
        let p = run(&o, |i| {
            i.quick_play = QuickPlay { server_address: Some("mc.example.com:1234".into()), ..Default::default() }
        });
        assert_eq!(&p.game[p.game.len() - 4..], ["--server", "mc.example.com", "--port", "1234"]);
        // worlds can't be opened by an old version
        let none = run(&o, |i| i.quick_play = QuickPlay { world_name: Some("W".into()), ..Default::default() });
        assert!(!none.game.iter().any(|a| a.contains("quickPlay")));
    }

    #[test]
    fn windows_and_mac_get_their_extras() {
        let i = instance(Path::new("/data/instances/Pack"), MODERN, r#"{"name":"Pack"}"#);
        let w = run(&i, |i| i.os = Os::Windows);
        assert!(w.options.contains(
            &"-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump".to_string()
        ));
        let m = run(&i, |i| i.os = Os::Mac);
        assert!(m.options.contains(&"-Dapple.laf.useScreenMenuBar=true".to_string()));
        assert!(m.options.contains(&"-Xdock:name=Pack".to_string()));
    }

    #[test]
    fn jar_mods_and_modpack_jars_change_the_path() {
        let dir = std::env::temp_dir().join(format!("bf-cmd2-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("jarmods")).unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("jarmods/forge.zip"), b"x").unwrap();
        std::fs::write(dir.join("bin/modpack.jar"), b"x").unwrap();
        std::fs::write(dir.join("bin/extra.jar"), b"x").unwrap();
        std::fs::write(dir.join("bin/minecraft.jar"), b"x").unwrap();
        let i = instance(&dir, MODERN, r#"{"name":"Pack"}"#);
        let a = run(&i, |_| {});
        let cp: Vec<&str> = a.classpath.split(':').collect();
        assert!(cp[0].ends_with("jarmods/forge.zip"));
        assert!(cp.iter().any(|p| p.ends_with("bin/extra.jar")));
        assert!(!cp.iter().any(|p| p.ends_with("bin/modpack.jar")));
        assert!(cp.last().unwrap().ends_with("bin/minecraft.jar"));
        assert!(a.options.contains(&"-Dfml.ignorePatchDiscrepancies=true".to_string()));
        assert!(a.notes.iter().any(|n| n == "Added in custom library extra.jar"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn module_bootstrap_loaders_keep_the_vanilla_jar_off() {
        let forge = MODERN.replace(r#""-Djava.library.path"#, r#""-DignoreList=a,b","-Djava.library.path"#);
        let i = instance(Path::new("/data/instances/Pack"), &forge, r#"{"name":"Pack"}"#);
        assert!(!run(&i, |_| {}).classpath.contains("client-1.21.1.jar"));
    }

    #[test]
    fn names_and_versions_of_libraries() {
        assert_eq!(name_and_version("org.lwjgl:lwjgl:3.3.3"), ("org.lwjgl:lwjgl".into(), "3.3.3".into()));
        assert_eq!(
            name_and_version("org.lwjgl:lwjgl:3.3.3:natives-linux"),
            ("org.lwjgl:lwjgl:natives-linux".into(), "3.3.3".into())
        );
        assert_eq!(name_and_version("a.b:c:1.0@zip"), ("a.b:c".into(), "1.0".into()));
        assert_eq!(name_and_version("old-style-name"), ("old-style-name".into(), "0".into()));
        assert!(compare_versions("9.7", "9.6").is_gt());
        assert!(compare_versions("10.0", "9.9").is_gt());
        assert!(compare_versions("1.0", "1.0.0").is_eq());
        assert!(compare_versions("abc", "1.0").is_eq());
    }

    #[test]
    fn a_pack_that_needs_more_metaspace_gets_it_on_small_computers_only() {
        let i = instance(Path::new("/data/instances/Pack"), MODERN, r#"{"name":"Pack","requiredPermGen":512}"#);
        // 16 GB / 8 = 2000 MB, bigger than the 512 the pack asks for: the player's 256 stands
        assert!(run(&i, |_| {}).options.contains(&"-XX:MetaspaceSize=256M".to_string()));
        // on a 2 GB computer 256 MB is a lot of what there is, so the pack's number is used
        assert!(run(&i, |i| i.total_ram_mb = 2048).options.contains(&"-XX:MetaspaceSize=512M".to_string()));
    }
}
