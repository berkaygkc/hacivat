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

## m2a-t14 — hacivat-driver-mysql

- **Library:** `mysql_async` 0.37.1 (`mysql_common` 0.37.3), `default-features = false` with
  `default-rustls-ring`: rustls 0.23.46 on the ring provider, TLS 1.2 enabled. One driver
  serves MySQL and MariaDB; MariaDB reaches it through the catalog fallback.
- **Stream injection: no (P10).** `Conn::new(opts)` always dials itself: a TCP connect, or a
  Unix socket when `Opts::socket` is set. Its `Endpoint` holds a `TcpStream`, and neither
  `Stream` nor `Endpoint` is public. `DriverCaps::stream_injection` is `false`.
  - **Direct TCP** (`direct: Some(Target::Tcp)`): `mysql_async`'s own TCP, with
    `ip_or_hostname(tls_host)`, `tcp_port(port)`, `resolved_ips(<the pipeline's resolved
    addresses>)` and `prefer_socket(false)`. The name TLS verifies is the real host name.
  - **Every other endpoint** (`direct: None` for a transport wrap, P13, and a Unix-socket
    target): a fresh `LoopbackForwarder` in `ForwarderMode::Tcp` (`127.0.0.1:0`,
    `max_accepts = 1`) per attempt. The driver dials `127.0.0.1:<port>`, and TLS checks the
    real host name through `SslOpts::with_danger_tls_hostname_override(tls_host)`. TLS stays
    end to end; the forwarder only moves bytes. The connection owns the forwarder's guard and
    runs it on `close`; a drop also stops it.
  - **Never a Unix socket.** `make_secure` returns early on a socket ("won't secure socket
    connection"), so Require and VerifyFull over one would silently be plaintext (R18).
  - **`prefer_socket(false)` on every connection.** Otherwise, on loopback, `mysql_async`
    reads `@@socket` after the handshake and reconnects through it. That bypasses the
    forwarder and TLS, or reaches a different local server.
- **TLS hooks.** `mysql_async` builds its own rustls `ClientConfig` (a `WebPkiServerVerifier`
  in a wrapper), so `TlsParams::config` and the platform verifier cannot be used.
  - `ssl_opts(mode) -> Option<SslOpts>`: Disable gives `None`. Every other mode starts from
    `with_disable_built_in_roots(true)`. Require adds `with_danger_accept_invalid_certs(true)`
    and `with_danger_skip_domain_validation(true)`. Auto and VerifyFull add nothing, so the
    chain and the name are both checked.
  - Trust is the CA file plus the OS roots from `rustls-native-certs` (it honours
    `SSL_CERT_FILE` and `SSL_CERT_DIR`, as OpenSSL does). Each is passed as DER
    (`PathOrBuf::Buf`) through `with_root_certs`. The OS roots are loaded once, off the async
    threads, and only roots that `RootCertStore::add` accepts are kept, because
    `mysql_async` fails the whole connect on the first root it cannot parse.
  - `webpki-roots` is compiled in but never consulted.
  - This is flat trust: there are no per-root OS distrust decisions, no revocation checks and
    no intermediate fetching, unlike the platform verifier that the PostgreSQL driver uses
    (ledger ruling).
  - The client identity is passed as file paths (`ClientIdentity::new(cert, key)`), so no key
    bytes are copied into `Opts`. `mysql_async` reads them during the handshake. It parses PEM
    keys of any type, but treats a DER key as PKCS #1, so a DER PKCS #8 or SEC1 key fails
    there.
  - **Auto** asks for TLS first. If the server lacks `CLIENT_SSL`
    (`DriverError::NoClientSslFlagFromServer`), it reconnects in plaintext **only toward a
    loopback address** (`classify_host(tls_host, resolved) == Loopback`, and also
    `plaintext_allowed`). This is stricter than `hacivat-net`'s generic `plaintext_allowed`
    (Auto toward any non-public address), on purpose. Everywhere else, as for Require and
    VerifyFull, the result is `RawErrorKind::TlsNotOffered` (P9), and no byte is sent after
    the greeting. The UI then offers its fixes. A private-network server without TLS
    therefore needs TLS turned on, or an explicit Disable.
  - **Why (controller ruling, fix round 1): plaintext password recovery.**
    - Without TLS, `caching_sha2_password` "perform full authentication" makes `mysql_async`
      always request the server's RSA public key and encrypt the password with the key that
      comes back. It has no setting to refuse public-key retrieval on an insecure
      connection.
    - An active attacker can therefore strip `CLIENT_SSL` from the greeting, answer "perform
      full authentication", send its own key and decrypt the password.
    - On loopback no such attacker sits in the path.
  - **Explicit Disable is an accepted risk the user opts into.** Disable signs in in
    plaintext on any network, with the same exposure, and Task 17 shows a warning for it.
  - **M2b follow-up:** a knob that refuses public-key retrieval (and the cleartext and
    `sha256_password` paths) when the connection is not secure, upstream or in the same
    `[patch]` fork as the VerifyCa fix below. Auto toward private networks can then be
    revisited.
  - Failures are read from the error chain, never from text. The rustls error is the one
    tokio-rustls wraps in an `io::Error`, or `TlsError::Tls`. `TlsFailure::from_rustls` maps
    both `NotValidForName` and `NotValidForNameContext` to `NameMismatch`.
- **Defect: VerifyCa is unsupported in M2a (P11).** `mysql_async` 0.37.1 decides
  `skip_domain_validation` by searching the verifier's error text for `NotValidForName`. The
  locked rustls 0.23.46 reports `CertificateError::NotValidForNameContext`, whose text is
  "certificate not valid for name …", so a name mismatch is never skipped. The override cannot
  help either, because the leaf's name is unknown before the handshake. The
  MySQL 8.4 auto-generated leaf's CN is `MySQL_Server_…_Auto_Generated_Server_Certificate`.
  - Validate rejects VerifyCa for MySQL and MariaDB with `IssueCode::TlsModeUnsupported`
    (Task 16), and the TLS Select disables it (Task 17).
  - The driver refuses it again with `RawErrorKind::Unsupported` before any byte is sent.
  - The Docker test `verify_full_with_the_container_ca_still_checks_the_name` pins the
    behaviour: the chain verifies against `mysql84-ca.pem`, and the handshake still fails
    with `NameMismatch`.
  - M2b revisits this with an upstream fix (match the error variant) or a fork under
    `[patch]`. The same fork would carry the public-key retrieval knob above.
  - `verified()` reports `Full` for VerifyCa, matching `ssl_opts`, which gives VerifyCa the
    full checks. Neither is reached in M2a.
- **Secrets (P49).** `Opts` keeps the password as a `String` and derives `Debug`.
  - The driver copies it out of the `SecretString` when it builds the `Opts`, right before
    `Conn::new`, and its own copy is dropped when the handshake finishes.
  - `Conn` keeps a clone of `Opts` for its whole life, for `reset` and `change_user`.
    Hacivat cannot zeroize it.
  - No code or test `Debug`-formats `Opts`, `OptsBuilder`, `SslOpts` or `Conn`. The test seams
    `opts_for` and `opts_for_forwarded` are read only through getters.
- **Other settings.**
  - No `LOCAL INFILE` handler is ever installed, so a server request is refused.
  - `enable_cleartext_plugin(false)` and `secure_auth(true)` are set explicitly.
  - A read-only profile runs `SET SESSION TRANSACTION READ ONLY` as a `setup` statement.
    It runs at connect and again after a `reset`.
  - `program_name` is sent as a connect attribute (the application name).
- **Driver options are never read.** The catalog's allow-list uses JDBC names (`sslMode`,
  `useSSL`, `connectTimeout`, `characterEncoding`), and none of them has a `mysql_async`
  meaning; the pipeline maps TLS mode and timeout from the profile.
  - `sanitize` lets a user-typed option through unless it is on the deny list. The deny list
    names only the JDBC spellings (`allowLoadLocalInfile`, `allowUrlInLocalInfile`,
    `autoDeserialize`).
  - So `mysql_async`'s own URL names (`enable_cleartext_plugin`, `socket`, `prefer_socket`,
    `require_ssl`, `verify_ca`, `verify_identity`, `built_in_roots`, `secure_auth`,
    `compression`, `max_allowed_packet`, `wait_timeout`, `stmt_cache_size`, `tcp_keepalive`,
    `tcp_nodelay`, `client_found_rows`, `reset_connection` and the `pool_*`/`*_ttl` names)
    would pass `sanitize` from a user-typed profile. They are harmless only because this
    driver never builds `Opts` from a URL or from `options`.
  - The unit test `mysql_async_native_options_never_reach_the_opts` pins that. Adding at
    least `enable_cleartext_plugin`, `socket`, `prefer_socket`, `require_ssl`, `verify_ca`,
    `verify_identity` and `built_in_roots` to `always_deny` is a catalog follow-up.
