//! Ported from Xodus tokens/device.rs; never treat corrupt storage as a new device.
use super::*;
use std::path::Path;
use tokens::{Slot, store_error};
use xodus::{
    licensing::{
        splicense::SPLicense,
        utils::{generate_string, parse_bcrypt_rsa_private},
    },
    models::{
        devicecredential::{Authentication, ClientInfo, DeviceAddRequest, DeviceInfo},
        secrets::Device,
        soap::BodyContent,
    },
};

/// polkit's launcher and `cat`, by absolute path: a program found through `PATH` could be anything the player's
/// environment puts first, and it would be run as root.
const PKEXEC: &str = "/usr/bin/pkexec";
const CAT: &str = "/usr/bin/cat";
/// The firmware's system table is readable by root only; this is the one thing BlockForge asks polkit to read.
const DMI: &str = "/sys/firmware/dmi/entries/1-0/raw";

/// The command that reads the firmware table as root through polkit, or None when `pkexec` or `cat` is not at its
/// place on this system (then the table is simply not read, and the Store gets the components without it).
fn privileged_probe(pkexec: &Path, cat: &Path) -> Option<tokio::process::Command> {
    if !pkexec.is_file() || !cat.is_file() {
        return None;
    }
    let mut command = tokio::process::Command::new(pkexec);
    command.arg(cat).arg(DMI).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    Some(command)
}

pub async fn ensure(client: &reqwest::Client, slot: &Slot, cancel: &Cancel) -> Result<()> {
    use xodus::tokens::store::TokenStoreError;
    cancelled(cancel)?;
    let device = match slot.tokens.get_device_license() {
        Ok(device) => device,
        Err(TokenStoreError::NotFound) => {
            if slot.root.join("provisioning").exists() {
                return Err(err(
                    ErrorKind::SignInRequired,
                    "Previous device provisioning is incomplete; explicit Store repair required",
                ));
            }
            let username = format!("02{}", generate_string(14));
            let password = generate_string(20);
            // Hardware probing may launch an optional polkit password prompt (Cancel is fine). It never blocks the UI
            // or GTK loop.
            let raw = match tokio::fs::read(DMI).await {
                Ok(raw) => Some(raw),
                Err(_) => {
                    // The sign-in page tells the player first that the system may ask for a password (F4).
                    match privileged_probe(Path::new(PKEXEC), Path::new(CAT)) {
                        Some(mut command) => {
                            interrupt(cancel, async {
                                Ok(tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
                                    .await
                                    .ok()
                                    .and_then(|r| r.ok())
                                    .filter(|r| r.status.success())
                                    .map(|r| r.stdout))
                            })
                            .await?
                        }
                        None => None,
                    }
                }
            };
            let components =
                tokio::task::spawn_blocking(move || xodus::hardware::probe_provision_components_with_smbios(raw))
                    .await
                    .map_err(internal)?;
            cancelled(cancel)?;
            tokens::atomic(&slot.root.join("provisioning"), b"1").map_err(io_error)?;
            let request = DeviceAddRequest {
                client_info: ClientInfo::default(),
                authentication: Authentication::new(username.clone(), password.clone()),
                device_info: Some(DeviceInfo { id: "DeviceInfo".into(), components, tpm_info: None }),
            };
            let marker = slot.root.join("provisioning");
            let provision = interrupt(cancel, async {
                xodus::api::live::login_device_credential(client, request).await.map_err(|e| {
                    // Only a request that provably never reached Microsoft may be retried freely.
                    // Anything later could have registered a device, so it keeps the repair marker.
                    if never_sent(&e) {
                        let _ = std::fs::remove_file(&marker);
                    }
                    network(e)
                })
            })
            .await?;
            let device = Device {
                username,
                password,
                puid: provision.puid,
                hwid: provision.hw_device_id,
                device_id: provision.license.binding.device_id.unwrap_or_default(),
                splicense: provision.license.splicense_block,
            };
            // Persist before authenticating: retries must reuse this registration.
            slot.tokens.save_device_license(&device).map_err(store_error)?;
            // The registration is saved, so it is no longer interrupted: leaving the marker would have the slot
            // asking for a repair after every successful sign-in.
            match std::fs::remove_file(&marker) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(io_error(e)),
            }
            device
        }
        Err(e) => return Err(store_error(e)),
    };
    if let Ok(xodus::models::secrets::Token::Legacy(token)) = slot.tokens.get_device_sts_token()
        && chrono::DateTime::parse_from_rfc3339(&token.lifetime.expires).is_ok_and(|t| t > chrono::Utc::now())
    {
        return Ok(());
    }
    let license = SPLicense::parse_base64(&device.splicense).map_err(integrity)?;
    let clep = license.clep_sign_state.ok_or_else(|| integrity("Missing device signing state"))?;
    let key = parse_bcrypt_rsa_private(&clep.try_get_rsa_key().map_err(integrity)?).map_err(integrity)?;
    let response = interrupt(cancel, async {
        xodus::api::live::authenticate_device(client, device.username, key).await.map_err(network)
    })
    .await?;
    let BodyContent::RequestSecurityTokenResponse(response) = response.body.body else {
        return Err(store_error("Invalid device authentication"));
    };
    let key = response
        .requested_security_token
        .encrypted_data
        .as_ref()
        .and_then(|data| data.key_info.key_name.clone())
        .ok_or_else(|| integrity("Missing device token key"))?;
    slot.tokens.save_device_token(key, login::convert(*response)?).map_err(store_error)
}

/// True when registration failed before a request could reach the server: a connection
/// that never opened (offline, DNS, refused) or a request that could not be built.
pub(super) fn never_sent(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(e) = error.downcast_ref::<reqwest::Error>() {
            return e.is_connect() || e.is_builder();
        }
        next = error.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_firmware_probe_runs_polkit_by_absolute_path_or_not_at_all() {
        let dir = crate::testutil::scratch("bedrock-pkexec");
        let (pkexec, cat) = (dir.join("pkexec"), dir.join("cat"));
        // neither program there, or only one of them: nothing is run
        assert!(privileged_probe(&pkexec, &cat).is_none());
        std::fs::write(&pkexec, b"").unwrap();
        assert!(privileged_probe(&pkexec, &cat).is_none());
        std::fs::write(&cat, b"").unwrap();
        // a folder in the place of a program is not a program
        let folder = dir.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        assert!(privileged_probe(&folder, &cat).is_none());
        let command = privileged_probe(&pkexec, &cat).expect("both are there");
        let command = command.as_std();
        assert_eq!(command.get_program(), pkexec.as_os_str());
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, [cat.as_os_str(), std::ffi::OsStr::new(DMI)]);
        // the real places are absolute, and the program name is never looked up through PATH
        assert!(Path::new(PKEXEC).is_absolute() && Path::new(CAT).is_absolute());
        assert!(Path::new(DMI).starts_with("/sys/firmware/dmi/"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
