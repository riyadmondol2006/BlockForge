//! Minecraft for Windows played as it is: on Windows the game is a native Store package, so BlockForge finds it with
//! the system's package API (for the signed-in user, never PowerShell), starts it by activating the package, sends
//! the player to the Microsoft Store page when it isn't installed, and backs up its worlds from
//! `%APPDATA%\Minecraft Bedrock`. Nothing is downloaded or decrypted here. The facts are in WIN-MAC.md (package
//! family names, product ids, the `!Game` app id, the `minecraft:` URIs, the GDK worlds folder).
//!
//! The names and the parsing are plain Rust and tested everywhere; the system calls are Windows-only and run on the
//! runtime's blocking workers.

use std::path::PathBuf;

use super::{BuildId, Edition};

/// One installed Minecraft package, as the package API reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Package {
    pub edition: Edition,
    /// `Microsoft.MinecraftUWP_1.26.5004.0_x64__8wekyb3d8bbwe`
    pub full_name: String,
    /// `1.26.5004.0`
    pub version: String,
    /// The install folder, from the package API (the drive and the folder name are the player's choice).
    pub path: PathBuf,
}

impl Package {
    pub fn build_id(&self) -> BuildId {
        BuildId { edition: self.edition, version: self.version.clone(), package_id: self.full_name.clone() }
    }
}

/// The package family name.
#[cfg(any(windows, test))]
pub(super) fn family(e: Edition) -> &'static str {
    match e {
        Edition::Release => "Microsoft.MinecraftUWP_8wekyb3d8bbwe",
        Edition::Preview => "Microsoft.MinecraftWindowsBeta_8wekyb3d8bbwe",
    }
}

/// The Microsoft Store product id.
pub(super) fn product_id(e: Edition) -> &'static str {
    match e {
        Edition::Release => "9NBLGGH2JHXJ",
        Edition::Preview => "9P5X4QVLC2XR",
    }
}

/// The Store app's page for the game: where it is bought and installed.
pub fn store_page(e: Edition) -> String {
    format!("ms-windows-store://pdp/?ProductId={}", product_id(e))
}

/// The application to activate: the package family and the game's app id.
#[cfg(any(windows, test))]
pub(super) fn aumid(e: Edition) -> String {
    format!("{}!Game", family(e))
}

/// The address the package registers, used when activation itself isn't available.
#[cfg(any(windows, test))]
pub(super) fn uri(e: Edition) -> &'static str {
    match e {
        Edition::Release => "minecraft:",
        Edition::Preview => "minecraft-preview:",
    }
}

/// The game's own folder under `%APPDATA%` (GDK builds, 1.21.120 and later).
pub(super) fn data_folder(e: Edition) -> &'static str {
    match e {
        Edition::Release => "Minecraft Bedrock",
        Edition::Preview => "Minecraft Bedrock Preview",
    }
}

/// The players' folders of the game on this computer (`%APPDATA%\Minecraft Bedrock\Users`): one per signed-in
/// player, and `Shared`. Only the environment is read here, not the disk.
pub(super) fn users_folder(e: Edition) -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join(data_folder(e)).join("Users"))
}

/// The version in a package full name of `family` (`<name>_<version>_<arch>_<resource>_<publisher>`), if it is one.
#[cfg(any(windows, test))]
pub(super) fn version_of(full_name: &str, family: &str) -> Option<String> {
    let (name, publisher) = family.split_once('_')?;
    let mut parts = full_name.split('_');
    if !parts.next()?.eq_ignore_ascii_case(name) {
        return None;
    }
    let version = parts.next()?;
    let _arch = parts.next()?;
    let _resource = parts.next()?;
    if !parts.next()?.eq_ignore_ascii_case(publisher) || parts.next().is_some() {
        return None;
    }
    let numbers: Vec<&str> = version.split('.').collect();
    (numbers.len() == 4 && numbers.iter().all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))
        .then(|| version.to_string())
}

/// The numbers of a version, for comparing.
#[cfg(any(windows, test))]
fn version_key(v: &str) -> Vec<u64> {
    v.split('.').map(|n| n.parse().unwrap_or(0)).collect()
}

