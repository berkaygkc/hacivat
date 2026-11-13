# ADR 0004 — Sessions and the read-only guard

- Status: Accepted
- Date: 2026-11-13
- Scope: M2b (HV-SESS-01 to -08, HV-CONN-04, -05, SQL-SAFE-05, MCP-09 step 1, D-035)

## Context

M2a holds one connection per profile and labels read-only connections without enforcing them
(R10). M2b turns connections into sessions:

- a session actor per connection, with a request queue, transaction state, a cancel ladder,
  environment replay and schema tracking;
- a `SessionManager` per profile that routes requests to the default session, client
  sessions, pools and jobs;
- liveness and reconnect with an honest account of the state a reconnect loses;
- a read-only guard in layers per engine (`hacivat-guard`, sans I/O), a role probe and an
  access label that never claims more than the server enforces.

These choices decide what a user's statement can do to their database, so every one is
recorded here by the task that makes it.

## Rule

Each M2b task that makes a choice in this area appends one section headed
`## m2b-tNN — <topic>` (append-only; a choice made in a fix round goes into the same task's
section before re-review). A section states the choice, the alternatives it rejected, how it
fails closed, and the tests (or bypass-suite results) that hold it. Topics, in the order of the
plan: the actor model, transaction state, the cancel ladder, the reconnect policy, the guard
layers per engine, the labels, the role probe and the bypass-suite results.

## m2b-t07 — session actor

**Actor model (HV-SESS-01).** One Tokio task per connection owns the `LiveConnection`. Requests
run one at a time from a FIFO `VecDeque`; while one runs the actor still reads commands (new
requests queue, cancels climb the ladder, a close waits for the request). The request future is
polled first (`biased` select), so its statement has started before any cancel is read. The
state is a `watch` of `SessionSnapshot` whose generation bumps on every change; events go out
on a `broadcast`.

- A panic in a driver call is caught (`AssertUnwindSafe` + `catch_unwind` on the request
  future): the session becomes `Broken(Internal)`, the connection is closed and discarded, and
  the panic message is logged, never the request's SQL. A second guard around the whole actor
  marks the snapshot `Broken(Internal)` if the actor itself panics. Each session has its own
  task, so other sessions are unaffected.
- A lost connection (`Io`, `Protocol`, `Timeout`, `Tls` during a request) breaks the session
  (`ConnectionLost { outcome_unknown }`); a broken session is never reused: every later request
  is `NotConnected` until Task 17's reconnect replaces it.
- A session is read-only when its spec **or** its profile says so (fail safe). Read-only
  statements go only through `readonly::run_guarded(.., replay)` with `replay` = the
  environment plan without the PostgreSQL client identity (Q15); the Task 1 stub refuses, so
  nothing runs unguarded before Task 18.
- Spawning runs neither the environment plan nor a schema probe (Q27). One exception, a
  safety one: a read-write MySQL session that starts Manual sends `SET autocommit = 0`, so the
  server never autocommits under a Manual badge. **Q27 clarification (fix round 1):** this is
  mode enforcement (B22), not the environment plan: it is internal, never re-probes, is
  re-applied after a reset, and its failure breaks the session.
- Requests and closes share one FIFO: a close never jumps ahead of a request queued before
  it, so a queued `COMMIT` runs before a close looks at the transaction (fix round 1, M2).
- The access label follows the session: a read-only session is never labelled `ReadWrite`,
  even when its connect was asked for read-write (fix round 1, M4).
- Driver calls outside the request (`backend_id`, `cancel_handle`, the spawn-time
  `autocommit`) are guarded too; a panic there breaks the session the same way. A panic
  message is logged redacted when `carries_secret` flags it, and cut to 200 characters.
- Rejected: one task per request (no single owner of the connection, harder cancel races);
  `JoinError`-based panic capture through `spawn` (would move the connection into a second
  task and lose it on panic anyway, with an extra hop per request).

