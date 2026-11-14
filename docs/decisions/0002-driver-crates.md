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
    *Round 1 re-review (R3):* `Io(TimedOut)` means only that the bound expired. When the
    connection ended first (a FATAL `57P01` from `pg_terminate_backend` or an admin shutdown,
    then the server closes), it is broken the same way, but a server error keeps its class
    (`Db`, with its SQLSTATE); any other kind becomes `Io(ConnectionAborted)`.
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
  the connection stays in step. *Round 1 re-review (R2):* after the first bad cell the rest of
  the result is still read to its `ReadyForQuery` (rows dropped, no settle bound), so a decode
  error early in a large result is `Other` on a usable connection, not a settle timeout.
- **COPY FROM STDIN (fix round 1, I1).** `tokio-postgres` fails a request that meets a
  `CopyInResponse` or `CopyBothResponse` (`unexpected message`), while the server waits in
  copy-in mode: no `ReadyForQuery` would come, `tx_status()` would read the old status and the
  next request's reply would be matched to the stuck one. The observer therefore answers `G`
  and `W` (after authentication) itself with `CopyFail("COPY FROM STDIN is not supported
  yet")`, plus a `Sync` when the last statement-running message was an extended `Execute`
  (the server ignores the `Sync` already sent during copy-in and then discards until the next
  one; it is not counted, so the single `ReadyForQuery` settles the one counted `Sync`).
  *Round 1 re-review (R1):* the `Sync` is injected only when the `Execute`'s own `Sync` has
  already started out (the writer clears a flag on `E` and sets it on `S`). After a partial
  write that stopped between `Execute` and `Sync`, the library's `Sync` follows the `CopyFail`
  and ends the discard itself; a second one would draw an uncounted `ReadyForQuery` and shift
  every later reply. Exactly one `Sync` reaches the server either way. The layering is
  unchanged: the injection runs on the framed observer only (above rustls under TLS). The
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

## m2b-t13 — hacivat-driver-mysql

This section supersedes the M2a P11 defect note ("VerifyCa is unsupported in M2a") and the
"TLS hooks" trust description of `m2a-t14`. The driver runs on the vendored `mysql_async`
(`third_party/mysql_async`, patches §1–§5, Task 8).

- **TLS is Hacivat's configuration (Q2, patch §5).** Every mode (Auto, Require, VerifyCa,
  VerifyFull) builds `SslOpts::default().with_rustls_client_config(tls.config.clone())
  .with_danger_tls_hostname_override(Some(tls_host))`. So MySQL and MariaDB get what
  PostgreSQL gets: the pin-aware recording verifier (pins, the presented chain for the
  certificate review, `TlsParams.outcome`), the platform verifier for Auto and VerifyFull,
  Task 3's `ChainOnly` verifier for VerifyCa, the CA file's certificates, and the client
  identity Hacivat read and decrypted (an encrypted PKCS #8 key, B25). `mysql_async` builds no
  `ClientConfig`, root store or `DangerousVerifier` of its own, and never opens a certificate or
  key file, so the client paths go only through `hacivat_net::paths::local_path` in
  `build_tls_params` (T12 carry-in).
  - **The OS-roots path is removed.** `rustls-native-certs` is gone from the crate (the named
    manifest exception), with `os_roots`, the DER root list, `with_disable_built_in_roots` and
    the `accept_invalid_certs`/`skip_domain_validation` flags. M2a's "flat trust" difference
    from PostgreSQL is gone: MySQL now uses the platform verifier, as PostgreSQL does.
  - **VerifyCa works (P11 superseded).** The driver no longer refuses it; `ChainOnly` checks
    the chain against the CA file and skips the name. `verify_ca_with_container_ca_skips_the_name`
    connects to `mysql84` with `mysql84-ca.pem` (`CaOnly`). Patch §1 stays as an upstream bug
    fix but is no longer on Hacivat's path.
  - **Platform note.** On Apple, Security.framework refuses MySQL's auto-generated leaf (ten-year
    validity, no subject alternative name) by policy even with its CA as an extra root; the
    failure is `UnknownIssuer`, which offers the review/pin flow. Elsewhere VerifyFull with the
    CA reaches the name check (`NameMismatch`). `pin_flow_on_mysql84` pins it on both.
  - **One handshake, one `SslOpts`.** `mysql_async` caches a connector per `SslOpts`, so every
    attempt builds fresh `Opts`, `SslOpts` and guard (T8 carry-in). Auto's plaintext fallback
    (loopback only, unchanged, Q41) builds `Opts` without TLS.
