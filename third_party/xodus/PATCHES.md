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

## Page verification (BlockForge addition, `msixvc::integrity`)

Upstream parses the XVD hash tree but checks no page (`TODO` for unencrypted sections; NTFS-only segments get no
hashes). BlockForge adds `crates/msixvc/src/integrity.rs` and checks every page:

- `Plan::new(header)` takes a header the caller trusts and lays out the tree; it refuses integrity-disabled,
  resilient and dynamic XVDs (no Store package uses them; their trees are laid out differently).
- `VerifyingReader` wraps the sequential upstream that `PrefixCacheFile` reads: the three header pages must equal
  the trusted header byte for byte, the hash-tree levels (stored top first) are checked from the header's
  `top_hash_block_hash` down, and every hashed page after them (user data, XVC info with the region table, dynamic
  header, drive) against its leaf, before it is handed on. A mismatch is final.
- `HashTree::load` does the same from a kept metadata prefix (staging, Repair).
- `XvdFile::require_integrity(tree)`; `extract_file`, `mount_mem_fd` and `download_file_http` check every page as
  stored (ciphertext before decryption) against the tree, refuse without one (`IntegrityError::NoTree`) and refuse a
  file not starting on a page. `page_hashes` gives every file's leaf hashes, plain ones included.
  `verify_plaintext_page` re-encrypts a decrypted page with the content key and data unit to check it offline.
- `integrity::find` finds an `IntegrityError` in an error chain so callers can report a refusal as such.
- Panics on crafted input became errors: `todo!` for key ids other than 0, `.expect("Unable to seek")`, encrypted
  regions outside the hashed pages (unchecked subtraction), region tables larger than the XVC info, out-of-range
  segment indices and hash slices, invalid UTF-16 names, GPT partition bounds, the unreachable block-size `todo!`;
  the segment metadata buffer is capped at 1 MiB instead of the package's 32-bit length.
- Feature `synthetic` exposes `integrity::synthetic`, a builder of small synthetic packages (no game bytes or keys)
  for tests; only BlockForge's dev-dependencies enable it.

The format, confirmed on Minecraft 1.26.6005.0's package (2087206912 bytes, volume flags 0x41, three levels of 1, 18
and 2980 pages for 506569 hashed pages): `top_hash_block_hash` = SHA-256 of the top level's one page; every lower tree
page = the first 24 bytes of SHA-256 in its entry in the level above; level-0 entry `i` covers hashed page `i` (from
the user data on) as stored: in a region with a key id the entry is SHA-256(ciphertext)[..20] followed by the 4-byte
XTS data unit, elsewhere SHA-256(page)[..24]. Leaves are compared on 20 bytes. Region headers' `hash` field is
SHA-256(region)[..8] as a little-endian u64 for FS-MD; it is not used.

What anchors the header (BlockForge side, `src/bedrock/store/verify.rs`): the catalogue gives no content hash
(`FileHash` is empty for every file and `HashOfHashes` null, checked for Release and Preview), the licence's 32-byte
entry id is SHA-256 of the package family name in UTF-16, not of content, and the header's RSA-PSS signature needs
Microsoft's retail public key, which is not published (xvdtool knows it only by hash). The CDN hosts have no
certificate of their own, but the CDN's TLS edge (`https://a248.e.akamai.net`, Akamai's certificate) serves them by
`Host` over HTTP/1.1, so the header's first three pages are fetched there, must agree with the catalogue (content ID,
version, build ID, size) and then anchor the tree. The receipt (adapter 2) records the header's SHA-256; adapter-1
builds are not trusted (no reuse; Play refuses; Repair downloads again).

Residual risk: the header is trusted as Akamai's edge serves it over TLS for Microsoft's hosts, not by Microsoft's
signature; an attacker able to break that TLS or controlling the CDN could still serve a self-consistent package.
If the edge stops serving these hosts over HTTPS, installs fail closed with a network error until another anchor is
added.
