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
