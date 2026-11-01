# Xbox sign-in fixtures (package D)

Synthetic test data for `src/bedrock/xbox`. Nothing here belongs to a real account: the key is a throwaway P-256
key made for these tests, and every token in the tests themselves is a made-up string.

The expected values in `signature.json` were computed outside BlockForge, so the tests check the Rust code against an
independent implementation rather than against itself:

```sh
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out synthetic-device-key.pem
openssl ec -in synthetic-device-key.pem -text -noout          # private scalar, X and Y
echo -n <X hex> | xxd -r -p | base64 -w0                       # x_b64 (standard base64, not URL-safe)
# sign input: ver(00000001) 00 filetime(8, BE) 00 "POST" 00 "/device/authenticate" 00 "" 00 body 00
{ printf '\x00\x00\x00\x01\x00'; echo -n 01dd6993f0150000 | xxd -r -p;
  printf '\x00POST\x00/device/authenticate\x00\x00'; cat device-auth-body.json; printf '\x00'; } > signinput.bin
sha256sum signinput.bin
openssl dgst -sha256 -sign synthetic-device-key.pem -out sig.der signinput.bin
openssl asn1parse -inform DER -in sig.der                      # r and s, for the 76-byte header
```

`filetime` is (1793491200 + 11644473600) * 10^7, the Windows FILETIME of 2026-11-01T00:00:00Z. The BCRYPT blob is
`45435332` (BCRYPT_ECDSA_PRIVATE_P256_MAGIC, little-endian) `20000000` (32, little-endian), then X, Y and the private
scalar, each 32 bytes big-endian: 104 bytes.
