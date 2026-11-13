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
