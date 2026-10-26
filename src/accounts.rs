//! `accounts.json`: Microsoft and offline accounts, in the Java edition's format.

use anyhow::Result;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths::Dirs;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub access_token: Option<String>,
    pub oauth_token: Option<OAuthToken>,
    pub xsts_auth: Option<XstsAuth>,
    pub access_token_expires_at: Option<String>,
    pub must_login: bool,
    pub offline: bool,
    pub username: String,
    pub minecraft_username: Option<String>,
    pub uuid: String,
    #[serde(flatten)]
    pub extras: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct OAuthToken {
    pub token_type: String,
    pub expires_in: i64,
    pub expires_at: String,
    pub scope: String,
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct XstsAuth {
    #[serde(rename = "IssueInstant")]
    pub issue_instant: String,
    #[serde(rename = "NotAfter")]
    pub not_after: String,
    #[serde(rename = "Token")]
    pub token: String,
    #[serde(rename = "DisplayClaims")]
    pub display_claims: Value,
}

impl Account {
    /// The name the player sees and the game is started with.
    pub fn name(&self) -> &str {
        self.minecraft_username.as_deref().filter(|n| !n.is_empty()).unwrap_or(&self.username)
    }

    pub fn uuid_nodashes(&self) -> String {
        self.uuid.replace('-', "")
    }

    pub fn offline(name: &str) -> Account {
        Account {
            offline: true,
            username: name.to_string(),
            minecraft_username: Some(name.to_string()),
            uuid: offline_uuid(name),
            access_token: Some("0".to_string()),
            ..Default::default()
        }
    }

    /// True when the Minecraft token has run out (or is about to) and has to be renewed before launching.
    pub fn needs_refresh(&self) -> bool {
        self.needs_refresh_within(120)
    }

    /// True when the Minecraft token is missing, or runs out within `seconds`.
    pub fn needs_refresh_within(&self, seconds: i64) -> bool {
        if self.offline {
            return false;
        }
        if self.must_login || self.access_token.as_deref().unwrap_or("").is_empty() {
            return true;
        }
        match self.access_token_expires_at.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
            Some(t) => t.timestamp() - seconds < chrono::Utc::now().timestamp(),
            None => true,
        }
    }

    pub fn kind(&self) -> &'static str {
        if self.offline { "Offline" } else { "Microsoft" }
    }
}

/// Whether Minecraft takes `name` as a player name: 3 to 16 letters, digits or underscores. Servers turn other names
/// away, and some game versions and mods fail on them.
pub fn valid_offline_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Java's `UUID.nameUUIDFromBytes("OfflinePlayer:<name>")`, so an offline player keeps the same world data and
/// skins-less identity in both editions.
pub fn offline_uuid(name: &str) -> String {
    let mut h = Md5::new();
    h.update(format!("OfflinePlayer:{name}").as_bytes());
    let mut b: [u8; 16] = h.finalize().into();
    b[6] = (b[6] & 0x0f) | 0x30;
    b[8] = (b[8] & 0x3f) | 0x80;
    let s = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}

/// Every saved account. One account that can't be read (a field of the wrong kind, a hand edit gone wrong) no longer
/// makes all of them disappear: each is read on its own, a field that doesn't fit is left at its default, and the file
/// as it was is kept as `accounts.json.broken-<time>` before anything is dropped, so the next save can't lose it.
pub fn load(dirs: &Dirs) -> Vec<Account> {
    let path = dirs.accounts_file();
    let Ok(bytes) = std::fs::read(&path) else { return Vec::new() };
    match serde_json::from_slice::<Value>(crate::config::strip_bom(&bytes)) {
        Ok(Value::Array(items)) => {
            let mut damaged = false;
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Object(map) => {
                        let (account, dropped) = crate::config::lenient_object::<Account>(map);
                        damaged |= !dropped.is_empty();
                        match account {
                            // an account without a name or id can't be used or told apart from another
                            Some(a) if !(a.username.is_empty() && a.uuid.is_empty()) => out.push(a),
                            _ => damaged = true,
                        }
                    }
                    _ => damaged = true,
                }
            }
            if damaged {
                crate::logs::warn(format!("Some of {} could not be read", path.display()));
                crate::config::keep_broken_copy(&path, false);
            }
            out
        }
        Ok(Value::Null) => Vec::new(),
        Ok(_) | Err(_) => {
            crate::logs::warn(format!("{} could not be read; no accounts were loaded", path.display()));
            crate::config::keep_broken_copy(&path, true);
            Vec::new()
        }
    }
}

