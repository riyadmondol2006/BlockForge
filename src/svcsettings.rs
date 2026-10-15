//! The settings this part of the launcher reads from `BlockForge.json`.
//!
//! The typed `Settings` struct only names the settings the rest of the program uses; everything else stays in
//! `Settings::extras` under the Java edition's field name. This module reads those with the Java edition's
//! defaults and checks them with its rules (`Settings.validate*`), and writes them back.

use serde_json::Value;

use crate::backups::BackupMode;
use crate::config::Settings;
use crate::net::ProxyType;
use crate::paths::Dirs;

/// The default number of files downloaded at once (`Settings.DEFAULT_CONCURRENT_CONNECTIONS`).
pub const DEFAULT_CONCURRENT_CONNECTIONS: u32 = 16;
pub const DEFAULT_CONNECTION_TIMEOUT: u32 = 60;
pub const DEFAULT_PROXY_PORT: u32 = 8080;

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceSettings {
    // Network
    pub concurrent_connections: u32,
    /// Seconds, for connecting and for waiting on data.
    pub connection_timeout: u32,
    pub modrinth_api_key: Option<String>,
    pub enable_proxy: bool,
    pub proxy_host: String,
    pub proxy_port: u32,
    pub proxy_type: ProxyType,
    // Folders
    pub custom_downloads_path: Option<String>,
    pub backups_path: Option<String>,
    // Backups
    pub enable_automatic_backup_after_launch: bool,
    pub backup_mode: BackupMode,
    // Logging and speed
    pub enable_logs: bool,
    pub enable_analytics: bool,
    pub analytics_client_id: String,
    pub class_data_cache: bool,
    pub warm_up_game_files: bool,
}

impl Default for ServiceSettings {
    fn default() -> Self {
        ServiceSettings {
            concurrent_connections: DEFAULT_CONCURRENT_CONNECTIONS,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
            modrinth_api_key: None,
            enable_proxy: false,
            proxy_host: String::new(),
            proxy_port: DEFAULT_PROXY_PORT,
            proxy_type: ProxyType::Http,
            custom_downloads_path: None,
            backups_path: None,
            enable_automatic_backup_after_launch: false,
            backup_mode: BackupMode::Normal,
            enable_logs: false,
            enable_analytics: false,
            analytics_client_id: uuid::Uuid::new_v4().to_string(),
            class_data_cache: true,
            warm_up_game_files: true,
        }
    }
}

/// The analytics id that older Java builds shipped; it is replaced like a missing one.
const OLD_SHARED_ANALYTICS_ID: &str = "30662333-d88f-4e21-8d77-95739af9bf78";

fn get_u32(settings: &Settings, key: &str) -> Option<i64> {
    settings.extras.get(key).and_then(|v| v.as_i64())
}

fn get_bool(settings: &Settings, key: &str) -> Option<bool> {
    // these four are typed fields of `Settings`, so they are not in `extras`
    match key {
        "enableLogs" => Some(settings.enable_logs),
        "enableAnalytics" => Some(settings.enable_analytics),
        "classDataCache" => Some(settings.class_data_cache),
        "enableConsole" => Some(settings.enable_console),
        _ => settings.extras.get(key).and_then(|v| v.as_bool()),
    }
}

