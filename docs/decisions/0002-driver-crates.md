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
  `Require`, so a server that declines TLS gets no startup message and no password. (That
  covers a plaintext server only; see Known limits for what a man in the middle can still
  do.) `Require`, `VerifyCa`
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
  failures are `Tls(TlsFailure)`, including a rustls alert on a live session (tokio-rustls
  reports it as `InvalidData` around the rustls error, which goes through
  `TlsFailure::from_rustls`). I/O keeps its `io::ErrorKind`, a malformed message is
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
    to `UnknownIssuer`, like the other policy failures. Apple's 825-day validity limit applies
    to certificates from a private CA too. For a long-lived private-CA certificate, the
    `tls.untrusted` advice to import the CA may therefore not help; only
    "encrypt without verifying" (`Require`) gets through.
  - **Cleartext and md5 password requests are always answered (controller ruling).**
    `tokio-postgres` answers `AuthenticationCleartextPassword` (`R` code 3) and
    `AuthenticationMD5Password` (code 5) whatever the transport is. A man in the middle can
    harvest the password (in clear, or as an md5 hash it can attack offline):
    - in `Require` mode, where any certificate is accepted, by terminating TLS itself;
    - in `Auto` on a loopback or private address (`Prefer`), by answering the `SSLRequest`
      with `N` (stripping TLS) and then asking for a cleartext password.

    P9 closes only the case of `Auto` toward a public address.
  - **M2b follow-ups:**
    - a `require_auth`-style guard on the injected stream. It sniffs the backend
      authentication request (`R`, codes 3 and 5) and refuses to answer unless the session
      is verified TLS (`VerifyCa`, `VerifyFull`, or `Auto` that negotiated TLS) or the mode is
      `Disable` (an explicit plaintext choice);
    - `tls-server-end-point` channel binding, so `SCRAM-SHA-256-PLUS` detects a terminating
      man in the middle even under `Require`.

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

## m2a-t15 — hacivat-driver-sqlite

**Library.** `rusqlite` 0.42.0 with `bundled` and `column_metadata`, over `libsqlite3-sys`
0.40.0, which compiles SQLite 3.54.1 from its amalgamation (a C compiler is needed on the build
host). No system SQLite is linked, so every platform runs the same engine, and the catalog
gives SQLite no version floor: it is always `Supported` (P4).

