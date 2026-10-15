//! `accounts.json`: Microsoft and offline accounts, in the Java edition's format.

use anyhow::Result;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::write_json_atomic;
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
        if self.offline {
            return false;
        }
        if self.must_login || self.access_token.as_deref().unwrap_or("").is_empty() {
            return true;
        }
        match self.access_token_expires_at.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
            Some(t) => t.timestamp() - 120 < chrono::Utc::now().timestamp(),
            None => true,
        }
    }

    pub fn kind(&self) -> &'static str {
        if self.offline { "Offline" } else { "Microsoft" }
    }
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

pub fn load(dirs: &Dirs) -> Vec<Account> {
    std::fs::read(dirs.accounts_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(dirs: &Dirs, accounts: &[Account]) -> Result<()> {
    write_json_atomic(&dirs.accounts_file(), &accounts)?;
    // the file holds sign-in tokens
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
}
