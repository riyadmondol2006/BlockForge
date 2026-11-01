//! Hiding credentials in the game's output before it is shown, sent on or saved (the same idea as the Java
//! edition's `LineCensor`, which it builds on, for the many shapes Bedrock's sign-in secrets come in).
//!
//! Wine, the sign-in helper and the game itself print what they are given when something goes wrong: a request with
//! its `Authorization` header, a JSON answer with a refresh token, the pre-sign-in payload, a private key. None of
//! it may reach a log file or the window. Every family below is covered by a test with dummy values.

use regex::{Captures, Regex};
use std::sync::OnceLock;

const MASK: &str = "**REDACTED**";

struct Rules {
    /// `Authorization: Bearer abc` and every other header that carries a credential
    header: Regex,
    /// `Bearer abc`, `XBL3.0 x=uhs;token`
    scheme: Regex,
    /// `"refresh_token": "abc"`, `access_token=abc`, `DAProperty=abc` ...
    pair: Regex,
    /// the three dot-separated base64 parts of a signed token
    jwt: Regex,
    /// Microsoft account tickets: `t=EwB...`, `p=EwB...`
    ticket: Regex,
    /// a base64 run too long to be anything but a key or a blob
    blob: Regex,
    /// the start of a PEM block (the private key's own lines follow)
    pem_begin: Regex,
    pem_end: Regex,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules {
        header: Regex::new(
            r"(?i)\b(authorization|proxy-authorization|x-xbl-contract-version-secret|x-authorization|cookie|set-cookie|signature|x-token|x-api-key|api-key)(\s*[:=]\s*)[^\r\n]*",
        )
        .unwrap(),
        scheme: Regex::new(r"(?i)\b(bearer|xbl3\.0|basic|mcauth|rps|ms-xsts)(\s+)\S+").unwrap(),
        pair: Regex::new(
            r#"(?i)(["']?)\b(access_?token|refresh_?token|id_?token|device_?code|user_?code_?secret|client_?secret|password|passwd|passphrase|da_?property|da_?token|da_?private_?key|proof_?key|private_?key|signing_?key|session_?ticket|entity_?token|xsts_?token|user_?token|device_?token|title_?token|license_?key|content_?key|secret|ticket|token|key_?blob)\b(["']?)(\s*[:=]\s*)("(?:[^"\\]|\\.)*"|'[^']*'|[^\s,;&}\]"']+)"#,
        )
        .unwrap(),
        jwt: Regex::new(r"\beyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]*").unwrap(),
        ticket: Regex::new(r"\b([tp])=(E[wW][A-Za-z0-9+/=!*$_-]{20,})").unwrap(),
        blob: Regex::new(r"[A-Za-z0-9+/_-]{100,}={0,2}").unwrap(),
        pem_begin: Regex::new(r"-----BEGIN [A-Z0-9 ]*(PRIVATE KEY|SECRET|CREDENTIAL)[A-Z0-9 ]*-----").unwrap(),
        pem_end: Regex::new(r"-----END [A-Z0-9 ]*-----").unwrap(),
    })
}

/// Whether a string looks like base64 data and not a path or a word: it has upper and lower case letters and digits.
fn looks_like_blob(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_uppercase())
        && s.bytes().any(|b| b.is_ascii_lowercase())
        && s.bytes().any(|b| b.is_ascii_digit())
}

/// Hides secrets in lines of output, one stream at a time (it remembers whether it is inside a private key's lines).
#[derive(Clone, Debug, Default)]
pub struct Redactor {
    literals: Vec<String>,
    in_pem: bool,
}

impl Redactor {
    /// `secrets`: values known beforehand (the licence token the game was started with, a device code) that are hidden
    /// wherever they appear, whole or percent-encoded. Anything under four bytes would hide too much to be one.
    pub fn new(secrets: &[String]) -> Self {
        let mut literals: Vec<String> = Vec::new();
        for s in secrets.iter().filter(|s| s.len() >= 4) {
            literals.push(s.clone());
            let encoded = percent_encode(s);
            if encoded != *s {
                literals.push(encoded);
            }
        }
        // the longest first, so a secret that holds another is hidden whole
        literals.sort_by_key(|l| std::cmp::Reverse(l.len()));
        literals.dedup();
        Self { literals, in_pem: false }
    }