- **Password guard, outcome-based (B8, Q20, patch §2).** `password_guard` is evaluated where
  the server answers "perform full authentication" (`caching_sha2_password` `0x04`, before both
  the cleartext-over-TLS and the RSA-key branches). The closure reads the attempt's
  `TlsParams.outcome` at that moment and allows the password when `InsecureAuth::Allow`, or when
  TLS ran and the outcome is `Full`, `CaOnly` or `Pinned` (`guard_allows`). It is never decided
  from the mode: Require with a pin passes, Require without one (`No`) is refused. A plaintext
  attempt never reads an outcome. `DriverError::InsecurePasswordRefused` maps to
  `RawErrorKind::InsecureAuthRefused`; the trail shows the TLS pair and `AuthStarted`.
  - **Why reading the shared slot is safe.** rustls gives the verifier no per-connection
    context, so the slot is shared by every handshake through one `TlsParams.config`. The guard
    runs only after this connection's handshake completed, which ran the verifier (session
    resumption is off, Task 3) and wrote the configuration's fixed level. A concurrent
    handshake through the same configuration can only write the same level or reset the slot
    to `None` (the guard then refuses once): never raise it. The pipeline still builds one
    `TlsParams` per attempt (T3/T14 carry); the `KILL QUERY` side connection is a separate
    `connect` with its own `DriverConnect`.
  - The Docker tests empty `hv_ro`'s `caching_sha2_password` cache entry before signing in (the
    account sets its own password again; no account with a known password has `RELOAD`), so
    full authentication really runs: Disable is refused under `RefuseUnlessVerified` and
    allowed under `Allow`; Require is refused without a pin and allowed with it.
- **Capabilities.** `multi_statements(!read_only)` (patch §3): a read-only connection never
  asks for `CLIENT_MULTI_STATEMENTS`, so `SELECT 1; DROP …` is a syntax error (1064) even for
  an account that could run both. `local_infile_capability(false)` always (patch §4), and no
  `LOCAL INFILE` handler is ever installed, so a server's `0xFB` request is refused.
  `enable_cleartext_plugin(false)` and `secure_auth(true)` stay: the guard's completeness relies
  on them (Task 8 review).
- **Execution (HV-EXEC-05).**
  - `Protocol::Simple` is `query_iter`: rows as text (bytes that are not UTF-8 become U+FFFD,
    never a panic), `max_rows` kept, the rest counted. With several statements (read-write
    only) the columns and rows are the last result set's that has columns.
  - `Protocol::Extended` is `exec_iter(sql, ())`, a prepared statement: rows counted, values
    dropped. `ER_UNSUPPORTED_PS` (1295) is `Unsupported` on a read-only connection and never
    re-sent as `COM_QUERY`; on a read-write connection it runs through `query_iter`. A prepared
    `CALL` runs, but the session's read-only mode (1792) and `hv_ro`'s missing `EXECUTE` (1370)
    stop its writes (`read_only_prepared_call_cannot_write`).
  - `rows_affected` is the last OK packet's affected-row count, never the number of rows
    returned (T5 carry-in): a result set reports 0 there, so the post-check's `SELECT`/`WITH`/
    `CALL`/`DO` rule reads a positive count as a write.
  - `command_tag` is the first keyword in ASCII upper case (Q35), after whitespace and
    comments. An executable comment (`/*!…`, `/*!50000 …`, `/*M!100000 …`) is code, so its
    keyword counts (`/*!50000 DROP TABLE t */` is `DROP`; `/*!*/ DELETE …` is `DELETE`).
  - Errors 1317 (`KILL QUERY`), 3024 (`max_execution_time`) and MariaDB's 1969
    (`max_statement_time`) are `RawErrorKind::Cancelled`; the server's message and code stay
    (the session reads 1317). The connection stays usable after each
    (`kill_query_side_connection_cancels_sleep`, the `max_execution_time` test).
