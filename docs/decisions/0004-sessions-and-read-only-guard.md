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

## m2b-t18 — read-only execution and labels

`hacivat-session::session::readonly` runs what `hacivat-guard` decides (HV-SESS-04,
SQL-SAFE-05, MCP-09 step 1, B21). Every `Request::Exec` of a read-only session, whether its
origin is the user or Hacivat, goes through `run_guarded`; the actor calls its crate-private
twin `guarded`, which also says whether the connection must be discarded. Outside the guard a
read-only session runs only fixed or quoted control text (fix round 1, m-5): `SwitchSchema`
(`SET search_path TO "…"` / ``USE `…` ``), the schema reprobe, `Ping` and `Reset` (Task 7).

**The guarded request (PostgreSQL, MySQL, MariaDB).**

- **1.** `check_input`, else `ReadOnlyDenied(Input)`. No I/O.
- **1a.** Any reading of the first keyword (`guard_keywords`) is `COMMIT`, `ROLLBACK`,
  `BEGIN`, `START`, `END`, `ABORT`, `SAVEPOINT` or `RELEASE` → `ReadOnlyDenied(Transaction)`
  (Q35). No I/O. This produces the denial; it is not a classifier, and steps 4–7 still catch
  what gets past it. The keyword is read the way the server's lexer reads it (fix round 1,
  I-1; the first version diverged from MySQL in six ways, review H1):
  - skipped: ASCII whitespace (vertical tab included), `;` (PostgreSQL drops an empty
    statement, so `;COMMIT` is one `COMMIT` even in the extended protocol), `--` line comments,
    MySQL/MariaDB `#` comments, block comments;
  - a line comment ends at a line feed on MySQL, MariaDB and SQLite (`MY_LEX_COMMENT`, SQLite's
    tokenizer), and at a line feed or carriage return on PostgreSQL (`non_newline` is
    `[^\n\r]`);
  - PostgreSQL block comments nest, other engines' do not;
  - MySQL and MariaDB `--` is a comment only before whitespace or a control character;
  - **executable comments are statement text (T13 carry):** `/*!…*/` on MySQL and MariaDB,
    `/*M!…*/` on MariaDB only (an ordinary comment on MySQL). An unversioned one always runs,
    so the keyword is read inside it. A version-gated one (`/*!NNNNN …*/`, `/*M!NNNNNN …*/`)
    runs on a server at least that version and is skipped on an older one, so it is read
    **both ways** — inside it, and after its end — and the stricter result wins: 1a refuses
    when any reading is a transaction keyword. `/*!99999 SELECT 1 */ COMMIT` is refused.
  - **A skipped version-gated comment may hold one nested comment** (fix round 2, I-1b): MySQL's
    and MariaDB's lexer skips it with `consume_comment(lip, 1)` ("version allowed to have one
    level of comment inside"), so in the skipped reading one inner `/*…*/` (`!` and `M!`
    included: inside a skipped comment they are plain comments) is skipped whole, the next
    `*/` ends the version-gated comment, and a deeper `/*` is plain text. Ordinary MySQL,
    MariaDB and SQLite comments do not nest (`consume_comment(lip, 0)`).
    `/*!99999 /* x */ SELECT 1 */ COMMIT` is refused.
  - Readings fork at each version-gated comment, and each comment is followed once (both of
    its readings reach the same text after it; review n-1): a chain of them stays linear. At
    most 16 forks deep and 64 readings; past either bound the readings are incomplete, which
    1a refuses. In practice: up to 16 version-gated comments before the first word are read,
    a 17th is refused (not guessed at), and a 1 MiB statement of them is refused at once.
    (Round 1 forked without the memo, so 7 sequential comments already overflowed 64
    readings.)
  - On PostgreSQL and SQLite `/*!` is an ordinary comment.

  The session's `schema::first_keyword` (Task 7: Manual-mode control words, schema reprobe)
  still reads `/*!…*/` as a comment. It decides nothing about read-only safety; left as is.
