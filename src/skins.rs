//! Player skins: fetching them from Mojang, keeping a copy, and drawing the head and the whole figure.
//!
//! Skins are saved as `configs/images/skins/<uuid without dashes>.png`, like the Java edition, so both editions
//! share them. A player without a skin of their own gets Steve or Alex, chosen from the id the way Minecraft does it.
//!
//! Where the Java edition asks Mojang's profile service (which needs a signed-in account), this also falls back
//! to the public session server, so a skin can be fetched for any id without a token. Capes are fetched and kept
//! the same way (`configs/images/capes/`), which the Java edition doesn't do.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine;
use image::{Rgba, RgbaImage};
use serde::Deserialize;

use crate::accounts::Account;
use crate::paths::Dirs;

const SESSION_PROFILE_URL: &str = "https://sessionserver.mojang.com/session/minecraft/profile";
const NAME_TO_UUID_URL: &str = "https://api.mojang.com/users/profiles/minecraft";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const SKIN_UPLOAD_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skins";
/// A site the skin dialog offers for finding skins.
pub const SKIN_SITE: &str = "https://www.minecraftskins.com/";

// ---------------------------------------------------------------------------------------------------------
// Where skins are kept, and the two default ones

pub fn skins_dir(dirs: &Dirs) -> PathBuf {
    dirs.configs.join("images").join("skins")
}

pub fn capes_dir(dirs: &Dirs) -> PathBuf {
    dirs.configs.join("images").join("capes")
}

/// `uuid` without dashes; `0` for an account that has none (the Java edition's `getUUIDNoDashes`).
pub fn uuid_no_dashes(uuid: &str) -> String {
    if uuid.is_empty() { "0".into() } else { uuid.replace('-', "") }
}

/// The saved skin of the player with this id.
pub fn skin_path(dirs: &Dirs, uuid: &str) -> PathBuf {
    skins_dir(dirs).join(format!("{}.png", uuid_no_dashes(uuid)))
}

/// The saved skin of an account.
pub fn skin_file(dirs: &Dirs, account: &Account) -> PathBuf {
    skin_path(dirs, &account.uuid)
}

/// The saved cape of an account.
pub fn cape_file(dirs: &Dirs, account: &Account) -> PathBuf {
    capes_dir(dirs).join(format!("{}.png", uuid_no_dashes(&account.uuid)))
}

/// Java's `UUID.hashCode()`, which Minecraft uses to decide between Steve and Alex.
pub fn java_uuid_hash(uuid: &str) -> i32 {
    let hex: String = uuid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 {
        return 0;
    }
    let msb = u64::from_str_radix(&hex[..16], 16).unwrap_or(0);
    let lsb = u64::from_str_radix(&hex[16..], 16).unwrap_or(0);
    let hilo = msb ^ lsb;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

/// Whether the default skin for this id is Alex (arms 3 pixels wide) rather than Steve.
pub fn default_is_alex(uuid: &str) -> bool {
    java_uuid_hash(uuid) & 1 != 0
}

/// The PNG file of the default skin for this id.
pub fn default_skin_png(uuid: &str) -> Vec<u8> {
    let hex = if default_is_alex(uuid) { crate::skins_data::ALEX_HEX } else { crate::skins_data::STEVE_HEX };
    hex::decode(hex).unwrap_or_default()
}

pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage> {
    Ok(image::load_from_memory(bytes).context("that isn't a picture the launcher can read")?.to_rgba8())
}

/// The skin of an account as pixels: the saved file, or the default skin when there is none or it can't be read.
pub fn load_skin(dirs: &Dirs, account: &Account) -> RgbaImage {
    std::fs::read(skin_file(dirs, account))
        .ok()
        .and_then(|b| decode_png(&b).ok())
        .or_else(|| decode_png(&default_skin_png(&account.uuid)).ok())
        .unwrap_or_else(|| RgbaImage::new(64, 64))
}

/// Whether a picture has a size Minecraft accepts for a skin: 64x64, or the old 64x32.
pub fn is_valid_skin(width: u32, height: u32) -> bool {
    width == 64 && (height == 64 || height == 32)
}

/// Whether the skin picture is the slim ("Alex") model by Mojang's rule for old skins: it has no way of saying,
/// so only the profile can tell. This is for a skin that was never uploaded: the default's own model.
pub fn default_is_slim(uuid: &str) -> bool {
    default_is_alex(uuid)
}

// ---------------------------------------------------------------------------------------------------------
// Drawing

fn white_pixels(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> u32 {
    let mut n = 0;
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            if img.get_pixel(xx, yy).0 == [255, 255, 255, 255] {
                n += 1;
            }
        }
    }
    n
}

