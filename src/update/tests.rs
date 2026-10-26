//! Tests for the updater. The cases from the Java edition's `UpdatePlanTest` come first.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::apply::{self, Installed};
use super::check::{self, Found, Offers, UpdateStatus};
use super::download::{self, DownloadProgress};
use super::plan::*;
use super::release::{Asset, Release};

const CURRENT: [u32; 4] = [3, 4, 41, 4];

fn release(names: &[&str]) -> Release {
    Release {
        tag_name: "v3.4.42.0".to_string(),
        assets: names.iter().map(|n| Asset::new(n, &format!("https://example.invalid/{n}"), 10)).collect(),
        ..Release::default()
    }
}

fn picked(release: &Release, kind: Kind, arch: &str) -> Option<String> {
    pick_asset(release, kind, arch).map(|a| a.name.clone())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("blockforge-update-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---- ported from UpdatePlanTest.java -----------------------------------------------------------------------------

#[test]
fn reads_version_tags() {
    assert_eq!(parse_version("v3.4.42.0"), Some([3, 4, 42, 0]));
    assert_eq!(parse_version("3.4.42"), Some([3, 4, 42, 0]));
    assert_eq!(parse_version("V3.4.42.1-rc1"), Some([3, 4, 42, 1]));
    assert_eq!(parse_version("3.4.41.4.Beta"), Some([3, 4, 41, 4]));
    assert_eq!(parse_version("4.0.0"), Some([4, 0, 0, 0]));
    assert_eq!(parse_version("nightly"), None);
    assert_eq!(parse_version("v1.2.3.4.5"), None);
    assert_eq!(parse_version(""), None);
    assert_eq!(parse_version("1..2"), None);
    assert_eq!(parse_version("1.2.3+build"), Some([1, 2, 3, 0]));
}

#[test]
fn only_higher_versions_are_updates() {
    assert!(is_newer("v3.4.41.5", &CURRENT));
    assert!(is_newer("v3.4.42", &CURRENT));
    assert!(is_newer("v3.5.0.0", &CURRENT));
    assert!(is_newer("v4.0", &CURRENT));
    // numbers compare as numbers, not text
    assert!(is_newer("v3.4.100.0", &CURRENT));

    assert!(!is_newer("v3.4.41.4", &CURRENT), "the same version is never offered");
    assert!(!is_newer("v3.4.41.3", &CURRENT));
    assert!(!is_newer("v3.4.9.9", &CURRENT));
    assert!(!is_newer("nightly", &CURRENT));
}

#[test]
fn the_rust_version_is_newer_than_the_java_one() {
    assert!(is_newer("v4.0.0", &[3, 4, 42, 2]));
    assert!(!is_newer("v3.4.42.2", &parse_version("4.0.0").unwrap()));
    assert!(parse_version(crate::VERSION).is_some());
}

#[test]
fn picks_the_file_for_this_computer() {
    let release = release(&[
        "BlockForge-3.4.42.0-windows-x64-setup.exe",
        "BlockForge-3.4.42.0-macos-arm64.dmg",
        "BlockForge-3.4.42.0-macos-x64.dmg",
        "BlockForge-3.4.42.0-linux-amd64.deb",
        "BlockForge-3.4.42.0-linux-x86_64.rpm",
    ]);
    assert_eq!(picked(&release, Kind::MacApp, "aarch64").unwrap(), "BlockForge-3.4.42.0-macos-arm64.dmg");
    assert_eq!(picked(&release, Kind::MacApp, "x86_64").unwrap(), "BlockForge-3.4.42.0-macos-x64.dmg");
    assert_eq!(picked(&release, Kind::WindowsInstaller, "amd64").unwrap(), "BlockForge-3.4.42.0-windows-x64-setup.exe");
    assert_eq!(picked(&release, Kind::Deb, "amd64").unwrap(), "BlockForge-3.4.42.0-linux-amd64.deb");
    assert_eq!(picked(&release, Kind::Rpm, "amd64").unwrap(), "BlockForge-3.4.42.0-linux-x86_64.rpm");

    // an ARM Linux machine must never get the x86 package
    assert_eq!(picked(&release, Kind::Deb, "aarch64"), None);
    assert_eq!(picked(&release, Kind::PageOnly, "amd64"), None);
}

#[test]
fn accepts_files_that_dont_name_a_cpu() {
    let release = release(&[
        "BlockForge-3.4.42.0-windows-setup.exe",
        "BlockForge-3.4.42.0-debian-ubuntu.deb",
        "BlockForge-3.4.42.0-sources.zip",
    ]);
    assert_eq!(picked(&release, Kind::WindowsInstaller, "amd64").unwrap(), "BlockForge-3.4.42.0-windows-setup.exe");
    assert_eq!(picked(&release, Kind::Deb, "amd64").unwrap(), "BlockForge-3.4.42.0-debian-ubuntu.deb");
    assert_eq!(picked(&release, Kind::MacApp, "aarch64"), None);
}

#[test]
fn reads_the_github_release_json() {
    let json = r###"{"tag_name":"v3.4.42.0","name":"BlockForge 3.4.42.0","draft":false,"prerelease":false,
        "html_url":"https://github.com/o/r/releases/tag/v3.4.42.0","body":"## Changes\n- faster",
        "assets":[{"name":"BlockForge-3.4.42.0-linux-x86_64.deb","size":123,
        "browser_download_url":"https://github.com/o/r/releases/download/v3.4.42.0/BlockForge-3.4.42.0-linux-x86_64.deb",
        "digest":"sha256:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789"}]}"###;
    let release: Release = serde_json::from_str(json).unwrap();
    assert_eq!(release.tag_name, "v3.4.42.0");
    assert!(!release.prerelease);
    assert_eq!(release.assets.len(), 1);
    let mut asset = release.assets[0].clone();
    assert_eq!(asset.size, 123);
    assert!(asset.browser_download_url.ends_with("-linux-x86_64.deb"));
    assert_eq!(asset.sha256().unwrap(), "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789");

    asset.digest = Some("md5:abc".into());
    assert_eq!(asset.sha256(), None);
    asset.digest = Some("sha256:tooshort".into());
    assert_eq!(asset.sha256(), None);
    asset.digest = None;
    assert_eq!(asset.sha256(), None);
}

#[test]
fn hex_matches_a_known_digest() {
    assert_eq!(download::sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

// ---- the files of a real BlockForge release ----------------------------------------------------------------------

const ALL_ASSETS: [&str; 9] = [
    "BlockForge-4.1.0-windows-x64-setup.exe",
    "BlockForge-4.1.0-windows-x64-portable.zip",
    "BlockForge-4.1.0-macos-arm64.dmg",
    "BlockForge-4.1.0-linux-x86_64.deb",
    "BlockForge-4.1.0-linux-x86_64.rpm",
    "BlockForge-4.1.0-linux-x86_64.AppImage",
    "BlockForge-4.1.0-linux-aarch64.deb",
    "BlockForge-4.1.0-linux-aarch64.AppImage",
    "SHA256SUMS.txt",
];

#[test]
fn every_kind_gets_its_own_file_from_a_full_release() {
    let release = release(&ALL_ASSETS);
    let x = "x86_64";
    assert_eq!(picked(&release, Kind::WindowsInstaller, x).unwrap(), "BlockForge-4.1.0-windows-x64-setup.exe");
    assert_eq!(picked(&release, Kind::WindowsPortable, x).unwrap(), "BlockForge-4.1.0-windows-x64-portable.zip");
    assert_eq!(picked(&release, Kind::MacApp, "aarch64").unwrap(), "BlockForge-4.1.0-macos-arm64.dmg");
    assert_eq!(picked(&release, Kind::MacApp, x), None, "no Intel Mac disk image in this release");
    assert_eq!(picked(&release, Kind::Deb, x).unwrap(), "BlockForge-4.1.0-linux-x86_64.deb");
    assert_eq!(picked(&release, Kind::Rpm, x).unwrap(), "BlockForge-4.1.0-linux-x86_64.rpm");
    assert_eq!(picked(&release, Kind::AppImage, x).unwrap(), "BlockForge-4.1.0-linux-x86_64.AppImage");
    assert_eq!(picked(&release, Kind::Deb, "aarch64").unwrap(), "BlockForge-4.1.0-linux-aarch64.deb");
    assert_eq!(picked(&release, Kind::AppImage, "arm64").unwrap(), "BlockForge-4.1.0-linux-aarch64.AppImage");
    assert_eq!(picked(&release, Kind::Rpm, "aarch64"), None);
    assert_eq!(picked(&release, Kind::PageOnly, x), None);
}

#[test]
fn the_windows_installer_never_gets_the_zip_and_the_zip_never_the_installer() {
    for order in [ALL_ASSETS.to_vec(), ALL_ASSETS.iter().rev().copied().collect()] {
        let release = release(&order);
        let setup = picked(&release, Kind::WindowsInstaller, "x86_64").unwrap();
        assert!(setup.ends_with("-setup.exe"), "{setup}");
        let zip = picked(&release, Kind::WindowsPortable, "x86_64").unwrap();
        assert!(zip.ends_with("portable.zip"), "{zip}");
        let deb = picked(&release, Kind::Deb, "x86_64").unwrap();
        assert!(deb.ends_with(".deb") && deb.contains("x86_64"), "{deb}");
    }
    // a Windows ARM computer gets nothing rather than the x64 program
    assert_eq!(picked(&release(&ALL_ASSETS), Kind::WindowsInstaller, "aarch64"), None);
}

#[test]
fn amd64_and_x86_64_are_the_same_cpu() {
    let release = release(&["BlockForge-4.1.0-linux-amd64.deb", "BlockForge-4.1.0-linux-aarch64.deb"]);
    for arch in ["x86_64", "amd64", "AMD64", "x64"] {
        assert_eq!(picked(&release, Kind::Deb, arch).unwrap(), "BlockForge-4.1.0-linux-amd64.deb", "{arch}");
    }
    let release = super::tests::release(&["BlockForge-4.1.0-linux-x86_64.deb"]);
    assert_eq!(picked(&release, Kind::Deb, "amd64").unwrap(), "BlockForge-4.1.0-linux-x86_64.deb");
}

#[test]
fn a_cpu_name_inside_another_word_does_not_count() {
    // "x64" must be a word of its own, so a name like "box64" is not an x64 build
    let release = release(&["BlockForge-4.1.0-box64.deb", "BlockForge-4.1.0-linux-arm64.deb"]);
    assert_eq!(picked(&release, Kind::Deb, "x86_64").unwrap(), "BlockForge-4.1.0-box64.deb");
    assert_eq!(picked(&release, Kind::Deb, "aarch64").unwrap(), "BlockForge-4.1.0-linux-arm64.deb");
}

#[test]
fn assets_without_a_download_address_are_skipped() {
    let mut release = release(&["BlockForge-4.1.0-linux-x86_64.deb"]);
    release.assets[0].browser_download_url.clear();
    assert_eq!(picked(&release, Kind::Deb, "x86_64"), None);
}

// ---- how a copy was installed ------------------------------------------------------------------------------------

fn env(system: System, exe: &str) -> InstallEnv {
    InstallEnv {
        system,
        exe: Some(PathBuf::from(exe)),
        appimage: None,
        flatpak: false,
        os_release: None,
        has_uninstaller: false,
        package_owner: None,
    }
}

const UBUNTU: &str = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n";
const MINT: &str = "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
const FEDORA: &str = "NAME=\"Fedora Linux\"\nID=fedora\n";
const OPENSUSE: &str = "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n";
const ARCH: &str = "ID=arch\n";

#[test]
fn linux_packages_are_told_apart_by_the_distribution() {
    let with = |release: &str| InstallEnv {
        os_release: Some(release.to_string()),
        ..env(System::Linux, "/usr/bin/blockforge")
    };
    assert_eq!(detect_kind(&with(UBUNTU)), Kind::Deb);
    assert_eq!(detect_kind(&with(MINT)), Kind::Deb);
    assert_eq!(detect_kind(&with(FEDORA)), Kind::Rpm);
    assert_eq!(detect_kind(&with(OPENSUSE)), Kind::Rpm);
    assert_eq!(detect_kind(&with(ARCH)), Kind::PageOnly);
    assert_eq!(detect_kind(&env(System::Linux, "/usr/bin/blockforge")), Kind::PageOnly);
}

#[test]
fn the_package_database_wins_over_the_distribution_name() {
    let mut e = env(System::Linux, "/usr/bin/blockforge");
    e.package_owner = Some(PackageOwner::Dpkg);
    assert_eq!(detect_kind(&e), Kind::Deb);
    e.package_owner = Some(PackageOwner::Rpm);
    e.os_release = Some(ARCH.to_string());
    assert_eq!(detect_kind(&e), Kind::Rpm);
}

#[test]
fn an_appimage_is_found_from_the_environment_variable() {
    let mut e = env(System::Linux, "/tmp/.mount_BlockFxyz/usr/bin/blockforge");
    e.appimage = Some(PathBuf::from("/home/me/Apps/BlockForge.AppImage"));
    e.os_release = Some(UBUNTU.to_string());
    assert_eq!(detect_kind(&e), Kind::AppImage);
    // an empty variable means no AppImage
    e.appimage = Some(PathBuf::new());
    assert_eq!(detect_kind(&e), Kind::PageOnly);
}

#[test]
fn a_binary_outside_usr_bin_is_not_a_package() {
    for exe in ["/home/me/bin/blockforge", "/usr/local/bin/blockforge", "/opt/blockforge/blockforge"] {
        let mut e = env(System::Linux, exe);
        e.os_release = Some(UBUNTU.to_string());
        assert_eq!(detect_kind(&e), Kind::PageOnly, "{exe}");
    }
}

#[test]
fn flatpak_and_cargo_builds_only_open_the_page() {
    let mut e = env(System::Linux, "/usr/bin/blockforge");
    e.os_release = Some(UBUNTU.to_string());
    e.flatpak = true;
    assert_eq!(detect_kind(&e), Kind::PageOnly);
    e.flatpak = false;
    e.appimage = Some(PathBuf::from("/x/BlockForge.AppImage"));
    e.flatpak = true;
    assert_eq!(detect_kind(&e), Kind::PageOnly, "Flatpak wins over everything");

    for exe in [
        "/home/me/src/blockforge-rs/target/debug/blockforge",
        "/home/me/src/blockforge-rs/target/release/blockforge",
        "/home/me/src/blockforge-rs/target/release/deps/blockforge-1a2b3c",
        "/home/me/src/blockforge-rs/target/x86_64-unknown-linux-gnu/release/blockforge",
    ] {
        assert_eq!(detect_kind(&env(System::Linux, exe)), Kind::PageOnly, "{exe}");
    }
    let mut win = env(System::Windows, "C:/src/blockforge-rs/target/release/blockforge.exe");
    win.has_uninstaller = false;
    assert_eq!(detect_kind(&win), Kind::PageOnly);
}

#[test]
fn windows_installer_and_portable_are_told_apart_by_the_uninstaller() {
    let mut e = env(System::Windows, "C:/Program Files/BlockForge/blockforge.exe");
    e.has_uninstaller = true;
    assert_eq!(detect_kind(&e), Kind::WindowsInstaller);
    e.has_uninstaller = false;
    e.exe = Some(PathBuf::from("C:/Games/BlockForge/BlockForge.EXE"));
    assert_eq!(detect_kind(&e), Kind::WindowsPortable);
    e.exe = None;
    assert_eq!(detect_kind(&e), Kind::PageOnly);
}

#[test]
fn a_mac_app_is_found_by_its_bundle_path() {
    let e = env(System::Mac, "/Applications/BlockForge.app/Contents/MacOS/blockforge");
    assert_eq!(detect_kind(&e), Kind::MacApp);
    assert_eq!(detect_kind(&env(System::Mac, "/Users/me/bin/blockforge")), Kind::PageOnly);
}

#[test]
fn reads_the_linux_family() {
    assert_eq!(linux_family(MINT).trim(), "linuxmint ubuntu debian");
    assert!(linux_family("NAME=x\nVERSION_ID=\"22.04\"\n").trim().is_empty());
}

// ---- checking GitHub ----------------------------------------------------------------------------------------------

#[test]
fn drafts_prereleases_and_odd_tags_are_never_offered() {
    let mut r = release(&[]);
    r.tag_name = "v9.0.0".to_string();
    assert!(matches!(check::judge(r.clone(), &CURRENT), Found::Newer(_)));
    r.draft = true;
    assert!(matches!(check::judge(r.clone(), &CURRENT), Found::UpToDate));
    r.draft = false;
    r.prerelease = true;
    assert!(matches!(check::judge(r.clone(), &CURRENT), Found::UpToDate));
    r.prerelease = false;
    r.tag_name = "nightly".to_string();
    assert!(matches!(check::judge(r.clone(), &CURRENT), Found::UpToDate));
    r.tag_name = "v3.4.41.4".to_string();
    assert!(matches!(check::judge(r, &CURRENT), Found::UpToDate));
}

#[test]
fn a_release_is_offered_once_per_session_and_never_when_skipped() {
    let mut offers = Offers::default();
    assert!(offers.should_offer("v4.1.0", None));
    assert!(!offers.should_offer("v4.1.0", None), "already shown this session");
    assert!(offers.should_offer("v4.2.0", None), "a newer one is shown again");
    assert!(!offers.should_offer("v4.3.0", Some("v4.3.0")), "skipped by the player");
    assert!(offers.should_offer("v4.3.0", Some("v4.2.0")), "skipping an older one doesn't hide this");
}

#[test]
fn status_texts_for_the_about_page() {
    assert_eq!(UpdateStatus::default(), UpdateStatus::NotChecked);
    assert!(UpdateStatus::Checking.text().starts_with("Checking"));
    assert!(UpdateStatus::Checking.is_checking());
    assert_eq!(UpdateStatus::UpToDate.text(), "You have the latest version");
    assert_eq!(UpdateStatus::Available("4.1.0".into()).text(), "BlockForge 4.1.0 is available");
    assert_eq!(UpdateStatus::Available("4.1.0".into()).available(), Some("4.1.0"));
    assert!(UpdateStatus::Error("offline".into()).text().contains("offline"));
    assert!(UpdateStatus::Error(String::new()).is_error());
}

#[test]
fn the_api_address_can_be_overridden() {
    // only this test touches the variable
    unsafe { std::env::set_var(check::API_OVERRIDE, "http://127.0.0.1:1/latest") };
    assert_eq!(check::api_url(), "http://127.0.0.1:1/latest");
    unsafe { std::env::set_var(check::API_OVERRIDE, "  ") };
    assert_eq!(check::api_url(), check::LATEST_RELEASE_API);
    unsafe { std::env::remove_var(check::API_OVERRIDE) };
    assert_eq!(check::api_url(), check::LATEST_RELEASE_API);
}

// ---- checksums ----------------------------------------------------------------------------------------------------

#[test]
fn finds_a_checksum_in_the_checksum_file() {
    let a = "a".repeat(64);
    let b = "B".repeat(64);
    let sums =
        format!("{a}  BlockForge-4.1.0-linux-x86_64.deb\n{b} *BlockForge-4.1.0-windows-x64-setup.exe\nnot a line\n");
    assert_eq!(download::find_checksum(&sums, "BlockForge-4.1.0-linux-x86_64.deb").unwrap(), a);
    assert_eq!(download::find_checksum(&sums, "BlockForge-4.1.0-windows-x64-setup.exe").unwrap(), b.to_lowercase());
    assert_eq!(download::find_checksum(&sums, "missing.deb"), None);
    assert_eq!(download::find_checksum("zz  file.deb", "file.deb"), None, "not a SHA-256");
    assert_eq!(download::find_checksum(&format!("{a}  dist/file.deb"), "file.deb"), Some(a));
}

#[test]
fn verify_checks_size_and_checksum() {
    let digest = download::sha256_hex(b"hello");
    assert!(download::verify(5, &digest, 5, Some(&digest)).is_ok());
    assert!(download::verify(5, &digest, 0, None).is_ok(), "nothing promised, nothing to compare");
    let short = download::verify(4, &digest, 5, Some(&digest)).unwrap_err().to_string();
    assert!(short.contains("incomplete"), "{short}");
    let bad = download::verify(5, &digest, 5, Some(&download::sha256_hex(b"other"))).unwrap_err().to_string();
    assert!(bad.contains("damaged"), "{bad}");
    assert!(download::verify(5, &digest, 5, Some(&digest.to_uppercase())).is_ok());
}

#[test]
fn hashes_a_file() {
    let dir = scratch("hash");
    std::fs::write(dir.join("f"), b"abc").unwrap();
    assert_eq!(download::sha256_file(&dir.join("f")).unwrap(), download::sha256_hex(b"abc"));
}

// ---- putting the update in place ---------------------------------------------------------------------------------

#[test]
fn an_appimage_is_replaced_in_one_step_and_made_runnable() {
    let dir = scratch("appimage");
    let target = dir.join("BlockForge.AppImage");
    let new = dir.join("downloaded.AppImage");
    std::fs::write(&target, b"old").unwrap();
    std::fs::write(&new, b"new program").unwrap();

    apply::replace_executable(&new, &target).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"new program");
    assert!(!dir.join(".BlockForge.AppImage.new").exists(), "nothing staged is left behind");
    assert!(new.exists(), "the download itself is untouched");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o111, 0o111);
    }
}