**Transaction state (HV-SESS-03, B22, Q14).** The status always comes from
`RawConn::tx_status()` after every round trip, never from parsing SQL. Manual mode on
PostgreSQL and SQLite sends `begin_sql(..)` before a statement when the status is `Idle`
(not before `BEGIN`/`START`/`COMMIT`/`END`/`ROLLBACK`/`ABORT`, and never before Hacivat's own
`ExecOrigin::Internal` statements, which must not open the user's transaction); SQLite uses
`BEGIN IMMEDIATE`
unless the first keyword is `SELECT` or `WITH`. MySQL switches `autocommit` at `SetMode`.
Manual → Auto with an open or failed transaction is `InProgress`; `SetIsolation` is refused
the same way and applies to the next `begin_sql` (MySQL's implicit Manual transactions keep the
server's level: a known limit). `Smart` needs the classifier (M3): `SetMode(Smart)` is
`NeedsClassifier`, and a profile that asks for Smart starts Manual (the safer mode) until then.
In `Failed`, any statement whose first keyword is not `ROLLBACK` is refused before it is sent
(advisory; the server refuses anyway). Read-only sessions are always Auto: `SetMode(Manual |
Smart)`, `Begin`, `Commit` and `Rollback` are `Unsupported`.

**Cancel ladder (HV-SESS-05, B23, Q18, Q37).** `cancel(seq)` acts only on the running request
(the stale-cancel guard): anything else is `AlreadyFinished`. The first rung drops the queue
(`Cancelled`) and cancels through the connection's `CancelHandle`; MySQL has none and runs
`KILL QUERY <thread id>` on a side connection (`SessionHooks.side`), pre-opened once a
statement has run 500 ms so a cancel through SSH lands within about a second. Callers get the
outcome within 2 s (`CANCEL_WAIT`), else `StillRunning`; at 10 s the session reports
`SlowCancel`. `Terminate` runs `SELECT pg_terminate_backend(<pid>)` or `KILL CONNECTION <id>` on
a side connection; `Deactivate` drops the request and the connection and marks
`Broken(Cancelled)`. PostgreSQL `57014` and MySQL `1317` count as cancelled only once a cancel
was asked for (a statement timeout also answers `57014`). Once a rung was climbed, a request
that ends with any error answers its callers `Cancelled` (a successful terminate ends it with a
lost connection). **Cancel race (corrected in fix round 1, review C1):** a cancel or kill still
in flight when its request ends is awaited for up to 2 s before the next request starts. If it
has ended, the session goes on. If not, it is aborted (with the side connect or pre-opened
connection it waits for) and awaited, and since what it already sent may still land, the
session is marked `Broken(Cancelled)` and its connection closed: the next request never runs
on it, and the session goes the lost/reconnect way (Task 17). A late kill therefore costs a
reconnect, never another statement. The server-side race of a protocol cancel that the server
processes after the statement ended remains the server's (PostgreSQL's CancelRequest is
fire-and-forget). Known cost (review M3, not changed): the finished request's reply also waits
for that bound, because the snapshot is published before the reply.

**`KILL` limits behind proxies (Q37).** `KILL QUERY`/`KILL CONNECTION` name a server thread ID
read at connect. Behind a connection-multiplexing proxy (ProxySQL, MaxScale, RDS Proxy,
Vitess) the side connection may reach a different backend, where the ID names another
session or none: the kill may fail (`Failed(class)`) or, in the worst case, hit an unrelated
session of the same user on that backend. Hacivat sends a kill only for the request it is
running, only by the ID its own connection reported, and never retries a failed kill. Users
behind such proxies should prefer `Deactivate`. The same holds for PostgreSQL behind
transaction-pooling PgBouncer (`pg_terminate_backend` by a pooled PID).

**Environment plan (HV-SESS-06, Q8, Q15, Q27).** In order: the client identity (PostgreSQL
`SET application_name`; MySQL's connect attribute carries it), the read-only setup of
`hacivat_guard::plan_for(kind).session_setup` for read-only sessions, no time zone until M3,
and the last schema only when one is known. Every literal or identifier goes through its own
quoting function: `quote_pg_ident` (`"` doubled), `quote_my_ident` (`` ` `` doubled),
`quote_pg_literal` (`'` doubled; an `E''` literal with doubled backslashes when the value has
one, so it reads the same whatever `standard_conforming_strings` is) and `quote_my_literal`
(`'` and `\` doubled). NUL is stripped from literals; a schema name with NUL (or an empty one)
is refused, never sent, because the server would cut its C string there.

**Schema tracking (HV-SESS-07, Q27).** The probe (`current_database()`, `current_schema()`,
`current_schemas(true)`, `current_user`; MySQL `DATABASE()`, `CURRENT_USER()`; SQLite
`PRAGMA database_list`) runs only after a user statement whose first keyword (ASCII-folded,
after whitespace and comments) is `SET`, `RESET`, `USE`, `BEGIN`, `START`, `COMMIT`, `END`,
`ROLLBACK`, `ABORT` or `DISCARD`, after a user commit or rollback, and after `SwitchSchema`;
`SchemaChanged` fires on a difference. Never after internal control statements or a reset.
Cost (accepted): the context can lag behind a change made by any other statement.

**Plan amendment (controller ruling on review M11): PostgreSQL restores the whole
`search_path`.** The plan's `RestoreSchema` emitted `SET search_path TO "<current_schema()>"`,
which narrowed a path such as `"$user", public` to one schema on every reset and every guarded
read-only statement. The PostgreSQL probe now also reads `current_setting('search_path')` (a
fifth column), parsed as the server reads it (commas outside double quotes split; a quoted
element is unquoted, `""` is `"`; an unquoted one is trimmed and ASCII-lower-cased) into
`SchemaCtx.search_path_setting`. The environment plan then carries
`EnvStep::RestoreSearchPath(list)` and emits `SET search_path TO "e1", "e2"`, each element
through `quote_pg_ident` (`"$user"` is PostgreSQL's documented spelling of the token, so
`$user` stays the token and is never resolved). An empty list, an empty element or a NUL is
never sent; the plan then falls back to `RestoreSchema(current_schema)` as before, as it does
when the setting is unknown and on MySQL. Tests:
`the_search_path_setting_is_restored_whole_with_user_kept`,
`the_setting_is_parsed_like_postgresql_reads_it` and
`a_two_element_search_path_survives_a_reset_and_a_reprobe` (a name that needs quotes). The
queue cap (review M12) is parked to M3, unchanged.

**Reset (HV-SESS-08, Q10, Q19).** `ROLLBACK` when not `Idle`, `RawConn::reset()`, then
PostgreSQL's verify probe `SELECT current_user = session_user`, which must answer true
(no row fails closed), then the full environment plan (identity included) and MySQL's
`autocommit = 0` for a Manual session; mode and isolation return to the session's start.
Any failure breaks the session and closes the connection.

**Tests.** `crates/core/hacivat-session/tests/{session_actor,tx,cancel,env_schema}.rs` (53
tests over `FakeDriver` through the real connect pipeline; fix round 1 adds
`a_late_kill_never_reaches_the_next_request`, `a_terminate_that_ends_the_request_reports_cancelled`,
`a_close_waits_for_requests_queued_before_it` and
`internal_statements_never_open_a_manual_transaction`). The fake now latches a cancel sent
between `execute` and its first poll, and drops one sent while nothing runs (T1 review M10;
`hacivat-driver` `fake::tests::an_early_cancel_is_latched_and_an_idle_cancel_is_a_no_op`).

## m2b-t05 — guard layers per engine

`hacivat-guard` decides; it does no I/O and has no async or runtime (`cargo xtask check-deps`,
`GuardNotRuntimeFree`). The session (Task 18) and the SQLite driver (Task 11) run what it
decides. Every function fails closed: an engine, answer or name it does not know is an
escape, a denial or a role that is not least privilege, never a pass.

**Plans (`plan_for`, SQL-SAFE-05; values shipped by Task 1 under Q8 and tested here unchanged).**

- PostgreSQL: session setup `SET default_transaction_read_only = on`,
  `standard_conforming_strings = on`, `client_encoding = 'UTF8'`,
  `idle_in_transaction_session_timeout = '30s'`; around each statement
  `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` and `SELECT 1` (the `SELECT 1` pins the
  read-only snapshot, #pg-ro-pin), the canary `SELECT txid_current_if_assigned()`, then
  `ROLLBACK`, never `COMMIT` (S-02). Reset `DiscardAll`: the name is kept, and the driver runs the
  `DISCARD ALL` sequence without `DEALLOCATE ALL` (Task 10, Q10), after the `ROLLBACK`, outside
  any transaction block (#pg-discard). It clears the same session state S-02 and #pg-discard
  require (temp objects, `LISTEN`, held cursors, advisory locks, GUCs, role); prepared
  statements are deallocated by name instead, so the driver's own statement cache survives.
- MySQL and MariaDB: `SET SESSION TRANSACTION READ ONLY`, `SET NAMES utf8mb4`; around each
  statement `START TRANSACTION READ ONLY` and `ROLLBACK`; no canary (MySQL has no
  "transaction ID assigned" probe that a read-only transaction can run); reset
  `COM_RESET_CONNECTION`.
- SQLite: no statements. Its layers are connection-level (below).
- Every other engine: `None`. A read-only session on it must not run.

**Post-checks (`check_post`).** A tripwire after the server's own enforcement, never instead
of it. In order: (1) PostgreSQL, MySQL, MariaDB: the transaction status after the statement is
not `InTx` or `Failed` (so `Idle`, or unknown when the session did not record it), or the
guard's `ROLLBACK` drew SQLSTATE `25P01` → `TransactionEndedByStatement`; (2) a write tag
succeeded → `WriteSucceeded`; (3) the canary returned a value (`Some(Some(_))`, any text, even
empty) → `CanaryAssigned`. SQLite (autocommit, `Idle` is normal) gets step 2 only. An engine
without a plan is always a gap. The tag check reads the first word of the command tag, case
folded (PostgreSQL tags; MySQL and SQLite report the first keyword, Q35):

- `INSERT`, `UPDATE`, `DELETE`, `MERGE`, `COPY` with `rows_affected > 0`. `COPY` covers both
  directions because the tag cannot tell `COPY … TO` from `COPY … FROM`, so a `COPY … TO`
  that returns rows is reported too (fails closed). Added beyond the plan's list: MySQL's
  row-writing verbs `REPLACE` and `LOAD` (`LOAD DATA`).
- `CREATE`, `DROP`, `ALTER`, `TRUNCATE` succeed as a gap **with or without a row count**:
  their tags never carry one (`CREATE TABLE`, `TRUNCATE TABLE`; MySQL reports 0), so the plan's
  literal "`rows_affected > 0`" rule could never fire for them. Added beyond the plan's list:
  `RENAME`, `GRANT`, `REVOKE`. None of these can succeed inside a read-only transaction, so
  there are no false positives.
- MySQL and MariaDB only (fix round 2, review M-1): `WITH`, `CALL`, `DO` and `SELECT` with
  `rows_affected > 0`. Each can write: `WITH … UPDATE/DELETE`, a procedure, a function with side
  effects, or `SELECT … INTO OUTFILE/DUMPFILE`. MySQL has no canary, so the tag is its only
  post-check signal. A statement that returns a result set reports 0 affected rows in its final
  OK packet, so a positive count is a write. **Task 13 contract:** `rows_affected` comes from
  the server's OK packet (mysql_async `affected_rows()`), never from the number of rows
  returned (that is `row_count`). Otherwise every MySQL `SELECT` would trip the gap: loud, but
  closed. The first keyword must skip leading comments and parentheses the way Task 18 step 1a
  does, or `/* x */ INSERT …` yields no tag. PostgreSQL is left out: its `SELECT n` tag counts
  the rows returned. `INTO OUTFILE` stays primarily the role probe's job (`FILE` →
  `ServerFileRoles` → guarded), and so do `COPY … TO PROGRAM`, `lo_export` and `dblink_exec`,
  which no read-only transaction stops.

  Known cost (review m-6): these tags also close the session in two cases that write nothing,
  which fails closed:
  - MySQL `SELECT … INTO @var` reports 1 affected row;
  - a `CALL` whose procedure ends with a statement that reports a row count.

  The alert then says the statement changed data. Both are rare in a read-only session, and
  Task 18's copy review should know about them.

Rejected: trusting `rows_affected` alone (DDL has none); treating an unknown transaction state
as clean (the session may not have recorded it, and the guard cannot then confirm that the
transaction survived).

**Input limits (`check_input`).** At most `MAX_STATEMENT_BYTES` (1 MiB) in UTF-8 bytes, not
characters, and no NUL (C-string APIs would cut the statement there and run a different one).

**SQLite (B21).** Hardening: `query_only`, `SQLITE_DBCONFIG_DEFENSIVE` on, `ATTACH_CREATE` and
`ATTACH_WRITE` off, `LIMIT_ATTACHED` 0, `LIMIT_LENGTH` 100,000,000, `LIMIT_SQL_LENGTH` 1 MiB (the
input limit), `LIMIT_VARIABLE_NUMBER` 32,766. The authorizer denies by default and allows only
`Read`, `Select`, `Recursive`, functions other than `load_extension`, `readfile`, `writefile`,
`edit` and `fts3_tokenizer`, the pragmas of `SQLITE_READ_PRAGMAS` **without an argument**
(Q16: `query_only`, `journal_mode`, `trusted_schema` and `foreign_keys` are readable; with an
argument each is a setter and stays denied), and the six `SQLITE_INTROSPECTION_PRAGMAS`
(`table_info`, `table_xinfo`, `index_list`, `index_info`, `index_xinfo`, `foreign_key_list`)
**with** a name argument (see the controller ruling below). Names are compared with ASCII case folding
(INV-3) and must match whole: a schema prefix (`main.query_only`), padding or a NUL is denied
(SQLite passes the bare pragma name and the schema separately, so this never denies a real
read). The `match` names every `SqliteAction`, so a new action needs a decision; `Other(code)`
is always denied. The authorizer returns `Deny`, never `Ignore`: `Ignore` turns a denied column
read into NULL and lets the statement run.

*Controller ruling (fix round, on Q16).* The "no argument" rule is for the setting pragmas.
The six introspection pragmas are only useful with a table or index name, so they are allowed
with an argument, in every form a user writes them. The sqlite3 shell's `.auth on` trace
shows what the authorizer receives:

- `PRAGMA table_info(t)`, `pragma_table_info('t')` → `PRAGMA "table_info" "t"`;
- `PRAGMA main.table_info(t)`, `pragma_table_info('t','main')` →
  `PRAGMA "table_info" "t" "main"` (the table-valued form prepares the same pragma and asks
  the authorizer again; the schema is a separate argument the adapter does not pass on);
- `PRAGMA main.query_only` → `PRAGMA "query_only" NULL "main"`.

The ruling also asked to refuse these six with a value assignment (`= …`). The authorizer
cannot do that: `PRAGMA table_info = t` arrives as `PRAGMA "table_info" "t"`, exactly like
`PRAGMA table_info(t)`. It does not need to: none of the six has a setter, and the `=` form
reads table `t` the same way. Assigning to a pragma that has a setter stays denied, because
every other pragma is refused whenever it carries an argument. A text-level refusal of
`= …` would have to sit in Task 11's adapter, which sees the SQL. It would add no safety.

**Role probe (MCP-09 step 1).** PostgreSQL runs five statements:

1. the role flags;
2. membership in `pg_read_server_files`, `pg_write_server_files`,
   `pg_execute_server_program`, `pg_signal_backend` and `pg_write_all_data`;
3. any user relation, sequence or schema the role could write;
4. TEMP on the database;
5. the `dblink`, `postgres_fdw`, `pg_net` and `http` extensions.

Least privilege is all of: no flag set, no predefined-role row, nothing writable, no egress
extension. TEMP adds `TempPrivilege` and keeps least privilege (Q21: the read-only transaction
already refuses `CREATE TEMP`, #pg-ro-temp; the reset drops temp objects).

*Fix round 2 (review I-1; controller ruling: the plan's rows 1 and 3 were a plan defect).*
The plan's row 1 read the attributes of `current_user` alone, and its row 3 checked only
ownership through `USAGE`, table-level privileges and tables. That missed:

- role attributes, which are never inherited, so `GRANT postgres TO hv_ro` reads clean while
  one `set_config('role', 'postgres', true)` in a guarded statement makes it a superuser, and
  `pg_read_file`, `lo_export` or `COPY … TO PROGRAM` then run inside the read-only transaction;
- column-level grants (`GRANT UPDATE (email) ON users`), which `has_table_privilege` ignores;
- `CREATE` on a schema: `public` on clusters created before PostgreSQL 15 or upgraded from one,
  and the database owner (through `pg_database_owner`) on 15+;
- sequences (`USAGE`/`UPDATE` let `nextval`/`setval` write).

The corrected statements keep the row shapes (one row of four booleans; one boolean), so
`interpret_probe` and Task 18 are unchanged.

- Row 1 is `bool_or` of each flag over every role `current_user` is a `MEMBER` of. That is
  always one row: a role is a member of itself, and a NULL fails closed as `Unparsed`.
- Row 3 is true when any member role:
  - owns a user relation (`r`, `p`, `v`, `m`, `f`, `S`);
  - holds `INSERT`/`UPDATE`/`DELETE`/`TRUNCATE` on one, or column-level `INSERT`/`UPDATE`
    (`has_any_column_privilege`);
  - holds `USAGE`/`UPDATE` on a sequence (`has_sequence_privilege`, behind a `CASE` because
    it errors on a non-sequence);
  - or holds `CREATE` on a schema outside `pg_*` and `information_schema`.

  Schemas are filtered with `!~ '^pg_'`, not `LIKE 'pg\_%'`, so the filter does not depend on
  `standard_conforming_strings`.

Validated on throwaway `postgres:13`, `:15` and `:18` containers (not the M2a matrix), as
`hv_ro`. The corrected rows flag every one of these (row 1 all true for the superuser
membership; row 3 true for all of them):

- a member of a superuser role;
- a column-level `UPDATE`;
- sequence `USAGE`;
- sequence `UPDATE`;
- `CREATE` on `public`;
- a member of a role with `INSERT`;
- a member of a table's owner;
- the database owner.

They stay clean for a `SELECT`-only role and for `pg_read_all_data` (15 and 18). On
PostgreSQL 18 the plan's rows read five of them as all clean, which would have shown
**Read-only (enforced)**:

- the superuser membership;
- the column-level `UPDATE`;
- sequence `USAGE`;
- `CREATE` on `public`;
- the database owner.

The plan's row 3 does catch inherited ownership and inherited table privileges.

Consequence: on a cluster where `PUBLIC` still holds `CREATE` on `public` (PostgreSQL ≤ 14, or
upgraded from one), the remediation role reads as `WritePrivileges` (guarded) until
`REVOKE CREATE ON SCHEMA public FROM PUBLIC`, because any role there can create tables. The
remediation now offers that revoke as an optional comment line (see Remediation below). Task 18's Docker run
should cover:

- a superuser membership;
- a column-level `UPDATE`;
- `CREATE` on `public`. MySQL and MariaDB run `SHOW GRANTS`; least
privilege is every `GRANT` line within `USAGE`, `SELECT`, `SHOW VIEW` (a column list such as
`SELECT (a)` counts as `SELECT`), with no `WITH GRANT OPTION`/`WITH ADMIN OPTION` and no
`PROXY`. A role grant is `RoleGrantNotExpanded` (fails closed: the probe does not expand it).
`REVOKE` lines (partial revokes) only remove privileges and are skipped. The line is tokenized
with quotes (`` ` ``, `'`, `"`, doubled-quote and backslash escapes) and parentheses, so a
quoted word is never read as a keyword. Privileges map to warnings (`ALL` on `*.*` and
`SUPER` → `Superuser`; `FILE` → `ServerFileRoles`; replication privileges → `Replication`;
`CREATE USER`, role administration, grant option, `PROXY` → `CreateRole`; `CONNECTION_ADMIN`
→ `SignalBackend`; data and schema privileges, `EXECUTE` and `LOCK TABLES` →
`WritePrivileges`; `CREATE TEMPORARY TABLES` → `TempPrivilege`, which on MySQL does **not**
keep least privilege, per the plan's three-privilege rule).

`Unparsed`, and not least privilege, for: the wrong number of row sets; a row set of the wrong
shape (missing, extra rows, extra columns, NULL); a boolean other than `t`/`f`/`true`/`false`;
a role or extension name outside the probe's own lists; an empty `SHOW GRANTS`; a line that is
not `GRANT`/`REVOKE`; an unterminated quote, unbalanced parentheses, an unquoted `;` or an
unquoted non-ASCII character (fix round 2, M-2: a no-break space used to read as a role
grant); no
`TO`; a second `ON`/`TO` after the grantee; an empty or quoted privilege; a privilege without a
row in the table (`PROCESS`, `SHOW DATABASES`, dynamic `*_ADMIN` privileges); any engine
without a probe. An `EgressExtension` warning carries the name from the probe's list, never
the server's text.

*Round 3 (review m-4, controller ruling).*

- Row 2 also lists `pg_maintain`, mapped to `WritePrivileges`. On PostgreSQL 17+ it allows
  `ANALYZE`, `VACUUM`, `REFRESH` and `LOCK … ACCESS EXCLUSIVE` on every relation, and `LOCK`
  runs inside a read-only transaction (S-10). On older servers the name simply never matches.
- Row 3 also turns true when a member role has `rolcreatedb`, or `CREATE` on the current
  database (`has_database_privilege`, which allows new schemas). These are folded into row 3
  because row 1 must keep its four columns. A finding reads as `WritePrivileges`.

Validated on throwaway PostgreSQL 13, 15 and 18:

- `CREATEDB`, membership in a CREATEDB role and database `CREATE` each set row 3;
- row 2 returns `pg_maintain` on 18;
- 13 and 15 accept the `IN` list without it;
- `pg_read_all_data` and a SELECT-only role stay clean.

Still not probed: `USAGE ON FOREIGN SERVER`, egress FDWs outside the fixed row-5 list, and
`SECURITY DEFINER` functions that `PUBLIC` can execute. Their writes are DDL or rely on the
read-only transaction (L4) failing.

*Large objects (review m-5).* No role setting prevents them. Any role may call `lo_create`,
and large-object ownership and `UPDATE` grants are not probed. Only the read-only transaction
(L4) stops large-object writes: `lo_create`, `lo_put`, `lo_import` and `lo_unlink` fail with
`25006` inside it. Task 23's bypass suite asserts this. `lo_export` writes a server file and
needs `pg_write_server_files` or superuser, both of which the probe reports.

**Labels (`access_label`, D-035).** Not read-only → `ReadWrite`. Read-only →
`ReadOnlyEnforced` only for PostgreSQL or MySQL whose findings say least privilege **and**
carry no warning that denies it (only PostgreSQL's `TempPrivilege` is neutral), so
self-contradicting findings are guarded. Everything else is `ReadOnlyGuarded`: MariaDB (C19:
guarded until it passes the bypass harness in the Docker matrix), SQLite, other engines, no
probe, a failed probe. Rejected: labelling MariaDB enforced on a clean `SHOW GRANTS`.

**Remediation.** PostgreSQL: `CREATE ROLE <role> LOGIN PASSWORD '<password>'`,
`GRANT pg_read_all_data TO <role>`, and `-- optional: REVOKE TEMP ON DATABASE <database> FROM
PUBLIC;` as a comment (revoking from `PUBLIC` affects every role, and TEMP alone keeps the label
enforced). Then `-- optional: REVOKE CREATE ON SCHEMA public FROM PUBLIC;`, also a comment
(controller ruling, fix round 2). Where `PUBLIC` still holds `CREATE` on `public` (clusters
created before PostgreSQL 15 or upgraded from one), the corrected probe reads every role as
`WritePrivileges`, so this line is what lets the remediation reach the enforced label there.
It stays optional because it affects every role, and it changes nothing on clusters that
never granted it. The approved mock (`design/m2b-secure/NOTES.md` and `index.html`) shows the
same string.

*PostgreSQL before 14 (review m-7, controller ruling).* `pg_read_all_data` does not exist
before 14, so the string above fails on PostgreSQL 13. `remediation_for(kind, major)` takes
the server's major version:

- below 14 it returns `CREATE ROLE …`, then `GRANT USAGE ON SCHEMA <schema>`,
  `GRANT SELECT ON ALL TABLES IN SCHEMA <schema>`,
  `GRANT SELECT ON ALL SEQUENCES IN SCHEMA <schema>` and
  `ALTER DEFAULT PRIVILEGES IN SCHEMA <schema> GRANT SELECT ON TABLES`, plus the same two
  optional comments. This adds a `<schema>` placeholder.
- 14 and later, and an unknown version, return the string above.

`remediation(kind)` is kept, because `hacivat-session` already re-exports it, and returns
the 14+ form. The mock shows the 14+ form. Validated on PostgreSQL 13, 15 and 18: the 13 form
reads as least privilege, and its default privileges reach a table created afterwards.

No `NOINHERIT`: a non-inheriting member would not get `pg_read_all_data` and could
read nothing (Q21). MySQL: `CREATE USER '<role>'@'%' IDENTIFIED BY '<password>'`,
`GRANT SELECT, SHOW VIEW ON <database>.* TO '<role>'@'%'`, which the probe reads as least
privilege (tested). MariaDB, SQLite and every other engine: `None`, since no role change makes
their label enforced. The strings carry only the `<role>`, `<password>` and `<database>`
placeholders; the UI shows and copies them unchanged.

**Tests.** `crates/core/hacivat-guard/tests/{plans,post,sqlite_authorizer,probe,labels}.rs`
(60 tests): exact plan values; every gap and its order; unknown transaction state; tags in any
case, with and without row counts; MySQL `WITH`/`CALL`/`DO`/`SELECT` with affected rows, and
PostgreSQL `SELECT n` left alone; byte-counted input limits; every pragma of the list without
an argument and every setting pragma with one, in both cases; the six introspection pragmas
with a name argument; malformed and look-alike names; every write, attach and raw
action code (-1..=40 and the `i32` extremes) denied; every PostgreSQL warning and every
malformed probe shape; the PostgreSQL 13/15/18 answers of each fix-round-2 scenario; a NULL
aggregate row; MySQL non-ASCII outside quotes; MySQL grant lines with odd whitespace, case, quoting, column lists,
role grants, grant options, partial revokes and a smuggled second statement; labels, including
self-contradicting findings; the exact remediation strings.

## m2b-t17 — liveness and recovery

**Supervisor (HV-CONN-04, B18, Q31).** A pure `Supervisor` decides; the default session's actor
ticks it every second (Tokio `interval`, `MissedTickBehavior::Skip`) while idle. Rules, in
order: busy → nothing (a running request is activity); a clock jump (`wall_delta − mono_delta >
5 s` or `mono_delta > tick + 5 s`, since a Windows `Instant` counts suspended time) →
`Suspect` once, and the actor ticks again at once: with the transaction idle the idle threshold
is compared against the time since the last activity on the wall clock or the monotonic one,
whichever is longer (`Disconnect`), else `Ping`; with a transaction open it is always `Ping`
(one immediate check, never a disconnect; fix round 1, M1); missed ticks are never replayed.
At the idle threshold an open (`InTx` or `Failed`) transaction gets `IdleTransaction` once per
idle period (D-014: notify only), otherwise `Disconnect`. A keep-alive is due from the later of
the last activity and the last ping. Activity is a user statement, a transaction command or a
schema switch; pings, internal statements and resets are not, so neither keep-alives nor
background introspection keep an idle production session open. Time the actor spends away from
its ticks (a ping, a reconnect) is resynced, never taken for a jump. The monotonic clock is
Tokio's, so a paused test clock drives it. Rejected: OS sleep notifications and the App Nap
opt-out (platform FFI, M9).

**Policies.** `LivenessPolicy::for_profile`: the profile's `keepalive_s` / `idle_disconnect_s`
(0 turns either off), else 60 s keep-alive over SSH and none otherwise, and idle by environment
(dev 4 h, test and staging 2 h, production 1 h, local or untagged off). A ping may take
`min(keepalive / 2, 10 s)`. `ManagerConfig::default()` stays M2a's: every profile without its
own keep-alive pings every `liveness`, no environment idle preset applies (only a profile's own
`idle_disconnect_s`; fix round 1, M3), and a loss is `Lost` (R12). So every M2a test and the
app boot keep their behaviour until Task 22 switches to `ManagerConfig::m2b()` (`presets` and
`reconnect`, two separate fields). Nothing on this branch calls `m2b()` but the Docker test;
Task 22's `boot.rs` line must.

**Recovery (HV-CONN-05, B4, B19).** Only the default console session reconnects, eagerly, in
place: its handle stays valid. Other sessions of the profile break as before (M3 reconnects
consoles lazily; pools open new ones), so one lost server or tunnel never starts a reconnect per
session. A loss is a recovery case when its class is `network.*`, `server.shutdown`,
`server.starting`, `karagoz.upstream_unreachable`, `karagoz.resume_expired`, or (plan
amendment, fix round 1, I3) `ssh.channel_open`: a bastion that cannot reach a database while it
restarts. Only recovery asks, so at a first connect `ssh.channel_open` stays a hard failure.

- **Only an idle session reconnects (fix round 1, C1, controller ruling).** When the lost
  connection had a transaction open (`InTx`, `Failed`), or the request in flight may have opened
  one (its state is unknown: a `Tx(Begin)`, a user statement with a `BEGIN`/`START`/`XA` or a
  `SET autocommit` anywhere in it, any user statement of a Manual session), or the user's own
  MySQL `autocommit = 0` is in force (see below), nothing is replayed
  and nothing the user sent for that connection ever runs on another: the session emits
  `SessionEvent::Lost` (with `open_tx`) and breaks; the profile is `Lost` (Retry), then
  `SessionLost { lost }`. Rejected: reconnecting and refusing the next `COMMIT` (a later
  statement would still run outside the lost transaction).
- **"Had a transaction open" is the last status seen before the loss (fix rounds 2 and 3, C2
  and R4).** A driver's status after a loss says nothing about the transaction: the PostgreSQL
  driver reports `Failed` for a connection it found closed (ADR 0002 m2b-t10). So the actor
  never reads (`observe`) a status after an error that broke the session, and the loss's
  `open_tx` is the session's status at that point: the last one observed before the loss (a
  keep-alive changes none; a `COMMIT` that succeeded before its follow-up probe found the loss
  left `Idle`, so no rollback is claimed for a transaction that committed). An idle autocommit
  session whose statement finds the loss therefore reconnects; that statement alone is
  `ConnectionLost { outcome_unknown: true }`, and the toast claims no rollback.
- **The user's own MySQL `autocommit = 0` (fix round 3, I8).** A successful user
  `SET [SESSION | LOCAL | @@session. | @@local. | @@]autocommit = 0 | OFF | FALSE` (MySQL,
  MariaDB) is remembered until `= 1 | ON | TRUE | DEFAULT`, a reset, or the app's own switch to
  Auto. While it holds, the server runs every statement in an implicit transaction, so the
  session treats a loss as one inside a transaction (`Lost`, `SessionLost { open_tx }`): it
  never reconnects, which would also silently turn autocommit back on. `SET GLOBAL` (only the
  word right before the variable is read) changes other sessions and is ignored; any doubt
  reads as this session's.
- **A connection-ending server error is a loss (fix round 2, I6).** A server error whose class
  is a recovery case after connect (PostgreSQL `57P01`/`57P02` `server.shutdown`, `57P03`
  `server.starting`, `08006` `network.reset`; on PostgreSQL only at severity `FATAL` or `PANIC`,
  fix round 3, I7: an `ERROR` with these SQLSTATEs is a remote server's failure relayed by
  `postgres_fdw` or `dblink`, and the local session and its transaction stay; the severity is
  the server's `S`, which is the non-localized `V` unless the server translates it, and a
  translated one that does not read `FATAL` is found closed at the next round trip instead), or
  one of MySQL's connection-ending codes (1053
  server shutdown, 1927/3169 connection killed, 4031 interaction timeout; 2006/2013 should a
  driver report them as server errors), breaks the session on the request it answered, which
  keeps the server's message. So the recovery starts there, not one request late, and a request
  that was never sent is never reported "outcome unknown". The MySQL codes live in the session
  layer (`recovery::connection_ending`), since the driver catalog (`errors.toml`) maps them to no
  class.
- An idle session publishes `Connecting`, not `Broken`. The pool's watcher therefore keeps the
  session's budget permit through the reconnect (T15 carry-in: a `Broken` session frees it, and
  an in-place reconnect must never run without one); a session whose recovery fails turns
  `Broken` and frees it then.
- The request that found the loss is answered `ConnectionLost { outcome_unknown: true }` and
  never re-run; queued requests that never ran, and every request sent while the session
  recovers, are answered `ConnectionLost { outcome_unknown: false }` at once (C1: nothing is
  queued onto the new connection). The only exception is an internal idempotent read (schema
  tree, completion): the one that found the loss is re-queued once at the head, and others wait
  for the new connection; none belongs to a transaction.
- The reconnector (`SessionHooks.reconnect`) is called once per loss, synchronously, before that
  answer, so whoever sees the answer already sees `Reconnecting`. It runs the B19 backoff (0, 1,
  2, 4, 8 s, then 30 s, ±20 % jitter from `getrandom`, Q30; five minutes) with headless
  `Purpose::Main` connects over the session overlay (Q17): the saved profile with this session's
  profile changes and every `update_profile` edit, the one-gesture plaintext consent, and the
  secrets that worked. It never prompts.
- **The replay is part of the attempt (fix round 1, I1).** Once a connect lands, the actor
  replays the environment plan (identity, read-only setup, last schema; MySQL's
  `autocommit = 0` when Manual) within the profile's connect timeout, still serving commands (a
  close ends it at once). A replay that fails or times out tells the reconnector
  (`Reconnector::replay_failed`, a provided trait method) and calls it again: that call
  continues the same loss's schedule and five-minute clock instead of starting over, so a
  server that accepts and then drops is never hammered.
- Endings: success → mode and isolation kept, the profile `Connected`, then `SessionLost { lost }`
  (always); an `auth.*` failure → `Failed` (also when the pipeline marks it `needs_interaction`:
  the server wants a password the overlay no longer has, and the error dialog offers the
  re-entry; the brief's `auth_failure_during_reconnect_stops_with_failed`); any other
  `needs_interaction` (an SSH one-time code or password, an unknown host key) → `Lost` at once,
  and the capsule's Retry connects interactively; a `cancelled` connect without a stop (the
  saved profile moved while it ran; T15 I1 refused it) → the next attempt, to the new
  destination (M7); any other non-recovery class → `Failed`; five minutes → `Lost` with the last
  failure's class; disconnect, `forget` or quit → ends at once (mid-backoff, mid-connect or
  mid-replay; the shutdown budget is kept).
- **Edits during the first connect (fix round 1, I2).** The manager keeps each profile's latest
  saved profile; when the connect lands, the `SessionManager` starts from it, not from the
  profile the connect started with. If the edit sent the connection elsewhere (T15's
  destination rule), the landed connection is closed (its permit returns after the close) and
  the saved destination is connected anew, without the draft password typed for the old one; a
  consent the saved profile took back is stripped as R1 strips it.

**What a reconnect loses (B19).** A first-keyword scan of successful user statements, outside
comments and quoted text: `CREATE [OR REPLACE] [GLOBAL|LOCAL] TEMP[ORARY]`, `SET` (not
`SET LOCAL` or `SET TRANSACTION`), `LOCK`, `PREPARE`/`DEALLOCATE`, and the session lock
functions (`pg_advisory_lock`, `pg_try_advisory_lock`, their `_shared` forms, `GET_LOCK`,
`sp_getapplock`; not `pg_advisory_xact_lock`). The transaction status at the loss gives
`open_tx`. The scan starts over after each reconnect and each reset. Known limits (fix round 1,
M5), each erring toward reporting more: `#` is never taken for a comment (it is a PostgreSQL
operator, so a lock call after it is still seen); a backslash inside quotes leaves the end of a
string unsure (MySQL escapes, PostgreSQL does not), so quoted text is read too. M3 replaces the
scan with the classifier.

**Idle disconnect.** The session emits `SessionEvent::IdleDisconnected` (an additive variant,
fix round 1, M2), then closes itself gracefully; the manager reports `LiveState::Disconnected`
and `ConnectionEvent::IdleDisconnected`, and the profile's sessions and overlay are dropped
(Q17). `ForSession` secrets stay (it is not the user's disconnect). Any other close of the
default session (only the `ConnectionManager` should close it; `default_session`'s docs say so)
ends the profile's connection as `Disconnected` without that event. A default session broken by
`cancelled` (a deactivated request, an unsettled cancel) also ends `Disconnected`, not
`Lost { cancelled }` (M7). Carried to M3: idle time is the default session's alone, and its end
closes every session of the profile.

**SSH group invalidation (B20).** On `TunnelEvent::Dead { key, generation }` every default
session over that generation is pinged; the ping fails over the dead chain and each session
starts its one recovery; the dependents' parked pooled sessions are closed, so none is handed out
again (fix round 1, M6). Reconnects of one generation's dependents take a per-generation lock,
so the first brings the chain back and the others share it (one SSH session, not one per
profile). Locks nobody holds are forgotten. `FakeDriver` connections that hold their stream
(`consumes_factory`) now fail their ping once that stream ends, so the tests see a connection
die with its tunnel (fix round 1, I4).

**T15 re-review carry-ins.** R1: a connect started before a TLS-mode change or an unticked
"allow public plaintext" never brings the consent (or an `AllowPublicPlaintext` change) back
into the overlay when it lands (a consent generation, bumped by `update_profile`). R2: an
`open` or `pooled` request still waiting for a permit or a released session when `close_all`
runs ends with `cancelled` and never connects afterwards.

**Tests.** `tests/liveness.rs` (pure supervisor, both clocks driven by hand), `tests/recovery.rs`
(schedule and jitter bounds, every class, the scan and its limits; unit tests for the
connection-ending codes and the batch scan), `tests/manager_m2b.rs`:
every `FakeDriver`-only test on a paused clock with condition waits (Q25, fix round 1, I5);
the SSH tests (a one-time code → `Lost`, one SSH death → one reconnect per dependent and one new
chain) on the in-process testkit server in real time. Regressions of the review's hostile
scenarios: H1 (a loss inside a transaction; requests during the backoff, Auto and Manual),
H2 (a failing replay follows the backoff and gives up), H3 (a jump with an open transaction
pings), H4 (a direct close is not an idle disconnect), H8 (a close during a hanging replay; the
replay timeout), H10 (an edit during the first connect, same and moved destination); round 2:
H11 (`57P01`, `57P03`, MySQL 4031 answering a statement start the recovery; an ordinary error
does not), H12 (an idle autocommit statement, a read-only session's schema switch and an
internal read that find the loss reconnect; a statement inside a transaction is still `Lost`),
and a batch with a later `BEGIN`, `SET autocommit` or `XA START` is `Lost`; round 3: H13 (the
user's MySQL `autocommit = 0`: a lost `UPDATE` and an idle loss are `Lost`, a later `COMMIT`
never runs, `= 1` and `SET GLOBAL` reconnect), H14 (a relayed `ERROR` `08006`/`08001`/`57P01`
keeps the session and its `Failed` transaction), H15 (a `COMMIT` whose follow-up probe finds the
loss reconnects with no rollback claimed). A Docker test ends the default
session's backend from the admin connection on pg17 and mysql84 (`pg_terminate_backend`,
`KILL`); an Auto-mode user `SELECT 1` finds the loss within three round trips, the session
reconnects, and `SessionLost` claims no rollback.