fn get_string(settings: &Settings, key: &str) -> Option<String> {
    settings.extras.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

impl ServiceSettings {
    /// Read the settings, fixing values the Java edition would also reject (it logs a warning and puts the default
    /// back). The warnings are returned so the caller can log them.
    pub fn read(settings: &Settings) -> (ServiceSettings, Vec<String>) {
        let mut s = ServiceSettings::default();
        let mut warnings = Vec::new();

        if let Some(v) = get_u32(settings, "concurrentConnections") {
            if (1..=100).contains(&v) {
                s.concurrent_connections = v as u32;
            } else {
                warnings.push(format!(
                    "Tried to set the number of concurrent connections to {v} which is not valid! Must be between 1 and 100. Setting back to default of {DEFAULT_CONCURRENT_CONNECTIONS}!"
                ));
            }
        }
        if let Some(v) = get_u32(settings, "connectionTimeout") {
            if (1..=600).contains(&v) {
                s.connection_timeout = v as u32;
            } else {
                warnings.push(format!("Tried to set the number of connection timeout to {v} which is not valid! Must be between 1 and 600. Setting back to default of 60!"));
            }
        }
        s.modrinth_api_key = get_string(settings, "modrinthApiKey").filter(|k| !k.trim().is_empty());

        s.enable_proxy = get_bool(settings, "enableProxy").unwrap_or(false);
        s.proxy_host = get_string(settings, "proxyHost").unwrap_or_default();
        if let Some(v) = get_u32(settings, "proxyPort") {
            s.proxy_port = v.clamp(0, u32::MAX as i64) as u32;
        }
        let proxy_type = get_string(settings, "proxyType");
        match proxy_type.as_deref().map(ProxyType::from_key) {
            Some(Some(t)) => s.proxy_type = t,
            Some(None) => {
                if s.enable_proxy {
                    warnings.push(format!(
                        "Tried to set proxy type to {} which is not valid! Proxy support disabled!",
                        proxy_type.unwrap_or_default()
                    ));
                    s.enable_proxy = false;
                }
            }
            None => {}
        }
        if s.enable_proxy && !(1..=65535).contains(&s.proxy_port) {
            warnings.push(format!(
                "Tried to set proxy port to {} which is not a valid port! Proxy support disabled!",
                s.proxy_port
            ));
            s.enable_proxy = false;
            s.proxy_port = DEFAULT_PROXY_PORT;
        }

        s.custom_downloads_path = get_string(settings, "customDownloadsPath").filter(|p| !p.is_empty());
        if let Some(p) = &s.custom_downloads_path {
            if !std::path::Path::new(p).is_dir() {
                warnings.push("Custom Downloads Path Is Incorrect! Deleting setting!".to_string());
                s.custom_downloads_path = None;
            }
        }
        s.backups_path = get_string(settings, "backupsPath").filter(|p| !p.is_empty());
        if let Some(p) = &s.backups_path {
            if !std::path::Path::new(p).is_dir() {
                warnings.push("Backups Path Is Incorrect! Deleting setting!".to_string());
                s.backups_path = None;
            }
        }

        s.enable_automatic_backup_after_launch =
            get_bool(settings, "enableAutomaticBackupAfterLaunch").unwrap_or(false);
        if let Some(mode) = get_string(settings, "backupMode").as_deref().and_then(BackupMode::from_key) {
            s.backup_mode = mode;
        }

        s.enable_logs = get_bool(settings, "enableLogs").unwrap_or(false);
        s.enable_analytics = get_bool(settings, "enableAnalytics").unwrap_or(false);
        if let Some(id) = get_string(settings, "analyticsClientId").filter(|i| !i.is_empty()) {
            s.analytics_client_id = id;
        }
        if s.enable_analytics && s.analytics_client_id.eq_ignore_ascii_case(OLD_SHARED_ANALYTICS_ID) {
            s.analytics_client_id = uuid::Uuid::new_v4().to_string();
        }
        s.class_data_cache = get_bool(settings, "classDataCache").unwrap_or(true);
        s.warm_up_game_files = get_bool(settings, "warmUpGameFiles").unwrap_or(true);
        (s, warnings)
    }

    /// Just the values, for callers that don't care about the warnings.
    pub fn from_settings(settings: &Settings) -> ServiceSettings {
        ServiceSettings::read(settings).0
    }

    /// Write every setting back under the Java edition's names. A missing path or key is removed rather than
    /// saved as an empty string, as Gson does with a null.
    pub fn write_to(&self, settings: &mut Settings) {
        let e = &mut settings.extras;
        e.insert("concurrentConnections".into(), Value::from(self.concurrent_connections));
        e.insert("connectionTimeout".into(), Value::from(self.connection_timeout));
        set_optional(e, "modrinthApiKey", self.modrinth_api_key.as_deref());
        e.insert("enableProxy".into(), Value::from(self.enable_proxy));
        e.insert("proxyHost".into(), Value::from(self.proxy_host.clone()));
        e.insert("proxyPort".into(), Value::from(self.proxy_port));
        e.insert("proxyType".into(), Value::from(self.proxy_type.key()));
        set_optional(e, "customDownloadsPath", self.custom_downloads_path.as_deref());
        set_optional(e, "backupsPath", self.backups_path.as_deref());
        e.insert("enableAutomaticBackupAfterLaunch".into(), Value::from(self.enable_automatic_backup_after_launch));
        e.insert("backupMode".into(), Value::from(self.backup_mode.key()));
        settings.enable_logs = self.enable_logs;
        settings.enable_analytics = self.enable_analytics;
        let e = &mut settings.extras;
        e.insert("analyticsClientId".into(), Value::from(self.analytics_client_id.clone()));
        settings.class_data_cache = self.class_data_cache;
        e.insert("warmUpGameFiles".into(), Value::from(self.warm_up_game_files));
    }
}

fn set_optional(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<&str>) {
    match value.filter(|v| !v.is_empty()) {
        Some(v) => {
            map.insert(key.to_string(), Value::from(v));
        }
        None => {
            map.remove(key);
        }
    }
}

/// Apply the proxy, timeout and connection limit from the settings to the shared HTTP client. Call it once,
/// before the first request. (Command-line use calls just this; the window calls [`init`], which does it too.)
pub fn init_network(dirs: &Dirs) -> ServiceSettings {
    let settings = Settings::load(dirs);
    let (svc, warnings) = ServiceSettings::read(&settings);
    for w in warnings {
        crate::logs::warn(w);
    }
    crate::net::set_modrinth_api_key(svc.modrinth_api_key.clone());
    let config = crate::net::NetConfig::from_settings(&svc);
    if let Some(p) = &config.proxy {
        crate::logs::warn(format!("Proxy set to {} {}:{}", p.kind.key(), p.host, p.port));
    }
    crate::http::configure(config);
    svc
}

/// Start-up for the window: opens the launcher log (`logs/blockforge.log`, rolling the last run's into
/// `logs/old/`), applies the network settings (see [`init_network`]) and writes the usual start-up lines to the log.
/// `debug` is the `--debug` flag and `debug_level` the `--debug-level` number. Returns the settings it read.
pub fn init(dirs: &Dirs, debug: bool, debug_level: Option<u8>) -> ServiceSettings {
    if let Some(level) = debug_level {
        crate::logs::set_debug_level(level);
    }
    if let Err(e) = crate::logs::init(dirs, debug) {
        eprintln!("Couldn't open the log file: {e}");
    }
    let svc = init_network(dirs);

    use sysinfo::System;
    let (total_mb, free_mb) = crate::platform::memory_mb();
    crate::logs::info(format!("{} Version: {}", crate::NAME, crate::VERSION));
    crate::logs::info(format!("App Arguments: {:?}", std::env::args().skip(1).collect::<Vec<_>>()));
    crate::logs::info(format!(
        "Operating System: {} ({})",
        System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_string()),
        std::env::consts::ARCH
    ));
    crate::logs::info(format!("RAM Available: {free_mb}MB of {total_mb}MB"));
    crate::logs::info(format!("Launcher Directory: {}", dirs.base.display()));
    svc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(json: &str) -> Settings {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn defaults_match_the_java_edition() {
        let (s, w) = ServiceSettings::read(&Settings::default());
        assert!(w.is_empty());
        assert_eq!((s.concurrent_connections, s.connection_timeout, s.proxy_port), (16, 60, 8080));
        assert_eq!(s.proxy_type, ProxyType::Http);
        assert!(!s.enable_proxy && !s.enable_logs && !s.enable_automatic_backup_after_launch);
        assert_eq!(s.backup_mode, BackupMode::Normal);
        assert!(s.class_data_cache && s.warm_up_game_files);
        assert_eq!(s.analytics_client_id.len(), 36);
    }

    #[test]
    fn reads_a_java_written_file() {
        let (s, w) = ServiceSettings::read(&settings(
            r#"{"concurrentConnections":32,"connectionTimeout":120,"enableProxy":true,"proxyHost":"10.0.0.2","proxyPort":3128,
            "proxyType":"SOCKS","backupMode":"NORMAL_PLUS_MODS","enableAutomaticBackupAfterLaunch":true,"enableLogs":true,
            "modrinthApiKey":"mrp_abc","classDataCache":false}"#,
        ));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((s.concurrent_connections, s.connection_timeout), (32, 120));
        assert!(s.enable_proxy);
        assert_eq!((s.proxy_host.as_str(), s.proxy_port, s.proxy_type), ("10.0.0.2", 3128, ProxyType::Socks));
        assert_eq!(s.backup_mode, BackupMode::NormalPlusMods);
        assert!(s.enable_automatic_backup_after_launch && s.enable_logs && !s.class_data_cache);
        assert_eq!(s.modrinth_api_key.as_deref(), Some("mrp_abc"));
    }

    #[test]
    fn invalid_values_are_put_back_like_java_does() {
        let (s, w) = ServiceSettings::read(&settings(
            r#"{"concurrentConnections":500,"connectionTimeout":0,"enableProxy":true,"proxyPort":70000,"proxyType":"HTTP",
            "backupsPath":"/definitely/not/here"}"#,
        ));
        assert_eq!((s.concurrent_connections, s.connection_timeout), (16, 60));
        assert!(!s.enable_proxy);
        assert_eq!(s.proxy_port, 8080);
        assert!(s.backups_path.is_none());
        assert_eq!(w.len(), 4, "{w:?}");

        let (s, _) =
            ServiceSettings::read(&settings(r#"{"enableProxy":true,"proxyHost":"h","proxyPort":1,"proxyType":"FTP"}"#));
        assert!(!s.enable_proxy, "an unknown proxy type turns the proxy off");
    }

    #[test]
    fn writing_keeps_unrelated_settings() {
        let mut st = settings(r#"{"maximumMemory":2048,"theme":"x","concurrentConnections":8,"backupsPath":"/tmp"}"#);
        let mut s = ServiceSettings::from_settings(&st);
        s.backups_path = None;
        s.backup_mode = BackupMode::Full;
        s.write_to(&mut st);
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["theme"], "x");
        assert_eq!(v["maximumMemory"], 2048);
        assert_eq!(v["concurrentConnections"], 8);
        assert_eq!(v["backupMode"], "FULL");
        assert!(v.get("backupsPath").is_none());
    }

    #[test]
    fn start_up_opens_the_log_and_reads_the_settings() {
        let base = std::env::temp_dir().join(format!("blockforge-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base);
        dirs.ensure().unwrap();
        std::fs::write(
            dirs.settings_file(),
            r#"{"connectionTimeout":45,"concurrentConnections":99,"enableLogs":true,"backupsPath":"/not/there"}"#,
        )
        .unwrap();
        // a log from an earlier run
        std::fs::write(crate::logs::log_file(&dirs), "last run\n").unwrap();

        let svc = init(&dirs, false, None);
        assert_eq!((svc.connection_timeout, svc.concurrent_connections, svc.enable_logs), (45, 99, true));
        assert!(svc.backups_path.is_none());
        crate::logs::info("hello from the test");

        let log = std::fs::read_to_string(crate::logs::log_file(&dirs)).unwrap();
        assert!(
            log.contains("INFO  - Backups Path Is Incorrect") || log.contains("WARN  - Backups Path Is Incorrect"),
            "{log}"
        );
        assert!(log.contains(&format!("{} Version: {}", crate::NAME, crate::VERSION)));
        assert!(log.contains("hello from the test"));
        assert!(
            log.contains("Launcher Directory: **USERSDIR**"),
            "the data folder's path is hidden unless debugging: {log}"
        );
        let old: Vec<_> = std::fs::read_dir(crate::logs::old_logs_dir(&dirs)).unwrap().flatten().collect();
        assert_eq!(old.len(), 1);
        assert_eq!(std::fs::read_to_string(old[0].path()).unwrap(), "last run\n");
        let _ = std::fs::remove_dir_all(&base);
    }
}
