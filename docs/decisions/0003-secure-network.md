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

## m2b-t04 — SSH config import and known hosts

- **Known hosts: the stores.** Hacivat's own file (`<data>/known_hosts`, OpenSSH format) is
  read and written; `~/.ssh/known_hosts` and `known_hosts2` are only read, never written
  (`trust_writes_hacivat_store_only`, `replace_only_touches_the_hacivat_store`).
  - `KnownHosts::load` does no I/O (Q42). Each file is read at the first `check` or `trust`,
    and every check `stat`s each file once and re-reads it when its `(mtime, len)` changed
    (Q24), so the copied `ssh-keygen -R` works on the next retry without a restart
    (`openssh_file_edit_is_seen_without_restart`,
    `an_edit_with_the_same_length_is_seen_through_the_mtime`). Hacivat's own file follows the
    same rule, so a hand edit is seen too.
  - One `Mutex` orders every check and every write (read, modify, write a `0600` temp file,
    `fsync`, rename; a missing directory is created `0700`). Concurrent trusts never lose a
    line (`concurrent_trusts_are_ordered`).
  - The name OpenSSH stores is the bare host on port 22 and `[host]:port` on any other port
    (`known_hosts::host_pattern`). Matching and the remediation command both use it: the mock
    brief's `ssh-keygen -R "[host]:22"` would not match a port-22 line (design/m2b-secure
    NOTES, open question 1).
- **Matching is OpenSSH's.** Patterns `*` and `?`, comma lists, `!` negation (a matching
  negated pattern excludes the line; a negated pattern alone matches nothing), hashed names
  (`|1|salt|hash`, HMAC-SHA1 over the lookup name, checked with `verify_slice`), host names
  ASCII lower-cased (INV-3) and compared as raw bytes. Lines are bytes: non-UTF-8 never
  panics, CRLF is accepted, a line over 16 KiB is skipped without being buffered and counted
  (`KnownHosts::skipped_lines`: `hacivat-net` has no logger, so the caller logs the count), and
  one file is read up to 64 MiB. Unknown `@` markers and undecodable keys skip their line; a key
  whose blob names another type than its line is refused. Comments are dropped and never
  surfaced, so they never reach a profile (the secret-scan rule has nothing to scan).
  - A wildcard line in the user's own file (`* <key>`) does make that key known for every host,
    as it does for OpenSSH: the file is the user's statement of trust. Hacivat itself never
    writes a pattern (below).
- **Decision order** (`check`): a matching `@revoked` line for the presented key wins
  (`ssh.host_key_revoked`); then the key listed for the host in any file makes it known; then a
  listed key **of the same type** makes it changed, with the old key and its source (Hacivat's
  store first, so the review can offer the replacement). A host that only has keys of other
  types is unknown, as in OpenSSH. A presented key whose type cannot be decoded counts any
  listed key as a change (fails closed).
  - Consequence for Task 9 (carried): a server that offers a key of a new type is "unknown",
    so `AcceptNew` would accept it. OpenSSH avoids most of this by preferring the host-key
    algorithms already listed for the host; the tunnel should do the same.
- **`@cert-authority` (Q32, pinned).** russh 0.65.1 does hand a host certificate to
  `check_server_key` (`PublicKeyOrCertificate::Certificate`) when the client offers certificate
  host-key algorithms, but `KnownHosts::check` takes a plain `HostKey`, and verifying a host
  certificate (CA signature, principals, validity) is not built in v1. So a CA line is parsed
  and recognised but never makes a plain key known (nor changed): the key is `Unknown`
  (`cert_authority_line_is_recognised`). Task 9 must not offer certificate host-key algorithms,
  or must refuse a certificate it is shown. Host certificates are v1.x.
- **Writes refuse what could weaken the store.**
  - `trust` and `replace` accept only host names made of `[A-Za-z0-9.-_:%]` (DNS names, IPv4,
    IPv6 with a scope id): no newline, space, comma, wildcard, `!`, bracket, `|`, `#` or `@` can
    inject a line, a pattern or a marker
    (`trust_refuses_names_that_could_inject_lines_or_patterns`).
  - The key must be exactly `<type> <base64>`, whose blob decodes, names that type and hashes
    to `HostKey.sha256`; the line written is re-encoded from the blob.
  - `trust` refuses a key revoked for the host (`PermissionDenied`) and a key that would change
    a known one (`AlreadyExists`): a changed key is a hard stop that only `replace` resolves, so
    no policy, `AcceptNew` included, can trust it (B11).
  - `replace` removes only the lines naming exactly this host (plain or hashed) with the old
    key, then appends the new one. Without such a line it writes nothing and returns `NotFound`
    (an old key reached only through a hand-written wildcard line in Hacivat's file is not
    replaced).