#[test]
fn a_failed_appimage_replacement_leaves_the_old_one_alone() {
    let dir = scratch("appimage-fail");
    let target = dir.join("BlockForge.AppImage");
    std::fs::write(&target, b"old").unwrap();
    let err = apply::replace_executable(&dir.join("does-not-exist"), &target).unwrap_err();
    assert!(err.to_string().contains("can't write"), "{err}");
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    assert!(!dir.join(".BlockForge.AppImage.new").exists());
}

fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn the_portable_program_is_swapped_and_the_old_one_kept_as_dot_old() {
    let dir = scratch("portable");
    let current = dir.join("blockforge.exe");
    std::fs::write(&current, b"old exe").unwrap();
    let zip = dir.join("BlockForge-4.1.0-windows-x64-portable.zip");
    write_zip(
        &zip,
        &[
            ("README.txt", b"hi"),
            ("BlockForge/unins000.exe", b"uninstaller"),
            ("BlockForge/blockforge.exe", b"new exe"),
        ],
    );

    let old = apply::swap_portable_zip(&zip, &current).unwrap();

    assert_eq!(std::fs::read(&current).unwrap(), b"new exe");
    assert_eq!(old, dir.join("blockforge.exe.old"));
    assert_eq!(std::fs::read(&old).unwrap(), b"old exe");
    assert!(!dir.join("blockforge.exe.new").exists());

    // a second update replaces the first one's leftover
    write_zip(&zip, &[("blockforge.exe", b"newer exe")]);
    apply::swap_portable_zip(&zip, &current).unwrap();
    assert_eq!(std::fs::read(&current).unwrap(), b"newer exe");
    assert_eq!(std::fs::read(dir.join("blockforge.exe.old")).unwrap(), b"new exe");
}