    pub fn apply(&mut self, line: &str) -> String {
        let r = rules();
        if self.in_pem {
            if r.pem_end.is_match(line) {
                self.in_pem = false;
            }
            return format!("{MASK} (private key)");
        }
        if r.pem_begin.is_match(line) {
            self.in_pem = !r.pem_end.is_match(line);
            return format!("{MASK} (private key)");
        }
        let mut out = line.to_string();
        for lit in &self.literals {
            if out.contains(lit.as_str()) {
                out = out.replace(lit.as_str(), MASK);
            }
        }
        // headers first: they take the whole rest of the line
        out = r.header.replace_all(&out, |c: &Captures| format!("{}{}{MASK}", &c[1], &c[2])).into_owned();
        out = r.scheme.replace_all(&out, |c: &Captures| format!("{}{}{MASK}", &c[1], &c[2])).into_owned();
        out = r
            .pair
            .replace_all(&out, |c: &Captures| {
                let value = &c[5];
                if value.starts_with(MASK) || value == MASK {
                    return c[0].to_string();
                }
                let quote = if value.starts_with('"') {
                    "\""
                } else if value.starts_with('\'') {
                    "'"
                } else {
                    ""
                };
                format!("{}{}{}{}{quote}{MASK}{quote}", &c[1], &c[2], &c[3], &c[4])
            })
            .into_owned();
        out = r.jwt.replace_all(&out, MASK).into_owned();
        out = r.ticket.replace_all(&out, |c: &Captures| format!("{}={MASK}", &c[1])).into_owned();
        out = r
            .blob
            .replace_all(&out, |c: &Captures| if looks_like_blob(&c[0]) { MASK.to_string() } else { c[0].to_string() })
            .into_owned();
        out
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Redactor {
        Redactor::new(&["dummy-licence-token-123".to_string()])
    }

    #[test]
    fn authorization_headers_lose_their_value() {
        let mut r = r();
        assert_eq!(r.apply("Authorization: Bearer abc.def.ghi"), format!("Authorization: {MASK}"));
        assert_eq!(r.apply("authorization=XBL3.0 x=123;tok"), format!("authorization={MASK}"));
        assert_eq!(r.apply("Cookie: sid=1; other=2"), format!("Cookie: {MASK}"));
        assert_eq!(r.apply("> Signature: AAAAdummysig=="), format!("> Signature: {MASK}"));
    }

    #[test]
    fn schemes_are_hidden_anywhere_in_a_line() {
        let mut r = r();
        let out = r.apply("sent header MCToken Bearer dummytoken-ABC123 to host");
        assert!(!out.contains("dummytoken"), "{out}");
        assert!(r.apply("XBL3.0 x=1234567;dummyxstsvalue").starts_with("XBL3.0 "));
        assert!(!r.apply("XBL3.0 x=1234567;dummyxstsvalue").contains("dummyxsts"));
    }

    #[test]
    fn json_and_assignment_pairs_are_hidden() {
        let mut r = r();
        let json = r.apply(r#"{"access_token":"dummyAccess1","refresh_token":"dummyRefresh2","expires_in":3600}"#);
        assert!(!json.contains("dummyAccess1") && !json.contains("dummyRefresh2"), "{json}");
        assert!(json.contains("\"expires_in\":3600"), "{json}");
        let kv = r.apply("DAProperty=dummyDA3 and DAToken=dummyDa4 client_secret = hunter22 id_token: dummyid5");
        for hidden in ["dummyDA3", "dummyDa4", "hunter22", "dummyid5"] {
            assert!(!kv.contains(hidden), "{kv}");
        }
        assert!(r.apply("private_key='zzzzzz'").contains("'**REDACTED**'"));
        assert_eq!(r.apply("device_code=D-999&x=1"), format!("device_code={MASK}&x=1"));
    }

    #[test]
    fn signed_tokens_tickets_and_blobs_are_hidden() {
        let mut r = r();
        let jwt = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJkdW1teSJ9.c2lnbmF0dXJl";
        assert_eq!(r.apply(&format!("got {jwt} ok")), format!("got {MASK} ok"));
        assert_eq!(r.apply("login t=EwBdummyticketvalue1234567890+/= done"), format!("login t={MASK} done"));
        let blob: String = "AbC1".repeat(40);
        assert_eq!(r.apply(&format!("key {blob}")), format!("key {MASK}"));
        // a long path or hash in lower case only is not a blob
        let path = format!("/usr/{}", "lib/".repeat(40));
        assert_eq!(r.apply(&path), path);
    }

    #[test]
    fn pem_blocks_are_hidden_to_the_end() {
        let mut r = r();
        assert!(r.apply("-----BEGIN EC PRIVATE KEY-----").contains("REDACTED"));
        assert!(r.apply("MHcCAQEEIdummykeylinedummykeyline").contains("REDACTED"));
        assert!(r.apply("-----END EC PRIVATE KEY-----").contains("REDACTED"));
        assert_eq!(r.apply("normal line again"), "normal line again");
        // a public key stays
        assert_eq!(r.apply("-----BEGIN PUBLIC KEY-----"), "-----BEGIN PUBLIC KEY-----");
    }

    #[test]
    fn known_secrets_are_hidden_plain_and_percent_encoded() {
        let mut r = Redactor::new(&["a b+c/d=e".to_string(), "ab".to_string()]);
        assert_eq!(r.apply("q=a%20b%2Bc%2Fd%3De;"), format!("q={MASK};"));
        assert_eq!(r.apply("raw a b+c/d=e!"), format!("raw {MASK}!"));
        // too short to be a secret: left alone
        assert_eq!(r.apply("ab"), "ab");
    }

    #[test]
    fn very_long_lines_are_handled() {
        let mut r = r();
        let line = format!("{} token=dummyvalue {}", "x ".repeat(30_000), "y ".repeat(5_000));
        let out = r.apply(&line);
        assert!(!out.contains("dummyvalue"));
    }
}