**Licence tree.** `rusqlite` and `libsqlite3-sys` are MIT, and the SQLite amalgamation is public
domain (cargo-deny reads only the crate's licence field). The other dependencies are
`bitflags`, `fallible-iterator`, `fallible-streaming-iterator`, `hashlink` and `smallvec`, plus
the build dependencies `cc`, `pkg-config` and `vcpkg`. All of them are MIT or Apache-2.0.
`sqlite-wasm-rs` (MIT) is only built for wasm32. Nothing new enters `deny.toml`.

**TLS and trust.** Not applicable: the database is a local file. `tls_summary` reports
`encrypted: false`, `verified: No`. The caps are `file_based: true`, `stream_injection: false`
and `in_protocol_tls: false`.

**Stream injection.** Not applicable: there is no network endpoint. `connect` accepts only
`Endpoint::File` and answers `Unsupported` for anything else.

**Secrets (P49).** None. SQLite has no user or password; `DriverConnect.password` is never
read, and `current_user` is `None`.

**Threading.** `rusqlite::Connection` is `Send` but not `Sync`. `connect` opens it inside
`spawn_blocking`, and it never leaves that thread. `SqliteConn` sends jobs over a bounded
`mpsc` channel and awaits each reply on a `oneshot`, so the async executor never blocks
(P33). `close` drops the sender, and resolves only after the thread has dropped the
connection.

**Open flags and hardening.**
- The flags are:
  - `SQLITE_OPEN_READ_ONLY` when read-only;
  - `READ_WRITE | CREATE` only when `create_if_missing`;
  - otherwise `READ_WRITE`.

  `NO_MUTEX` is always added. `SQLITE_OPEN_URI` is never set.
- Read-only wins whenever the endpoint, the request or a `mode=ro` option asks for it, so
  read-only never creates a file. Options can only tighten. Every other option is ignored;
  `vfs` and `load_extension` are always denied by `sanitize` anyway.
- `SQLITE_DBCONFIG_DEFENSIVE` is on, so `writable_schema` cannot corrupt the file.
- `SQLITE_DBCONFIG_TRUSTED_SCHEMA` is off. The file may come from anywhere, so SQL stored in
  its schema (views, triggers) cannot call functions that are not marked innocuous.
- Extension loading is off. rusqlite is built without its `load_extension` feature, and
  SQLite keeps `load_extension()` disabled until a C call enables it (a test checks that the
  SQL function is refused).
- `PRAGMA schema_version` is read at connect, because SQLite reads a file lazily. A file that
  is not a database (`SQLITE_NOTADB`, 26) or is locked fails in the connect step, not at the
  first query.
- The busy timeout is set to `DriverConnect.connect_timeout` before that read, replacing
  rusqlite's 5 s default. A file another process holds locked fails the connect within that
  time with `SQLITE_BUSY` (5, `internal`). The timeout stays on the connection for later
  statements.

**File path rules.**
- Only an absolute path, or exactly `:memory:`, reaches SQLite. Anything else, including the
  empty path, fails before SQLite sees it with `RawErrorKind::Io(InvalidInput)` and no
  server message. That error classifies as `internal`, so Validate (T16) should reject such a
  path first; this check is the backstop.
- The reason is a build defect we cannot turn off without `unsafe`. The bundled build defines
  `SQLITE_USE_URI`, so SQLite parses a name starting with `file:` as a URI (`?vfs=`,
  `?mode=memory`, `?immutable=1`) even without `SQLITE_OPEN_URI`. An absolute path never
  starts with `file:`. A relative path would also depend on the app's working directory.
  The empty path opens a private temporary database.
- Windows device namespaces are refused the same way: `\\.\…` (for example
  `\\.\PhysicalDrive0`) and `\\?\GLOBALROOT\…`.
- UNC paths (`\\server\share\…`, `\\?\UNC\…`) are refused with `Io(InvalidInput)`, as
  defence in depth (T7 refuses them too). Opening one performs SMB authentication, which leaks
  the user's NTLM hash to the host. A dead share can also block the thread, and SQLite's
  locking over SMB is unreliable.
- When something exists at the path, it must be a regular file. `fs::metadata` follows
  symlinks, so a symlink to a file opens. A directory, FIFO, socket or device is refused with
  `Io(InvalidInput)` before SQLite opens it, so a read can never block the connection's thread
  (a FIFO with no writer blocks forever). Any other `metadata` error is returned as
  `Io(kind)`. The check and the open are not atomic: a local process that swaps the file in
  between can still win. That race needs local write access to the directory.
- Whether a path lies under the approved roots is `sanitize`'s rule (T2), applied by the
  caller to `SanitizeInput.paths` for an untrusted origin. The driver opens what it is given;
  it does not expand `~`.
- A missing file without `create_if_missing` (or with read-only) is `SQLITE_CANTOPEN` (14),
  and no file is created.

**Errors.** Every SQLite failure is `RawErrorKind::Db`. Its `ServerMessage` carries the
extended result code in `vendor_code`; T10's table reduces it to the low byte. The text is
SQLite's own, verbatim, with `lang_hint` `en`. SQLite has no SQLSTATE. A `CANTOPEN` message
includes the path the user gave; it holds no secret.

**Probe.** `PRAGMA journal_mode` is always reported as `SqliteJournal { mode }`, ASCII
lowercase. `SqliteFileNotWritable` is reported when read-write was asked for but SQLite opened
the main database read-only, which it does when the file is write-protected.

**Cancel.** `CancelMechanism::Interrupt`. In M2a, `cancel_handle()` is `None`. In M2b it
becomes rusqlite's `InterruptHandle` (`sqlite3_interrupt`), taken on the connection thread
before the first job and safe to call from any thread.

**Known limits and defects.**
- `SQLITE_USE_URI` is compiled in; see the path rules above.
- `SQLITE_ENABLE_LOAD_EXTENSION=1` is compiled in. rusqlite's `DbConfig` omits
  `SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION`, so we rely on SQLite's default (off) and never
  enable it.
- Read-only here means the open flag only. The full read-only guard belongs to
  `hacivat-guard` (spec §4, SQLite row) and is not in M2a.
- The connection budget (1, or 1 writer and 2 readers in WAL mode, D-035) is the session
  layer's. The driver opens one connection per `connect`.

**M2b notes for `hacivat-guard`.** In M2a a read-only connection is only the
`SQLITE_OPEN_READ_ONLY` flag. These gaps are known, and M2b must close them:
- **`ATTACH` and `VACUUM INTO` still write.** `ATTACH 'new.db'` creates and opens a writable
  database next to the read-only one. `VACUUM INTO 'copy.db'` writes a full copy of the data.
  Both name a file chosen by whoever wrote the SQL (spec §4, ver/01 #sqlite-ro).
- **Turn off `SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE` and `SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE`**
  (SQLite 3.49+; the bundled engine is 3.54.1). rusqlite exposes them only with its
  `modern_sqlite` feature, which is a manifest change.
- **Set `SQLITE_LIMIT_ATTACHED` to 0**, so no `ATTACH` can succeed. This is
  `Connection::set_limit` behind rusqlite's `limits` feature.
- **Install a default-deny authorizer** (`Connection::authorizer`, rusqlite `hooks` feature).
  It must deny:
  - `ATTACH` and `DETACH`;
  - every write action code;
  - `VACUUM INTO`;
  - PRAGMAs not on an allow-list, `query_only` included;
  - `load_extension`, `readfile` and `writefile`.

  Then add `PRAGMA query_only = 1`, the other `sqlite3_limit` caps, a progress-handler timeout
  and `sqlite3_stmt_readonly` (necessary, not sufficient).
- **Resolve symlinks first.** Canonicalize the path (`fs::canonicalize`, which resolves every
  symlink and `..`) before any approved-roots check, and open the canonical path. That way the
  checked file is the opened file, and a symlink cannot point an approved name outside the
  roots. T2's `sanitize` already canonicalizes component by component for untrusted origins.
  The guard must apply the same rule to the path the driver finally opens, including names
  given to `ATTACH` if any `ATTACH` is ever allowed.

## m2a-t22 — gate: the loopback forwarder slot race

`LoopbackForwarder` in `ForwarderMode::Tcp` listens on `127.0.0.1:0` and accepts one connection
(`max_accepts = 1`). Between `bind` and the driver's `connect`, another process on the same
machine that knows the port can connect first and take the slot. The driver's own connection
then fails, or, if the other process relays to the real server, it sits on a connection that
carries the session. This is inherent to a loopback listener and is the same exposure as
`ssh -L`. The ruling (controller, M2a) accepts it:

- The port is chosen by the kernel and is not announced anywhere but to the driver.
- Only local processes of any user on the machine can reach it, and such a process can already
  read the user's memory in most threat models M2a covers.
- TLS still verifies the real server name (`with_danger_tls_hostname_override`), so a process
  that wins the slot cannot read an encrypted session without the server's certificate.
  Under `Auto` plaintext is allowed only on loopback or private networks (see Known limits).
- M2b revisits this together with the SSH transport, which will also use the forwarder
  (a Unix socket in a `0700` directory where the driver can use one).

## m2a-finalfix — known limits (addendum to m2a-t13 and m2a-t14)

- **P49 holds only for PostgreSQL.** `tokio-postgres` keeps no copy of the password after
  authentication (`config_for` builds the `Config` at connect time and drops it after the
  handshake). `mysql_async::Conn` keeps a clone of its `Opts`, with the password as a plain
  `String`, for the connection's whole life (`reset` and `change_user` reuse it), so a MySQL or
  MariaDB password stays in process memory until the connection closes and cannot be zeroized.
  Hacivat never formats `Opts` or `Conn`. Closing the gap needs an upstream change or the
  `[patch]` fork planned for M2b (see m2a-t14).
- **The MySQL CA is read once.** `build_tls_params` reads and checks the CA file and keeps its
  certificates in `TlsParams::ca_certs`; the MySQL driver hands exactly those to `mysql_async`
  and never reads the file again. An unreadable CA is `config.invalid` (`TlsCaUnreadable`)
  before any I/O, as for PostgreSQL, never a silent fall-back to the OS roots alone.
- **No forwarder socket directory.** The MySQL forwarder listens on loopback TCP only
  (`ForwarderMode::Tcp`, R18), so M2a creates no private `0700` socket directory; the unused
  `ConnectDeps::private_dir` and the boot's `fwd` directory are gone. The SSH transport in M2b
  brings one back where a driver can use a Unix socket.

## m2b-t11 — hacivat-driver-sqlite

**Two independent read-only controls.** The file's read-only mode (`Endpoint::File.read_only`
or a `mode=ro` option) only opens the file `SQLITE_OPEN_READ_ONLY`, as in M2a. A read-only
profile or session (`DriverConnect.read_only`, which the pipeline sets from the profile OR the
session, and also passes on the endpoint) opens it read-only too and then hardens the
connection. A read-write profile gets no authorizer and none of the settings below; M2a's
defensive and untrusted-schema settings stay on every connection.

**Paths.** `check_path` refuses `hacivat_net::is_network_path` first (two leading separators,
`\??\`, UNC, device and `GLOBALROOT` prefixes, on every OS), before any file system call; then
only an absolute path or `:memory:` reaches SQLite (M2a).

**The hardened open** (`harden.rs`, after the header read), in this order, each setting read
back so an ignored one fails the open (`RawErrorKind::Other`), never a silent weaker session:
1. `SQLITE_OPEN_READ_ONLY` (the open flags);
2. `SQLITE_DBCONFIG_DEFENSIVE`;
3. `SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE` and `_ENABLE_ATTACH_WRITE` off;
4. `SQLITE_LIMIT_ATTACHED` 0, `LENGTH` 100,000,000, `SQL_LENGTH` 1 MiB, `VARIABLE_NUMBER`
   32,766 (`hacivat_guard::sqlite_hardening`);
5. `PRAGMA query_only = 1`;
6. the authorizer: rusqlite's `AuthAction` mapped to `hacivat_guard::SqliteAction`, answered
   with `sqlite_authorize`'s verdict. Every action code maps: the nine `CREATE_*` codes
   (temp, view, trigger, index, virtual table) to `Create`, the nine `DROP_*` codes to `Drop`,
   `REINDEX` and every `Unknown` code (0 `SQLITE_COPY`, codes past 33, a known code whose
   arguments rusqlite did not expect) to `Other(code)`, and a variant a later rusqlite adds to
   `Other(-1)`; `Other` is denied. Tests drive codes 0–33 (and `Unknown` for −1…40, `i32::MIN`,
   `i32::MAX`) through the adapter, and real statements reach every emitted code 1–33 through
   a shadow authorizer (controller carry-in from the T5 review).

**Execution** (`exec.rs`). One statement per `execute`: rusqlite's `prepare` refuses a tail
that holds a statement (`MultipleStatement` → `Db`, `SQLITE_ERROR`, reason
`multiple-statements`); whitespace, comments and `;` in the tail are fine, and leading `;` are
empty statements SQLite's parser skips. Text with no statement runs nothing. On a read-only
profile `sqlite3_stmt_readonly` must also be true, else `Db` with `SQLITE_AUTH` and reason
`readonly-required`, before the statement steps. This catches what never asks the authorizer
(`VACUUM`, `VACUUM INTO` by any expression or parameter). Rows come back as text (integers in
decimal, reals with a fractional digit, blobs as `\x` + hex); at most `max_rows` are kept and
the rest counted. `command_tag` is the first keyword in ASCII upper case (Q35); a statement
that is not read-only reports `sqlite3_changes` as `rows_affected` (0 when nothing changed),
a read reports none. Unbound parameters are NULL (`raw_query`).

**The journal-mode exemption (Q16).** `PRAGMA journal_mode` without an argument compiles to
`OP_JournalMode` in "query" mode, so `sqlite3_stmt_readonly` calls it a write, yet Q16 lists it
as readable (the brief's test reads it through `execute`). The authorizer records what it was
asked while the statement was prepared; the stmt-readonly rule is waived only when that was
exactly one action, `Pragma { journal_mode, no argument }`. Every form that sets the mode has an
argument and is denied by the authorizer; the file is read-only and `query_only` is on anyway.

**Transactions and cancel.** `tx_status` is `InTx` exactly when `sqlite3_get_autocommit` is
off, read on the connection's thread after every `execute` and `batch` (failed ones too).
`reset` is `Ok(())` and `backend_id` is `None`. `batch` runs trusted control text as several
statements; the authorizer still applies on a read-only profile.

**Cancel, latched per request (fix round 1, review I1).** `sqlite3_interrupt` alone loses an
early cancel: SQLite clears the interrupt flag whenever a statement starts while none is active
(`sqlite3RunParser`, `sqlite3Step`), so a cancel sent after `execute` was submitted but before
the connection's thread started the statement did nothing (17 of 20 in review). The handle
(`SqliteCancel`) therefore keeps a latch:
- `execute` and `batch` take a ticket on the async side before the job is sent. Tickets are
  handed out in order and their jobs run in order on the connection's thread, which marks the
  job it runs and records the highest ticket that has ended.
- `cancel()` acts only while some submitted ticket has not ended: under the latch's lock it
  cancels every ticket handed out so far (`cancelled_upto = last`) and calls
  `sqlite3_interrupt`. A cancel while nothing is pending does nothing, and it never covers a
  ticket handed out later, so it can never reach the next request (Task 7's C1). The latch
  follows the job, not the future (fix round 2, R1-M1): a request cancelled and then dropped
  before its job ran still stops, and the next request's ticket does not reset it.
- The job reads the latch right after `prepare` (before the first step), and a progress handler,
  installed on every connection, reads it for the job it runs every 1,000 VM steps and
  interrupts the statement. An interrupt sent just after the last statement stopped is cleared
  by SQLite when the next statement starts.
- `close()` (fix round 2, R1-I1) sets a `closing` flag and interrupts before dropping the job
  sender; the progress handler and the post-prepare checks honour it. A statement whose request
  future was dropped without a cancel (the session's abandoned path) and any job queued behind
  it stop, so `close` returns and the connection's thread exits instead of spinning forever.
- `batch` reads the latch before `execute_batch` only (R1-M2, accepted): a cancel that lands
  between two short statements of a trusted batch, none of which reaches 1,000 VM steps, lets
  the batch finish and report `Ok`. Reporting `Cancelled` after the work committed would be
  wrong, and the text is short and trusted.

An interrupted or latched request is `RawErrorKind::Cancelled` (extended code 9,
`SQLITE_INTERRUPT`). The handle is safe from any thread and a no-op after close. Tests: 50
immediate cancels after submit, each bounded and all interrupted; `batch` likewise; a cancel
before or after a statement, or for a dropped request, never reaches the next one; a cancelled
then dropped endless request still stops (20 rounds); `close` after dropping two endless
requests returns within the bound on both profiles. The progress handler is only the latch,
not a timeout.

**Accepted limitation: WAL side files (fix round 1, review I3; controller ruling).** A
read-only session on a WAL-mode database may create `-wal` (empty) and `-shm` next to the file
and leave them after close: SQLite's WAL reader needs the shared-memory index, and a read-only
connection never checkpoints or unlinks them. The alternatives are unsafe or unavailable:
`immutable=1` ignores a concurrent writer (wrong reads, possible corruption reports), and
`readonly_shm` needs existing files and the URI form the driver refuses. The database file
itself stays byte-identical (test `a_read_only_session_on_a_wal_database_leaves_the_database_file_unchanged`,
which also checks the `-wal` holds no frames). The other tests now list every file but the
database itself, so a side file elsewhere would fail them.

**Findings recorded for review.**
- A denied *function* is `SQLITE_ERROR` with "not authorized to use function: …" (SQLite's
  resolver rewrites the code); every other denial is `SQLITE_AUTH`. The disabled extension
  loader on a read-write connection answers "not authorized" only when the statement runs.
- `readfile`, `writefile` and `edit` are not compiled into the bundled engine ("no such
  function" comes before the authorizer); they stay on the guard's deny list for any build
  that adds them, matched with ASCII case folding.
- A file opened read-only by its own mode without a read-only profile keeps M2a's behaviour
  (only the flag); the guard and its labels apply to read-only profiles and sessions.
- Not in this task: a progress-handler timeout (the session's cancel ladder and the latched
  interrupt cover runaway statements; the progress handler is where a timeout would go).
- Statements that never ask the authorizer: `VACUUM` and `VACUUM INTO` (refused by
  `readonly-required`), and `VACUUM temp`, which SQLite skips (`vacuum.c` ignores the temp
  schema) and which is read-only to `sqlite3_stmt_readonly`: a harmless no-op.

## m2b-t10 — hacivat-driver-pg

- **Wire observer (`wire::Observer`, Q11, Q12).** `tokio-postgres` keeps the backend PID
  private (`CancelToken.process_id` is `pub(crate)`), returns at an `ErrorResponse` before
  the `ReadyForQuery` that follows, and exposes no command tag text. A thin stream wrapper reads
  the protocol as it passes instead of forking the library. It frames backend messages (type
  byte, big-endian `i32` length counting itself; below 4 or above 1 GiB is `InvalidData`),
  skips bodies by count and copies only the first bytes of `R` (4), `K` (4), `Z` (1) and `C`
  (64) into fixed buffers: no allocation per message, so a large `DataRow` or `CopyData` can
  neither desync it nor cost memory.
  - **Layering.** `Disable`: one observer, framed from the first byte. With an `SSLRequest`
    the raw observer reads only the one-byte answer: after `S` it passes the TLS records
    through untouched, and `PgTlsStream` puts a second, framed observer above rustls, so every
    `R`, `K`, `Z` and `C` is read **decrypted**; after `N` (`prefer`) the raw observer frames
    the plaintext itself. Both share one `WireState` (`Arc`).
  - **PID.** `BackendKeyData` after `AuthenticationOk` and before the first `ReadyForQuery`;
    `backend_id()` reads it.
  - **Transaction status, exactly (Q12).** Every `ReadyForQuery` after `AuthenticationOk`
    records its status byte (`I`/`T`/`E`; anything else is `InvalidData`) and advances a
    counter with a `tokio::sync::Notify`. The plan's "wait until the count advances" is not
    exact for the extended protocol: a successful `Parse`/`Sync` round already advanced it
    before the failing `Bind`/`Execute`/`Sync`, and a dropped statement's `Close`/`Sync` adds
    one more. So the observer also frames the **frontend** side after `AuthenticationOk` (the
    startup messages carry no type byte, and nothing is written between `AuthenticationOk` and
    the first request, so framing starts on a boundary) and counts every `Query`, `Sync` and
    `FunctionCall`, each of which draws exactly one `ReadyForQuery`. After an error,
    `execute`, `batch` and `reset` wait (at most 1 s, or until the connection ends) until
    every one is answered. When all have arrived, `tx_status()` is exact without a sleep.
    *Fix round 1 (I2):* when the bound expires first (a late segment on a slow tunnel), the
    status is unknown, so the connection is **broken**: the error becomes a lost connection
    (`Io(TimedOut)`, the server's message kept), the connection task is aborted, every later
    call fails with `Io(NotConnected)`, and `tx_status()` reads `Failed`, never the stale
    status. The session reconnects; a rollback that was not needed is harmless.
  - **Command tag.** The last `CommandComplete` text (cleared before each request) is
    `ExecSummary::command_tag`, which the read-only post-check reads (`INSERT 0 3`,
    `CREATE TABLE`); `rows_affected` is its count (`RowStream::rows_affected`, or the simple
    protocol's `CommandComplete`).
- **Insecure-auth guard (B8, Q20).** An `AuthenticationCleartextPassword` (code 3) or
  `AuthenticationMD5Password` (code 5) under `InsecureAuth::RefuseUnlessVerified`, when the TLS
  outcome is not `Full`, `CaOnly` or `Pinned`, fails the read with an `io::Error` wrapping
  `InsecureAuthRefusedMarker`, mapped to `RawErrorKind::InsecureAuthRefused`. The refused bytes
  are removed from the read buffer, so `tokio-postgres` never holds a complete request and
  never writes a password response, whether the request arrives whole or a byte at a time. The
  outcome is captured from `TlsParams.outcome` right after this connection's handshake (a later
  handshake on the same configuration, such as a cancel, cannot change it). The check runs on
  every `R` message, also after `AuthenticationOk` (a legitimate server never sends one there).
  It also runs when the profile has no password: the refusal comes before
  `PasswordRequired`, so nobody is asked for a password that could not be sent. SCRAM (code 10)
  always passes: it never reveals the password.
- **Channel binding (B9).** `PgTlsStream::channel_binding()` returns `tls-server-end-point`
  (`hacivat_net::tls_server_end_point` of the rustls session's `peer_certificates()[0]`), so
  SCRAM runs as `SCRAM-SHA-256-PLUS` when the server offers it (`channel_binding=prefer`, the
  `tokio-postgres` default); a leaf without an end-point hash (Ed25519, unknown algorithms)
  offers none. A scripted server and the `pg17-tls` target with `channel_binding=require`
  verify it.
- **Execution.** `Protocol::Simple` streams `simple_query_raw`: the last statement's rows as
  text, `max_rows` kept and the rest counted (`truncated`), never buffered beyond the cap.
  `Protocol::Extended` prepares the one statement (several are `42601` from the server),
  takes the column names from the statement, runs it with no parameters and counts the rows
  without keeping them. `batch` is `batch_execute`. The command tag, rows and columns of a
  multi-statement simple query are the **last** statement's, so `INSERT …; SELECT 1` reads as
  `SELECT 1`: a caller that decides from the tag (Task 18's read-only post-check) must run
  user statements one at a time through `Protocol::Extended` (review M2). *Fix round 1 (I3):*
  cells are read with `try_get`; a cell that is not UTF-8 (a `SQL_ASCII` database sends stored
  bytes unconverted) fails the statement with `RawErrorKind::Other` instead of panicking, and
  the connection stays in step.
- **COPY FROM STDIN (fix round 1, I1).** `tokio-postgres` fails a request that meets a
  `CopyInResponse` or `CopyBothResponse` (`unexpected message`), while the server waits in
  copy-in mode: no `ReadyForQuery` would come, `tx_status()` would read the old status and the
  next request's reply would be matched to the stuck one. The observer therefore answers `G`
  and `W` (after authentication) itself with `CopyFail("COPY FROM STDIN is not supported
  yet")`, plus a `Sync` when the last statement-running message was an extended `Execute`
  (the server ignores the `Sync` already sent during copy-in and then discards until the next
  one; it is not counted, so the single `ReadyForQuery` settles the one counted `Sync`). The
  bytes go out only on a frontend message boundary: a message `tokio-postgres` is writing is
  finished first. The server ends the copy with `ErrorResponse` and one `ReadyForQuery`, which
  completes the failed request inside `tokio-postgres` too; the call returns
  `RawErrorKind::Unsupported`, and the status is exact (`Failed` inside `BEGIN`, `Idle` in
  autocommit). Simple, extended and `batch` paths are Docker-tested; `COPY … TO STDOUT` ends
  with its own `ReadyForQuery` and is left alone. The driver never uses `Client::copy_in`;
  it must not while the observer refuses copy-in.
- **Notices (Q12).** The connection is driven with `poll_fn(|cx| connection.poll_message(cx))`
  instead of `connection.await` (which logged notice text at `info` and dropped it). Notice
  SQLSTATEs go into a per-connection buffer of 100 (the oldest dropped first), drained before
  and after each `execute`; the text is never logged or kept. Notifications are dropped.
- **Reset (Q10).** `CLOSE ALL; SET SESSION AUTHORIZATION DEFAULT; RESET ALL; UNLISTEN *;
  SELECT pg_advisory_unlock_all(); DISCARD PLANS; DISCARD TEMP; DISCARD SEQUENCES;`, then
  `SELECT name FROM pg_prepared_statements WHERE from_sql` and one `DEALLOCATE "<name>"`
  (quoted identifier, `"` doubled) per row. No path sends `DEALLOCATE ALL`: a statement named
  `ALL` is `DEALLOCATE "ALL"`. `DEALLOCATE ALL` would drop `tokio-postgres`'s cached type-lookup
  statements and break the next extended query on an enum column (`26000`; Docker-tested).
  `RESET ALL` returns to the startup values, so the read-only `-c
  default_transaction_read_only=on` survives a reset (Docker-tested).
- **Cancel (B23).** `cancel_handle()` returns `PgCancel { token, factory, tls }`: a new stream
  from the connection's own `StreamFactory` (direct, tunnel or proxy alike) and
  `CancelToken::cancel_query_raw` over our connector with the connection's `sslmode` and rustls
  configuration (verifier, pins, CA, client certificate), bounded at 10 s. **A session that ran
  over TLS never cancels in plaintext:** under `prefer`, a server (or a man in the middle)
  answering the cancel's `SSLRequest` with `N` fails the request before the PID and secret
  key are written (`TlsRefusedMarker`, `TlsNotOffered`). The cancel's handshake gets its own
  failure slot; it runs the same verifier, whose `presented`/`outcome` slots are the
  connection's (nothing reads them after the connect, and the observer captured the outcome
  already).
  - *Fix round 1 (review M1), not closed here.* "One `TlsParams` per handshake" would need
    a fresh verifier with its own slots. That verifier lives in `hacivat-net` (`Recording`,
    private) and is built only by `build_tls_params`, which needs the pin endpoint's port and
    the client key's passphrase; the driver has neither (the endpoint carries no port behind a
    tunnel, and holding the passphrase in a cancel handle would retain a secret). The residual
    risk is fail-closed: every handshake through one configuration writes either nothing or
    the same verified level, so a cancel racing a reconnect that shares the `TlsParams` can
    only clear `outcome` (the guard then refuses an insecure method once), never raise it. The
    pipeline (Task 14/17) must build a new `TlsParams` per connect attempt and never reuse one
    a live cancel handle holds; a `TlsParams::fork()` in `hacivat-net` (same verifier
    settings, new slots) would let the cancel build its own.
  - **The cancel race is inherent (review M5).** `cancel()` returning `Ok` means only that the
    request was written; the server signals the backend asynchronously, and the statement may
    already have ended (libpq behaves the same). Task 7's sequence guard covers the session
    side; ADR 0004 is the place for the session's rule.
  - **Poolers (review M7).** Behind PgBouncer and similar, `BackendKeyData` carries the
    pooler's key, so `backend_id()` is not a server PID: an escalation that terminates a
    backend by this PID (Task 7/17) must check `pg_backend_pid()` first or not run through a
    pooler.
- **TLS summary.** `verified` is now the handshake's own outcome (a pinned `Require` reads
  `Pinned`), falling back to the mode's level only when none was recorded.
- **Known effect until Task 14.** The pipeline's `insecure_auth_for` is still Task 1's
  fail-closed stub (`RefuseUnlessVerified` everywhere), so an md5 server reached through the
  pipeline (`pg13` in the Docker matrix, loopback) is refused with `auth.insecure_method` until
  Task 14 allows loopback, Unix sockets, `Disable` and consented plaintext (B8).