#[test]
fn a_zip_without_the_program_changes_nothing() {
    let dir = scratch("portable-bad");
    let current = dir.join("blockforge.exe");
    std::fs::write(&current, b"old exe").unwrap();
    let zip = dir.join("update.zip");
    write_zip(&zip, &[("readme.txt", b"no program here")]);
    let err = apply::swap_portable_zip(&zip, &current).unwrap_err();
    assert!(err.to_string().contains("doesn't contain"), "{err}");
    assert_eq!(std::fs::read(&current).unwrap(), b"old exe");
    assert!(!dir.join("blockforge.exe.old").exists());
    assert!(!dir.join("blockforge.exe.new").exists());

    std::fs::write(&zip, b"this is not a zip").unwrap();
    assert!(apply::swap_portable_zip(&zip, &current).is_err());
    assert_eq!(std::fs::read(&current).unwrap(), b"old exe");
}

#[test]
fn a_swap_that_cannot_finish_puts_the_old_program_back() {
    let dir = scratch("swap-back");
    let current = dir.join("blockforge.exe");
    std::fs::write(&current, b"old exe").unwrap();
    // the new file is missing, so the second rename fails
    assert!(apply::swap_in(&dir.join("missing.new"), &current).is_err());
    assert_eq!(std::fs::read(&current).unwrap(), b"old exe");
    assert!(!dir.join("blockforge.exe.old").exists());
}