/// Take the sign-in of each account in `mine` from `on_disk` where the one on disk is newer (it runs out later):
/// a renewal that happened elsewhere (before a game started, in another window) saved tokens a copy held in memory
/// doesn't have, and writing that copy back would put the old, possibly no longer accepted, refresh token in their
/// place.
pub fn merge_newer_tokens(mine: &mut [Account], on_disk: &[Account]) {
    let expiry =
        |a: &Account| a.access_token_expires_at.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    for a in mine.iter_mut().filter(|a| !a.offline) {
        let Some(disk) = on_disk.iter().find(|d| d.uuid == a.uuid && !d.offline) else { continue };
        let newer = match (expiry(disk), expiry(a)) {
            (Some(d), Some(m)) => d > m,
            (Some(_), None) => true,
            _ => false,
        };
        if newer {
            a.access_token = disk.access_token.clone();
            a.access_token_expires_at = disk.access_token_expires_at.clone();
            a.oauth_token = disk.oauth_token.clone();
            a.xsts_auth = disk.xsts_auth.clone();
            a.must_login = disk.must_login;
        } else if expiry(disk) == expiry(a) {
            // a renewal that failed half way still saved what it learnt: a refresh token Microsoft had already
            // replaced, or that the sign-in is no longer accepted
            let refresh = |x: &Account| x.oauth_token.as_ref().map(|o| o.refresh_token.clone());
            if refresh(disk) != refresh(a) {
                a.oauth_token = disk.oauth_token.clone();
            }
            a.must_login |= disk.must_login;
        }
    }
}

pub fn save(dirs: &Dirs, accounts: &[Account]) -> Result<()> {
    // the file holds sign-in tokens, so it is only ever readable by its owner, from the moment it exists
    let bytes = serde_json::to_vec_pretty(&accounts)?;
    crate::config::write_atomic(&dirs.accounts_file(), &bytes, true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dirs.accounts_file(), std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }

    #[test]
    fn java_accounts_file_round_trips() {
        let json = r#"[{"accessToken":"x","offline":false,"username":"a","minecraftUsername":"B","uuid":"u",
            "collapsedPacks":[],"mustLogin":false}]"#;
        let a: Vec<Account> = serde_json::from_str(json).unwrap();
        assert_eq!(a[0].name(), "B");
        let v = serde_json::to_value(&a).unwrap();
        assert!(v[0].get("collapsedPacks").is_some());
    }

    #[test]
    fn token_renewal_window() {
        let mut a: Account =
            serde_json::from_str(r#"{"accessToken":"x","offline":false,"username":"a","uuid":"u"}"#).unwrap();
        let in_secs = |n: i64| (chrono::Utc::now() + chrono::Duration::seconds(n)).to_rfc3339();
        a.access_token_expires_at = Some(in_secs(3600));
        assert!(!a.needs_refresh());
        assert!(!a.needs_refresh_within(1800));
        assert!(a.needs_refresh_within(7200));
        a.access_token_expires_at = Some(in_secs(30));
        assert!(a.needs_refresh());
        a.offline = true;
        assert!(!a.needs_refresh_within(7200));
    }
}

/// Accounts must survive whatever happens to `accounts.json`: losing them means signing in again, and losing an
/// offline player's name changes its UUID and so its inventory in every world.
#[cfg(test)]
mod robustness_tests {
    use super::*;

