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