- **Transaction status (HV-SESS-03).** `tx` comes from `SERVER_STATUS_IN_TRANS` in the last OK
  packet of each call. An ERR packet carries no status flags and never sets it. The brief's "an
  ERR keeps the previous value" is not exact on its own: `START TRANSACTION; <failing
  statement>` (read-write multi-statement) leaves the server in a transaction the previous
  value does not show, a deadlock (1213) rolls one back, and a failing DDL inside a transaction
  (`CREATE TABLE` of an existing table, 1050) has already committed it implicitly. So after a
  **server** error (the connection is still in step) the driver sends one `COM_PING` and
  reads the status flags of its OK packet; if that fails too, or after a transport error, the
  previous value stays. The Docker tests
  `a_failed_multi_statement_reports_the_transaction_it_opened` and
  `a_failing_ddl_inside_a_transaction_commits_it` fail without the re-read.
  - *Fix round 1 (review I1):* the re-read was a `DO 0` statement, which replaced the
    diagnostics area: `SHOW WARNINGS`, `SHOW ERRORS`, `GET DIAGNOSTICS`, `@@warning_count` and
    `ROW_COUNT()` no longer described the failed statement (often the only place a MySQL error
    names its real cause, such as a foreign-key 150/1005), and every error added a statement to
    the general and slow logs. `COM_PING` is not a statement and touches none of them
    (`a_failed_statement_keeps_its_diagnostics`, on MySQL and MariaDB).
- **Reset and identity.** `reset` is `Conn::reset` (`COM_RESET_CONNECTION`), after which
  `mysql_async` runs the `setup` statements again, so a read-only connection is
  `SET SESSION TRANSACTION READ ONLY` again (`reset_keeps_a_read_only_session_read_only`). It
  clears user variables, temporary tables, prepared statements and `GET_LOCK` locks, ends the
  transaction, and keeps the current database (Q15: `USE orders` survives). A server without
  the command is `Unsupported`. `backend_id` is the handshake's thread ID (`CONNECTION_ID()`).
- **Cancel.** `cancel_handle` stays `None`: the session sends `KILL QUERY <backend_id>` on a
  side connection it opens through the pipeline (Task 7, Q37), so the driver keeps no
  credentials for a connection of its own.
- **TLS summary.** `verified` is the outcome this connection's handshake recorded (`Pinned`,
  `CaOnly`, `Full`, `No`), read right after the handshake, never a guess from the mode.
- **Known limits.**
  - **Client certificates on the matrix.** The MySQL servers verify a presented client
    certificate against their own auto-generated CA (`ssl_ca = ca.pem`), whose key never leaves
    the container. `encrypted_pkcs8_client_key` therefore proves the decrypted identity reaches
    the server by its `unknown_ca` alert; signing in with a client certificate needs a target
    whose `ssl_ca` trusts a committed test CA (a Task 16/23 fixture).
  - **Until Task 14** the pipeline's `insecure_auth_for` is the fail-closed stub
    (`RefuseUnlessVerified` everywhere): a loopback MySQL `Require` profile (outcome `No`) is
    refused on a `caching_sha2_password` cache miss (`auth.insecure_method`).

## m2b-fix-f2 — hacivat-driver-mysql

Conformance finding F2 (Task 23): on MySQL 8.0 and 8.4 a read-only (or any) session of an
account without `SYSTEM_VARIABLES_ADMIN` that ran `SET GLOBAL …` or `SET PERSIST …` was told
"connection lost, outcome unknown", and the session reconnected; over `COM_QUERY` the server
answers `1227`.

- **Root cause: the server.** A packet capture shows MySQL answering the `COM_STMT_PREPARE` of
  such a statement with a bare 7-byte OK packet (`00 00 00 02 00 00 00`) instead of the
  12-byte prepare reply or an ERR packet. `mysql_async` cannot read it as a prepare reply
  ("can't parse: buf doesn't have enough data", an I/O error), drops the connection, and the
  server logs the client's hang-up as an aborted connection ("Got an error reading
  communication packets"). `mysql_async` is right not to accept the packet, so nothing is
  patched (no `§6`). A scan of 73 statements prepared as `hv_ro` on 8.0 and 8.4 found the
  same reply for exactly the `SET` forms that change a global or persisted variable:
  `SET GLOBAL`, `SET PERSIST`, `SET PERSIST_ONLY`, `SET @@global.…`, `SET @@persist.…`,
  `SET GLOBAL TRANSACTION …`, `SET @a = 1, GLOBAL …`, also inside `/*!…*/`. Every other
  refusal is an ERR packet (`1227`, `1142`, `1044`), every statement MySQL will not prepare is
  `1295`, and MariaDB 10.6–11.4 answers `1227` in both protocols.
- **Fix: a `SET` is never prepared.** `Protocol::Extended` (`exec_iter(sql, ())`) decides the
  route from the text first (`extended_route`): a `SET` runs over `COM_QUERY` (`query_iter`,
  rows counted), so the refusal is the server's `1227` with its message, on the same
  connection, and the status is read again by the `COM_PING` as for any server error. A `SET`
  binds no parameters here (none are ever bound), returns no rows and is one statement, so
  its effect is the same over either protocol; every other statement is prepared as before,
  so the server still finds what it will not prepare (`1295`: read-only `Unsupported`,
  read-write `COM_QUERY`).
- **Which texts are a `SET`.** Every reading of the text's start the server may make:
  whitespace, `#` and `-- ` comments (MySQL's `--` only before whitespace or a control
  character) and block comments (which do not nest) are skipped; an unversioned `/*!…*/` always
  runs; a version-gated `/*!NNNNN …*/`, and `/*M!…*/` (MariaDB runs it, MySQL skips it), that
  holds a statement is read both ways. A read-write connection runs the text as `COM_QUERY`
  when any reading is a `SET` (it may run a multi-statement text, as `Protocol::Simple` does).
  A read-only connection does only when every reading that holds a statement is a `SET`, so
  the prepared protocol's refusal of a statement it cannot prepare is never bypassed; a text
  read one way as a `SET` and another way as something else is `Unsupported` before anything
  is sent (`/*!99999 SELECT 1 */ SET …`). More than 64 readings stay prepared.