- **2.** The plan's `before` statements through `batch` (trusted text). If one fails, nothing
  else runs; the connection is kept only when it reports no transaction and is not lost (`BEGIN`
  ran, `SELECT 1` failed: the guard's transaction is open → discarded). **Then the connection
  must report `InTx`** (fix round 1, m-1, review H3): a `before` that did not open the read-only
  transaction is `EnforcementGap(TransactionEndedByStatement)` — the closest reason the
  `GapReason` set has: the guard found no transaction — the connection is discarded, and the
  user statement is never sent. (This also settles the open question whether MySQL reports
  `SERVER_STATUS_IN_TRANS` after `START TRANSACTION READ ONLY`: if a server did not, every
  read-only statement would end here, loudly, before any user text ran. The fix round 2
  Docker run confirmed MySQL 8.0, 8.4 and MariaDB 10.6, 10.11, 11.4 report it.)
- **3.** The user statement through `execute` with `Protocol::Extended` and the caller's row
  limit, whatever protocol the caller asked for: one statement, prepared (PostgreSQL refuses a
  second one with `42601`; MySQL's read-only connection never re-sends an unpreparable
  statement as text). A server refusal is `ExecErrorKind::Db` with the server's message,
  whatever its code (the read-only ones: PostgreSQL `25006`, `42501`; MySQL `1792`, `1142`,
  `1290`; SQLite `SQLITE_READONLY`, `SQLITE_AUTH`). MySQL's `Unsupported` (an unpreparable
  statement on a read-only connection) is `ReadOnlyDenied(Protocol)`. A lost connection ends
  the request at once: the session is discarded.
- **4.** `tx_status()` → `PostFacts.tx_after_statement`, always recorded (T5: an unknown state
  is a gap). PostgreSQL's status is the wire `ReadyForQuery` byte (Task 10), MySQL's the OK
  packet's (or `COM_PING`'s after an error) status flags (Task 13).
- **5.** PostgreSQL's canary `SELECT pg_catalog.txid_current_if_assigned()` through
  `execute(Simple)`, only when the guard's transaction can still answer: the statement
  succeeded, or the status is `InTx`. After a failed statement the transaction is aborted
  (`Failed`), nothing in it can be committed, and the canary would only fail; skipping it hides
  nothing (an `Idle` one is caught at step 7). A canary that fails or answers anything but one
  row of one value (`NULL` or text) cannot be read: the request fails `Internal` and the
  connection is discarded — after `check_post`, so a gap found from the other facts is still
  reported as a gap. A user `SET LOCAL statement_timeout` can only break the user's own
  session this way.
- **6.** The plan's `after` statements (`ROLLBACK`) through `execute(Simple)`, collecting their
  `notice_codes` (`25P01` = the statement already ended the transaction). A failed `ROLLBACK`
  discards the connection.