#[test]
fn package_commands_use_the_tools_that_exist() {
    let file = Path::new("/home/me/.local/share/blockforge/temp/update/BlockForge-4.1.0-linux-x86_64.deb");
    let all = |_: &str| true;
    let none = |_: &str| false;
    let deb = apply::package_command(Kind::Deb, file, all).unwrap();
    assert_eq!(&deb[..3], ["apt-get", "install", "-y"]);
    assert_eq!(deb[3], file.to_string_lossy());
    let rpm = apply::package_command(Kind::Rpm, Path::new("/x/a.rpm"), all).unwrap();
    assert_eq!(rpm, ["dnf", "install", "-y", "/x/a.rpm"]);
    let suse = apply::package_command(Kind::Rpm, Path::new("/x/a.rpm"), |t| t == "zypper").unwrap();
    assert_eq!(suse[0], "zypper");
    assert!(suse.contains(&"--allow-unsigned-rpm".to_string()));
    assert_eq!(apply::package_command(Kind::Deb, file, none), None);
    assert_eq!(apply::package_command(Kind::AppImage, file, all), None);

    assert_eq!(apply::manual_command(Kind::Deb, Path::new("/x/a.deb"), all), "sudo apt-get install -y /x/a.deb");
    assert_eq!(
        apply::manual_command(Kind::Rpm, Path::new("/x y/it's.rpm"), all),
        "sudo dnf install -y '/x y/it'\\''s.rpm'"
    );
    // with no tool found the usual one for the family is still named
    assert_eq!(apply::manual_command(Kind::Rpm, Path::new("/x/a.rpm"), none), "sudo dnf install -y /x/a.rpm");
}