- Tests: `a_set_asked_for_as_a_prepared_statement_runs_as_text` (unit, every case above),
  `a_refused_set_global_is_a_server_error_on_a_live_connection` (Docker, every MySQL-family
  target, read-write and read-only: `1227`, MariaDB's `SET PERSIST` `1064`, the same backend,
  `Idle`, nothing changed). The session's F2 test moved to `docker_m2b_bypass.rs`.
- **Client certificates on the matrix (the `m2b-t13` known limit, closed).** `mysql84-x509`
  (compose and `targets.toml` `m2b-fix-f2-x509`) trusts a test CA that `mysql-x509-certs`
  generates at `docker up` into the artifacts directory (nothing committed), serves a
  certificate that CA signed, and has `hv_x509 … REQUIRE X509`. The session test
  `mysql_client_certificate_sign_in` (`docker_m2b_tls_ssh.rs`) signs in with the client
  certificate and its PKCS #8 key, and with the encrypted key and its passphrase, under
  `VerifyCa` with the test CA (`CaOnly`); without a certificate the server answers `1045`.

## m3a-t15 — SQLite typed

**One job per typed query** (`typed.rs`, the methods in `open.rs`). `query` takes a cancel
ticket and sends one job to the connection's thread; the job prepares, checks, and answers
`Started` (or the error), so `query` returns once the statement is prepared: SQLite's refusals
(syntax, two statements, the read-only authorizer, `readonly-required`) fail `query` itself,
errors while stepping (`SQLITE_BUSY`, constraints, the interrupt) come from `next_event`. The job
then steps the statement (`Statement<'conn>` on its stack: no self-referential type, no borrow
across threads) into `ChunkBuilder::with_schema` chunks and sends each one on a channel of depth
4. A consumer that stops reading stops the stepping: the job parks on the full channel, so the
memory of an unread result is the first chunk plus four (unit test
`memory_is_bounded_by_the_channel_depth`: 100,000 rows, the job parked after at most 901).

