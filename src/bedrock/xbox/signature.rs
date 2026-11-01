//! The profile's stable Xbox device key and the request signatures Xbox Live's auth services expect.
//!
//! Ported from BedrockOnLinux bol/auth.py (`xbl_preauth`: the device key, `_sign_header`, the proof key and the
//! BCRYPT blob), MIT, with credit. The signed bytes match WineGDK's `DeviceAuth_SignRequest`.
//!
//! What is signed (SHA-256, ECDSA P-256) is every field followed by a NUL byte:
//!
//! ```text
//! version(4, BE) 00 filetime(8, BE) 00 method 00 path 00 authorization 00 body 00
//! ```
//!
//! and the `Signature` header is standard base64 of version(4) + filetime(8) + r||s(64): 76 bytes. The body that
//! is signed is the very byte string that is sent: callers serialize once and hand the same bytes to both.

use std::fs;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde::Serialize;

use super::cache::{self, DEVICE_ID_FILE, KEY_FILE};

pub(super) const SIGNATURE_VERSION: u32 = 1;
/// Seconds from 1601-01-01 (FILETIME's start) to 1970-01-01.
const FILETIME_UNIX_OFFSET: u64 = 11_644_473_600;
/// `BCRYPT_ECDSA_PRIVATE_P256_MAGIC`, "ECS2".
pub(super) const BCRYPT_ECDSA_PRIVATE_P256_MAGIC: u32 = 0x3253_4345;
pub(super) const BCRYPT_BLOB_LEN: usize = 104;
pub(super) const HEADER_LEN: usize = 76;

/// The JSON Web Key Xbox Live binds the device to: `x` and `y` in standard base64 (with `+`, `/` and padding),
/// which is what WineGDK and the real GDK send, not the URL-safe JWK form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ProofKey {
    pub alg: &'static str,
    pub crv: &'static str,
    pub kty: &'static str,
    #[serde(rename = "use")]
    pub use_: &'static str,
    pub x: String,
    pub y: String,
}

/// One profile's device identity: the P-256 key and the `{uuid}` device ID that go with it, kept across launches
/// and across sign-outs, as Xbox Live expects of one device.
pub(super) struct DeviceKey {
    pair: EcdsaKeyPair,
    scalar: [u8; 32],
    x: [u8; 32],
    y: [u8; 32],
    pub device_id: String,
}

impl std::fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceKey").field("device_id", &self.device_id).field("x", &hex::encode(self.x)).finish()
    }
}

impl DeviceKey {
    /// The saved key, or a new one saved privately when there is none (or it can't be read, as the original does).
    /// Blocking file work: call it on a worker.
    pub fn load_or_create(dir: &Path) -> std::io::Result<DeviceKey> {
        let _held = cache::lock(dir)?;
        let (key_path, id_path) = (dir.join(KEY_FILE), dir.join(DEVICE_ID_FILE));
        if let (Ok(der), Ok(id)) = (fs::read(&key_path), fs::read_to_string(&id_path)) {
            if let Some(key) = valid_device_id(id.trim()).then(|| DeviceKey::from_pkcs8(&der, id.trim())).flatten() {
                return Ok(key);
            }
        }
        let rng = SystemRandom::new();
        let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| std::io::Error::other("could not make a device key"))?;
        let device_id = format!("{{{}}}", uuid::Uuid::new_v4());
        let key = DeviceKey::from_pkcs8(doc.as_ref(), &device_id)
            .ok_or_else(|| std::io::Error::other("the new device key could not be read back"))?;
        cache::write_private(&key_path, doc.as_ref())?;
        cache::write_private(&id_path, device_id.as_bytes())?;
        Ok(key)
    }

    /// A key from a PKCS#8 document (what ring makes, and what `openssl genpkey` makes).
    pub fn from_pkcs8(der: &[u8], device_id: &str) -> Option<DeviceKey> {
        let scalar = pkcs8_p256_scalar(der)?;
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, der, &SystemRandom::new()).ok()?;
        let public = pair.public_key().as_ref();
        if public.len() != 65 || public[0] != 0x04 {
            return None;
        }
        let (mut x, mut y) = ([0u8; 32], [0u8; 32]);
        x.copy_from_slice(&public[1..33]);
        y.copy_from_slice(&public[33..65]);
        Some(DeviceKey { pair, scalar, x, y, device_id: device_id.to_string() })
    }

    pub fn proof_key(&self) -> ProofKey {
        ProofKey {
            alg: "ES256",
            crv: "P-256",
            kty: "EC",
            use_: "sig",
            x: STANDARD.encode(self.x),
            y: STANDARD.encode(self.y),
        }
    }

    /// `BCRYPT_ECCPRIVATE_BLOB`, so WineGDK's xgameruntime can `BCryptImportKeyPair` the very key that signed the
    /// host's requests: magic (u32 LE), key length 32 (u32 LE), then X, Y and the private scalar, 32 bytes each,
    /// big-endian.
    pub fn bcrypt_blob(&self) -> [u8; BCRYPT_BLOB_LEN] {
        let mut blob = [0u8; BCRYPT_BLOB_LEN];
        blob[0..4].copy_from_slice(&BCRYPT_ECDSA_PRIVATE_P256_MAGIC.to_le_bytes());
        blob[4..8].copy_from_slice(&32u32.to_le_bytes());
        blob[8..40].copy_from_slice(&self.x);
        blob[40..72].copy_from_slice(&self.y);
        blob[72..104].copy_from_slice(&self.scalar);
        blob
    }

    /// The `Signature` header for a request sent now.
    pub fn signature_header(&self, method: &str, path: &str, body: &[u8]) -> Option<String> {
        self.signature_header_at(filetime_now(), method, path, body)
    }

    pub fn signature_header_at(&self, filetime: u64, method: &str, path: &str, body: &[u8]) -> Option<String> {
        let input = sign_input(SIGNATURE_VERSION, filetime, method, path, "", body);
        let sig = self.pair.sign(&SystemRandom::new(), &input).ok()?;
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(&SIGNATURE_VERSION.to_be_bytes());
        header.extend_from_slice(&filetime.to_be_bytes());
        header.extend_from_slice(sig.as_ref());
        (header.len() == HEADER_LEN).then(|| STANDARD.encode(header))
    }

    #[cfg(test)]
    pub fn public_xy(&self) -> ([u8; 32], [u8; 32]) {
        (self.x, self.y)
    }
}

