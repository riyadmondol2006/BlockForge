//! Signing in with a Microsoft account: the sign-in code flow, and renewing a saved login.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::accounts::{Account, OAuthToken, XstsAuth};
use crate::http::client;

/// The Java edition's sign-in app. Saved refresh tokens belong to it, so using the same one is what lets an
/// existing login keep working here. `BLOCKFORGE_MS_CLIENT_ID` swaps in an app of your own.
pub const DEFAULT_CLIENT_ID: &str = "90890812-00d1-48a8-8d3f-38465ef43b58";
const SCOPE: &str = "XboxLive.signin offline_access";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/launcher/login";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

fn client_id() -> String {
    std::env::var("BLOCKFORGE_MS_CLIENT_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_CLIENT_ID.into())
}

#[derive(Deserialize, Clone, Debug)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    #[serde(default = "five")]
    pub interval: u64,
}

fn five() -> u64 {
    5
}

pub async fn start_device_login() -> Result<DeviceCode> {
    let r =
        client().post(DEVICE_CODE_URL).form(&[("client_id", client_id()), ("scope", SCOPE.to_string())]).send().await?;
    if !r.status().is_success() {
        bail!("Microsoft refused to start a sign-in: {}", r.text().await.unwrap_or_default());
    }
    Ok(r.json().await?)
}

/// Wait for the player to enter the code in their browser, then run the rest of the sign-in.
pub async fn finish_device_login(code: &DeviceCode) -> Result<Account> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(code.expires_in);
    let mut interval = code.interval.max(1);
    loop {
        if std::time::Instant::now() > deadline {
            bail!("The sign-in code ran out. Start again.");
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        let r = client()
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", &client_id()),
                ("device_code", &code.device_code),
            ])
            .send()
            .await?;
        let ok = r.status().is_success();
        let body: Value = r.json().await?;
        if ok {
            return account_from_oauth(oauth_from_json(&body)?, None).await;
        }
        match body["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some("authorization_declined") => bail!("The sign-in was declined."),
            Some("expired_token") => bail!("The sign-in code ran out. Start again."),
            other => bail!("Microsoft sign-in failed: {}", other.unwrap_or("unknown error")),
        }
    }
}

fn oauth_from_json(v: &Value) -> Result<OAuthToken> {
    let expires_in = v["expires_in"].as_i64().unwrap_or(3600);
    Ok(OAuthToken {
        token_type: v["token_type"].as_str().unwrap_or("Bearer").to_string(),
        expires_in,
        expires_at: (chrono::Utc::now() + chrono::Duration::seconds(expires_in))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        scope: v["scope"].as_str().unwrap_or(SCOPE).to_string(),
        access_token: v["access_token"].as_str().context("no access token in the answer")?.to_string(),
        refresh_token: v["refresh_token"].as_str().unwrap_or_default().to_string(),
    })
}

/// Renew a saved login in place. The Minecraft token lasts a day; the refresh token keeps the rest alive.
pub async fn refresh(account: &mut Account) -> Result<()> {
    let old = account.oauth_token.clone().context("this account has no saved sign-in; add it again")?;
    let r = client()
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &client_id()),
            ("refresh_token", &old.refresh_token),
            ("scope", SCOPE),
        ])
        .send()
        .await?;
    let status = r.status();
    let body: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let error = body["error"].as_str().unwrap_or("unknown");
        // only Microsoft saying no to the saved sign-in means signing in again; a busy or broken server doesn't
        if sign_in_rejected(status.as_u16(), error) {
            account.must_login = true;
            bail!("Microsoft no longer accepts the saved sign-in ({error}). Sign in again.");
        }
        bail!("Microsoft's sign-in service didn't answer properly ({status}). Try again in a little while.");
    }
    let mut oauth = oauth_from_json(&body)?;
    if oauth.refresh_token.is_empty() {
        oauth.refresh_token = old.refresh_token;
    }
    // Microsoft may have just replaced the refresh token: keep the new one even if a later step fails, or the next
    // try would send one that is no longer good
    account.oauth_token = Some(oauth.clone());
    let fresh = account_from_oauth(oauth, Some(account)).await?;
    *account = fresh;
    Ok(())
}

/// Whether Microsoft's answer to a token refresh means the saved sign-in is no good any more (revoked, expired,
/// the password changed), as opposed to a busy or failing server.
pub fn sign_in_rejected(status: u16, error: &str) -> bool {
    matches!(status, 400 | 401)
        && matches!(error, "invalid_grant" | "interaction_required" | "unauthorized_client" | "invalid_client")
}

/// Whether an error only means there is no connection (no network, a dropped line, a timeout).
pub fn is_offline_error(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<reqwest::Error>().is_some_and(|r| r.is_connect() || r.is_timeout()))
}

