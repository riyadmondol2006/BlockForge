# Xodus source and downstream changes

Base: https://github.com/xodus-gaming/xodus at
`64d39eb87a56c7d0d7e7fde0b233654ac5477b0f` (Rust 1.98+).
Only `crates/xodus`, `crates/msixvc`, `crates/msixvc-common` are vendored.
The upstream workspace manifest is retained as provenance; the root BF workspace
owns resolution and Cargo.lock. Crate workspace dependencies were expanded with
upstream versions. No Xodus CLI/service or eappx dependency is built.

BOL patches from BedrockOnLinux `cc6ac3235bfbbf9d05fe6870b09ef71e7f1a4f52`:

| Patch | Application |
| --- | --- |
| 0001 | Applied to msixvc streaming cache, including race regression tests. |
| 0002 | Applied to xodus license enum, including Trial/unknown tests. |
| 0003 | Applied to xodus content license parsing and tests. |
| 0004 | CLI-only; ported into store/ipc.rs headers/host-context and helper. Both hosted legs use the same construction. |
| 0005 | CLI-only; ported into store/license.rs. Missing entries return SignInRequired. |
| 0006 | Applied to xodus delegation token conversion and tests. |
| 0007 | Applied to xodus HTTP licensing rejection and reason tests. |

Original patches are retained in patches/. GPL license: LICENSE. BOL's MIT
license: BOL-LICENSE; ported orchestration credits BedrockOnLinux contributors.
Keep the original per-file notices, including SPLicense's CikExtractor MIT notice.

Additional BF changes (review these when rebasing):

- Disable native keyring backend by default, opt-in `native-keychain` feature.
  BF supplies a path backend; it never calls init_secrets or mutates HOME.
- Explicit rustls reqwest features, matching BF, without default native TLS.
- HttpRead implements AsyncSeek for range-based extraction; checks range end,
  total, response length and stable remote size in addition to starting offset.
- Extraction write errors propagate instead of retrying forever and printing.
- License content endpoint injection for mocked HTTP acceptance tests.
- Device registration XML/status errors return RSTError; malformed/missing HMAC
  state and empty exchange collections return errors rather than panicking.
- Fallible CLEP signing/device/HMAC key accessors; bounded RSA blob parsing,
  short SPLicense signature-block guard. BF uses the fallible accessors.
- Upstream live provisioning test is ignored and no longer prints its token.

Root patch pin: ntfs `5edc794c3ec73a2722dad24abefd05f270d3e066`.
xal remains upstream's immutable `6aa2420f8f0d5e6f7460ff13f399037583a942f4`.
- XAL facade/re-export is opt-in (`xal-auth`); retain its upstream git pin, but
  use its same small `cvlib` directly for the correlation header. Acquisition
  does not compile XAL's reqwest 0.11/native TLS stack. Host Xbox is package D.
- Encrypted-region coverage is exposed for staging refusal; synthetic XVD/AES
  fixture verifies padded ciphertext retention and logical plaintext length.
- SPLicense blocks have a 16 MiB allocation cap; stream resumes are bounded.
- TokenManager clones share a lock for read/merge/write token updates. Slot-level
  file leases additionally serialize cross-process provisioning/account changes.
- The hardware component builder accepts optional precollected SMBIOS. BF invokes
  the optional pkexec probe asynchronously with cancellation and a 30 second
  timeout, then uses the upstream no-firmware fallback. No prompt during startup.
- Expanded dependencies explicitly enable chrono/uuid serialization, previously
  supplied incidentally by the unused XAL facade. Formatting follows cargo fmt.
- Synthetic XVD coverage exposed FILETIME nanosecond overflow for the valid zero
  timestamp (1601). Convert in wide seconds/remainder arithmetic; regression tests
  cover zero, Unix epoch, and both signed extremes.

## Verifying the carried patches

`patches/SHA256SUMS` (paths relative to the BlockForge root) pins the seven files;
they are byte-identical to BedrockOnLinux's `third_party/xodus/patches` at
`cc6ac323`. To re-check that each still applies and is carried:

```sh
sha256sum -c third_party/xodus/patches/SHA256SUMS
mkdir -p "$SCRATCH/pristine" && git -C "$XODUS" archive 64d39eb87a56c7d0d7e7fde0b233654ac5477b0f crates | tar -x -C "$SCRATCH/pristine"
cd "$SCRATCH/pristine" && for p in "$BF"/third_party/xodus/patches/*.patch; do git apply --check "$p" && git apply "$p"; done
```

All seven apply cleanly to the pin (checked 2026-11-01). They cannot be reverse-applied
verbatim to the vendored tree because it is `cargo fmt`-formatted at BlockForge's
width and carries the changes listed above; instead every added line of 0001/2/3/6/7
was compared whitespace-insensitively with the vendored file. 233 of 241 lines match
as written; the other 8 are the same code reflowed onto fewer lines (checked by hand:
`Flushing { written: written as u64 }`, `refusal_reason(&body)`, the `"message"`
lookup, the `delegationcompact` arm, the `base64url` closure, the content-key and
`LicenseKeys { value }` builders). Each patch's own tests are kept and run with
`cargo test --locked -p msixvc -p xodus`.

## Known limit carried from upstream: no per-page hash check

The package CDN roots the catalogue returns are plain `http://` (for example
`assets1.xboxlive.com`), and those hosts answer TLS with the CDN's certificate
(`a248.e.akamai.net`), so the URL is used as given. `XvdFile::extract_file_ex`
does not compare pages with the XVD hash tree (`data_hashs`; upstream has a
`TODO` for unencrypted sections), and AES-XTS is not authenticated. What is checked:
the catalogue (over HTTPS) gives the package size and URL, every range response
is validated, the XVD content ID must match the edition, and the licence supplies
the key. Hash-tree verification, ideally up to the signed XVD header, should be
added once its hash format has been confirmed against a real package; it was not
guessed here, because a wrong guess would refuse every real download.