/// What gets signed; see the module comment. The authorization field is empty for the auth services, which take
/// no `Authorization` header.
pub(super) fn sign_input(version: u32, filetime: u64, method: &str, path: &str, auth: &str, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + 8 + method.len() + path.len() + auth.len() + body.len() + 6);
    v.extend_from_slice(&version.to_be_bytes());
    v.push(0);
    v.extend_from_slice(&filetime.to_be_bytes());
    v.push(0);
    for part in [method.as_bytes(), path.as_bytes(), auth.as_bytes(), body] {
        v.extend_from_slice(part);
        v.push(0);
    }
    v
}

/// Now as a Windows FILETIME: 100-nanosecond ticks since 1601.
pub(super) fn filetime_now() -> u64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (since.as_secs() + FILETIME_UNIX_OFFSET) * 10_000_000 + u64::from(since.subsec_nanos()) / 100
}

/// `{8-4-4-4-12}` hex, as written by [`DeviceKey::load_or_create`].
fn valid_device_id(id: &str) -> bool {
    id.strip_prefix('{').and_then(|s| s.strip_suffix('}')).is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
        && id.len() == 38
}

/// One DER element: (tag, contents, rest).
fn der(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = match first {
        n if n < 0x80 => (n as usize, rest),
        0x81 => (*rest.first()? as usize, &rest[1..]),
        0x82 => ((usize::from(*rest.first()?) << 8) | usize::from(*rest.get(1)?), rest.get(2..)?),
        _ => return None,
    };
    let contents = rest.get(..len)?;
    Some((tag, contents, &rest[len..]))
}

