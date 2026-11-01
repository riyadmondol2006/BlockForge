//! Ported from Xodus CLI license.rs and BedrockOnLinux bol/xodus.py (including patch 0005).
use super::*;
use tokens::store_error;
use xodus::{
    licensing::{content::LicenseContentError, splicense::SPLicense},
    models::{
        live::ExchangeUserTokenOutcome,
        secrets::{LegacyToken, Token},
        soap,
    },
    tokens::TokenManager,
};

pub fn legacy(token: std::result::Result<Token, impl std::fmt::Display>) -> Result<LegacyToken> {
    match token.map_err(store_error)? {
        Token::Legacy(t) => Ok(t),
        _ => Err(store_error("Unexpected token type")),
    }
}
pub async fn user_ticket(client: &reqwest::Client, tokens: &TokenManager, audience: &str) -> Result<String> {
    let device = legacy(tokens.get_device_sts_token())?;
    let user = tokens.get_user().map_err(store_error)?;
    let response = xodus::api::live::exchange_user_token(
        client,
        legacy(tokens.get_user_sts_token())?,
        user.username,
        device,
        None,
        Some("Silent".into()),
        "{d6d5a677-0872-4ab0-9442-bb792fce85c5}".into(),
        &[(audience.into(), Some(soap::PolicyReference::mbi_ssl()))],
    )
    .await
    .map_err(network)?;
    let ExchangeUserTokenOutcome::Issued(body) = response else {
        return Err(store_error("Exchange needs sign-in"));
    };
    let mut compact = None;
    for response in login::issued(body)? {
        let address = response.applies_to.endpoint_reference.address.clone();
        match login::convert(response)? {
            Token::Compact(t) => compact = Some(t),
            Token::Legacy(t) => {
                let address = t.key_name.clone().unwrap_or(address);
                tokens.save_user_token(address, Token::Legacy(t)).map_err(store_error)?;
            }
        }
    }
    compact.ok_or_else(|| store_error("Missing audience ticket"))
}
pub async fn acquire(
    client: &reqwest::Client,
    slot: &tokens::Slot,
    content_id: String,
    cancel: &Cancel,
) -> Result<[u8; 32]> {
    interrupt(cancel, async {
        let dev = legacy(slot.tokens.get_device_sts_token())?;
        let user = slot.tokens.get_user().map_err(store_error)?;
        // Check every prerequisite before issuing any request.
        legacy(slot.tokens.get_user_sts_token())?;
        let response = xodus::api::live::exchange_device_token(
            client,
            dev,
            "{d6d5a677-0872-4ab0-9442-bb792fce85c5}".into(),
            "www.microsoft.com".into(),
            Some(soap::PolicyReference::mbi_ssl()),
        )
        .await
        .map_err(network)?;
        let Token::Compact(device) = login::convert(response)? else {
            return Err(store_error("Invalid device ticket"));
        };
        let user_ticket = user_ticket(client, &slot.tokens, "www.microsoft.com").await?;
        let (_, license) = xodus::licensing::content::get_license_content(
            client,
            device,
            user_ticket,
            user.puid,
            content_id,
            "neutral".into(),
        )
        .await
        .map_err(license_error)?;
        let game = SPLicense::parse_base64(&license.splicense_block).map_err(integrity)?;
        if game.content_keys.len() != 1 {
            return Err(integrity("Unexpected content keys"));
        }
        let device = slot.tokens.get_device_license().map_err(store_error)?;
        let device = SPLicense::parse_base64(&device.splicense).map_err(integrity)?;
        let key = device
            .encrypted_device_key
            .ok_or_else(|| integrity("Missing device key"))?
            .try_derive_device_key()
            .map_err(integrity)?;
        let (_, packed) = game.content_keys.into_iter().next().ok_or_else(|| integrity("Missing content key"))?;
        let full = packed.unpack(&key).map_err(integrity)?;
        Ok(*full)
    })
    .await
    .map_err(unavailable)
}
pub fn unavailable(mut error: BedrockError) -> BedrockError {
    if matches!(error.kind, ErrorKind::Network) {
        error.kind = ErrorKind::LicenseUnavailable;
        error.message = "Store licensing is unavailable; network access is required to Play".into();
    }
    error
}
pub fn license_error(error: LicenseContentError) -> BedrockError {
    match error {
        LicenseContentError::NotEntitled { .. } => {
            err(ErrorKind::NotEntitled, "This Store account does not own this content")
        }
        LicenseContentError::Refused { status, reason } => {
            // Never forward arbitrary server text: a response could echo credentials. Retain the machine reason.
            let code = reason.split(':').next().unwrap_or("");
            let code = match code {
                "InvalidMsaTicketFormat"
                | "Unauthorized"
                | "Forbidden"
                | "NotEntitled"
                | "NoEntitlement"
                | "InvalidToken"
                | "InvalidRequest" => code,
                _ => "RequestRefused",
            };
            err(ErrorKind::LicenseUnavailable, format!("Store licensing refused the request (HTTP {status}, {code})"))
        }
        LicenseContentError::Request(_) => {
            err(ErrorKind::LicenseUnavailable, "Store licensing is unavailable; network access is required to Play")
        }
        LicenseContentError::Unreadable(_) => integrity("Unreadable license"),
    }
}
