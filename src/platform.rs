//! What computer we are on, in the words Mojang's JSON uses.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    Linux,
    Windows,
    Mac,
}

pub fn os() -> Os {
    if cfg!(target_os = "windows") {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    }
}

/// `linux`, `windows` or `osx` — the names used in library rules.
pub fn os_rule_name() -> &'static str {
    match os() {
        Os::Linux => "linux",
        Os::Windows => "windows",
        Os::Mac => "osx",
    }
}

/// `x86_64`, `aarch64`, ... as Rust names it.
pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

pub fn is_arm64() -> bool {
    arch() == "aarch64"
}

/// The key Mojang's Java runtime index uses for this computer.
pub fn java_runtime_platform() -> &'static str {
    match (os(), arch()) {
        (Os::Linux, "x86") => "linux-i386",
        (Os::Linux, _) => "linux",
        (Os::Mac, "aarch64") => "mac-os-arm64",
        (Os::Mac, _) => "mac-os",
        (Os::Windows, "aarch64") => "windows-arm64",
        (Os::Windows, "x86") => "windows-x86",
        (Os::Windows, _) => "windows-x64",
    }
}

pub fn path_separator() -> &'static str {
    if os() == Os::Windows { ";" } else { ":" }
}

pub fn java_exe() -> &'static str {
    if os() == Os::Windows { "java.exe" } else { "java" }
}

/// Total memory in MB (0 when unknown) and the part of it that is free right now.
pub fn memory_mb() -> (u64, u64) {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()));
    (sys.total_memory() / 1_048_576, sys.available_memory() / 1_048_576)
}

/// A child process that does not open a console window of its own on Windows (the launcher has no console, so every
/// program it starts would flash one). Everywhere else it is a plain `Command`.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// [`command`] for `tokio` processes.
pub fn tokio_command(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut c = tokio::process::Command::new(program);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
