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