/// Draw `src` over `dst` the way Java's `drawImage` does (source over, straight alpha).
fn blend(dst: &mut Rgba<u8>, src: Rgba<u8>) {
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    for i in 0..3 {
        let c = (src[i] as f32 * sa + dst[i] as f32 * da * (1.0 - sa)) / out_a;
        dst[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Copy a piece of `skin` onto `canvas`, optionally flipped left to right. Pieces that fall outside the skin are skipped.
fn part(
    canvas: &mut RgbaImage,
    skin: &RgbaImage,
    (sx, sy, w, h): (u32, u32, u32, u32),
    (dx, dy): (i64, i64),
    mirror: bool,
) {
    if sx + w > skin.width() || sy + h > skin.height() {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let px = *skin.get_pixel(sx + if mirror { w - 1 - x } else { x }, sy + y);
            let (cx, cy) = (dx + x as i64, dy + y as i64);
            if cx >= 0 && cy >= 0 && (cx as u32) < canvas.width() && (cy as u32) < canvas.height() {
                blend(canvas.get_pixel_mut(cx as u32, cy as u32), px);
            }
        }
    }
}

/// The face: 8x8 pixels with the hat layer on top. Like the Java edition, a hat layer that is mostly solid white
/// (junk from some old skin editors) is left out. `None` if the picture is too small to be a skin.
pub fn head_pixels(skin: &RgbaImage) -> Option<RgbaImage> {
    if skin.width() < 48 || skin.height() < 16 {
        return None;
    }
    let mut head = RgbaImage::new(8, 8);
    part(&mut head, skin, (8, 8, 8, 8), (0, 0), false);
    if white_pixels(skin, 40, 8, 8, 8) <= 32 {
        part(&mut head, skin, (40, 8, 8, 8), (0, 0), false);
    }
    Some(head)
}

/// The front of the player: 16 pixels wide and 32 tall, with the outer layer (hat, jacket, sleeves, trousers) on
/// top, as the Java edition's skin preview draws it. `slim` makes the arms 3 pixels wide.
pub fn figure_pixels(skin: &RgbaImage, slim: bool) -> RgbaImage {
    let mut c = RgbaImage::new(16, 32);
    let modern = skin.height() == 64;
    let arm = if slim && modern { 3 } else { 4 };
    // inner layer: head, body, arms, legs. The player's right side is on the viewer's left.
    part(&mut c, skin, (8, 8, 8, 8), (4, 0), false);
    part(&mut c, skin, (20, 20, 8, 12), (4, 8), false);
    part(&mut c, skin, (44, 20, arm, 12), (4 - arm as i64, 8), false);
    if modern {
        part(&mut c, skin, (36, 52, arm, 12), (12, 8), false);
    } else {
        part(&mut c, skin, (44, 20, arm, 12), (12, 8), true);
    }
    part(&mut c, skin, (4, 20, 4, 12), (4, 20), false);
    if modern {
        part(&mut c, skin, (20, 52, 4, 12), (8, 20), false);
    } else {
        part(&mut c, skin, (4, 20, 4, 12), (8, 20), true);
    }
    // outer layer
    part(&mut c, skin, (40, 8, 8, 8), (4, 0), false);
    if modern {
        part(&mut c, skin, (20, 36, 8, 12), (4, 8), false);
        part(&mut c, skin, (44, 36, arm, 12), (4 - arm as i64, 8), false);
        part(&mut c, skin, (52, 52, arm, 12), (12, 8), false);
        part(&mut c, skin, (4, 36, 4, 12), (4, 20), false);
        part(&mut c, skin, (4, 52, 4, 12), (8, 20), false);
    }
    c
}

/// The outside of a cape: 10x16 pixels (or a multiple of that for high resolution capes).
pub fn cape_front(cape: &RgbaImage) -> Option<RgbaImage> {
    if cape.width() < 22 || cape.height() < 17 || cape.width() % 64 != 0 {
        return None;
    }
    let s = cape.width() / 64;
    Some(image::imageops::crop_imm(cape, s, s, 10 * s, 16 * s).to_image())
}

// ---------------------------------------------------------------------------------------------------------
// Talking to Mojang

/// What Mojang knows about how a player looks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Textures {
    pub id: String,
    pub name: String,
    pub skin_url: Option<String>,
    /// The skin's arms are the slim ("Alex") width.
    pub slim: bool,
    pub cape_url: Option<String>,
}

#[derive(Deserialize)]
struct SessionProfile {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    properties: Vec<Property>,
}

#[derive(Deserialize)]
struct Property {
    name: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize, Default)]