- **7.** `check_post`, on a successful statement and on a failed one (with no command tag, so
  only the transaction state and the rollback notice count) (fix round 1, I-2):
  - PostgreSQL: an error inside its transaction always leaves it `Failed`, so `Idle` means the
    statement ended it.
  - MySQL and MariaDB: a procedure may run `COMMIT`, an explicit
    `START TRANSACTION READ WRITE` (which overrides the session's read-only setting), a write,
    `COMMIT`, and then `SIGNAL`: the write is committed and the statement still fails (review
    H2). So a failed statement is judged too. **The one exception:** MySQL commits implicitly
    before DDL and administration statements and then refuses them under the session's
    read-only setting (`1792`), which escapes nothing; a refused `CREATE` must not close the
    session. A failed statement is therefore not judged only when **every** reading of its
    first keyword that holds a statement is in the allowlist `IMPLICIT_COMMIT_KEYWORDS`:
    `CREATE`, `ALTER`, `DROP`, `RENAME`, `TRUNCATE`, `GRANT`, `REVOKE`, `ANALYZE`, `OPTIMIZE`,
    `REPAIR`, `FLUSH`, `INSTALL`, `UNINSTALL`, `CACHE`, `LOAD`, and since fix round 2 the
    preparable replication and administration verbs `RESET`, `CHANGE`, `STOP` (review note:
    a refused `RESET MASTER` no longer closes the session). Any other keyword (`CALL`, `DO`,
    `SET`, an unknown word, a parenthesis), and mixed readings (`/*!99999 CREATE */ CALL p()`),
    are judged. A reading that holds no statement — a version-gated comment that held the
    whole statement, skipped — runs an empty query on the server (`1065`, no implicit commit),
    so it does not count (review n-4): a failing `/*!50000 ALTER … */` is only an error. A
    `mysqldump` view header (`/*!50001 CREATE … */ /*!50013 DEFINER=… */ …`) still has a
    `DEFINER` reading and is judged (the readings do not model that version gates are
    monotonic; doing so could drop a reading the server takes). The exemption trusts the
    server's read-only refusal after the implicit commit for these classes only; a successful
    one is still judged. (The first version skipped every failed MySQL statement: wrong for
    `CALL`.)
  - SQLite runs in autocommit; a failed statement has nothing to judge.

  A gap → `EnforcementGap(reason)`: the connection is discarded **unreset**, the actor marks the
  session `Broken(Internal)`, closes the connection, answers the queued requests
  `NotConnected` and emits `SessionEvent::EnforcementGap(reason)` before the reply.
- **8.** When `plan.reset != None`: `conn.reset()` (PostgreSQL's `DISCARD ALL` sequence, MySQL's
  `COM_RESET_CONNECTION`), then `replay` — the session's `EnvPlan::replay()` (read-only setup,
  the restored `search_path` or database, Q15) — through `batch`. A failed reset or replay
  discards the connection. Last, a connection that still reports a transaction is discarded.

If step 3 fails, steps 4–8 still run. A user statement never goes through `batch`: only the
guard's constants and the environment plan's quoted text do.

**MySQL procedures and the layers (fix round 1, I-2).** A role with `EXECUTE` on a procedure
that controls its own transaction defeats L4 on MySQL (`START TRANSACTION READ WRITE` inside
the procedure overrides `SESSION TRANSACTION READ ONLY`). L6 catches it when the procedure
leaves no transaction (`Idle` → gap, failed or not). It does not when the procedure ends inside
a new `READ ONLY` transaction (`InTx`) with a `CALL` row count of 0. Only L5 is honest about it:
`EXECUTE` is outside MySQL least privilege, so such a role is labelled guarded. Carried to Task
23's bypass suite: a `CALL` of a procedure that commits, writes and commits.

**SQLite.** Steps 1 and 1a, then the statement and `check_post` on success (its write-tag
tripwire; the SQLite driver reports `rows_affected` only for a statement that is not
read-only). The plan has no statements and no reset, so nothing else runs: the layers are
connection-level (Task 11: read-only open, hardening, authorizer).

**Discard, not reuse.** `Guarded::discard` carries the broken class: the classified class of a
lost connection, else `Internal`. The actor sets `Core::broken` from it, so `run_one` closes the
connection and publishes `Broken(class)`. Fail closed: a guard that could not finish is never
trusted with the next request. The cancel ladder is unchanged: a PostgreSQL cancel that lands in
step 3 leaves the transaction `Failed` and steps 6–8 run; one that lands in step 6 or 8 fails
that step and discards the connection; `Escalation::Deactivate` drops the request and breaks the
session (Task 7), so a timed-out statement never leaves a read-write transaction behind (the
server rolls back the closed connection's transaction). Read-only sessions are always Auto (B22,
Q14): `run_guarded` never runs inside a user transaction.

