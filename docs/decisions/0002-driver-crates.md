# ADR 0002 — Database driver crates for M2a

- Status: Accepted
- Date: 2026-11-12
- Scope: the M2a engines (PostgreSQL, MySQL and MariaDB, SQLite) and the libraries behind them

## Context

M2a connects to PostgreSQL 14+, MySQL 8.0+, MariaDB 10.6+ and SQLite through native Rust
drivers compiled into the app (HV-DRV-01). Each engine lives in its own core crate
(`hacivat-driver-pg`, `hacivat-driver-mysql`, `hacivat-driver-sqlite`) behind the `Driver` and
`RawConn` traits of `hacivat-driver`. The driver spike (`docs/superpowers/plans/2026-11-12-m2a-connect.md`,
Tech Stack) chose:

- `tokio-postgres` with `connect_raw` over an injected stream, TLS through `tokio-rustls`;
- `mysql_async` with rustls on the ring provider, reached through the loopback forwarder (R18)
  because it cannot take an injected stream;
- `rusqlite` with the bundled SQLite and `column_metadata`.

Workspace-wide rules from Task 1: rustls uses the ring provider only (`aws-lc-rs` and
`aws-lc-sys` are banned in `deny.toml`), and `webpki-roots` (CDLA-Permissive-2.0, pulled by
`mysql_async`'s rustls TLS) has a scoped licence exception.

## Rule

Each driver task records its crate here in one section headed `## m2a-tNN — <crate>`
(append-only, P41): the library and version, its licence tree, how TLS and trust work, where
secrets are copied into library-owned buffers (P49), and known defects.

## m2a-t13 — hacivat-driver-pg

- **Library:** `tokio-postgres` 0.7.18 (with `postgres-protocol` 0.6.12 and `postgres-types`
  0.2.14), `tokio-rustls` 0.26 and `rustls` 0.23 on the ring provider. `tokio-postgres-rustls`
  is not used: the connector is our own, so it runs on the caller's `TlsParams` and records
  failures.
- **Stream injection: yes.** Every connection runs `Config::connect_raw(stream, tls)` over a
  stream from the request's `StreamFactory`. `connect_raw` reads only the user, password,
  database, `options` and `application_name` of the `Config`, never a host or port, so direct
  TCP, Unix sockets and tunnels are the same code path. The stream is wrapped in a named type
  (`Injected`) because a bare `Box<dyn AsyncStream>` in the generic call fails the compiler's
  higher-ranked lifetime check for a `Send` future.
- **TLS: our own `TlsConnect`.** It wraps the stream with `tokio_rustls::TlsConnector` over
  `TlsParams.config` and `TlsParams.server_name`, reports `TlsStarted` and `TlsDone`, and
  turns a failed handshake into a `TlsFailure`: the verifier's recorded failure first, else
  `TlsFailure::from_rustls` on the rustls error inside the I/O error (alerts such as
  `certificate_required`). The result is written into `TlsParams::failure` when the verifier
  left it empty. The driver clears `failure` before each attempt, so a reused `TlsParams`
  never classifies a new attempt with an old failure. Channel binding is not offered, so
  SCRAM runs as `SCRAM-SHA-256` (not `-PLUS`), as libpq does when it cannot bind.
- **sslmode (P9).** `Disable` runs without an `SSLRequest`. `Auto` is `Prefer` only when
  `TlsParams.plaintext_allowed` (loopback or private address); toward a public address it is
  `Require`, so the password never crosses the network in plaintext. `Require`, `VerifyCa`
  and `VerifyFull` are `Require`; verification lives in the rustls configuration. The
  library's "server does not support TLS" (its own constant, not server text) is
  `RawErrorKind::TlsNotOffered`, which Task 10 classifies as `tls.not_supported_by_server`.
  A handshake that fails under `Prefer` is an error: there is no silent fallback to
  plaintext after a failed verification.
- **Startup options.** `application_name` from the request; the sanitized extras `options`
  (verbatim, minus a dangling trailing backslash) and `extra_float_digits` (as
  `-c extra_float_digits=<v>`, escaped); then `-c default_transaction_read_only=on` last when
  the profile is read-only, so no earlier `-c` can turn it off. libpq's client-side keys
  (`sslmode`, `sslrootcert`, `connect_timeout`, `target_session_attrs`, ...) are the
  pipeline's; `tokio-postgres` always sends `client_encoding=UTF8`.
- **Detection and probes.** `SHOW server_version_num` gives the version numbers
  (`170011` is 17.11, `90624` is 9.6.24); `SELECT version()` is the raw text, so a
  pre-release (`17beta1`, `18devel`, `16rc1`) is `NotTested`. `pg_is_in_recovery()` gives the
  replica flag and the `Replica` probe warning. `tls_summary` reads `pg_stat_ssl` for
  `pg_backend_pid()`, with the local handshake as the fallback. Parameterless probes use the
  simple-query protocol; `ping` is the empty query.
- **Errors.** A server error keeps SQLSTATE, severity (verbatim, possibly localized), text,
  detail and hint. When the localized severity (`S`) differs from the never-localized `V`
  field, `lang_hint` is `und`, so no English-text rule reads a translated message. TLS
  failures are `Tls(TlsFailure)`, I/O keeps its `io::ErrorKind`, a malformed message is
  `Protocol`, and the connect timeout is `Timeout`. A server that asks for a password when
  none was given surfaces as `Other` (`tokio-postgres` reports it as a configuration error).
- **Secrets (P49).** The password is copied into `tokio_postgres::Config` (as bytes) only at
  connect time, by `config_for`, and the `Config` is dropped right after the handshake.
  `Config`'s own `Debug` redacts the password; Hacivat never formats it. `tokio-postgres`
  keeps no other copy after authentication.
- **Cancel (M2b).** `cancel_handle()` is `None` in M2a. Cancel is feasible over the same
  factory: `CancelToken::cancel_query_raw(stream, tls)` takes any stream, so M2b can open a
  fresh stream from the factory (through the same tunnel) and send the CancelRequest on it.
- **Licences.** The crate's normal dependency tree (about 270 entries) is MIT, Apache-2.0,
  their dual forms, Unicode-3.0, ISC (`rustls-webpki`, `untrusted`), BSD-3-Clause
  (`subtle`), Unlicense OR MIT (`memchr`, `aho-corasick`, `byteorder`), Apache-2.0 OR
  BSL-1.0 OR MIT (`whoami`) and `ring`'s Apache-2.0 AND ISC. All of these are in the
  `deny.toml` allow-list. Neither `webpki-roots` nor `aws-lc-rs` is in the tree.
- **Known limits.**
  - A self-signed certificate that has `CA:TRUE` (the default of a bare
    `openssl req -x509`, and the pg17-tls fixture) is rejected by webpki as
    `CaUsedAsEndEntity` before any issuer check. That covers `VerifyCa` everywhere and
    `Auto`/`VerifyFull` off Apple. It surfaces as `TlsFailure::Other`, not `UnknownIssuer`.
  - On macOS the platform verifier rejects a server certificate valid longer than Apple's
    maximum with OSStatus -67901 before the trust check. `hacivat-net` maps -67901 and -67902
    to `UnknownIssuer`, like the other policy failures.