struct TexturePayload {
    #[serde(default)]
    textures: TextureSet,
}

#[derive(Deserialize, Default)]
struct TextureSet {
    #[serde(rename = "SKIN")]
    skin: Option<TextureEntry>,
    #[serde(rename = "CAPE")]
    cape: Option<TextureEntry>,
}

#[derive(Deserialize)]
struct TextureEntry {
    url: String,
    #[serde(default)]
    metadata: Option<TextureMeta>,
}

#[derive(Deserialize)]
struct TextureMeta {
    #[serde(default)]
    model: String,
}

/// Read the answer of the session server (`/session/minecraft/profile/<id>`).
pub fn parse_session_profile(json: &[u8]) -> Result<Textures> {
    let profile: SessionProfile = serde_json::from_slice(json).context("Mojang's answer wasn't what was expected")?;
    let mut out = Textures { id: profile.id, name: profile.name, ..Default::default() };
    let Some(prop) = profile.properties.iter().find(|p| p.name == "textures") else { return Ok(out) };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(prop.value.trim())
        .context("the textures value wasn't valid base64")?;
    let payload: TexturePayload = serde_json::from_slice(&decoded).context("the textures value wasn't valid JSON")?;
    if let Some(skin) = payload.textures.skin {
        out.slim = skin.metadata.as_ref().is_some_and(|m| m.model.eq_ignore_ascii_case("slim"));
        out.skin_url = Some(skin.url);
    }
    out.cape_url = payload.textures.cape.map(|c| c.url);
    Ok(out)
}

/// Read the answer of the profile service (`/minecraft/profile`), which lists skins and capes with their state.
pub fn parse_service_profile(json: &[u8]) -> Result<Textures> {
    let v: serde_json::Value = serde_json::from_slice(json).context("Mojang's answer wasn't what was expected")?;
    let active = |key: &str| {
        v[key]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["state"].as_str().is_some_and(|st| st.eq_ignore_ascii_case("ACTIVE"))))
    };
    let skin = active("skins");
    Ok(Textures {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        name: v["name"].as_str().unwrap_or_default().to_string(),
        skin_url: skin.and_then(|s| s["url"].as_str()).map(str::to_string),
        slim: skin.and_then(|s| s["variant"].as_str()).is_some_and(|m| m.eq_ignore_ascii_case("slim")),
        cape_url: active("capes").and_then(|c| c["url"].as_str()).map(str::to_string),
    })
}

/// The skin and cape of a player by id, from the public session server. No sign-in needed.
pub async fn fetch_textures(uuid: &str) -> Result<Textures> {
    let id = uuid_no_dashes(uuid);
    let r = crate::http::client()
        .get(format!("{SESSION_PROFILE_URL}/{id}"))
        .send()
        .await
        .context("couldn't reach Mojang's session server")?;
    match r.status().as_u16() {
        200 => parse_session_profile(&r.bytes().await?),
        204 | 404 => bail!("Mojang has no player with the id {id}"),
        429 => bail!("Mojang is asking for fewer requests; try again in a minute"),
        s => bail!("Mojang's session server answered HTTP {s}"),
    }
}

/// The skin and cape of a signed-in account, from the profile service.
pub async fn fetch_textures_with_token(token: &str) -> Result<Textures> {
    let r =
        crate::http::client().get(MC_PROFILE_URL).bearer_auth(token).send().await.context("couldn't reach Mojang")?;
    if !r.status().is_success() {
        bail!("Mojang's profile service answered HTTP {}", r.status().as_u16());
    }
    parse_service_profile(&r.bytes().await?)
}

/// A player's id and exact name from their name (`MojangAPIUtils.getUUID`).
pub async fn lookup_uuid(username: &str) -> Result<(String, String)> {
    if username.is_empty() || !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("\"{username}\" isn't a Minecraft name");
    }
    let r = crate::http::client()
        .get(format!("{NAME_TO_UUID_URL}/{username}"))
        .send()
        .await
        .context("couldn't reach Mojang")?;
    match r.status().as_u16() {
        200 => {
            let v: serde_json::Value = r.json().await?;
            let id = v["id"].as_str().context("Mojang's answer had no id")?.to_string();
            let name = v["name"].as_str().unwrap_or(username).to_string();
            Ok((id, name))
        }
        204 | 404 => bail!("There is no Minecraft player called {username}"),
        s => bail!("Mojang answered HTTP {s}"),
    }
}