**Role probe and label (`access_for`, called by the pipeline's PostConnect).**

- Read-write: `ReadWrite`, nothing runs.
- Read-only PostgreSQL: first **the connection's reset** (fix round 2, I-3b), the same
  `conn.reset()` every guarded request ends with: its `SET SESSION AUTHORIZATION DEFAULT`
  returns to the signed-in login and its `RESET ALL` restores a startup or role-default `role`.
  So the identity check and the probe judge the identity every guarded request after the
  first runs as (round 1 judged the identity before the first reset: a superuser login running
  as a reader would have read enforced, and the first request's reset would have made it the
  superuser again). A failed reset is `Unparsed`, guarded. One round trip per read-only
  connect.
- Then `SELECT current_user, session_user` (fix round 1, I-3). The probe judges
  `current_user`'s roles, but `SET ROLE NONE` returns to the session user, so a session running
  as another role (a role default `ALTER ROLE login SET role = …`) could read enforced while one
  `DO $$ … SET ROLE NONE … $$` escalates. The two names are compared here (no server operator;
  `current_user` and `session_user` are SQL keywords, not names `search_path` resolves);
  anything but one row of two equal non-NULL names is `Unparsed`, guarded, and the probe does
  not run.
- **No role switch in a read-only connect's options** (fix round 2, I-3b, named exceptions in
  `hacivat-driver` `sanitize.rs`, `hacivat-driver-pg` `connect.rs` and the pipeline's
  `validate.rs`/`mod.rs`, blocks `m2b-t18-guarded-exec`). `sanitize::pg_options_switch_role`
  reads an `options` value as the server does (`pg_split_opts`; `-c name=value`,
  `-cname=value`, `-c` then `name=value`, `--name=value`; names ASCII-folded, `-` as `_`) and
  finds `role` or `session_authorization`. `sanitize::read_only_violations` turns that into an
  `AlwaysDenied` violation, whatever the origin and the unsafe toggle; the pipeline's
  validation shows it as `OptionRejected` (`ConfigInvalid`, before any I/O) for a read-only
  profile and for a read-only session of a read-write profile. The PostgreSQL driver's
  `startup_options` refuses it too on a read-only connect (`Unsupported`, never dropped
  silently), the belt for a caller outside the pipeline. A read-write connect keeps `-c role=`.
  Observed on pg17: a startup `-c session_authorization=` left `current_user` the login, so
  the reset is what guards the in-session switches (`SET SESSION AUTHORIZATION`, `SET ROLE`),
  tested on pg17.
- Then (PostgreSQL) or directly (MySQL) `role_probe_sql`'s fixed statements (never user text),
  each through `execute(Simple)` with at most 10,000 rows, then `interpret_probe` and
  `access_label`. A statement that fails, an answer that was truncated, or a probe longer than
  5 s is `Unparsed` → guarded (T5 review: a slow probe must end guarded; review H4 tests it).
- **Qualified names (fix round 1, m-3).** The canary and every function, aggregate and relation
  of the PostgreSQL probe are `pg_catalog.`-qualified, so a `search_path` that names another
  schema before `pg_catalog` (an imported URL may set `search_path`, a safe GUC) cannot shadow
  them with pre-existing functions or tables. Operators (`=`, `!~`, `<>`, `IN`) stay
  unqualified: shadowing one needs a pre-existing operator on the exact operand types in a
  schema on the path, and `OPERATOR(pg_catalog.…)` rewrites of T5's validated SQL wait for a
  Docker run (residual, defense in depth only).
- Read-only MariaDB (C19), SQLite and every other engine: `ReadOnlyGuarded`, no probe, no
  warnings.