/// The newest of the packages found for one family (other launchers can register more than one).
#[cfg(any(windows, test))]
pub(super) fn newest(found: Vec<Package>) -> Option<Package> {
    found.into_iter().max_by(|a, b| version_key(&a.version).cmp(&version_key(&b.version)))
}

/// Whether a process's program is the game of the package installed at `install` (`Minecraft.Windows.exe` under it).
#[cfg(any(windows, test))]
pub(super) fn is_game_image(image: &str, install: &std::path::Path) -> bool {
    let image = image.replace('/', "\\").to_ascii_lowercase();
    let root = install.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    image.starts_with(&format!("{root}\\")) && image.ends_with("\\minecraft.windows.exe")
}

#[cfg(windows)]
pub(super) use sys::*;

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM,
        WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Storage::Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily};
    use windows_sys::Win32::System::Com::{
        CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE};
    use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};

    use super::super::{BedrockError, Edition, ErrorKind, Result, StopMode};
    use super::{Package, aumid, family, is_game_image, newest, uri, version_of};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn from_wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: the system hands back NUL-terminated strings inside the buffer it filled
        unsafe {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        }
    }

    /// The newest Minecraft package of `e` installed for the current user, if any. Blocking; for a worker.
    pub(in super::super) fn detect(e: Edition) -> Option<Package> {
        let fam = wide(family(e));
        let (mut count, mut len) = (0u32, 0u32);
        // SAFETY: the first call only asks for the sizes; the second gets buffers of exactly those sizes
        let first = unsafe {
            GetPackagesByPackageFamily(fam.as_ptr(), &mut count, std::ptr::null_mut(), &mut len, std::ptr::null_mut())
        };
        if first == ERROR_SUCCESS && count == 0 {
            return None;
        }
        if first != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        let mut names: Vec<PWSTR> = vec![std::ptr::null_mut(); count as usize];
        let mut buf: Vec<u16> = vec![0; len as usize];
        let second = unsafe {
            GetPackagesByPackageFamily(fam.as_ptr(), &mut count, names.as_mut_ptr(), &mut len, buf.as_mut_ptr())
        };
        if second != ERROR_SUCCESS {
            return None;
        }
        let found = names
            .iter()
            .take(count as usize)
            .filter_map(|&p| {
                let full_name = from_wide(p);
                let version = version_of(&full_name, family(e))?;
                let path = package_path(&full_name)?;
                Some(Package { edition: e, full_name, version, path })
            })
            .collect();
        newest(found)
    }

    fn package_path(full_name: &str) -> Option<PathBuf> {
        let name = wide(full_name);
        let mut len = 0u32;
        let first = unsafe { GetPackagePathByFullName(name.as_ptr(), &mut len, std::ptr::null_mut()) };
        if first != ERROR_INSUFFICIENT_BUFFER || len == 0 {
            return None;
        }
        let mut buf: Vec<u16> = vec![0; len as usize];
        let second = unsafe { GetPackagePathByFullName(name.as_ptr(), &mut len, buf.as_mut_ptr()) };
        (second == ERROR_SUCCESS).then(|| PathBuf::from(from_wide(buf.as_ptr())))
    }

    // IApplicationActivationManager, by hand: windows-sys has the functions of COM but not its interfaces, and this
    // one call isn't worth the whole `windows` crate.
    const CLSID_APPLICATION_ACTIVATION_MANAGER: GUID = GUID::from_u128(0x45ba127d_10a8_46ea_8ab7_56ea9078943c);
    const IID_IAPPLICATION_ACTIVATION_MANAGER: GUID = GUID::from_u128(0x2e941141_7f97_4756_ba1d_9decde894a3d);

    #[repr(C)]
    struct ActivationVtbl {
        _query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        _add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        activate_application: unsafe extern "system" fn(*mut c_void, PCWSTR, PCWSTR, i32, *mut u32) -> HRESULT,
    }

    /// Start the game the supported way, by activating its package, and return its process id. When activation
    /// isn't possible the package's `minecraft:` address is opened instead, and the id is found among the processes.
    /// Never elevated: BlockForge runs as the player. Blocking; for a worker.
    pub(in super::super) fn activate(e: Edition, install: &Path) -> Result<Option<u32>> {
        match activate_package(e) {
            Ok(pid) => Ok(Some(pid)),
            Err(why) => {
                crate::logs::warn(format!("Bedrock: activating the package failed ({why}); opening {}", uri(e)));
                let verb = wide("open");
                let file = wide(uri(e));
                let r = unsafe {
                    ShellExecuteW(
                        std::ptr::null_mut(),
                        verb.as_ptr(),
                        file.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        1,
                    )
                };
                // ShellExecute's success is a value above 32
                if (r as isize) <= 32 {
                    return Err(BedrockError::new(
                        ErrorKind::Runtime,
                        "Windows couldn't start Minecraft. Open it once from the Start menu, then try again.",
                    ));
                }
                Ok(find_game(install, Duration::from_secs(20)))
            }
        }
    }

    fn activate_package(e: Edition) -> std::result::Result<u32, String> {
        // SAFETY: COM is set up on this worker thread before the object is made, and the object is released after
        // its one call
        unsafe {
            // an apartment already set up on this thread (S_FALSE, or another model) is fine for one local call
            let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            let mut object: *mut c_void = std::ptr::null_mut();
            let hr = CoCreateInstance(
                &CLSID_APPLICATION_ACTIVATION_MANAGER,
                std::ptr::null_mut(),
                CLSCTX_LOCAL_SERVER,
                &IID_IAPPLICATION_ACTIVATION_MANAGER,
                &mut object,
            );
            if hr < 0 || object.is_null() {
                return Err(format!("CoCreateInstance 0x{hr:08x}"));
            }
            let vtbl = *(object as *const *const ActivationVtbl);
            let id = wide(&aumid(e));
            let none = wide("");
            let mut pid = 0u32;
            let hr = ((*vtbl).activate_application)(object, id.as_ptr(), none.as_ptr(), 0, &mut pid);
            ((*vtbl).release)(object);
            if hr < 0 { Err(format!("ActivateApplication 0x{hr:08x}")) } else { Ok(pid) }
        }
    }

    /// The process of the game installed at `install`, looked for until `within` has passed.
    fn find_game(install: &Path, within: Duration) -> Option<u32> {
        let until = Instant::now() + within;
        loop {
            if let Some(pid) = game_process(install) {
                return Some(pid);
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn game_process(install: &Path) -> Option<u32> {
        // SAFETY: the snapshot is closed below; entries are read into a structure of the size the call is told
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = None;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more {
                let exe = from_wide(entry.szExeFile.as_ptr());
                if exe.eq_ignore_ascii_case("Minecraft.Windows.exe")
                    && image_of(entry.th32ProcessID).is_some_and(|img| is_game_image(&img, install))
                {
                    found = Some(entry.th32ProcessID);
                    break;
                }
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            found
        }
    }

    /// Whether the game installed at `install` is running (started by BlockForge or not).
    pub(in super::super) fn running(install: &Path) -> bool {
        game_process(install).is_some()
    }

    fn image_of(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
            CloseHandle(h);
            ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    /// Wait until the game is over and return its exit code. Activation can hand back a helper that starts the game
    /// and ends, so once that ends the game's own process is looked for and followed. Blocking; for a worker.
    pub(in super::super) fn wait(pid: Option<u32>, install: &Path) -> Option<i32> {
        let mut current = pid.or_else(|| find_game(install, Duration::from_secs(20)));
        let mut code = None;
        while let Some(pid) = current {
            code = wait_one(pid);
            current = find_game(install, Duration::from_secs(5)).filter(|&p| p != pid);
        }
        code
    }

    fn wait_one(pid: u32) -> Option<i32> {
        unsafe {
            let h: HANDLE = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut code = None;
            if WaitForSingleObject(h, u32::MAX) == WAIT_OBJECT_0 {
                let mut c = 0u32;
                if GetExitCodeProcess(h, &mut c) != 0 {
                    code = Some(c as i32);
                }
            }
            CloseHandle(h);
            code
        }
    }

    unsafe extern "system" fn close_windows_of(hwnd: HWND, pid: LPARAM) -> i32 {
        let mut owner = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner as LPARAM == pid {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
        1
    }

    /// Close the game: asked to close its windows (it saves, as when its close box is pressed), or with Force ended
    /// at once. Blocking; for a worker.
    pub(in super::super) fn stop(install: &Path, mode: StopMode) -> Result<()> {
        let Some(pid) = game_process(install) else { return Ok(()) };
        match mode {
            StopMode::Graceful => {
                unsafe { EnumWindows(Some(close_windows_of), pid as LPARAM) };
                let until = Instant::now() + Duration::from_secs(20);
                while game_process(install) == Some(pid) {
                    if Instant::now() >= until {
                        return Err(BedrockError::new(
                            ErrorKind::Busy,
                            "Minecraft didn't close. Close it in the game, or press Stop again to end it.",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
                Ok(())
            }
            StopMode::Force => unsafe {
                let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if h.is_null() {
                    return Err(BedrockError::new(ErrorKind::Runtime, "Windows didn't let BlockForge end Minecraft"));
                }
                TerminateProcess(h, 1);
                CloseHandle(h);
                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_give_their_version_only_for_their_own_family() {
        let release = family(Edition::Release);
        assert_eq!(
            version_of("Microsoft.MinecraftUWP_1.26.5004.0_x64__8wekyb3d8bbwe", release).as_deref(),
            Some("1.26.5004.0")
        );
        // the wiki writes it in capitals
        assert_eq!(
            version_of("MICROSOFT.MINECRAFTUWP_1.26.5004.0_x64__8wekyb3d8bbwe", release).as_deref(),
            Some("1.26.5004.0")
        );
        assert_eq!(
            version_of("Microsoft.MinecraftWindowsBeta_1.26.7022.0_x64__8wekyb3d8bbwe", family(Edition::Preview))
                .as_deref(),
            Some("1.26.7022.0")
        );
        assert_eq!(version_of("Microsoft.MinecraftWindowsBeta_1.26.7022.0_x64__8wekyb3d8bbwe", release), None);
        assert_eq!(version_of("Microsoft.MinecraftUWP_1.26_x64__8wekyb3d8bbwe", release), None);
        assert_eq!(version_of("Microsoft.MinecraftUWP_1.26.5004.0_x64__otherpublisher", release), None);
        assert_eq!(version_of("Microsoft.MinecraftUWP_1.26.5004.0_x64__8wekyb3d8bbwe_extra", release), None);
    }

    #[test]
    fn the_newest_registered_copy_is_the_one_played() {
        let pkg = |v: &str| Package {
            edition: Edition::Release,
            full_name: format!("Microsoft.MinecraftUWP_{v}_x64__8wekyb3d8bbwe"),
            version: v.into(),
            path: PathBuf::from("C:\\XboxGames\\Minecraft for Windows\\Content"),
        };
        let picked = newest(vec![pkg("1.21.12001.0"), pkg("1.26.5004.0"), pkg("1.26.402.0")]).unwrap();
        assert_eq!(picked.version, "1.26.5004.0");
        assert_eq!(picked.build_id().package_id, picked.full_name);
        assert!(newest(Vec::new()).is_none());
    }

    #[test]
    fn the_ways_to_the_game_are_the_documented_ones() {
        assert_eq!(store_page(Edition::Release), "ms-windows-store://pdp/?ProductId=9NBLGGH2JHXJ");
        assert_eq!(store_page(Edition::Preview), "ms-windows-store://pdp/?ProductId=9P5X4QVLC2XR");
        assert_eq!(aumid(Edition::Release), "Microsoft.MinecraftUWP_8wekyb3d8bbwe!Game");
        assert_eq!(aumid(Edition::Preview), "Microsoft.MinecraftWindowsBeta_8wekyb3d8bbwe!Game");
        assert_eq!(uri(Edition::Preview), "minecraft-preview:");
        assert_eq!(data_folder(Edition::Release), "Minecraft Bedrock");
    }

    #[test]
    fn only_the_game_under_its_own_install_is_followed() {
        let install = std::path::Path::new("D:\\XboxGames\\Minecraft for Windows\\Content\\");
        assert!(is_game_image("D:\\XboxGames\\Minecraft for Windows\\Content\\Minecraft.Windows.exe", install));
        assert!(is_game_image("d:/xboxgames/minecraft for windows/content/minecraft.windows.exe", install));
        assert!(!is_game_image(
            "D:\\XboxGames\\Minecraft Preview for Windows\\Content\\Minecraft.Windows.exe",
            install
        ));
        assert!(!is_game_image("D:\\XboxGames\\Minecraft for Windows\\Content\\gamelaunchhelper.exe", install));
    }
}
