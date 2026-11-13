# ADR 0003 — Secure network: TLS pins, SSH tunnels, known hosts, the vault

- Status: Accepted
- Date: 2026-11-13
- Scope: M2b (HV-SEC-01, -02, -05; HV-CONN-01 transport)

## Context

M2a connects directly, verifies TLS non-interactively and keeps secrets in the OS keyring
(ADR 0002, the M2a plan). M2b adds what a database client needs on real networks:

- TLS certificate review and pins, the question before credentials cross a public network in
  plaintext, the insecure-auth guard, channel binding and encrypted client keys;
- SSH tunnels (jump hosts, `~/.ssh/config` import, the agent, keyboard-interactive) with
  Hacivat's own `known_hosts` next to OpenSSH's;
- an encrypted-file vault where no keyring is usable, and an in-memory session overlay that
  keeps the secrets of a live connection for its reconnects and side connections.

Each of these is a security choice. The plan's Global Constraints require every one to be
recorded here, by the task that makes it, before review.

## Rule

Each M2b task that makes a security choice in this area appends one section headed
`## m2b-tNN — <topic>` (append-only; a choice made in a fix round goes into the same task's
section before re-review). A section states the choice, the alternatives it rejected, how it
fails closed, and the tests that hold it. Topics, in the order of the plan: TLS pins and review,
the plaintext question, the insecure-auth guard, channel binding, SSH, known hosts, the agent,
the vault, the session overlay, the keyring CI and the deferrals.

## m2b-t01 — crypto dependencies and the SSH stack

- **Provider.** rustls stays on ring only (M2a P5). `aws-lc-rs` and `aws-lc-sys` stay banned in
  `deny.toml`. russh 0.65.1 is built with `default-features = false` and the features `ring`,
  `rsa` and `flate2` (Q28), so its `aws-lc-rs` default is off.
- **Proof (Task 1, recorded from the lock at the interface commit).**
  - `cargo tree -i aws-lc-rs --workspace`: no package matches; `aws-lc-rs` is not in
    `Cargo.lock` at all (the same for `aws-lc-sys`).
  - `cargo tree -e features -i russh --workspace --depth 1`:
    ```
    russh v0.65.1
    ├── russh feature "flate2"
    ├── russh feature "ring"
    └── russh feature "rsa"
    ```
  - `cargo tree -i ring --workspace --depth 1`:
    ```
    ring v0.17.14
    ├── rcgen v0.14.10
    │   [dev-dependencies]
    ├── russh v0.65.1
    ├── rustls v0.23.46
    └── rustls-webpki v0.103.16
    ```
  - `cargo tree -i rsa --workspace --depth 1`:
    ```
    rsa v0.10.0-rc.18
    ├── russh v0.65.1
    └── ssh-key v0.7.0-rc.11
    ```
  - `cargo tree -i pageant --workspace --target x86_64-pc-windows-msvc --depth 1` (the Windows
    agent transport):
    ```
    pageant v0.2.5
    └── russh v0.65.1
    ```