- **Detection.**
  - `SELECT @@version, @@version_comment, @@read_only, @@wait_timeout, @@local_infile,
    @@super_read_only`. MariaDB has no `super_read_only` (error 1193), so the query is retried
    without it.
  - `detect_mysql_family` picks the family.
  - A `5.5.5-` handshake prefix is stripped only when a digit follows it.
  - The probe reports `ServerReadOnly`, `SuperReadOnly`, `LowWaitTimeout` (`wait_timeout`
    under 60) and `LocalInfileOn`.
  - `tls_summary` reads `SHOW SESSION STATUS LIKE 'Ssl_version'` and `'Ssl_cipher'`.
  - `list_databases` is `SHOW DATABASES` minus `information_schema`, `mysql`,
    `performance_schema` and `sys`.
  - `ping` is `COM_PING`.
  - `current_user` is `CURRENT_USER()` without its host part.
- **Cancel side channel (M2b).** `cancel_handle` is `None` in M2a. A `KILL QUERY <id>` needs a
  second connection to the same server. It opens through the same factory: Direct, or another
  one-shot TCP forwarder over the profile's transport. `Conn::id()` gives the thread id.
- **Licence tree** (`cargo tree -p hacivat-driver-mysql -e normal | grep -E
  'webpki|ring|native|aws'`):

  ```
  │   │   │   ├── ring v0.17.14
  │   │   │   ├── rustls-webpki v0.103.16
  │   │   │   │   ├── ring v0.17.14 (*)
  │   └── webpki-roots v1.0.12
  ├── rustls-native-certs v0.8.5
  ```

  - `mysql_async` and `mysql_common` are MIT OR Apache-2.0.
  - `ring` is Apache-2.0 AND ISC, and `rustls-webpki` is ISC.
  - `rustls-native-certs` is Apache-2.0 OR ISC OR MIT.
  - `webpki-roots` 1.0.12 is CDLA-Permissive-2.0. It enters only through `mysql_async`'s
    `rustls-tls` feature, under the scoped `[[licenses.exceptions]]` entry that Task 1 wrote
    in `deny.toml` (P1).
  - No `aws-lc-rs` or `aws-lc-sys` is present.
  - `cargo deny --all-features check licenses bans sources` passes.