    fn temp_dirs(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("bf-accounts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dirs = Dirs::at(&base);
        std::fs::create_dir_all(&dirs.configs).unwrap();
        dirs
    }

    fn kept_copies(dirs: &Dirs) -> usize {
        std::fs::read_dir(&dirs.configs)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".broken-"))
            .count()
    }

    #[test]
    fn one_damaged_account_does_not_take_the_others_with_it() {
        let dirs = temp_dirs("one");
        std::fs::write(
            dirs.accounts_file(),
            r#"[
                {"offline":true,"username":"Alex","minecraftUsername":"Alex","uuid":"u1","accessToken":"0"},
                {"offline":false,"username":"Steve","uuid":"u2","mustLogin":"yes","oauthToken":{"expires_in":"soon"}},
                42,
                {"offline":true,"username":"Tester","uuid":"u3"}
            ]"#,
        )
        .unwrap();
        let all = load(&dirs);
        let names: Vec<&str> = all.iter().map(|a| a.username.as_str()).collect();
        assert_eq!(names, ["Alex", "Steve", "Tester"]);
        // Steve's broken fields fell back to their defaults, so he is asked to sign in again
        let steve = &all[1];
        assert!(!steve.must_login);
        assert!(steve.oauth_token.is_none());
        assert!(steve.needs_refresh(), "no token: must sign in again, not launch with nothing");
        assert_eq!(kept_copies(&dirs), 1, "the file as it was is kept before anything is dropped");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn a_garbled_file_is_moved_aside_and_saving_starts_fresh() {
        let dirs = temp_dirs("garbled");
        std::fs::write(dirs.accounts_file(), "[{\"username\":\"Alex\",\"uuid\":").unwrap();
        assert!(load(&dirs).is_empty());
        assert_eq!(kept_copies(&dirs), 1);
        save(&dirs, &[Account::offline("Alex")]).unwrap();
        assert_eq!(load(&dirs)[0].name(), "Alex");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn missing_null_and_bom_files_load_as_no_accounts_without_copies() {
        let dirs = temp_dirs("empty");
        assert!(load(&dirs).is_empty());
        std::fs::write(dirs.accounts_file(), "null").unwrap();
        assert!(load(&dirs).is_empty());
        let mut bom = b"\xEF\xBB\xBF".to_vec();
        bom.extend_from_slice(br#"[{"offline":true,"username":"Alex","uuid":"u"}]"#);
        std::fs::write(dirs.accounts_file(), bom).unwrap();
        assert_eq!(load(&dirs).len(), 1);
        assert_eq!(kept_copies(&dirs), 0);
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dirs = temp_dirs("private");
        save(&dirs, &[Account::offline("Alex")]).unwrap();
        let mode = std::fs::metadata(dirs.accounts_file()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn odd_offline_names_still_give_stable_uuids() {
        for name in ["", " ", "名前", "🙂", "a b", &"x".repeat(1000), "Robert'); DROP TABLE--"] {
            let a = offline_uuid(name);
            assert_eq!(a, offline_uuid(name));
            assert_eq!(a.len(), 36, "{name}");
            assert_eq!(&a[14..15], "3", "version 3 UUID like Java's nameUUIDFromBytes");
        }
    }

    #[test]
    fn garbage_expiry_dates_mean_sign_in_again() {
        let mut a = Account { username: "x".into(), access_token: Some("t".into()), ..Default::default() };
        for when in ["", "tomorrow", "2026-13-45T99:99:99Z", "9999999999"] {
            a.access_token_expires_at = Some(when.into());
            assert!(a.needs_refresh(), "{when}");
        }
        a.access_token_expires_at = None;
        assert!(a.needs_refresh());
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn player_names_follow_minecrafts_rule() {
        for ok in ["Steve", "abc", "A_B_C", "x1234567890123_y", "___"] {
            assert!(valid_offline_name(ok), "{ok}");
        }
        for bad in
            ["", "ab", "a b", "seventeen_chars__", "Stéve", "名前名前", "🙂🙂🙂", "a-b", "a.b", "Steve\n", "<script>"]
        {
            assert!(!valid_offline_name(bad), "{bad:?}");
        }
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn ms(uuid: &str, token: &str, expires_in_secs: i64) -> Account {
        Account {
            username: "u".into(),
            uuid: uuid.into(),
            access_token: Some(token.into()),
            access_token_expires_at: Some(
                (chrono::Utc::now() + chrono::Duration::seconds(expires_in_secs)).to_rfc3339(),
            ),
            oauth_token: Some(OAuthToken { refresh_token: format!("refresh-{token}"), ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn a_stale_copy_never_writes_over_newer_tokens() {
        let mut window = vec![ms("a", "old", 60), ms("b", "mine", 7200), Account::offline("Alex")];
        let disk = vec![ms("a", "renewed", 86_000), ms("b", "older", 30)];
        merge_newer_tokens(&mut window, &disk);
        assert_eq!(window[0].access_token.as_deref(), Some("renewed"));
        assert_eq!(window[0].oauth_token.as_ref().unwrap().refresh_token, "refresh-renewed");
        assert_eq!(window[1].access_token.as_deref(), Some("mine"), "the window's newer one stays");
        assert!(window[2].offline);
        // a renewal that failed after Microsoft replaced the refresh token: same expiry, newer refresh token on disk
        let mut window = vec![ms("c", "same", 3600)];
        let mut disk_c = window[0].clone();
        disk_c.oauth_token = Some(OAuthToken { refresh_token: "rotated".into(), ..Default::default() });
        merge_newer_tokens(&mut window, &[disk_c]);
        assert_eq!(window[0].oauth_token.as_ref().unwrap().refresh_token, "rotated");
        // an account removed in the window stays removed: only accounts in the window are looked up
        let mut window = vec![ms("b", "mine", 7200)];
        merge_newer_tokens(&mut window, &disk);
        assert_eq!(window.len(), 1);
    }
}