- **RUSTSEC-2023-0071 (the `rsa` crate's Marvin timing issue; no fixed version) is accepted.**
  russh's RSA support pulls `rsa`. The advisory concerns RSA decryption and signing timing that
  an attacker measures. Hacivat only signs SSH authentication challenges, locally, with the
  user's own key; no remote party can time repeated private-key operations on attacker-chosen
  input. Ed25519 keys are preferred in the copy. The `deny` CI job checks licences, bans and
  sources only (unchanged from M2a), so this is a documented acceptance, not a CI pass (Q45).
- **russh 0.65.1 and its pre-release dependencies (fix round 1, M8).**
  - russh 0.65.1 (published 2026-11-03) pins exact pre-releases: `rsa =0.10.0-rc.18`,
    `ssh-key =0.7.0-rc.11` and `pkcs1 =0.8.0-rc.4`. They are pinned with `=`, so
    `Cargo.lock` cannot drift to another release candidate.
  - `ring` covers only part of russh. The graph also brings the RustCrypto 0.5/0.14
    generation: aes, ctr, chacha20, curve25519-dalek 5, ed25519-dalek 3, p256/p384/p521,
    ml-kem, scrypt and bcrypt-pbkdf, next to the digest-0.10 crates Hacivat uses itself.
    None of it is aws-lc or OpenSSL; every licence is on the allow-list.
  - **Rejected: russh 0.54.x**, the last line on stable `rsa ^0.9` / `pkcs1 ^0.7`. It
    depends on an unofficial fork (`internal-russh-forked-ssh-key =0.6.11`),
    `libcrux-ml-kem ^0.0.3` and `rand 0.8`/`getrandom 0.2`, and it is over a year behind on
    SSH fixes. By 0.60 russh already pins `rsa` release candidates.
  - Downgrading buys nothing on the advisory: RUSTSEC-2023-0071 affects `rsa` 0.9 equally.
  - Revisit when russh moves to stable `rsa` 0.10 / `ssh-key` 0.7. A bump is a reviewed
    change: `cargo update -p russh --precise <v>`, listed in the commit body.
- **Other crypto crates** (all on the cargo-deny allow-list): `x509-parser` 0.18 (certificate
  review), `sha2` 0.10 (pins, fingerprints, the RFC 5929 end-point hash), `pkcs8` 0.10.2 with
  `encryption`, `pkcs5`, `pem`, `std` and `sha1-insecure` (encrypted client keys, including
  OpenSSL 1.0's PBES2 with an HMAC-SHA1 PRF; Q5, Q36), `hmac` 0.12 and `sha1` 0.10 (hashed
  `known_hosts` entries), `chacha20poly1305` 0.10.1, `argon2` 0.5.3 and `getrandom` 0.3 (the
  vault, `getrandom::fill` with no `rand_core`; Q30).
- **Stubs fail closed until their owners fill them** (Task 1, "No silent weakening"): a TLS pin
  for the endpoint makes every handshake fail with `TlsFailure::PinMismatch` and forbids the
  plaintext fallback; the host-key check and the tunnel refuse; the vault is locked and stores
  nothing; the SSH transport is still refused at Validate (`TransportUnavailable`).

## m2b-t03 — TLS pins and review

- **A pin is the SHA-256 of the leaf certificate's DER (B5), not of its SPKI.** It is the
  fingerprint the review shows and users compare (`openssl x509 -fingerprint -sha256`), and it
  is what the copy promises ("trusts exactly this certificate"). `alg/01` says SPKI; a leaf pin
  is stricter (a re-issued certificate with the same key is a change to review), which is the
  safe direction. *Rejected:* SPKI pins (survive renewal silently), CA pins (a different trust
  question, covered by Import CA).
- **What a pin checks.** In every mode except Disable, the endpoint's pin replaces the
  certificate check: the leaf's SHA-256 must equal the pin, and now must lie inside the leaf's
  `notBefore`…`notAfter` (D-005: a pin never extends the dates). The chain, the name, CA:TRUE,
  key usages and Apple's lifetime policy are not checked, because the user reviewed exactly
  this certificate. Handshake signatures are always verified with ring's algorithms, so the
  server proves it holds the pinned certificate's key. Require with a pin verifies the pin.
  The outcome is `Pinned`; without a pin the M2a mode check applies and the outcome is `Full`
  (Auto, VerifyFull), `CaOnly` (VerifyCa) or `No` (Require).
- **The endpoint rule.** A pin applies to `pin_endpoint(tls_host, port)` only: the TLS host as
  typed, compared in ASCII lowercase without a trailing dot, IPv6 in brackets, and the same
  port. A pin for `db:5432` never applies to `db:5433`; `DB` and `db.` are `db` (DNS names are
  case-insensitive, so a case-sensitive match would let `DB` skip the pin of `db` and fall back
  to chain trust and plaintext). Stored endpoints are normalised the same way when they are
  compared (`TlsSettings::pin_for`). An IP literal and a host name are different endpoints.
  *Fix round 1:* an IP literal is written in its canonical form (`0::1` and
  `0:0:0:0:0:0:0:1` are `[::1]`), and an IPv4-mapped IPv6 address is its IPv4 address
  (`::ffff:10.0.0.5` is `10.0.0.5`, where it connects), so re-typing an address in another
  spelling cannot drop its pin. A zone id is not an IP literal to Rust's parser and stays as
  typed (lowercase).
- **A pin forbids plaintext (Q13).** `plaintext_allowed` is false whenever the endpoint has a
  pin, `allow_public_plaintext` included. **Disable with a pin (T1 review M2):** Disable never
  runs TLS, so the pin cannot be checked. `build_tls_params` keeps returning `None` for
  Disable (B5; Disable is an explicit choice, never a fallback), and
  `TlsSettings::disable_ignores_pin(host, port)` is the flag Validate and the dialog show.
  *Rejected:* refusing Disable outright (B5 and the brief's `pins_never_apply_in_disable`
  keep Disable working).
- **Classification (T1 review M1).** A mismatch is a `rustls::Error::InvalidCertificate(
  CertificateError::Other(..))` carrying Hacivat's own `PinMismatchError`, which
  `TlsFailure::from_rustls` maps to `PinMismatch` by type (a downcast, never by text). A driver
  that reads a fresh, empty `failure` slot (the pipeline's `fresh_tls`) therefore still reports
  `tls.pin_changed`; MySQL through patch §5 gets the same. *Rejected:*
  `ApplicationVerificationFailure` plus the slot (lost when the slot is replaced).
- **Recording.** The verifier writes the presented chain (leaf first) before its verdict, so a
  failed handshake can be reviewed; each certificate check first clears `outcome` and
  `failure`, and writes `outcome` only on success. *Fix round 1:* any verifier error (the
  handshake signature included) clears `outcome` again, so an impostor that replays the pinned
  leaf without its key fails as `Protocol` with no outcome, never `Pinned`. Errors after the
  verifier (a bad `Finished`) are the driver's: an outcome is meaningful only after a
  successful handshake. **TLS session resumption is off** in the
  configuration: a resumed session skips the verifier and would leave `presented` and
  `outcome` empty (MySQL's side connection reuses the configuration, and the insecure-auth
  guard reads `outcome`). One `TlsParams` serves one connect attempt: its slots belong to the
  configuration, so two concurrent handshakes through one `TlsParams` would share them (the
  pipeline builds the params per attempt; M2a ruling).
- **One configuration for both engines (Q2).** `TlsParams.config` holds the pin-aware verifier,
  the roots and the decrypted client identity; PostgreSQL uses it through `tokio-rustls`, MySQL
  through patch §5. Nothing in it is engine-specific. ring is the only provider.
- **Certificate review.** `x509-parser` 0.18 parses each certificate; anything that is not one
  well-formed DER certificate (trailing bytes included) makes the review `None`, so no "trust"
  action is offered. Names are RFC 4514 strings (last RDN first, `,` and `+`, RFC 4514's
  escapes, short names CN/L/ST/O/OU/C/STREET/DC/UID, else the dotted OID; UTF-8 values
  verbatim, `BMPString` decoded, other values as `#hex`). *Fix round 1:* control characters
  (C0, DEL, C1) and Unicode bidi formatting characters (U+061C, U+200E/F, U+202A–202E,
  U+2066–2069) are written as RFC 4514 hex pairs of their UTF-8 (`\E2\80\AE`), so a
  hostile name cannot reorder or split the text the review shows; letters such as `ş` and
  `İ` stay verbatim. RFC 4514's own `\XX` form was chosen over a `\u{..}` notation so the
  string stays a valid RFC 4514 name. Roles go by position: the first is
  the leaf, the last is `Root` when its subject equals its issuer, the rest are intermediates.
  10,000 deterministic DER mutations run in `tls_review.rs` without a panic.
- **Channel binding (B9, Q36).** `tls_server_end_point` hashes the leaf DER with the hash of its
  `signatureAlgorithm`: MD5 and SHA-1 become SHA-256 (RFC 5929 §4.1); SHA-224/256/384/512 for
  RSA PKCS #1 v1.5, ECDSA (`ecdsa-with-SHA512` included) and DSA; RSASSA-PSS uses the hash in
  its parameters (SHA-1, the RFC 4055 default when absent, becomes SHA-256). Ed25519, Ed448,
  unknown algorithms, unknown PSS hashes and malformed DER give `None`: no binding is offered.
- **Encrypted client keys (B25, Q36).** PKCS #8 `EncryptedPrivateKeyInfo` (PEM or DER) with
  PBES2: PBKDF2 with HMAC-SHA-1 (OpenSSL 1.0, `sha1-insecure`) or SHA-2, or scrypt; AES-CBC.
  No passphrase is `KeyEncrypted`; a failed decryption, or plaintext that is not a PKCS #8 key
  (valid padding by chance), is `KeyPassphraseWrong`; an unsupported scheme is `KeyUnreadable`;
  a legacy `Proc-Type: 4,ENCRYPTED` key is `KeyEncryptedLegacy` whether or not a passphrase is
  offered (Hacivat does not implement OpenSSL's legacy EVP_BytesToKey). A key file that asks for
  more than 10 million PBKDF2 iterations or 256 MiB of scrypt memory is `KeyUnreadable` before
  any derivation, so a crafted file in an imported profile cannot hold the core runtime. The
  passphrase is a `SecretString`, exposed only to the KDF. *Fix round 1:* the key is loaded
  by ring (`ring::sign::any_supported_type`) from a borrowed buffer and installed through a
  one-key resolver (`SingleCertAndKey`, with rustls' own `keys_match` check), so no owned copy
  of the DER reaches rustls: a decrypted key stays in pkcs8's zeroizing `SecretDocument`, and a
  file's bytes and a PEM-decoded DER sit in `Zeroizing`, all wiped once ring holds the key.
  pkcs5 0.7's derived AES key is not zeroized (upstream). `is_network_path` stays the first check
  before any file is opened (M2a N2).
- **Fails closed.** A pin mismatch, an expired or not-yet-valid pinned leaf, or an unparsable
  pinned leaf refuses the handshake; a pin turns the plaintext fallback off; a review that
  cannot parse offers no trust action; an unknown signature algorithm offers no channel
  binding.
- **Tests.** `crates/core/hacivat-net/tests/tls_pins.rs` (pins in every mode, IP-only SAN,
  CA:TRUE and 10-year certificates, rotation with both fingerprints, trusted chain with another
  pin, expiry and not-yet-valid, name mismatch, the endpoint rule, fresh recording without
  resumption, Q13, Disable, encrypted keys and the KDF cap, no passphrase in `Debug`);
  `tests/tls_review.rs` (RFC 4514, roles, malformed DER, the fuzz run, fingerprints, end-point
  hashes); the `src/tls.rs` unit tests (endpoint normalisation, the Disable flag, M1
  classification, escapes).