/// The name a player has now (`MojangAPIUtils.getCurrentUsername`). Mojang's name history service is gone, so
/// this asks for the profile.
pub async fn current_username(uuid: &str) -> Option<String> {
    fetch_textures(uuid).await.ok().map(|t| t.name).filter(|n| !n.is_empty())
}

/// Whether `username` can still be taken by a new profile (`MojangAPIUtils.checkUsernameAvailable`).
pub async fn username_available(access_token: &str, username: &str) -> bool {
    let url = format!("https://api.minecraftservices.com/minecraft/profile/name/{username}/available");
    match crate::http::client().get(url).bearer_auth(access_token).send().await {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v["status"].as_str().map(|s| s == "AVAILABLE"))
            .unwrap_or(false),
        Ok(_) => false,
        Err(e) => {
            crate::logs::error(format!("Failed to check username availability: {e}"));
            false
        }
    }
}

/// Make a Minecraft profile with this name for an account that owns the game but never made one
/// (`MojangAPIUtils.createMcProfile`).
pub async fn create_profile(access_token: &str, username: &str) -> Result<()> {
    let r = crate::http::client()
        .post(MC_PROFILE_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "profileName": username }))
        .send()
        .await
        .context("couldn't reach Mojang")?;
    if !r.status().is_success() {
        let body = r.text().await.unwrap_or_default();
        crate::logs::error(&body);
        bail!("Mojang didn't create the profile: {}", body.chars().take(200).collect::<String>());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Keeping skins up to date

/// What `update_skin` did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SkinUpdate {
    /// The player's own skin was downloaded and saved.
    Downloaded,
    /// The player has no skin; the default one was saved.
    DefaultSaved,
    /// Nothing new could be fetched; the skin saved earlier stays.
    Kept,
}