- The engine is the connection's as well as the profile's: a MySQL profile whose server turned
  out to be MariaDB (the MySQL driver's `kind()` after `server_info`) is MariaDB; two engines
  that disagree otherwise are `[Unparsed]`, guarded.
- **Side connections (fix round 1, m-2).** A `Department::Cancel` connect (cancel, kill) runs
  no user SQL and its label is never shown, so the pipeline skips the probe for it (named
  exception in `pipeline/mod.rs` and `steps.rs`, block `m2b-t18-guarded-exec`): read-only →
  `ReadOnlyGuarded`, read-write → `ReadWrite`, no warnings. A cancel no longer waits for up to
  six probe round trips.
- **Probe timeout (review m-4).** On the 5 s timeout the in-flight probe statement (read-only
  text) is dropped and the connection is kept; tokio-postgres drains an abandoned response, and
  a `mysql_async` desync would surface as a protocol error on the next statement, which
  discards the session (closed, but noisy). `access_for` runs outside the pipeline's connect
  deadline, so a connect may take up to 5 s longer than `connect_timeout`.
- The probe runs once per Main connect, before the session exists (spawn runs nothing, Q27).
  The T7 tests' `executed()` lists now start with the identity check (and the probe, where the
  fake answers it) wherever the connect is read-only.

**Remediation by server version (T5 carry).** `readonly::remediation_for_server(&ServerInfo)` =
`hacivat_guard::remediation_for(kind, major)`: PostgreSQL before 14 gets the schema-grant form
(with `<schema>`), an unknown version the 14+ form, MariaDB `None` (its `ServerInfo.kind` is the
detected one). The app reads `LiveInfo.server` and fills `<schema>` (Task 22).

**Probe timing on 100k tables (review probe).** Measured by the Docker test
`pg_probe_on_100k_tables_is_measured` (full set only). It creates a throwaway database
`hv_probe_scale` of 100,000 tables, times `access_for` as `hv_ro`, prints the time and drops the
database again (a database left by an interrupted run is dropped first; fix round 1, m-6). It
asserts the label only, which already means the probe ended within its 5 s bound; no
wall-clock assertion gates CI (fix round 1, I-4). It also writes the Docker artifact
`t18-probe-timing.txt`. **Measured (fix round 2, `--set full`, pg17, local Docker on the
development Mac): the whole probe — reset, identity check and the five statements — took
153 ms over 100,000 tables** (`hv_ro`, enforced). Row 3's `EXISTS` finds nothing for a
least-privilege role, so it scans every relation once; well under the 2 s the review asked
about and the 5 s bound.

**Docker fixtures (review m-6).** The fixture roles are created idempotently and stay on the
matrix volumes: `hv_ro` (as the init script makes it), `hv_ro_su` (superuser member),
`hv_ro_col` (column `UPDATE`), `hv_ro_create` (`CREATE` on `public`), `hv_ro_setrole` (superuser
member and `hv_ro` member, `role = hv_ro` by default), all with the matrix's throwaway password.
The `REVOKE TEMP`/`REVOKE CREATE ON SCHEMA public` from `PUBLIC` repeat the init script's own
statements. MySQL: `hv_t18_ddl` is dropped before the DDL leg and never created. The test
binary creates each target's fixtures once, under a lock: the tests run in parallel, and
concurrent `CREATE ROLE`/`GRANT`/`CREATE TABLE IF NOT EXISTS` race on the catalogs (the first
Docker run failed five tests on exactly that). A fixture failure names the statement only up
to its first literal (a literal may hold the matrix password) and the server's code.

**Carried (fix round 2 review).** To Task 10's owner (n-2): `RESET_SQL`'s
`SELECT pg_advisory_unlock_all()` is unqualified; after `RESET ALL` restores a startup
`search_path`, a shadow function could keep advisory locks held across guarded requests (a lock
leak, not a write): use `pg_catalog.pg_advisory_unlock_all()`. To Task 23's bypass suite (n-3):
a MySQL account can always change its own password (`ALTER USER USER() IDENTIFIED BY …`,
`SET PASSWORD = …`), which L5 cannot prevent; assert both are refused in a guarded read-only
session (1792, or a gap). Also to Task 23 (I-2): a `CALL` of a procedure that commits, writes
and commits.