#[test]
fn leftovers_in_the_update_folder_are_cleaned_up() {
    let temp = scratch("cleanup");
    let folder = apply::update_folder(&temp);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("BlockForge-4.1.0-linux-x86_64.deb"), b"x").unwrap();
    apply::cleanup_leftovers(&temp);
    assert!(!folder.exists());
}

#[test]
fn a_page_only_copy_is_never_installed() {
    let err = apply::install(Path::new("/nonexistent"), Kind::PageOnly).unwrap_err();
    assert!(err.to_string().contains("can't be updated automatically"));
    let _ = Installed::Exit;
}

// ---- downloading from a local server --------------------------------------------------------------------------------

/// A tiny HTTP server that answers every request with `body` (at most `requests` times).
async fn serve(body: Vec<u8>, requests: usize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..requests {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let mut buffer = [0u8; 2048];
            let _ = socket.read(&mut buffer).await;
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&body).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{address}")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn downloads_and_verifies_a_file() {
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let dir = scratch("download");
    runtime().block_on(async {
        let url = serve(body.clone(), 4).await;
        let mut asset = Asset::new("BlockForge-4.1.0-linux-x86_64.AppImage", &format!("{url}/file"), body.len() as u64);
        asset.digest = Some(format!("sha256:{}", download::sha256_hex(&body)));
        let progress = DownloadProgress::default();
        let cancel = AtomicBool::new(false);

        let expected = asset.sha256();
        let file = download::download(&asset, expected.as_deref(), &dir, &progress, &cancel).await.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), body);
        assert_eq!(progress.fraction(), 1.0);
        assert!(!dir.join("BlockForge-4.1.0-linux-x86_64.AppImage.part").exists());

        // a wrong checksum is refused and nothing is left behind
        std::fs::remove_file(&file).unwrap();
        let wrong = download::sha256_hex(b"something else");
        let err = download::download(&asset, Some(&wrong), &dir, &progress, &cancel).await.unwrap_err();
        assert!(err.to_string().contains("damaged"), "{err}");
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "no partial file");

        // a size that doesn't match is refused
        asset.size += 1;
        let err = download::download(&asset, None, &dir, &progress, &cancel).await.unwrap_err();
        assert!(err.to_string().contains("incomplete"), "{err}");

        // cancelling stops the download
        asset.size = body.len() as u64;
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let err = download::download(&asset, None, &dir, &progress, &cancel).await.unwrap_err();
        assert!(download::was_cancelled(&err), "{err:#}");
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
    });
}