fn write_cached(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("png.part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).with_context(|| format!("saving {}", path.display()))
}

async fn download_png(url: &str) -> Result<Vec<u8>> {
    let bytes = crate::http::get_bytes(url).await?;
    decode_png(&bytes).context("the skin Mojang sent can't be read")?;
    Ok(bytes)
}

/// Download the account's current skin (and cape) and keep them, like the Java edition's `updateSkin`. For a
/// Microsoft account the profile service is asked with its token; if that fails (the token ran out, or Mojang's
/// service is down) the public session server answers instead.
pub async fn update_skin(dirs: &Dirs, account: &Account) -> Result<SkinUpdate> {
    crate::logs::info(format!("Downloading skin for {}", account.name()));
    let file = skin_file(dirs, account);
    let textures = if account.offline {
        None
    } else {
        let by_token = match account.access_token.as_deref().filter(|t| !t.is_empty() && !account.needs_refresh()) {
            Some(token) => fetch_textures_with_token(token).await.ok(),
            None => None,
        };
        match by_token {
            Some(t) => Some(t),
            None => match fetch_textures(&account.uuid).await {
                Ok(t) => Some(t),
                Err(e) => {
                    crate::logs::warn(format!("Couldn't fetch the skin of {}: {e:#}", account.name()));
                    None
                }
            },
        }
    };

    let Some(textures) = textures else {
        return save_default_if_missing(&file, &account.uuid, account.offline);
    };

    // the cape is a nicety: its failure never fails the skin
    match &textures.cape_url {
        Some(url) => match download_png(url).await {
            Ok(bytes) => {
                let _ = write_cached(&cape_file(dirs, account), &bytes);
            }
            Err(e) => crate::logs::debug(format!("Couldn't download the cape: {e:#}")),
        },
        None => {
            let _ = std::fs::remove_file(cape_file(dirs, account));
        }
    }

    match textures.skin_url {
        None => {
            crate::logs::warn("Couldn't download skin because no skin was found. Using default skin");
            save_default_if_missing(&file, &account.uuid, false)
        }
        Some(url) => match download_png(&url).await {
            Ok(bytes) => {
                write_cached(&file, &bytes)?;
                Ok(SkinUpdate::Downloaded)
            }
            Err(e) => {
                crate::logs::warn(format!("Couldn't download the skin of {}: {e:#}", account.name()));
                if file.exists() { Ok(SkinUpdate::Kept) } else { save_default_if_missing(&file, &account.uuid, false) }
            }
        },
    }
}

fn save_default_if_missing(file: &Path, uuid: &str, _offline: bool) -> Result<SkinUpdate> {
    if file.exists() {
        return Ok(SkinUpdate::Kept);
    }
    write_cached(file, &default_skin_png(uuid))?;
    Ok(SkinUpdate::DefaultSaved)
}

/// The "Update skins" tool: the skin of every account. Returns one line per account for the report. The
/// progress text says which account is being done.
pub async fn update_all(
    dirs: &Dirs,
    accounts: &[Account],
    progress: &crate::install::Progress,
) -> Vec<(String, Result<SkinUpdate>)> {
    let mut out = Vec::new();
    for (i, a) in accounts.iter().enumerate() {
        progress.stage(format!("Downloading skin for {} ({} of {})", a.name(), i + 1, accounts.len()));
        out.push((a.name().to_string(), update_skin(dirs, a).await));
    }
    out
}

// ---------------------------------------------------------------------------------------------------------
// Changing the skin

/// Build the body of a `multipart/form-data` upload with a `variant` field and a PNG `file`.
pub fn multipart_body(boundary: &str, slim: bool, png: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(png.len() + 400);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\n{}\r\n",
            if slim { "slim" } else { "classic" }
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// Check a skin file the player picked: a PNG of 64x64 (or 64x32) pixels.
pub fn check_skin_file(path: &Path) -> Result<RgbaImage> {
    let is_png = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png"));
    let img = if is_png { std::fs::read(path).ok().and_then(|b| decode_png(&b).ok()) } else { None };
    match img {
        Some(i) if is_valid_skin(i.width(), i.height()) => Ok(i),
        _ => bail!("That file isn't a skin. Skins are PNG images of 64x64 (or 64x32) pixels."),
    }
}

/// Upload a skin for a signed-in account (`MojangAPIUtils.uploadSkin`). Offline players can't have a skin.
pub async fn upload_skin(access_token: &str, png: &[u8], slim: bool) -> Result<()> {
    let boundary = format!("----BlockForge{}", uuid::Uuid::new_v4().simple());
    let r = crate::http::client()
        .post(SKIN_UPLOAD_URL)
        .bearer_auth(access_token)
        .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
        .body(multipart_body(&boundary, slim, png))
        .send()
        .await
        .context("couldn't reach Mojang")?;
    let status = r.status().as_u16();
    if !r.status().is_success() {
        let body = r.text().await.unwrap_or_default();
        crate::logs::error(format!("Error updating skin. HTTP {status}, response was {body}"));
        bail!("{}", upload_error_text(status));
    }
    Ok(())
}

/// What to tell the player when Minecraft's skin service answers `status` to an upload.
pub fn upload_error_text(status: u16) -> &'static str {
    match status {
        400 => "Minecraft's skin service didn't accept this picture. Pick a 64x64 PNG skin and try again.",
        401 | 403 => "Microsoft didn't accept the sign-in. Sign in to this account again, then try once more.",
        429 => "The skin was changed too many times in a short while. Wait a minute and try again.",
        500..=599 => "Minecraft's skin service isn't working right now. Try again in a little while.",
        _ => {
            "The skin couldn't be uploaded. Check your connection and try again, or sign in again if it keeps failing."
        }
    }
}

/// The whole "Change skin" action: renew the sign-in if needed, upload, and fetch the new skin so it shows.
/// Returns the account with its renewed tokens, which the caller should save.
pub async fn change_skin(dirs: &Dirs, account: &Account, file: &Path, slim: bool) -> Result<Account> {
    if account.offline {
        bail!(
            "{} is an offline player. Skins are kept by Microsoft, so sign in with a Microsoft account to change one.",
            account.name()
        );
    }
    check_skin_file(file)?;
    let png = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let fresh = crate::launch::fresh_account(dirs, account).await?;
    let token =
        fresh.access_token.clone().filter(|t| !t.is_empty()).context("this account is not signed in; sign in again")?;
    upload_skin(&token, &png, slim).await?;
    let _ = update_skin(dirs, &fresh).await;
    Ok(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(px))
    }

    #[test]
    fn steve_or_alex_follows_javas_uuid_hash() {
        assert_eq!(java_uuid_hash("00000000-0000-0000-0000-000000000000"), 0);
        assert_eq!(java_uuid_hash("00000000-0000-0001-0000-000000000000"), 1);
        assert_eq!(java_uuid_hash("00000001-0000-0000-0000-000000000000"), 1);
        assert!(!default_is_alex("00000000-0000-0000-0000-000000000000"));
        assert!(default_is_alex("00000000-0000-0001-0000-000000000000"));
        assert!(default_is_alex("00000001-0000-0000-0000-000000000000"));
        // both halves flip the answer, so they cancel
        assert!(!default_is_alex("00000001-0000-0000-0000-000000000001"));
        assert!(!default_is_alex("not a uuid"));
        // dashes don't matter
        assert_eq!(
            java_uuid_hash("069a79f444e94726a5befca90e38aaf5"),
            java_uuid_hash("069a79f4-44e9-4726-a5be-fca90e38aaf5")
        );
    }

    #[test]
    fn the_default_skins_are_the_real_ones() {
        let steve = decode_png(&default_skin_png("00000000-0000-0000-0000-000000000000")).unwrap();
        let alex = decode_png(&default_skin_png("00000000-0000-0001-0000-000000000000")).unwrap();
        assert_eq!((steve.width(), steve.height()), (64, 32));
        assert_eq!((alex.width(), alex.height()), (64, 64));
        assert!(head_pixels(&steve).is_some() && head_pixels(&alex).is_some());
    }

    #[test]
    fn the_head_is_the_face_with_a_sensible_hat() {
        let mut skin = solid(64, 64, [0, 0, 0, 0]);
        for y in 8..16 {
            for x in 8..16 {
                skin.put_pixel(x, y, Rgba([200, 100, 50, 255]));
            }
        }
        // a hat with one visible red pixel
        skin.put_pixel(41, 9, Rgba([255, 0, 0, 255]));
        let head = head_pixels(&skin).unwrap();
        assert_eq!((head.width(), head.height()), (8, 8));
        assert_eq!(head.get_pixel(0, 0).0, [200, 100, 50, 255]);
        assert_eq!(head.get_pixel(1, 1).0, [255, 0, 0, 255], "the hat layer is drawn over the face");

        // a hat that is solid white is junk and left out
        for y in 8..16 {
            for x in 40..48 {
                skin.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        assert_eq!(head_pixels(&skin).unwrap().get_pixel(1, 1).0, [200, 100, 50, 255]);
        assert!(head_pixels(&solid(32, 16, [0, 0, 0, 255])).is_none(), "too small to be a skin");
    }

    #[test]
    fn translucent_layers_blend() {
        let mut base = Rgba([100, 100, 100, 255]);
        blend(&mut base, Rgba([200, 200, 200, 128]));
        assert!((base[0] as i32 - 150).abs() <= 1 && base[3] == 255, "{base:?}");
        let mut empty = Rgba([0, 0, 0, 0]);
        blend(&mut empty, Rgba([10, 20, 30, 255]));
        assert_eq!(empty.0, [10, 20, 30, 255]);
    }

    #[test]
    fn the_figure_has_its_parts_where_the_java_preview_puts_them() {
        // paint every body part of a modern skin with its own colour, then look for them on the figure
        let mut skin = solid(64, 64, [0, 0, 0, 0]);
        let paint = |skin: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: [u8; 3]| {
            for yy in y..y + h {
                for xx in x..x + w {
                    skin.put_pixel(xx, yy, Rgba([c[0], c[1], c[2], 255]));
                }
            }
        };
        paint(&mut skin, 8, 8, 8, 8, [1, 0, 0]); // head
        paint(&mut skin, 20, 20, 8, 12, [2, 0, 0]); // body
        paint(&mut skin, 44, 20, 4, 12, [3, 0, 0]); // right arm
        paint(&mut skin, 36, 52, 4, 12, [4, 0, 0]); // left arm
        paint(&mut skin, 4, 20, 4, 12, [5, 0, 0]); // right leg
        paint(&mut skin, 20, 52, 4, 12, [6, 0, 0]); // left leg
        let fig = figure_pixels(&skin, false);
        assert_eq!((fig.width(), fig.height()), (16, 32));
        assert_eq!(fig.get_pixel(4, 0)[0], 1, "head");
        assert_eq!(fig.get_pixel(11, 7)[0], 1);
        assert_eq!(fig.get_pixel(4, 8)[0], 2, "body");
        assert_eq!(fig.get_pixel(0, 8)[0], 3, "the player's right arm is on the viewer's left");
        assert_eq!(fig.get_pixel(12, 8)[0], 4, "left arm");
        assert_eq!(fig.get_pixel(4, 20)[0], 5, "right leg");
        assert_eq!(fig.get_pixel(8, 20)[0], 6, "left leg");
        assert_eq!(fig.get_pixel(0, 0)[3], 0, "outside the figure stays clear");
        // slim arms are one pixel narrower, and start one pixel further in
        let slim = figure_pixels(&skin, true);
        assert_eq!(slim.get_pixel(0, 8)[3], 0);
        assert_eq!(slim.get_pixel(1, 8)[0], 3);
        assert_eq!(slim.get_pixel(12, 8)[0], 4);
        assert_eq!(slim.get_pixel(15, 8)[3], 0);
    }

    #[test]
    fn an_old_skin_mirrors_the_right_arm_and_leg() {
        let mut skin = solid(64, 32, [0, 0, 0, 0]);
        skin.put_pixel(44, 20, Rgba([9, 0, 0, 255])); // first pixel of the one arm
        skin.put_pixel(4, 20, Rgba([7, 0, 0, 255])); // first pixel of the one leg
        let fig = figure_pixels(&skin, false);
        assert_eq!(fig.get_pixel(0, 8)[0], 9);
        assert_eq!(fig.get_pixel(15, 8)[0], 9, "the other arm is the same arm flipped");
        assert_eq!(fig.get_pixel(4, 20)[0], 7);
        assert_eq!(fig.get_pixel(11, 20)[0], 7, "the other leg is the same leg flipped");
    }

    #[test]
    fn capes_show_their_front() {
        let mut cape = solid(64, 32, [0, 0, 0, 0]);
        cape.put_pixel(1, 1, Rgba([5, 0, 0, 255]));
        let front = cape_front(&cape).unwrap();
        assert_eq!((front.width(), front.height()), (10, 16));
        assert_eq!(front.get_pixel(0, 0)[0], 5);
        let hd = solid(128, 64, [0, 0, 0, 255]);
        assert_eq!(cape_front(&hd).unwrap().width(), 20);
        assert!(cape_front(&solid(10, 10, [0, 0, 0, 0])).is_none());
    }

    #[test]
    fn skin_sizes() {
        assert!(is_valid_skin(64, 64) && is_valid_skin(64, 32));
        assert!(!is_valid_skin(128, 128) && !is_valid_skin(64, 48));
    }

    #[test]
    fn reads_the_session_servers_answer() {
        let payload = serde_json::json!({
            "timestamp": 1, "profileId": "x", "profileName": "Steve",
            "textures": {
                "SKIN": {"url": "http://textures.minecraft.net/texture/abc", "metadata": {"model": "slim"}},
                "CAPE": {"url": "http://textures.minecraft.net/texture/cape1"}
            }
        });
        let value = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
        let answer = serde_json::json!({"id": "069a79f444e94726a5befca90e38aaf5", "name": "Notch", "properties": [{"name": "textures", "value": value}]});
        let t = parse_session_profile(answer.to_string().as_bytes()).unwrap();
        assert_eq!(t.name, "Notch");
        assert_eq!(t.skin_url.as_deref(), Some("http://textures.minecraft.net/texture/abc"));
        assert!(t.slim);
        assert_eq!(t.cape_url.as_deref(), Some("http://textures.minecraft.net/texture/cape1"));

        let plain = serde_json::json!({"id":"a","name":"B","properties":[]});
        let t = parse_session_profile(plain.to_string().as_bytes()).unwrap();
        assert!(t.skin_url.is_none() && !t.slim);
    }

    #[test]
    fn reads_the_profile_services_answer() {
        let answer = br#"{"id":"abc","name":"Alex","skins":[
            {"id":"1","state":"INACTIVE","url":"http://t/old","variant":"CLASSIC"},
            {"id":"2","state":"ACTIVE","url":"http://t/new","variant":"SLIM"}],
            "capes":[{"id":"c","state":"ACTIVE","url":"http://t/cape"}]}"#;
        let t = parse_service_profile(answer).unwrap();
        assert_eq!(t.skin_url.as_deref(), Some("http://t/new"));
        assert!(t.slim);
        assert_eq!(t.cape_url.as_deref(), Some("http://t/cape"));
        let none = parse_service_profile(br#"{"id":"abc","name":"Alex","skins":[],"capes":[]}"#).unwrap();
        assert!(none.skin_url.is_none() && none.cape_url.is_none());
    }

    #[test]
    fn a_refused_upload_says_why() {
        assert!(upload_error_text(401).contains("Sign in"));
        assert!(upload_error_text(403).contains("Sign in"));
        assert!(upload_error_text(400).contains("64x64"));
        assert!(upload_error_text(429).contains("Wait"));
        assert!(upload_error_text(503).contains("isn't working"));
        assert!(upload_error_text(418).contains("connection"));
    }

    #[test]
    fn the_upload_body_is_well_formed_multipart() {
        let body = multipart_body("XYZ", true, &[0x89, b'P', b'N', b'G']);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--XYZ\r\nContent-Disposition: form-data; name=\"variant\"\r\n\r\nslim\r\n--XYZ\r\n"));
        assert!(text.contains("name=\"file\"; filename=\"skin.png\"\r\nContent-Type: image/png\r\n\r\n"));
        assert!(text.ends_with("\r\n--XYZ--\r\n"));
        assert!(multipart_body("B", false, b"").windows(7).any(|w| w == b"classic"));
    }

    fn scratch(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("blockforge-skins-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        Dirs::at(base)
    }

    #[tokio::test]
    async fn an_offline_player_gets_a_default_skin_once() {
        let dirs = scratch("offline");
        let acct = Account::offline("Notch");
        assert!(!skin_file(&dirs, &acct).exists());
        assert_eq!(update_skin(&dirs, &acct).await.unwrap(), SkinUpdate::DefaultSaved);
        let saved = std::fs::read(skin_file(&dirs, &acct)).unwrap();
        assert_eq!(saved, default_skin_png(&acct.uuid));
        assert!(skin_file(&dirs, &acct).ends_with(format!("configs/images/skins/{}.png", acct.uuid.replace('-', ""))));
        // a second run leaves it alone
        std::fs::write(skin_file(&dirs, &acct), b"custom").unwrap();
        assert_eq!(update_skin(&dirs, &acct).await.unwrap(), SkinUpdate::Kept);
        assert_eq!(std::fs::read(skin_file(&dirs, &acct)).unwrap(), b"custom");
        // an unreadable file still draws the default
        assert_eq!(load_skin(&dirs, &acct).width(), decode_png(&default_skin_png(&acct.uuid)).unwrap().width());
        let _ = std::fs::remove_dir_all(&dirs.base);
    }

    #[test]
    fn skin_files_are_checked_before_upload() {
        let dir = std::env::temp_dir().join(format!("blockforge-skinfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("ok.png");
        solid(64, 64, [1, 2, 3, 255]).save(&ok).unwrap();
        assert!(check_skin_file(&ok).is_ok());
        let big = dir.join("big.png");
        solid(128, 128, [1, 2, 3, 255]).save(&big).unwrap();
        assert!(check_skin_file(&big).is_err());
        let text = dir.join("skin.txt");
        std::fs::write(&text, "x").unwrap();
        assert!(check_skin_file(&text).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- live checks against Mojang: `cargo test --lib live_ -- --ignored --nocapture` ----

    #[tokio::test]
    #[ignore = "uses the network"]
    async fn live_skin_of_a_known_player() {
        let (id, name) = lookup_uuid("Notch").await.unwrap();
        assert_eq!(id, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(name, "Notch");
        let textures = fetch_textures(&id).await.unwrap();
        println!("textures: {textures:?}");
        let url = textures.skin_url.clone().expect("Notch has a skin");
        let bytes = download_png(&url).await.unwrap();
        let skin = decode_png(&bytes).unwrap();
        println!("skin is {}x{}", skin.width(), skin.height());
        assert!(is_valid_skin(skin.width(), skin.height()));
        let head = head_pixels(&skin).unwrap();
        assert_eq!((head.width(), head.height()), (8, 8));
        assert!(head.pixels().any(|p| p[3] == 255));

        // the whole path with a cache folder
        let dirs = scratch("live");
        let mut acct = Account::offline("Notch");
        acct.offline = false;
        acct.uuid = "069a79f4-44e9-4726-a5be-fca90e38aaf5".into();
        acct.minecraft_username = Some("Notch".into());
        acct.access_token = None;
        assert_eq!(update_skin(&dirs, &acct).await.unwrap(), SkinUpdate::Downloaded);
        assert!(skin_file(&dirs, &acct).is_file());
        println!(
            "saved {} bytes to {}",
            std::fs::metadata(skin_file(&dirs, &acct)).unwrap().len(),
            skin_file(&dirs, &acct).display()
        );
        assert_eq!(current_username(&acct.uuid).await.as_deref(), Some("Notch"));
        let _ = std::fs::remove_dir_all(&dirs.base);
    }
}