async fn post_json(url: &str, body: Value, what: &str) -> Result<Value> {
    let r = client().post(url).json(&body).header("Accept", "application/json").send().await?;
    let status = r.status();
    let text = r.text().await?;
    if !status.is_success() {
        bail!("{what} failed ({status}): {}", explain_xbox_error(&text));
    }
    Ok(serde_json::from_str(&text)?)
}

fn explain_xbox_error(text: &str) -> String {
    let code = serde_json::from_str::<Value>(text).ok().and_then(|v| v["XErr"].as_u64());
    match code {
        Some(2148916233) => "this Microsoft account has no Xbox profile yet; sign in at xbox.com once".into(),
        Some(2148916235) => "Xbox Live isn't available in your country".into(),
        Some(2148916236) | Some(2148916237) => "this account needs adult verification".into(),
        Some(2148916238) => "this is a child account; an adult has to add it to a family".into(),
        _ => text.chars().take(200).collect(),
    }
}

async fn account_from_oauth(oauth: OAuthToken, previous: Option<&Account>) -> Result<Account> {
    let xbl = post_json(
        XBL_URL,
        json!({
            "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com",
                           "RpsTicket": format!("d={}", oauth.access_token)},
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }),
        "Xbox sign-in",
    )
    .await?;
    let xbl_token = xbl["Token"].as_str().context("Xbox returned no token")?.to_string();

    let xsts = post_json(
        XSTS_URL,
        json!({
            "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl_token]},
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }),
        "Xbox authorisation",
    )
    .await?;
    let uhs = xsts["DisplayClaims"]["xui"][0]["uhs"].as_str().context("Xbox returned no user hash")?.to_string();
    let xsts_token = xsts["Token"].as_str().context("Xbox returned no token")?.to_string();

    let mc = post_json(
        MC_LOGIN_URL,
        json!({"xtoken": format!("XBL3.0 x={uhs};{xsts_token}"), "platform": "PC_LAUNCHER"}),
        "Minecraft sign-in",
    )
    .await?;
    let mc_token = mc["access_token"].as_str().context("Minecraft returned no token")?.to_string();
    let mc_expires = mc["expires_in"].as_i64().unwrap_or(86400);

    let r = client().get(MC_PROFILE_URL).bearer_auth(&mc_token).send().await?;
    if r.status().as_u16() == 404 {
        bail!("This Microsoft account doesn't own Minecraft: Java Edition.");
    }
    if !r.status().is_success() {
        bail!("Couldn't read the Minecraft profile ({})", r.status());
    }
    let profile: Value = r.json().await?;
    let name = profile["name"].as_str().context("profile has no name")?.to_string();
    let id = profile["id"].as_str().context("profile has no id")?.to_string();

    let mut account = previous.cloned().unwrap_or_default();
    account.offline = false;
    account.must_login = false;
    account.username = account.username.clone();
    if account.username.is_empty() {
        account.username = name.clone();
    }
    account.minecraft_username = Some(name);
    account.uuid = dashed(&id);
    account.access_token = Some(mc_token);
    account.access_token_expires_at = Some(
        (chrono::Utc::now() + chrono::Duration::seconds(mc_expires)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    account.oauth_token = Some(oauth);
    account.xsts_auth = Some(XstsAuth {
        issue_instant: xsts["IssueInstant"].as_str().unwrap_or_default().to_string(),
        not_after: xsts["NotAfter"].as_str().unwrap_or_default().to_string(),
        token: xsts_token,
        display_claims: xsts["DisplayClaims"].clone(),
    });
    Ok(account)
}

fn dashed(id: &str) -> String {
    if id.contains('-') || id.len() != 32 {
        return id.to_string();
    }
    format!("{}-{}-{}-{}-{}", &id[0..8], &id[8..12], &id[12..16], &id[16..20], &id[20..32])
}

#[cfg(test)]
mod refresh_answer_tests {
    use super::*;

    #[test]
    fn only_a_real_rejection_means_signing_in_again() {
        assert!(sign_in_rejected(400, "invalid_grant"));
        assert!(sign_in_rejected(400, "interaction_required"));
        // busy, broken or unknown answers keep the account as it is
        for (status, error) in [
            (429, "invalid_grant"),
            (503, "unknown"),
            (500, ""),
            (400, "temporarily_unavailable"),
            (502, "invalid_grant"),
        ] {
            assert!(!sign_in_rejected(status, error), "{status} {error}");
        }
    }

    #[tokio::test]
    async fn no_connection_is_recognised_as_being_offline() {
        // nothing listens on port 9 of this computer: the connection is refused at once
        let e = client().get("http://127.0.0.1:9/").send().await.unwrap_err();
        let e = anyhow::Error::from(e).context("renewing the sign-in");
        assert!(is_offline_error(&e));
        assert!(!is_offline_error(&anyhow::anyhow!("Microsoft no longer accepts the saved sign-in")));
    }
}