- `typed_caps() = { suspend: true, binds: false, close_needs_kill: false, idle_bound }`.
- **Suspension.** A suspendable query stops after `first_rows` rows, then after each
  `fetch_more(n)` batch of `n` (`0` means `chunk_rows`). At that read-ahead point the job steps
  one row more: no row → the result ends with no `Suspended` (the statement is finalized, so no
  cursor and no lock stay behind; `FakeDriver` behaves the same, PostgreSQL does not); a row →
  it is held as the next batch's first, `Suspended` goes out and the job blocks on its command
  channel (`FetchMore(n)` / `Close(how)`). Rejected: suspending exactly at the limit, as
  PostgreSQL does, which keeps a SHARED lock for a cursor that has nothing left to give. Cost of
  the read-ahead: when the next matching row is far away (a selective scan), the step that finds
  it runs before `Suspended`; the rows already sent are on screen, and a cancel stops the step.
- **Chunks.** Sealed at `first_rows` rows (the first), `chunk_rows` (later ones), `chunk_bytes`
  or `is_full()`, and at each batch's end. A statement without columns (DML, DDL) is one result
  with an empty schema; `ResultEnd.rows_affected` is `sqlite3_changes` for a statement that
  writes (0 when nothing changed, as M2b), none for a read; the command tag is the first keyword
  (Q35). Text without a statement is one empty result with no tag.
- **One open query per connection.** The job holds the thread while the query streams or waits,
  and any other job would queue behind a command only the same owner can send, so `execute`,
  `batch`, `ping` and `server_info` answer `Unsupported` while a query is open and `probe`
  reports nothing; a second `query` is `Unsupported`.
- **Closing.** `Close(Now)` (and `AfterKill`, never needed) finalizes; `Close(Drain(d))` steps
  on, counting rows, until the result ends (`Completed { rows }`) or `d` passes (`Closed`). The
  cancel latch gained a deadline (`SqliteCancel::set_deadline`) that the progress handler honours,
  so even one long step stops at `d`. The job answers after it finalized: the lock is gone when
  `close_query` returns. A `Close` while rows stream is seen at the next row (the job polls its
  command channel each row); chunks already in flight are dropped by `close_query`, so `Closed {
  rows }` counts what `next_event` handed out. A closed command channel (the connection dropped
  without `close`) ends the job and finalizes the statement; a closed event channel too. The job
  sees its channels only between rows, so dropping a connection with a query open also closes
  the cancel latch (`Drop for SqliteConn`), which stops a step that never yields a row.
- **Cancel.** The ticket covers the whole job, suspension included: the progress handler and
  `sqlite3_interrupt` stop a running step (the interrupt flag stays set while the suspended
  statement is active, so it also stops the next step), and the job reads the latch after
  prepare and when a batch resumes. A cancel never reaches the next query (the M2b latch rules).

**Types** (`decode.rs`). SQLite's five affinity rules on the declared type (ASCII case folding):
INTEGER → `I64`, TEXT → `Var(Text)`, REAL → `F64`; `BOOL`/`BOOLEAN` → `Bool` while a value is 0
or 1; `DATE`/`DATETIME`/`TIMESTAMP` → text verbatim; other NUMERIC, BLOB affinity, no declared
type and expressions → the storage class of the column's first non-NULL value in the first chunk
(fallback `Int` for NUMERIC, `Bytes` for the rest). Until every such column has a value the rows
are held as whole `Value`s and replayed once the plan is known (usually at the first row), so
`ResultStart` waits for that. A value whose storage class is not its column's is a whole-value
exception of its own class (`TEXT` `'abc'` in an `INTEGER` column reads back as text); integers
are never widened to floats. Text and blobs go through `push_bytes` or
`ChunkBuilder::capped_value` (cell cap, total length, `Lossy` for text that is not UTF-8, never
replaced). `NativeType::Sqlite { decl, affinity }`; `OriginHint::Sqlite { database, table,
column }` from the `column_metadata` feature (workspace dependency), captured for M6.