- **`~/.ssh/config` import (B12)** is Hacivat's own parser, and it only reads files: nothing in
  a config is executed (`ProxyCommand` entries are listed as unusable).
  - `Host` patterns with `*`, `?` and `!`, ASCII case ignored; first value wins (`IdentityFile`
    accumulates); `Match` blocks are skipped with a notice; `keyword=value`, double quotes and
    `#` comments are read as OpenSSH reads them.
  - `Include`: `~`, relative paths under `~/.ssh`, a `*` glob in the last segment only, files in
    lexical order, missing files ignored; an `Include` inside a `Host` block applies only to
    that block's hosts. At most 8 levels and 256 files per import (a glob that includes itself
    would otherwise read `files^8`), then `ConfigNotice::IncludeTooDeep`. Only regular files are
    opened, at most 16 MiB each, so `/dev/zero` or a FIFO cannot hang the import; a symlink
    outside the home directory is followed and read, never executed.
  - Tokens `%d %h %p %r %u %%` expand in `IdentityFile` and `HostName` (`%h` is the alias
    there; Q43); any other token makes the entry unusable (`UnsupportedToken`). A relative
    `IdentityFile` is taken under the home directory (an app has no shell working directory).
  - A host's `user` is OpenSSH's effective user: its `User`, else the local user, so `%r` and
    every hop are complete.
  - `StrictHostKeyChecking`: `yes` → Strict, `accept-new` → AcceptNew, `ask` → Ask, `no`/`off`
    → Ask with `StrictNoMappedToAsk`. Nothing maps to "accept all".
  - `UserKnownHostsFile` and `IdentityAgent` (the 1Password and Secretive agents) are noted,
    not imported (`UserKnownHostsIgnored`, `IdentityAgentIgnored`; Q43).
  - **Secret scan.** Every imported value passes the caller's `is_secret`
    (`hacivat_driver::url::carries_secret`). A hit makes the entry `SecretInValue`, and the
    value is not kept anywhere in `SshConfig` (its `Debug` never shows it); a secret-looking
    `Host` pattern is dropped.
  - **Jump chains.** `ProxyJump j1,j2` on `a` gives `[j1, j2, a]`, each hop resolved through its
    own blocks; `[user@]host[:port]` and `ssh://` URIs are read. As with `ssh -J`, only the
    first jump follows its own `ProxyJump`. A jump host that no `Host` line declares is listed
    too, resolved through the wildcard blocks. A cycle, a recursion over 8 levels or a chain
    over 9 hops (`MAX_SSH_HOP` + 1) is `ConfigIssue::IncludeTooDeep`; an unusable hop makes the
    whole chain unusable. A hop authenticates with its first `IdentityFile`, else the agent.
  - **Host-key policy of a chain (fix round 1, I1).** OpenSSH checks each hop with that hop's
    own `StrictHostKeyChecking`; `SshSpec` carries one policy for the chain, so the import takes
    the **strictest** over every hop: Strict > Ask > AcceptNew, and a hop without the setting
    counts as Ask (OpenSSH's default). A `yes` bastion behind `Host *` `accept-new` therefore
    keeps the chain Strict, and `AcceptNew` holds only when every hop says `accept-new`
    (`a_strict_bastion_is_not_weakened_by_a_wildcard_accept_new`). Rejected: the final alias's
    policy for every hop (it silently weakened a stricter jump host).
  - **A password in a `ProxyJump` URI** (`user:password@host`, which `carries_secret` does not
    flag) is stripped before the value is stored: only `user@host` reaches `proxy_jump` and the
    hops, and `ConfigNotice::ProxyJumpPasswordIgnored { alias }` tells the user (fix round 1).
    The password starts at the first `:` of the user part, literal or percent-encoded (`%3A`,
    since OpenSSH URL-decodes the user of an `ssh://` URI), and the `ssh://` scheme is matched in
    any ASCII case (round 2).
  - An empty or unknown `StrictHostKeyChecking` value (an error to OpenSSH) counts as Ask and
    takes the first-value slot, so a later, weaker `accept-new` cannot win (round 2).
  - Fix round 1 also: a keyword without a value is ignored (OpenSSH rejects it); an alias
    declared in a file included under a `Host` whose patterns it does not match is not listed
    (it could never get that file's settings); resolving the import is linear in practice
    (blocks whose patterns are all literal are indexed by name, so only wildcard blocks are
    checked for every alias; 20,000 hosts import in well under a second).
  - `known_hosts` reads (fix round 1): every file is opened only after a regular-file check on
    the path and checked again on the open handle; a FIFO at the store path makes `trust` fail
    (`InvalidInput`) instead of blocking under the mutex. A file over 64 MiB is cut there and
    the cut counts as one skipped line in `skipped_lines` (its partial last line is dropped).

## m2b-t06 — vault

- **Choice (B13, D-016 option A, Q30).** Where no OS keyring is usable (in practice Linux
  without Secret Service), `Forever` secrets live in `<data>/vault.bin`: one
  XChaCha20-Poly1305 blob over a JSON map `{ "<service>": "<secret>" }`, under a 32-byte
  Argon2id key (v1.3; default `m = 65536 KiB`, `t = 3`, `p = 1`) derived from a master password
  of at least 8 characters. The 64-byte header (`HVVAULT1`, `format_major`/`format_minor` u16 LE,
  `m_kib`/`t`/`p` u32 LE, 16-byte salt, 24-byte nonce) is the associated data, so no header
  byte (a downgraded cost, a version) can change without the unlock failing. Salt and nonce
  come from `getrandom::fill`; every write draws a fresh 192-bit nonce and keeps the salt, so
  the key stays the same and nonce reuse is a random collision only. A forgotten master
  password is unrecoverable: `reset` deletes the file.
- **Rejected.** AES-GCM (a 96-bit nonce is too short for random nonces per write);
  scrypt/PBKDF2 (Argon2id is the current recommendation and already RustCrypto); a per-entry
  file layout (it leaks how many secrets and which profiles); storing the service names in
  clear for lookups (they name profiles and hops, so they stay inside the ciphertext).
- **Fails closed.**
  - The header is unauthenticated until the key exists, so its parameters are bounded before
    any derivation: `m_kib` 8,192–1,048,576, `t` 1–10, `p` 1–8, else `VaultCorrupt` with no
    key derived (a crafted `m_kib` would otherwise abort the process on allocation). Only a
    vault built `with_params(p)` (tests) also accepts a header equal to `p`.
  - A header that does not parse, a truncated file, a bad magic or a symlink/non-file at the
    path is `VaultCorrupt`; a body that fails the AEAD under a header that parses is
    `WrongMasterPassword` (the AEAD cannot tell tampering from a wrong password). Nothing is
    written on any unlock failure, and a locked vault refuses `put` and `delete`.
  - A newer `format_major` is `VaultReadOnly`: `get` works, `put` and `delete` refuse.
  - Writes are atomic: `vault.bin.tmp` is created fresh (`create_new`, mode `0600`; a stale or
    planted entry there is removed, never written through), written, `fsync`ed, renamed over
    `vault.bin`, then the directory is `fsync`ed on Unix. The temp file only ever holds
    ciphertext. A symlink or other non-file at `vault.bin` is refused, never followed; a read
    checks that the opened handle is the file that was checked (device and inode on Unix). A
    share or device path (`hacivat_net::is_network_path`'s rule, copied because
    `hacivat-secrets` cannot depend on `hacivat-net`) is refused before any file system call.
    A failed write changes nothing: the in-memory map is replaced only after the rename, and
    the error is `VaultReadOnly` (the file cannot be written).
  - **Forever-put errors never become hard errors (Q38).** `SecretStore::put(.., Forever)`
    turns `VaultLocked`, `VaultReadOnly` and `VaultCorrupt` into `Stored::FellBackTo {
    retention: UntilRestart, reason }` (the M2a toast), like `Unavailable`, `Denied` and
    `TooLarge`. A vault that does not exist yet is "locked" for `put` (the UI offers Create)
    and holds nothing for `get` and `delete` (`Ok(None)`, `Ok(())`), so the pipeline prompts
    for the password instead of asking to unlock a vault that is not there.
- **Secrets in memory.** The key is a `Zeroizing<[u8; 32]>`; the map holds `SecretString`
  values. The JSON is written by hand into a `Zeroizing<Vec<u8>>` sized exactly up front (plus
  the tag), encrypted in place, and never reallocated while it holds plaintext; the body is
  decrypted in place in a `Zeroizing<Vec<u8>>` and parsed straight into `SecretString` with
  secrecy's `serde` feature (no plain `String` value). `lock` drops both, which zeroizes them.
  Argon2's working memory is supplied by the vault and overwritten (`black_box`ed) before it
  is freed, because `argon2` is built without its `zeroize` feature (Task 1 owns the
  manifest).
  - **Residual, accepted:** a secret containing `"`, `\` or a control character is unescaped
    by serde_json through its internal scratch buffer, which is freed without zeroizing; and
    Argon2's initial Blake2b state lives briefly on the stack. Neither reaches a file, a log or
    `Debug`.
- **Timing.** The unlock cost depends only on the header parameters (Argon2id), the Poly1305
  tag compare is constant-time, and the JSON is parsed only after the tag verified.
- **Fallback backend (Q23).** `FallbackBackend` probes the keyring once (`OnceLock`), on first
  use, without prompting (P36). Probe true routes everything to the keyring; probe false to
  the vault. `choice()`/`as_vault()` follow the probed choice, and `probe_available()` is true
  either way, so Forever stays enabled with the vault label.
- **pgpass (B27).** An empty password field is `None` (libpq sends no password), not
  `Some("")`; `PgPassMatch::password` is `Option<SecretString>`.
- **Tests.** `crates/core/hacivat-secrets/tests/vault.rs` (round trip, no plaintext and mode
  `0600`, header layout, wrong password, tamper, truncation, bad magic, the header as AAD, a
  downgraded cost, read-only newer major, crafted parameters, `open` bounds, short master,
  existing file, fresh nonces with a kept salt, reset, a planted symlink at `vault.bin` and at
  the temp path, a failed write, network paths, `Debug`), `tests/fallback.rs` (probe once,
  keyring or vault, the store's choice, Q38 fallbacks; `linux_without_secret_service_uses_vault`
  runs in CI `vault-linux`), `tests/pgpass.rs` (`empty_password_field_is_none`).
- **Fix round 1 (review I1, I2, M1–M8; controller ruling: both Importants now, not at M8).**
  - **Several instances (I2).** Until M8's single-instance lock, two Hacivat processes may
    share `vault.bin`. Every unlock, `get`, read-modify-write (`put`, `delete`), `create` and
    `reset` holds an exclusive `std::fs::File::lock` on `vault.bin.lock` (mode `0600`, never
    deleted, a symlink there is not used). Under it the instance re-reads the header: a changed
    nonce reloads the map with the held key before the change is applied (no lost update); a
    changed salt or parameters, or a missing file (reset or re-created elsewhere), locks the
    instance (`VaultLocked`), so the old vault is never written back; a file the held key cannot
    open locks it with `VaultCorrupt`. A write without the lock (a read-only directory) is
    refused (`VaultReadOnly`); a read without it goes ahead. Rejected: deferring to M8 (a reset
    in one window would be undone by the next save in another).
  - **Temp files (I2).** Each write uses its own `vault.bin.<32 random hex>.tmp` (`create_new`,
    `0600`); temp files a crash left behind are removed under the lock, which no write in flight
    can hold. No more collisions between writers.
  - **Racing replace (I1).** The read is check (`lstat`) → open → compare dev/inode; a mismatch
    or a vanished file is a replace in progress and the read is retried (16 times), then
    `Backend("vault-read")`, never `VaultCorrupt`. Writers take the lock, so in practice only a
    writer outside Hacivat can cause a retry.
  - **A symlink at `vault.bin` on a write** is now `VaultCorrupt` (the write re-reads the file
    first and a link is not a vault file) instead of `VaultReadOnly`; it is still never
    followed and nothing is written.
  - **M1** `Debug` uses `try_lock` and shows `unlocked: "busy"` rather than wait. **M2** the key
    is one heap `Box<Zeroizing<[u8; 32]>>`, so moves copy only the pointer. **M3** `reset`
    `fsync`s the directory. **M4** only "not found" (or "not a directory") means no vault; any
    other failure to look is `Denied`/`Backend`, so a locked vault in an unreadable directory
    is never reported empty or deleted. **M7** Argon2's memory is reserved with
    `try_reserve_exact`: a reservation the machine cannot make is `Backend("vault-memory")`, not
    an abort (the Q30 bounds stay as ruled). **M8** `unlock` holds the state lock and the file
    lock from the read to the install. The `argon2` `zeroize` feature is on (controller-ruled
    manifest exception) and the working blocks are still wiped here, since argon2 does not wipe
    caller-supplied memory.
  - **Recorded, not changed.** **M5:** off Unix the symlink refusal is check-then-use only
    (`File::open` follows a reparse point swapped in after the check); accepted because the vault
    is used only where no keyring exists, in practice Linux. **M6:** no KDF upgrade path in v1:
    a vault keeps the parameters it was created with; a later default does not re-key existing
    vaults (a future format minor can add re-keying on unlock).

## m2b-t12 — profiles and paths

- **Pins in `connections.toml` fail closed.** `tls.pins = [{ endpoint, sha256 }]`, the hash
  written as 64 lower-case hex digits and read in either case (ASCII hex only, no prefix or
  separators). A pin with the wrong length, a non-hex digit or no endpoint makes the whole entry
  invalid (`InvalidEntry`, kept on disk untouched): the profile does not load rather than load
  without its pin, which would trust the chain again and allow the plaintext fallback the pin
  forbids (Q13). *Rejected:* dropping the bad pin with a `ValueIgnored` notice (the next save
  would erase it from disk). The same holds for a malformed `[ssh]` table.
- **No secret in `[ssh]`.** The SSH table holds no secret by design (passwords and passphrases
  live in the secret store per hop, under `ssh_retention`, Q23). Inside it, a key matching the
  shared rule (`is_secret_param`) or containing `secret` is dropped on load
  (`SecretKeyIgnored`, never echoed) and never written back.
- **Approved roots on canonical forms (B26, Q22).** For an untrusted origin, every TLS file,
  the SQLite file and every SSH key path (the hop's key file and imported `IdentityFile`s) must
  lie under an approved root. The path and every root are canonicalised (symlinks resolved), a
  Windows `\\?\C:\` is read as `C:\`, and a missing file is resolved through its parent folder.
  Refused: a path that cannot be canonicalised (relative, a missing folder), a dangling symlink
  (SQLite would create the file wherever it points), and a canonical form on a share, a device
  or a verbatim UNC path. Share and device paths are refused on the typed text first, before
  any file system call (M2a N2). A TLS file outside the roots is not read at all (a FIFO there
  would hang Validate).
- **Check-then-use window, accepted.** Validate checks paths; it does not hand the driver or the
  tunnel an open file. They open the file again by path, so a symlink swapped in between the
  check and the open escapes the check. Closing it would mean opening every file once in
  Validate and passing descriptors down to rustls, SQLite and russh, which none of them take.
  The attacker would need write access to a folder inside the user's approved roots (their own
  home), which already lets them replace the file itself; the check exists against a profile
  that names a file elsewhere, and that it stops.
- **SSH stays refused until the SSH step exists.** Validate still gives an SSH tunnel
  `TransportUnavailable` (one marked line in `validate.rs`) besides its hop issues, because
  until Task 14's transport step the pipeline would reach the database directly. Task 14
  removes that line.
- **Fix round 1.** A hop's host takes the database host's rules (`HostInvalid` for
  credentials such as `user:pw@host`, whitespace, control characters or URL syntax) and port 0
  is `PortInvalid`, so removing that line never lets a malformed hop reach a resolver. `~` is
  expanded by one function, `hacivat_net::paths::local_path`, which both Validate's roots check
  and the TLS readers use (Task 14's tunnel must use it for key files), so the checked file is
  the opened file; it also refuses a share or device once `~` is expanded (a home folder on a
  share) before any file system call. Inside `[ssh]`, keys containing `token`, `otp` or
  `answer` are dropped as secrets too. A hop `key_file` next to another `auth` loads with
  `ValueIgnored` before the next save drops it.

## m2b-t09 — SSH tunnels

- **Host keys are checked inside the key exchange, before any authentication byte.** Every hop's
  russh handler delegates `check_server_key` to the policy; the only `Ok(true)` follows a passed
  check, and `connect` also refuses a handshake whose handler never recorded one
  (`host_key_is_checked_before_any_auth`, `jump_hop_host_key_is_checked_before_its_auth`: the
  server counts zero authentication requests).
  - `Known` passes. `Changed` and `Revoked` refuse in every policy, with or without a prompter
    (`changed_key_is_never_accepted_in_any_policy`, `revoked_key_is_refused_in_every_policy`).
    `Unknown`: `Ask` with a prompter asks, `true` stores the key; `Ask` headless and `Strict`
    refuse (`ssh.host_key_unknown`); `AcceptNew` stores without a question.
  - `AcceptNew` is only as safe as the stored key, so a failed write refuses the connect. A key
    the user accepted is used even when the write fails (the next connect asks again). `trust`
    refusing (a revoked key, or a changed one appearing between check and write) maps back to
    `Revoked` or `Changed`.
  - **Other key types (T4 carry-in).** Before each hop, the stores are asked which key types
    they list for `host:port` (a probe key per type; `Changed` means "a key of this type is
    listed"). When any is listed, only those types' host-key algorithms are offered. A server
    without them fails the negotiation, and the hop is retried once offering every type so the
    presented key can be shown. An `Unknown` key for a host listed under another type is
    reported as `HostKeyChanged` (old: the listed key), never as new, so `AcceptNew` cannot
    accept and store it (`key_of_another_type_than_the_known_one_is_changed_not_new`). Types
    russh cannot negotiate (`ssh-dss`, `sk-*`) count as listed too: such a host can only be
    changed, never new. The "listed under another type" answer comes from one more probe whose
    type does not decode, which `check` answers with any listed key of any type (fix round 1,
    M1), so a type outside the eight probed (`ssh-xmss@openssh.com`) counts as well
    (`a_host_listed_under_an_unprobed_type_is_changed_not_new`); the eight-type probe only
    restricts the negotiation.
  - Certificate host-key algorithms are never offered (`host_key_certificates` empty); a
    certificate presented anyway is refused (`Protocol`). SHA-1 `ssh-rsa` signatures are not
    negotiated (OpenSSH ≥ 8.8 default); a server that only signs with SHA-1 fails to connect.
  - `HostKey` is built from the key's own OpenSSH form (`PublicKey::to_openssh` →
    `HostKey::from_openssh`): RSA keys are `ssh-rsa`, never `rsa-sha2-512` (T4 fix-round
    carry-in; `rsa_host_keys_are_named_by_their_key_type`).
  - russh only calls `check_server_key` at the initial exchange; a re-key runs inside the
    encrypted session it authenticated.
- **Authentication order (B11).** Per hop: agent, then key files (the hop's own; the bastion's
  for a jump host; the imported `IdentityFile`s, deduplicated), then password, then
  keyboard-interactive. `IdentitiesOnly` moves the key files before the agent
  (`identities_only_tries_the_key_file_first`) and, as in OpenSSH, the agent then offers only
  the keys of the configured identities (the public key read from the identity file, which an
  OpenSSH key carries unencrypted, or from `<file>.pub`), so unrelated agent keys never spend
  MaxAuthTries (fix round 1, M6; `identities_only_offers_only_the_configured_agent_key`). A hop whose method is a password tries password
  then keyboard-interactive (servers that take passwords through PAM offer only the latter); a
  keyboard-interactive hop tries only that. A method the server stopped offering is skipped,
  and a closed session ends the hop as `ssh.auth` (MaxAuthTries:
  `many_agent_keys_without_identities_only_reports_auth_with_hop`).
  - Secrets: a budget of three attempts per key and per password, the stored secret first and
    then `prompter.secret` (attempts numbered from 1, as the pipeline's password prompt).
    Keyboard-interactive: three logins of at most eight rounds each, prompts forwarded verbatim
    (`KbdRequest.prompts[i].text`, `echo`). A prompter answering `None` is `SshError::Cancelled`.
  - Agent: `SSH_AUTH_SOCK` on Unix, `\\.\pipe\openssh-ssh-agent` on Windows, read from the
    environment when the manager is built (`AgentEndpoint::from_env`); never from a profile. Tests
    inject one (`TunnelManager::with_agent`). An `Agent` hop with no reachable agent and no
    other method that succeeds is `AgentUnavailable`. Agent certificates are not offered (v1).
- **Secrets in memory.** Key files are read into `Zeroizing` buffers (at most 1 MiB, regular
  files only) and decrypted on the blocking pool; passphrases and passwords stay
  `SecretString` until the call into russh. **Accepted gap:** russh's API takes the password
  and keyboard-interactive answers as `String` (`authenticate_password(.., impl Into<String>)`,
  `authenticate_keyboard_interactive_respond(Vec<String>)`), so one copy per attempt lives in
  russh's buffers unzeroized until freed. The decrypted `PrivateKey` (ssh-key) zeroizes on drop.
  russh's `Debug` for auth methods redacts the password, and nothing of Hacivat's logs; the
  test captures every `log` record at TRACE (`ssh_secrets_never_logged`).
  - **Session overlay (Q17; fix round 1, I3).** `TunnelLease::take_secrets()` hands back, once
    and only on success, the password the server accepted and the passphrase that decrypted the
    accepted key, per hop (stored or typed), shaped like `TunnelRequest.secrets` so a reconnect
    passes them straight back; a rejected or unused secret is never reported, a failed acquire
    drops (zeroizes) what it collected, and a lease that joined an open chain gets none
    (`the_password_that_authenticated_is_handed_back_once`, `secrets_are_handed_back_by_hop`).
    **Keyboard-interactive answers are deliberately excluded** (controller ruling): they are
    often one-time codes, and replaying one on a reconnect is both useless and a
    security-relevant reuse; a reconnect through such a hop prompts again
    (`joined_leases_and_keyboard_answers_hand_back_nothing`). `HopSecrets` and the lease keep a
    redacted `Debug`.
- **Time (P23, Q44).** `connect_timeout` bounds each hop's TCP connect, handshake and
  authentication on a clock that stops while a host-key, secret or keyboard prompt is open
  (`prompt_time_never_counts_toward_the_connect_timeout`, `slow_server_times_out_with_the_hop`).
  Prompts are per acquire: the acquire's child token ends them on cancel or when the acquire
  future is dropped, so a prompter that never answers holds only its own handshake
  (`cancel_during_host_key_prompt_releases_everything`).
- **Sharing (`ChainKey`).** SHA-256 over a domain tag, the policy, and per hop the lower-cased
  host, port, user, auth method (and key path), `IdentitiesOnly` and the identity files, each
  length-prefixed, and the profile's `keepalive_s` (fix round 1, M4: profiles that differ only
  in keep-alive keep their own interval; `keepalive_is_part_of_the_chain_key`). Secrets are not
  part of it: two profiles with the same bastion, user and
  method share one session. A shared chain is reused only while alive; `share = false` (test
  connection, CONN-07) never reuses and is never reused. Two concurrent acquires of one chain
  may both connect; the second to finish joins the first and closes its own.
- **Release.** A lease's drop decrements its chain (saturating; a drop runs once). At zero users
  a timer closes the chain after `idle_grace` unless a lease came back meanwhile (an epoch
  counter guards stale timers; `reacquire_within_grace_reuses_the_session`). An unshared chain
  closes at once. Closing disconnects hops last first, waiting (at most 2 s each) for each
  session to end; a chain that fails part-way closes the hops already up the same way before
  `acquire` returns (`partial_chain_failure_releases_in_reverse`). Without a runtime (a drop
  outside tokio), the handles are dropped instead and each session ends on its own, inner hops
  first.
  - **Kill switch (fix round 1, I1).** russh does not read its handle channel while a key
    exchange runs, so dropping the `connect_stream` future or the `Handle` of a hop that stalls
    mid-exchange (a silent bastion or middlebox, a re-key that never finishes) would leave its
    task, socket and `sessions_open` count forever. Every hop's transport therefore sits behind
    a kill switch: once tripped it reads end of file and refuses writes, so the session task
    ends and drops the socket. A guard trips it unless the handshake returned, the host key was
    verified, the hop authenticated and it joined its chain, which covers a timeout, a cancel,
    a dropped acquire and a refusal; closing (and dropping) a chain trips every hop's switch
    after its disconnect (`a_key_exchange_that_stalls_is_released_on_timeout`,
    `a_key_exchange_that_stalls_is_released_on_cancel`,
    `a_stalled_inner_hop_releases_the_hops_before_it`).
- **Death and generations (B20).** Keep-alives run per hop (`keepalive_s` from the profile,
  else `TunnelConfig.keepalive`; `keepalive_max`). When any hop's session ends while the chain
  is not closing (a cut, keep-alive exhaustion), the chain leaves the registry, the rest of it
  closes, and `TunnelEvent::Dead { key, generation }` is sent once; the next acquire builds
  generation + 1 (`keepalive_failure_emits_dead_once_with_generation`). A release is not a
  death. A chain is marked established before its last death check (fix round 1, M2), so a
  session ending at hand-over either fails the acquire or sends `Dead`, never neither.
- **Channel hygiene (Q46).** The client only opens `direct-tcpip` channels, originator
  `127.0.0.1:0` (`direct_tcpip_originator_is_loopback_zero`); every server-initiated channel
  type is refused (`server_initiated_channels_are_refused`); no pty, shell, exec, X11 or agent
  forwarding is ever requested. The database host goes into the last `direct-tcpip` request and
  is resolved by the bastion (`database_host_is_resolved_by_the_bastion`). A channel `open`
  runs to its end on its own task, without a timeout of its own (russh fails a pending open
  when the session ends); the timeout bounds only the caller's wait. When the caller gives up
  first (its timeout, a cancel, a dropped future), a channel the server confirms later is
  closed at once (fix round 1, I2; `a_channel_confirmed_after_the_timeout_is_closed`). A refused channel is `SshError::ChannelOpen { hop, reason }` inside the
  `io::Error` (`classify_channel_err`); `reason` is a stable id, never the server's text.
- **Logging is the caller's.** `hacivat-net` has no logger: the lease reports the keys it
  stored (`TunnelLease::trusted_keys`) and `KnownHosts::skipped_lines` stays on the store; the
  pipeline (Task 14) logs both.
- **testkit.** `TestSshServer` is russh's server on loopback (IPv4 or IPv6) with a fresh
  Ed25519 key, OpenSSH-like `MaxAuthTries` (key probes count) and every method kept on offer
  after a failure; `forward_delay` confirms `direct-tcpip` opens late and `channels_live`
  counts the relays still open. `TestAgent` speaks the agent protocol's identity and sign requests itself:
  russh's agent server takes a `futures::Stream` and the testkit has no `futures` dependency
  (Task 1 owns the manifests). Ed25519 and ECDSA keys sign; RSA keys refuse.

## m2b-t10 — PostgreSQL insecure-auth guard, channel binding and cancel transport

- **The insecure-auth guard is enforced on the wire (B8, Q20).** The PostgreSQL driver refuses
  `AuthenticationCleartextPassword` and `AuthenticationMD5Password` before `tokio-postgres`
  can answer them, unless `InsecureAuth::Allow` or the handshake's recorded outcome is `Full`,
  `CaOnly` or `Pinned`. It is outcome-based: Require with a pin is allowed, Require without one
  is not. Details, including why the refused bytes never reach the library, are in ADR 0002
  `m2b-t10`.
- **Channel binding (B9).** SCRAM binds to `tls-server-end-point` when the server offers
  `SCRAM-SHA-256-PLUS`, so a man in the middle that terminates TLS with another certificate
  cannot relay a SCRAM exchange. Without a binding hash (Ed25519 leaves) SCRAM runs unbound,
  as libpq does with `channel_binding=prefer`.
- **Cancel keys stay encrypted (B23).** A cancel request travels over the session's own
  transport with its TLS configuration; a TLS session's cancel is never sent in plaintext,
  even when the server answers its `SSLRequest` with `N`.

## m2b-t14 — pipeline policies

- **The insecure-auth policy (B8, Q20).** `insecure_auth_for(profile, class, mode)` is `Allow`
  for a loopback address or a Unix socket, for `Disable`, and under `Auto` for a profile that
  remembered the public-plaintext answer (`tls.allow_public_plaintext`); otherwise
  `RefuseUnlessVerified`, and the drivers then allow an insecure password method only after a
  `Full`, `CaOnly` or `Pinned` handshake (outcome-based for both engines, never mode-based).
  The one-gesture consent (`Answer::ConnectPlaintext`, or the session overlay's) sends `Allow`
  for the retry it answers. Through SSH the class is `ssh_address_class` of the database host,
  so a private name behind a bastion is still guarded (B11). Cost, as B8 records: an md5-only
  private server without TLS under `Auto` needs an explicit `Disable`. The Docker matrix's pg13
  (md5) connects on loopback, the allowed path; it is not a refusal case.
- **Questions never loop.** `prompts::ask_and_retry` retries the driver connect at most once
  per question kind (certificate, plaintext) per connect. A pin that still fails, or a second
  plaintext refusal, returns the error; a scripted UI that answers `Pin` forever gets one
  question.
- **The certificate question (B6).** Asked only for `tls.untrusted` with an unknown issuer and a
  chain that parses: a pin never extends a certificate's dates (B5), so an expired or
  not-yet-valid one is never offered a pin (the review is still attached). `Pin` adds
  `ProfileChange::AddPin` for `pin_endpoint(tls host, port)` and rebuilds the TLS parameters
  with it; `ImportCa(path)` adds `SetCaFile`, approves the path (`ApprovedRoots::add`) and
  rebuilds with it as the CA file. `ImportCa` is the file picker's answer (Task 19); the
  pipeline still refuses a relative path or a share/device path (`local_path`) before it
  opens, approves or records anything. `tls.hostname` is never offered a pin. The interim
  "Encrypt without verifying" (M2a R3) is offered by no class any more.
- **Pins are hard stops (B5, Q13, Q26).** `tls.pin_changed` is never asked; its error carries
  the review with both fingerprints. The pipeline never builds `ProfileChange::ReplacePin`
  (only `cert_dialog::review`, Task 19). A pinned endpoint never reaches plaintext: the TLS
  parameters keep `plaintext_allowed = false` even with `allow_public_plaintext`, a
  `tls.not_supported_by_server` there is never asked and offers only Edit connection, and
  neither the overlay's consent nor any answer applies. `Disable` with a pin only logs a warning
  (T3 ruling M2: Disable is the user's choice; the dialog shows the flag).
- **The public-plaintext question (B7).** Only under `Auto`, after a first attempt that stopped
  at the TLS step with no credential sent (P9). `ConnectPlaintext { remember }` retries with
  plaintext allowed and `InsecureAuth::Allow`, plus `AllowPublicPlaintext` with `remember`;
  `UseSsh` returns `[ConnectThroughSsh, EditConnection]`. The overlay's one-gesture consent
  (Q17) retries without asking, headless included.
- **`auth.insecure_method` carries the certificate when TLS was up (Q20)**, so "Review
  certificate" can pin and the next handshake is `Pinned`. Without a recorded chain (no TLS)
  the fixes narrow to Edit connection (Task 2 ruling 10).
- **One verifier record per handshake.** `TlsParams::fork` (hacivat-net) gives a copy empty
  `failure`, `presented` and `outcome` slots and a configuration whose recording verifier
  writes into them, with the same checks (mode verifier, pin, dates) and the same client
  certificate; it holds no secret. Every driver attempt and every PostgreSQL cancel uses a
  fork, so no handshake reads another's chain or outcome (T3 carry-in, T10 review M1).
- **SSH transport (B11, Q44).** The tunnel is acquired outside the connect clock: the hops'
  own timeouts bound the network and prompt time never counts. The database host is never
  resolved locally (Resolve is skipped; it goes into the `direct-tcpip` request) and is the TLS
  server name. `Endpoint::Network.direct` is `None`, so MySQL uses its forwarder. A Test never
  shares a tunnel (`share = false`). Hop key files are opened through
  `hacivat_net::paths::local_path` (the path Validate checked), and a private key file that
  the group or others can access is refused like OpenSSH's `sshkey_perm_ok` (the hop's own key
  fails the connect; an imported `IdentityFile` is skipped). Without `getuid` (no `unsafe`) the
  check applies to every file, not only the user's own: it can only refuse more. The trusted
  host keys (`TunnelLease::trusted_keys`) and `KnownHosts::skipped_lines` are logged
  (fingerprints and counts only).
- **Secrets order and storage (R8, Q17, Q23).** Per hop: the session overlay, then the dialog's
  draft (Test), then the store with `ssh_retention`; the database password: the draft, then the
  overlay, then the store. Store reads run on the blocking pool, outside the deadline, and only
  for the slots a hop can use. What the user typed (the database password, an SSH password or
  key passphrase at a hop prompt, a TLS key passphrase) is stored only after the whole connect
  succeeded, never by a Test or a probe, never when the connect was cancelled, and never a
  keyboard-interactive answer. A failed connect's typed SSH secret is dropped: a tunnel left in
  its idle grace authenticates the next connect without a prompt, so that secret is asked for
  (and stored) the next time a tunnel is opened.
- **The vault on the connect path (B13, Q33, Q38).** A locked vault is unlocked before a read
  (interactive, up to 3 master passwords, once per connect); cancelled, wrong three times or
  headless, the connect goes on as if nothing were stored (the password is then asked for, or a
  headless connect fails with `needs_interaction`). Before the first `Forever` secret is
  stored, a vault that does not exist yet is offered (`Question::CreateVault`, created on the
  blocking pool); declining keeps the secret until restart. A locked, read-only or damaged
  vault on a `Forever` put falls back to `UntilRestart` (Task 6), never a hard error.
- **What the overlay gets (Q17).** `LiveConnection.secrets_used` holds the database password
  of a password profile, the hop secrets that authenticated (`TunnelLease::take_secrets`; for a
  lease that joined an open chain, the chain's, since fix round 1) and the TLS key passphrase in
  use; `plaintext_consent` the consent; `profile_changes` every change (also on a later
  failure, in `ConnectError.profile_changes`). The overlay keeps them in memory only (Task 15).
- **Headless never asks.** An unknown host key under `Ask`, a password hop without a password,
  a keyboard-interactive hop, an untrusted pinnable certificate, the public-plaintext question
  and an encrypted TLS key without a passphrase all fail with `needs_interaction`, so a
  reconnect becomes Lost with Retry (Q17).
- **Validate and the TLS key passphrase (T3 carry-in).** The pipeline's Validate does not
  report an encrypted client key (its TLS step opens it); `validate_with_passphrase` gives the
  dialog the same check with its passphrase field.
- **Fix round 1 (review I1–I4, M1, M2, M6; controller rulings).**
  - **Consent never widens the guard outside its retry (I1).** A plaintext consent (answered,
    or carried by the session overlay) sends `InsecureAuth::Allow` only on the plaintext retry
    it answered: under `Auto`, without a pin, while the attempt's TLS parameters allow
    plaintext. The first attempt, Require, VerifyCa, VerifyFull and any pinned endpoint (the
    remembered `allow_public_plaintext` included) stay `RefuseUnlessVerified` unless the address
    is local or the mode is Disable.
  - **Consent is bound to the destination (I2).** The pipeline reads the consent only from the
    request the current overlay builds; Task 15's `update_profile` drops the overlay's consent
    and secrets when host, port, user, auth, transport or client certificate change, and the
    consent when the TLS mode changes or the remembered answer is unticked. The dialog clearing
    `allow_public_plaintext` on a host change is Task 19's.
  - **The plaintext backstop follows the request (I3).** After the driver connected, an
    unencrypted connection whose last request did not allow plaintext is refused
    (`tls.not_supported_by_server`; `[EditConnection]` on a pinned endpoint): Require and
    Verify*, a public host under Auto without consent, and a pinned endpoint at any address,
    through SSH too. It no longer re-derives the rule from the mode and the address class.
  - **A lease that joins an open chain gets its secrets (I4, ruling a).** `TunnelManager` keeps
    the secrets that authenticated a chain with the chain (zeroized when it drops, never in
    `Debug`), and `take_secrets` hands a copy to a lease that joins it. They reach
    `secrets_used`, so the overlay can serve a headless reconnect. Only a connect that typed a
    secret stores it (R8): a joining connect stores nothing, because the chain may have been
    opened by another profile with the same hops, users and methods, whose typed secret must
    not be written into this profile's slots. Residual (re-review N2): a lease of another
    profile that joins the same chain receives the chain's SSH secrets in its in-memory overlay
    (never stored). They can outlive the first profile's forget or disconnect, but they only
    ever go back to the same hops (same `ChainKey`), and the overlay drops them on any
    destination change.
  - **A cancelled connect stores nothing (re-review N1).** `remember_secrets` returns every key
    it wrote; when the connect turns out cancelled, all of them are removed (the database
    password, the hop secrets and the TLS key passphrase), not only the one in flight.
  - **No pin or CA for dates (M1).** An expired or not-yet-valid certificate offers only Edit
    connection and carries no review; an unknown issuer without a reviewable chain offers
    Import CA and Edit connection.
  - **Cancel during storing (M2).** The create-vault question closes unanswered on a cancel, an
    answer after a cancel creates nothing, and a cancel during `remember_secrets` fails the
    connect as cancelled after the puts undid themselves.
  - **The refused channel's hop (M6).** `ssh.channel_open` names the hop that refused, by its
    own host.

## m2b-t15 — session overlay

- **What is kept (Q17).** Each profile's `SessionManager` keeps one in-memory overlay while the
  profile is connected: the profile with every `ProfileChange` of its successful connects
  applied (a pin, a CA file, the remembered public-plaintext answer), the one-gesture plaintext
  consent, and the `SessionSecrets` that worked (database password, SSH hop passwords and
  passphrases, TLS key passphrase). The secrets live only in `SecretString` buffers, which
  zeroize on drop. They are taken out of the `LiveConnection` (`mem::take`) as the connect
  lands, so the session actor's connection does not hold a second copy.
- **Accepted cost.** A secret with retention `Never` (or a passphrase typed once) stays in
  memory for as long as the profile is connected, not only for the connect that used it. That
  is what lets a reconnect (Task 17), a pooled introspection session or a cancel's side
  connection connect headless without asking again.
- **Where it goes.** Every later connect of the same profile passes the overlay as
  `ConnectRequest { profile, overlay_secrets, plaintext_consent }`: sessions, pooled sessions,
  side connections (`Department::Cancel`, outside the budget, headless), and Task 17's
  reconnects, which read `SessionManager::overlay()`. Each connect gets its own copy, which
  zeroizes when the connect ends. Nothing else reads it: no file, no log, no event.
- **When it goes.** `close_all` (disconnect, `forget`, quit) drops it. It also cancels every
  connect still in flight and bumps an epoch: a connect that lands after `close_all` is closed
  and registers neither a session nor an overlay, so a secret cannot come back after
  disconnect through a late connect. It is never persisted.
- **`update_profile`** replaces the overlay's profile and keeps the consent and secrets, as the
  plan says, but only while the profile still names the same destination (id, engine,
  endpoint, user, auth provider, transport and client certificate). An edit that points the
  profile elsewhere drops the secrets and the consent, so a password typed for one server is
  never sent to another. The next connect then asks, or a headless reconnect stops with
  `needs_interaction`, as the plan's recovery rule already handles. The plaintext consent
  alone also goes when the TLS mode changes or "allow public plaintext" is unticked: the user
  took back what the consent answered (fix round 1, M3).
- **A connect in flight across an edit (fix round 1, I1).** A connect is absorbed only while
  the saved profile still names the destination it connected to; otherwise it is closed and
  refused, as after `close_all`, so another server's password, plaintext consent or pin and CA
  changes never reach the overlay. A new overlay always starts from the saved profile, never
  from a connect's snapshot, so a saved edit is never lost to a slow connect.
- **Q17 clarification (controller ruling).** The overlay applies to every later connect of a
  connected profile, pooled sessions included, not only to reconnects and side connections:
  pooled connects are headless and would otherwise fail on a `Never` password or a pin
  answered once. One-time codes never reach it (Task 9 excludes keyboard-interactive answers).
- **No side-connection cache in M2b (controller ruling).** The plan's "cached for 30 s" is
  dropped: each cancel or kill opens a fresh side connection through the overlay, and Task 7's
  actor closes it after its one statement (an unused MySQL prewarm too). The 500 ms prewarm
  (Q37) already hides the slow first connect. A cache would keep an idle authenticated
  connection open for no request; revisit in M3 if cancel latency through SSH needs it.
- **Debug.** `SessionOverlay` derives `Debug`, and every secret in it is a `SecretString`, which
  prints redacted. `SessionManager`'s `Debug` shows only whether an overlay exists. Neither
  type derives `PartialEq`.

## m2b-t19 — TLS UI: certificate review, pin changed, plaintext question

- **The safe answer is the default (Q34; PO question pending, default Cancel).** The
  certificate review opens with Cancel focused, in both variants. Enter acts on key down only
  (never held, never with a modifier) and its key up is suppressed, so the focused button's
  keyboard click never runs twice. In the untrusted variant Enter presses the focused button:
  a pin takes a deliberate Tab to "Pin this certificate". The pin-changed variant is a hard
  stop: Enter cancels whatever has the focus, and "Replace pin" runs only on its own click (the
  pointer, or Space on the focused button). The public-plaintext question opens on "Use an SSH
  tunnel…"; "Connect without encryption" is never the default.
- **Only an explicit click replaces a pin (Q26).** `cert_dialog::change_for` builds
  `ReplacePin { old: review.pinned, new: { review.endpoint, review.sha256 } }` only for the
  Replace pin choice, and that choice exists only in a review opened from an error (the
  "Review certificate" fix, or the dialog's test strip): a pipeline question never offers it.
  Applying a change (`cert_dialog::apply_change`, used by `ConnectionsModel::apply_changes` and
  by the dialog's draft) is guarded the same way: `AddPin` never replaces an endpoint's pin,
  and `ReplacePin` replaces it only while it still is `old`, the fingerprint the user compared.
  A stale review, or two reviews of one profile, therefore change nothing. Changes apply to
  the profile the review was opened for, by ID, and to nothing else.
- **Dates (B5).** When today is outside the reviewed certificate's validity window (the wall
  clock), the review offers neither Pin, Replace pin nor Import CA, and says why. The error's
  own fix list wins over the registry's (`ConnectError::fixes`, Task 14's narrowing), so an
  expired certificate's error offers only Edit connection, and "Review certificate…" needs a
  certificate to review.
- **What is shown.** The full SHA-256 (32 bytes, two mono lines of 16) in every review; Copy
  writes exactly those 32 colon-separated bytes. Certificate names are RFC 4514 strings with
  controls and bidi marks already escaped (Task 3); they are drawn verbatim in mono, never inside
  a translated sentence. The TLS section's "Pinned certificate" row abbreviates the fingerprint
  with an ellipsis (its first four and last two bytes, the mock's form); it identifies the pin,
  the review is where fingerprints are compared.
- **A Test writes nothing outside the dialog (T14 review M3).** `TestReport` carries the test's
  `profile_changes`; the dialog applies them to its draft only, and only while the draft still
  names the tested engine and endpoint. A Test connects with a copy of the approved roots, so
  the CA file its certificate question imports is approved for that test only; the dialog
  approves it app-wide when the profile that names it is saved. A file picked from the error
  dialog's "Import CA…" (a saved profile, connected again at once) or from the dialog's own
  pickers is approved when picked (T16 ruling A).
- **The remembered plaintext consent is bound to an endpoint (T14 review I2).** The dialog
  drops `tls.allow_public_plaintext` as soon as the host or the port changes, whatever changed
  them (typing, a paste, an engine switch); a Test's "Don't ask again" binds it to the tested
  endpoint.
- **Disable with a pin (T3 ruling M2).** `IssueCode::TlsDisableIgnoresPin` is a notice the TLS
  section shows; Validate never reports it, so it neither blocks a save nor a connect.
- **Key passphrase (B25).** The client certificate's key passphrase is a masked field beside
  the Remember select, stored like the password under the profile's retention; Save validates
  the key with it (`validate_with_passphrase`), so an encrypted key with the right passphrase is
  no issue.
