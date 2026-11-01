#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn main() {
    use std::{
        io::Read,
        os::unix::{fs::MetadataExt, process::CommandExt},
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut args = std::env::args_os().skip(1);
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--plan")) {
            return Err("Expected --plan <private-file>".into());
        }
        let path = args.next().ok_or("Missing plan path")?;
        if args.next().is_some() {
            return Err("Unexpected runtime arguments".into());
        }
        let meta = std::fs::symlink_metadata(&path)?;
        let own = std::fs::metadata("/proc/self")?.uid();
        if !meta.is_file() || meta.uid() != own || meta.mode() & 0o077 != 0 || meta.len() > 1024 * 1024 {
            return Err("Runtime plan must be a private owned regular file under 1 MiB".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        let plan: bedrock_runtime::InnerPlan = serde_json::from_slice(&bytes)?;
        // Inside the container /usr is SteamRT's. On the host this refuses, so Wine never runs on host libraries.
        let os = std::fs::read_to_string("/usr/lib/os-release")?;
        let build = format!("BUILD_ID=\"{}\"", bedrock_runtime::STEAMRT_BUILD);
        if !os.lines().any(|l| l == "ID=steamrt") || !os.lines().any(|l| l == build) {
            return Err("Expected pinned SteamRT3 container; refusing host Wine fallback".into());
        }
        let container = bedrock_runtime::CONTAINER_ENV
            .iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
            .collect();
        let spec = bedrock_runtime::inner_command_with_container(&plan, &container)?;
        eprintln!("bedrock-runtime: inside SteamRT3 {}", bedrock_runtime::STEAMRT_BUILD);
        // The prefix's fonts are links into the engine; this says in the log whether the container can see them.
        // A missing font is worth a line, not a refusal to start.
        let (mut readable, mut unreadable) = (0, 0);
        if let Ok(dir) = std::fs::read_dir(plan.prefix.join("drive_c/windows/Fonts")) {
            for entry in dir.flatten() {
                let mut first = [0; 1];
                match std::fs::File::open(entry.path()).and_then(|mut f| f.read_exact(&mut first)) {
                    Ok(()) => readable += 1,
                    Err(_) => unreadable += 1,
                }
            }
        }
        eprintln!("bedrock-runtime: {readable} font files readable inside container, {unreadable} unreadable");
        Err(spec.command().exec().into())
    })();
    if let Err(e) = result {
        eprintln!("bedrock-runtime: {e}");
        std::process::exit(125);
    }
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {
    eprintln!("Bedrock runtime is Linux-only");
    std::process::exit(125);
}