#[test]
fn the_checksum_comes_from_the_checksum_file_when_the_asset_has_no_digest() {
    let dir_hash = download::sha256_hex(b"payload");
    runtime().block_on(async {
        let sums = format!("{dir_hash}  BlockForge-4.1.0-linux-x86_64.deb\n");
        let url = serve(sums.into_bytes(), 2).await;
        let mut rel = release(&["BlockForge-4.1.0-linux-x86_64.deb"]);
        rel.assets.push(Asset::new("SHA256SUMS.txt", &format!("{url}/SHA256SUMS.txt"), 0));
        let asset = rel.assets[0].clone();
        assert_eq!(download::expected_sha256(&rel, &asset).await.unwrap().unwrap(), dir_hash);

        // the asset's own digest is preferred
        let mut with_digest = asset.clone();
        with_digest.digest = Some(format!("sha256:{}", "c".repeat(64)));
        assert_eq!(download::expected_sha256(&rel, &with_digest).await.unwrap().unwrap(), "c".repeat(64));

        // no digest and no checksum file: nothing promised
        let bare = release(&["BlockForge-4.1.0-linux-x86_64.deb"]);
        assert_eq!(download::expected_sha256(&bare, &bare.assets[0]).await.unwrap(), None);
    });
}

#[test]
fn a_checksum_file_that_cant_be_read_stops_the_update_instead_of_skipping_the_check() {
    runtime().block_on(async {
        // the release lists a checksum file, but its address answers nothing (a dropped connection, a 404)
        let mut rel = release(&["BlockForge-4.1.0-linux-x86_64.deb"]);
        rel.assets.push(Asset::new("SHA256SUMS.txt", "http://127.0.0.1:9/SHA256SUMS.txt", 0));
        let asset = rel.assets[0].clone();
        let err = download::expected_sha256(&rel, &asset).await.unwrap_err();
        assert!(format!("{err:#}").contains("can't be checked"), "{err:#}");
    });
}