**Hostile files (R12, P28).** rusqlite `expect`s on metadata that is not UTF-8
(`column.rs:137-147, 181-212`). Every read runs inside `catch_unwind` on the connection's thread:
the name (`exec::column_name`, also used by M2b's `execute` now), the declared type
(`columns().nth(i)`, which maps only that column) and the origin. A valid name is kept when only
the declared type is unreadable, and the column is `Opaque` (`NativeType::Unknown`, a `Values`
buffer holding each value whole, of its own class); an unreadable name becomes `?` and, because
rusqlite reads the name first, its declared type is unreadable too; an origin with any
unreadable part is left out. Cell values have no such exposure (rusqlite asserts only on NULL
pointers, which is out of memory). The caught panic's message is printed by the default hook;
the connection stays usable. The workspace builds with `panic = "unwind"` everywhere.

**Read-only hardening, unchanged on the typed path.** The job clears the authorizer trail,
prepares (the authorizer judges here), then requires `sqlite3_stmt_readonly` unless the
statement was exactly the journal-mode query (Q16), as `execute` does; `query_only`,
`LIMIT_ATTACHED` 0 and the attach settings are untouched. A statement with a parameter is
`Unsupported` on the typed path (binds are M3b; an unbound parameter would run as NULL, which can
silently change what a `DELETE … WHERE id = ?` does). `Protocol::Simple` and binds are
`Unsupported`.

**Busy (R4).** After the open the busy timeout is 5 s (`EXEC_BUSY_TIMEOUT`; the header read
still waits the connect timeout). `SQLITE_BUSY` stays the engine's error (`Db`, extended code,
text verbatim) with no retry; `hacivat_driver_sqlite::error_hint` maps it to the Fluent id
`error-exec-sqlite-busy-hint` (`hacivat-core.ftl`, EN and TR), which the error view shows under
the server text. The id is in the `error-` family: the catalog grammar
(`hacivat-l10n-schema::is_valid_id`) does not admit `exec-` yet. `ServerMessage::hint` (the
server's own hint) stays empty: a Hacivat sentence there would be shown as server text. The
class stays `internal` (the M2a catalog's `5 = "internal"`):
the taxonomy has no "locked" class, and adding one is not this task's. A cancel during a busy
wait takes effect only when the wait ends (SQLite's busy handler sleeps without checking the
interrupt), so up to 5 s.

**Idle bound (ER6).** A live statement holds a SHARED lock in a rollback-journal mode, so no
other process can commit: `idle_bound` is 30 s. It is `None` only when every database with a
file (`PRAGMA database_list`) is in WAL mode, where a cursor only keeps a checkpoint from
completing; a database without a file (`:memory:`, `temp`) holds no lock another process sees.
An attached rollback-journal file makes it 30 s again; modes that cannot be read make it 30 s
(fail closed). Read when the connection opens, on `probe`, and by every typed query after
prepare, so the bound read after `query` is the open cursor's.

**Known limits.** The read-ahead step can be long (above). `typed_caps()` read between queries
reflects the last query's start; a `PRAGMA journal_mode = …` takes effect at the next query.
`OriginHint` needs a table column (expressions have none).

**Tests.** `tests/typed.rs` (19 + 1 perf): the million-row suspend/resume/finalize (first chunk
501, `fetch_more(10000)` → 4,096/4,096/1,808, `Closed { rows: 10501 }`, another connection can
commit at once); whole results without suspension; sealing by rows and bytes; the read-ahead end;
DML/DDL; storage-class exceptions; the affinity table; declared types, native types and origins;
large and invalid text; the read-only authorizer refusing `ATTACH`, writes and `VACUUM` on the
typed path; the interrupt during one long step, at a suspended cursor and while the consumer
stalls; `SQLITE_BUSY` with the hint (`PRAGMA busy_timeout` = 5000); a closed command channel
ending the job; a dropped connection stopping one endless step, also while a drain waits on it;
one open query per connection; drains (complete, endless rows, one endless step); the idle bound
per journal mode; `Simple`, binds and parameters refused. Calls that a regression would block
forever are wrapped in a two-minute guard (`unhung`), so they fail instead of hanging. The perf
test (`HACIVAT_PERF=1`, release) asserts the first chunk of a million rows within 20 ms (median
of 10). `tests/typed_hostile.rs` (7): a declared type and names that are not UTF-8, the M2b
path, an origin through a view of a table whose name is not UTF-8, 300 generated cells kept
exactly, and every decoder over every storage class. Unit tests: the memory bound
(`open.rs`), the busy hint's codes (`lib.rs`), the limits and caps (`typed.rs`). Mutation
checks: each of the column-name guard, the declared-type guard, the parameter refusal, the
channel depth, the busy timeout and the drain deadline, removed, fails its test.