**Tests.** `crates/core/hacivat-session/tests/readonly.rs` (FakeDriver, 37): the exact
PostgreSQL and MySQL sequences with replay and one reset; a smuggled `COMMIT` → gap, `Broken`,
no open connection, the event, no reset, later requests `NotConnected`; canary non-NULL → gap;
the transaction keywords; MySQL/MariaDB executable comments; the six review-H1 forms,
both-way readings and the one-level nesting of a skipped version-gated comment (I-1b) refused
before I/O, and what the server reads as a read left alone; a chain of 16 version-gated
comments read, a 17th refused; engine comment rules and look-alike words; the 1 MiB limit
before I/O; a failed statement still rolls back and resets (server message kept); a cancelled
one skips the canary and still resets; a failed `BEGIN` runs nothing else; a `before` step that
leaves a transaction open breaks the session; a `before` that opens no transaction never sends
the statement (H3); a failed MySQL `CALL` that left no transaction → gap (H2), and `DO`, a
parenthesis, `SET` and mixed readings too; a failed MySQL DDL (a whole-statement
version-gated one and `RESET`/`CHANGE`/`STOP` included) → `Db`, reset, kept; MySQL write with
rows → gap; `25P01` on the rollback → gap without reset; an unreadable canary → `Internal` and
broken; a PostgreSQL error with no transaction → gap; a failed reset breaks the session; an
engine without a plan runs nothing; SQLite runs only the statement; the probe: read-only only,
reset, identity check, then the fixed statements; a failed reset is `Unparsed` (I-3b); a
switched role (and an unreadable identity answer) → `Unparsed` without the probe (I-3); a
read-only connect (profile or session) with `-c role=`/`-c session_authorization=` refused as a
visible `OptionRejected` before I/O, read-write kept (I-3b); a failure is `Unparsed`, a 6 s
probe is `Unparsed` (H4),
TEMP keeps the enforced label, superuser, MySQL `SHOW GRANTS`, MariaDB/SQLite unprobed, a MySQL
profile on MariaDB; a side connection runs no probe (H5); the label on a pipeline connect;
remediation by version. `hacivat-guard` `plans.rs`/`probe.rs`: the qualified SQL.
`hacivat-driver` `tests/sanitize.rs` and `hacivat-driver-pg` (`connect.rs` unit,
`tests/unit.rs`): the role-switch reader, the read-only violation, the driver's refusal and
read-write's `-c role=`.
`tests/docker_readonly.rs` (**run in fix round 2 with `--set full`: 10/10 pass**, plus the
other session Docker files and `hacivat-driver-pg`'s, all green): `hv_ro` reads a table, a
write is refused, `COMMIT` denied (pg17); `hv_ro` enforced and the superuser guarded with its
warning; the three T5 fixture roles guarded with their warnings; a role default reads guarded
`Unparsed`, and a typed `-c role=`/`-c session_authorization=` on a read-only connect is
refused (`ConfigInvalid`, `OptionRejected`); a superuser connection switched in-session to
`hv_ro` (`SET SESSION AUTHORIZATION`, `SET ROLE`) reads guarded with `Superuser` after the
probe's reset (I-3b); `hv_ro` enforced on every PostgreSQL line (13–18; this also confirms the
`pg_catalog`-qualified probe SQL on every server); MySQL 8.4 `hv_ro` enforced, MariaDB guarded
with no warnings; MySQL 8.0/8.4 and MariaDB sessions read, refuse a write, refuse
executable-comment `COMMIT`s (both readings, and one nested level), and a refused
`CREATE TABLE` is `Db`, the session stays usable and the table is absent (the I-2 premise,
confirmed); advisory lock and `LISTEN` do not survive a request; the 100k-table measurement
(above); and, without a server, SQLite guarded and a SQLite session that reads and refuses.