/// The private scalar of a PKCS#8 P-256 key (RFC 5208 wrapping RFC 5915), which ring can sign with but doesn't
/// hand out, and the BCRYPT blob needs.
fn pkcs8_p256_scalar(doc: &[u8]) -> Option<[u8; 32]> {
    const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
    const PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
    let (0x30, info, _) = der(doc)? else { return None };
    let (0x02, _version, rest) = der(info)? else { return None };
    let (0x30, algorithm, rest) = der(rest)? else { return None };
    let (0x04, private, _) = der(rest)? else { return None };
    let (0x06, oid, params) = der(algorithm)? else { return None };
    let (0x06, curve, _) = der(params)? else { return None };
    if oid != EC_PUBLIC_KEY || curve != PRIME256V1 {
        return None;
    }
    let (0x30, ec_key, _) = der(private)? else { return None };
    let (0x02, version, rest) = der(ec_key)? else { return None };
    let (0x04, d, _) = der(rest)? else { return None };
    if version != [1] || d.is_empty() || d.len() > 32 {
        return None;
    }
    let mut scalar = [0u8; 32];
    scalar[32 - d.len()..].copy_from_slice(d);
    Some(scalar)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_ASN1, ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
    use serde_json::Value;

    pub(in crate::bedrock::xbox) fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../tests/bedrock-fixtures/xbox/signature.json")).unwrap()
    }

    pub(in crate::bedrock::xbox) fn fixture_key() -> DeviceKey {
        let pem = include_str!("../../../tests/bedrock-fixtures/xbox/synthetic-device-key.pem");
        let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = STANDARD.decode(b64).unwrap();
        DeviceKey::from_pkcs8(&der, fixture()["device_id"].as_str().unwrap()).unwrap()
    }

    fn hex_field(name: &str) -> Vec<u8> {
        hex::decode(fixture()[name].as_str().unwrap()).unwrap()
    }

    fn public_point() -> Vec<u8> {
        [vec![4u8], hex_field("x_hex"), hex_field("y_hex")].concat()
    }

    /// Check a `Signature` header against the request it claims to sign, with a public key from the fixture
    /// (computed by OpenSSL), not from the key that signed.
    pub(in crate::bedrock::xbox) fn verify_header(public: &[u8], header: &str, path: &str, body: &[u8]) -> bool {
        let Ok(raw) = STANDARD.decode(header) else { return false };
        if raw.len() != HEADER_LEN || raw[..4] != SIGNATURE_VERSION.to_be_bytes() {
            return false;
        }
        let filetime = u64::from_be_bytes(raw[4..12].try_into().unwrap());
        let input = sign_input(SIGNATURE_VERSION, filetime, "POST", path, "", body);
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public).verify(&input, &raw[12..]).is_ok()
    }

    #[test]
    fn the_sign_input_is_nul_delimited_exactly_as_openssl_was_given_it() {
        let f = fixture();
        let body = include_bytes!("../../../tests/bedrock-fixtures/xbox/device-auth-body.json");
        let input = sign_input(1, f["filetime"].as_u64().unwrap(), "POST", "/device/authenticate", "", body);
        assert_eq!(hex::encode(&input), f["sign_input_hex"].as_str().unwrap());
        let digest = ring::digest::digest(&ring::digest::SHA256, &input);
        assert_eq!(hex::encode(digest.as_ref()), f["sign_input_sha256"].as_str().unwrap());
    }

    #[test]
    fn the_filetime_counts_from_1601_in_100ns_ticks() {
        let f = fixture();
        let unix = f["unix_seconds"].as_u64().unwrap();
        assert_eq!((unix + FILETIME_UNIX_OFFSET) * 10_000_000, f["filetime"].as_u64().unwrap());
        let now = filetime_now();
        let unix_now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert!(now / 10_000_000 - FILETIME_UNIX_OFFSET >= unix_now - 1);
    }

    #[test]
    fn our_header_is_76_bytes_and_verifies_with_the_independent_public_key() {
        let f = fixture();
        let key = fixture_key();
        let body = include_bytes!("../../../tests/bedrock-fixtures/xbox/device-auth-body.json");
        let filetime = f["filetime"].as_u64().unwrap();
        let header = key.signature_header_at(filetime, "POST", "/device/authenticate", body).unwrap();
        let raw = STANDARD.decode(&header).unwrap();
        assert_eq!(raw.len(), 76);
        assert_eq!(&raw[..4], &[0, 0, 0, 1]);
        assert_eq!(&raw[4..12], &hex_field("sign_input_hex")[5..13], "the header's time is the signed time");
        // The signature is over the independently computed sign input, under the independently computed key.
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public_point())
            .verify(&hex_field("sign_input_hex"), &raw[12..])
            .unwrap();
        assert!(verify_header(&public_point(), &header, "/device/authenticate", body));
        // ...and only over that: another path or one changed body byte fails.
        assert!(!verify_header(&public_point(), &header, "/user/authenticate", body));
        let mut changed = body.to_vec();
        changed[10] ^= 1;
        assert!(!verify_header(&public_point(), &header, "/device/authenticate", &changed));
    }

    #[test]
    fn the_header_openssl_made_is_read_the_same_way() {
        let f = fixture();
        let body = include_bytes!("../../../tests/bedrock-fixtures/xbox/device-auth-body.json");
        let header = f["openssl_header_b64"].as_str().unwrap();
        assert_eq!(header.len(), 104);
        assert!(verify_header(&public_point(), header, "/device/authenticate", body));
        let raw = STANDARD.decode(header).unwrap();
        assert_eq!(hex::encode(&raw[12..]), f["openssl_signature_rs_hex"].as_str().unwrap());
        // The DER form OpenSSL wrote verifies too, so r||s was taken out of it correctly.
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public_point())
            .verify(&hex_field("sign_input_hex"), &hex_field("openssl_signature_der_hex"))
            .unwrap();
    }

    #[test]
    fn the_coordinates_are_standard_base64() {
        let f = fixture();
        let proof = fixture_key().proof_key();
        assert_eq!(proof.x, f["x_b64"].as_str().unwrap());
        assert_eq!(proof.y, f["y_b64"].as_str().unwrap());
        assert!(proof.y.contains('+') && proof.y.contains('/'), "this fixture's y tells standard from URL-safe");
        assert_eq!(
            serde_json::to_string(&proof).unwrap(),
            format!(
                r#"{{"alg":"ES256","crv":"P-256","kty":"EC","use":"sig","x":"{}","y":"{}"}}"#,
                f["x_b64"].as_str().unwrap(),
                f["y_b64"].as_str().unwrap()
            )
        );
    }

    #[test]
    fn the_bcrypt_blob_is_104_bytes_and_imports_as_the_same_key() {
        let f = fixture();
        let blob = fixture_key().bcrypt_blob();
        assert_eq!(blob.len(), 104);
        assert_eq!(hex::encode(blob), f["bcrypt_blob_hex"].as_str().unwrap());
        assert_eq!(u32::from_le_bytes(blob[0..4].try_into().unwrap()), 0x3253_4345);
        assert_eq!(u32::from_le_bytes(blob[4..8].try_into().unwrap()), 32);
        // Import it the way BCryptImportKeyPair reads it: ring checks the scalar and the point belong together.
        let public = [&[4u8][..], &blob[8..40], &blob[40..72]].concat();
        let rng = SystemRandom::new();
        let imported =
            EcdsaKeyPair::from_private_key_and_public_key(&ECDSA_P256_SHA256_FIXED_SIGNING, &blob[72..], &public, &rng)
                .unwrap();
        let sig = imported.sign(&rng, b"imported").unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public_point()).verify(b"imported", sig.as_ref()).unwrap();
        // A blob whose scalar is not the point's is refused, so the check above means something.
        let mut wrong = blob;
        wrong[103] ^= 1;
        assert!(
            EcdsaKeyPair::from_private_key_and_public_key(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &wrong[72..],
                &public,
                &rng
            )
            .is_err()
        );
    }

    #[test]
    fn the_device_key_is_made_once_and_then_kept() {
        let dir = crate::testutil::scratch("xbox-key").join("xbox");
        let first = DeviceKey::load_or_create(&dir).unwrap();
        let again = DeviceKey::load_or_create(&dir).unwrap();
        assert_eq!(first.device_id, again.device_id);
        assert_eq!(first.public_xy(), again.public_xy());
        assert!(valid_device_id(&first.device_id), "{}", first.device_id);
        use std::os::unix::fs::PermissionsExt;
        for name in [KEY_FILE, DEVICE_ID_FILE] {
            assert_eq!(fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // The scalar read out of ring's own PKCS#8 matches its public key.
        let blob = first.bcrypt_blob();
        let public = [&[4u8][..], &blob[8..72]].concat();
        let rng = SystemRandom::new();
        EcdsaKeyPair::from_private_key_and_public_key(&ECDSA_P256_SHA256_FIXED_SIGNING, &blob[72..], &public, &rng)
            .unwrap();
    }

    #[test]
    fn a_damaged_key_is_replaced_rather_than_used() {
        let dir = crate::testutil::scratch("xbox-key").join("xbox");
        let first = DeviceKey::load_or_create(&dir).unwrap();
        fs::write(dir.join(KEY_FILE), b"garbage").unwrap();
        let second = DeviceKey::load_or_create(&dir).unwrap();
        assert_ne!(first.public_xy(), second.public_xy());
    }

    #[test]
    fn malformed_pkcs8_is_refused() {
        assert!(pkcs8_p256_scalar(b"").is_none());
        assert!(pkcs8_p256_scalar(&[0x30, 0x82, 0xff]).is_none());
        assert!(DeviceKey::from_pkcs8(&[0x30, 0x03, 0x02, 0x01, 0x00], "{x}").is_none());
    }
}