/// The end-to-end check for `scripts`/by hand: `BLOCKFORGE_UPDATE_API` points at a fake release (served by
/// `python3 -m http.server`), `BLOCKFORGE_TEST_APPIMAGE` is a scratch file standing in for the running AppImage.
/// Run with `cargo test --lib fake_release -- --ignored --nocapture`.
#[test]
#[ignore]
fn fake_release_end_to_end() {
    let Some(target) = std::env::var_os("BLOCKFORGE_TEST_APPIMAGE").map(PathBuf::from) else {
        panic!("set BLOCKFORGE_TEST_APPIMAGE to a scratch file");
    };
    let dir = scratch("fake-release");
    runtime().block_on(async {
        let found = check::check().await.unwrap();
        let Found::Newer(release) = found else { panic!("the fake release should be newer, got {found:?}") };
        println!("found {} with {} files", release.tag_name, release.assets.len());
        let asset = pick_asset(&release, Kind::AppImage, std::env::consts::ARCH).expect("an AppImage in the release");
        println!("picked {}", asset.name);
        let expected = download::expected_sha256(&release, asset).await.unwrap();
        println!("expected sha256 {expected:?}");
        assert!(expected.is_some());
        let progress = DownloadProgress::default();
        let file =
            download::download(asset, expected.as_deref(), &dir, &progress, &AtomicBool::new(false)).await.unwrap();
        println!("downloaded to {} ({} bytes)", file.display(), std::fs::metadata(&file).unwrap().len());
        apply::replace_executable(&file, &target).unwrap();
        println!("replaced {}", target.display());
    });
}
