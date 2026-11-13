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
- **The user's own MySQL `autocommit = 0` (fix rounds 3 and 4, I8, I9, R5).** The session
  follows the server's `SERVER_STATUS_AUTOCOMMIT` flag, which every OK packet carries (the
  MySQL driver reports it through `RawConn::autocommit`, a provided trait method; after a
  server error its `COM_PING` re-reads the flags). So a procedure, a prepared statement,
  `SET autocommit = @v`, `init_connect` or a global default are all seen. When a driver
  reports no flag, the statements are the fallback: a user
  `SET [SESSION | LOCAL | @@session. | @@local. | @@]autocommit = 0 | OFF | FALSE` is
  remembered whatever its outcome (a batch's `SET` applies even when a later statement fails,
  I9; it errs toward "off"), and `= 1 | ON | TRUE | DEFAULT` clears it on success, as do a
  reset and the app's own switch to Auto. `SET GLOBAL` (only the word right before the variable
  is read) changes other sessions and is ignored. While autocommit is off outside the app's own
  Manual mode (which sets it itself and replays it), the server runs every statement in an
  implicit transaction, so the session treats any loss as one inside a transaction (`Lost`,
  `SessionLost { open_tx }`): it never reconnects, which would also silently turn autocommit
  back on.
- **A connection-ending server error is a loss (fix round 2, I6).** A server error whose class
  is a recovery case after connect (PostgreSQL `57P01`/`57P02` `server.shutdown`, `57P03`
  `server.starting`, `08006` `network.reset`; on PostgreSQL only at severity `FATAL` or `PANIC`,
  fix round 3, I7: an `ERROR` with these SQLSTATEs is a remote server's failure relayed by
  `postgres_fdw` or `dblink`, and the local session and its transaction stay), or
  one of MySQL's connection-ending codes (1053
  server shutdown, 1927/3169 connection killed, 4031 interaction timeout; 2006/2013 should a
  driver report them as server errors), breaks the session on the request it answered, which
  keeps the server's message. So the recovery starts there, not one request late, and a request
  that was never sent is never reported "outcome unknown". The MySQL codes live in the session
  layer (`recovery::connection_ending`), since the driver catalog (`errors.toml`) maps them to no
  class. Known limits (fix round 4, R6, R7): the severity read is the server's `S` field
  (`ServerMessage.severity`), which is the non-localized `V` only when the server does not
  translate it; a translated `FATAL` (the Turkish `ÖLÜMCÜL`) is not recognised, so that
  connection is found closed one round trip later, and that next request, never sent, is
  answered `ConnectionLost { outcome_unknown: true }`. No locale renders `ERROR` as `FATAL`, so
  the rule never misfires. The rule applies to `DbmsKind::Postgres`; when the PostgreSQL-wire
  engines (Timescale, Yugabyte, Redshift, Cockroach; `Family::Other` today) get drivers, they
  must take it too (by family, not kind), or be detected one round trip later in the same way.
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

**Events before ends (fix round 4, I10, the Task 18 merge).** A session sends what explains
its end before it publishes that end: `Lost` before `Broken`, `IdleDisconnected` before
`Disconnected`, and Task 18's `EnforcementGap` before `Broken` (moved at the merge). The
manager's monitor reads events before states (`biased`), and before it stops on `Broken` or
`Disconnected` it drains and relays whatever is still queued, so a read-only default session's
gap notice is never lost behind its `Lost`. In the read-only path the guard's discard breaks
the session before the status is read (T17's order kept at the merge), so C2 cannot come back
there.

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
loss reconnects with no rollback claimed); round 4: H16 (`SET autocommit = 0; <typo>` then a
lost `UPDATE` is `Lost`, with and without the server's flag), R5 (a procedure, a prepared
statement, a variable and a server default seen through the flag; the flag back on reconnects;
the app's own Manual mode still reconnects and replays), I10 (a read-only default session's
guard gap relayed before `Lost`, 20 rounds on a multi-thread runtime; a read-only statement that
finds the loss reconnects with no rollback claimed). A Docker test on mysql84 runs
`SET autocommit = 0; SELEC 1` (the SET applies, the batch fails), then an `UPDATE` in flight
killed from the admin connection: `Lost` with `SessionLost { open_tx }`, never a reconnect. A
Docker test ends the default
session's backend from the admin connection on pg17 and mysql84 (`pg_terminate_backend`,
`KILL`); an Auto-mode user `SELECT 1` finds the loss within three round trips, the session
reconnects, and `SessionLost` claims no rollback.

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
- **A read-only connect's `options` are allowlisted by shape** (fix rounds 2 and 3, I-3b and
  I-3c; named exceptions in `hacivat-driver` `sanitize.rs`, `hacivat-driver-pg` `connect.rs`
  and the pipeline's `validate.rs`/`mod.rs`, blocks `m2b-t18-guarded-exec`). The server parses
  `options` with POSIX `getopt`, whose single-dash flags combine: `-ncrole=x`, `-nc role=x`,
  `-n-role=x`, `-ec role=x`, `-Pcsession_authorization=x` all set the setting (review round 2
  connected with three of them on pg17, `current_user = pg_read_all_data`). Round 2's reader
  of "every spelling" missed them, so round 3 stops recognising spellings and allowlists
  shapes: `sanitize::pg_options_read_only_allowed` splits the value as the server's
  `pg_split_opts` does (C `isspace`, `\` escapes) and accepts only `-c` followed by a
  `name=value` word, `-cname=value` and `--name=value`, none naming `role` or
  `session_authorization` (ASCII-folded, `-` read as `_`, surrounding whitespace ignored). Any
  other word — another backend switch (`-e`, `-P`, `-fs`), combined flags, a dangling `-c`, a
  non-option word — refuses the value. `sanitize::read_only_violations` turns a refusal into an
  `AlwaysDenied` violation, whatever the origin and the unsafe toggle; the pipeline's
  validation shows it as `OptionRejected` (`ConfigInvalid`, before any I/O) for a read-only
  profile and for a read-only session of a read-write profile. The PostgreSQL driver's
  `startup_options` applies the same allowlist to the text it sends on a read-only connect
  (`Unsupported`, never dropped silently), the belt for a caller outside the pipeline. A
  read-write connect keeps any value (`-c role=` is a legitimate feature). Imported URLs were
  already limited to `pg_options_safe`'s shapes and safe GUCs.
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
153 ms over 100,000 tables** (`hv_ro`, enforced); the round 2 re-review measured 139 ms and
the fix round 3 run 143 ms. Row 3's `EXISTS` finds nothing for a least-privilege role, so it
scans every relation once; well under the 2 s the review asked about and the 5 s bound.

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
and commits. And (round 2 re-review note): MySQL `RESET PERSIST`, `SET PERSIST` and
`SET GLOBAL` run inside the read-only transaction without an implicit commit and change server
state that L4 does not stop; they need administrative privileges outside the least-privilege
set, so only L5 (guarded) covers them, the same class as the `EXECUTE` residual above. Task 23
should assert them.

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
read-only connect (profile or session) with `-c role=`/`-c session_authorization=`, combined
`getopt` flags (`-ncrole=`, `-nc role=`, `-n-role=`, `-Pcsession_authorization=`) or another
switch (`-e`) refused as a visible `OptionRejected` before I/O, read-write kept (I-3b, I-3c); a
failure is `Unparsed`, a 6 s
probe is `Unparsed` (H4),
TEMP keeps the enforced label, superuser, MySQL `SHOW GRANTS`, MariaDB/SQLite unprobed, a MySQL
profile on MariaDB; a side connection runs no probe (H5); the label on a pipeline connect;
remediation by version. `hacivat-guard` `plans.rs`/`probe.rs`: the qualified SQL.
`hacivat-driver` `tests/sanitize.rs` and `hacivat-driver-pg` (`connect.rs` unit,
`tests/unit.rs`): the read-only shape allowlist (the review's seven combined-flag bypasses,
other switches, a dangling `-c`, non-option words refused; the three shapes accepted), the
read-only violation, the driver's refusal and read-write's `-c role=`.
`tests/docker_readonly.rs` (**run in fix rounds 2 and 3 with `--set full`: 10/10 pass**, plus
the other session Docker files and `hacivat-driver-pg`'s, all green): `hv_ro` reads a table, a
write is refused, `COMMIT` denied (pg17); `hv_ro` enforced and the superuser guarded with its
warning; the three T5 fixture roles guarded with their warnings; a role default reads guarded
`Unparsed`, and a typed `-c role=`/`-c session_authorization=` on a read-only connect is
refused (`ConfigInvalid`, `OptionRejected`), as are `-ncrole=`, `-nc role=`, `-n-role=`,
`-ncsession_authorization=` and `-e` (I-3c, the forms the review connected with), while
`-c statement_timeout=30s --lock-timeout=5s` connects `hv_ro` enforced; a superuser connection switched in-session to
`hv_ro` (`SET SESSION AUTHORIZATION`, `SET ROLE`) reads guarded with `Superuser` after the
probe's reset (I-3b); `hv_ro` enforced on every PostgreSQL line (13–18; this also confirms the
`pg_catalog`-qualified probe SQL on every server); MySQL 8.4 `hv_ro` enforced, MariaDB guarded
with no warnings; MySQL 8.0/8.4 and MariaDB sessions read, refuse a write, refuse
executable-comment `COMMIT`s (both readings, and one nested level), and a refused
`CREATE TABLE` is `Db`, the session stays usable and the table is absent (the I-2 premise,
confirmed); advisory lock and `LISTEN` do not survive a request; the 100k-table measurement
(above); and, without a server, SQLite guarded and a SQLite session that reads and refuses.

## m2b-t23 — bypass-suite results

Task 23 runs the SQL-SAFE-06 catalogue against the Docker matrix with **no classifier** (Review
Focus 4, exit criterion E4). Hacivat has none in M2b: every read-only statement goes through the
session's guard (Task 18). So each row is stopped, or neutralised by the request's end, by the
layers alone: 1a (the transaction-keyword refusal, before any I/O), L3 (the read-only
transaction and its pinned snapshot), L4 (the session settings: `default_transaction_read_only`,
MySQL's `SESSION TRANSACTION READ ONLY`), L5 (the principal's privileges), L6 (the post-checks,
the `ROLLBACK`, the canary, the reset and the environment replay) and the extended protocol (one
statement per request; MySQL never re-sends an unpreparable statement as text).

**Method (`crates/core/hacivat-session/tests/docker_m2b_bypass.rs`).**

- Each row runs in a real read-only default session (`ConnectionManager`, `ManagerConfig::m2b()`)
  as `hv_ro` (least privilege). The **server-blocked rows also run as the test account** — a
  superuser on PostgreSQL, the owner of every fixture on MySQL and MariaDB — where no privilege
  can stop them: only 1a, L3, L4 and the protocol do. That is the "stand alone" evidence.
- **A side channel judges every row**, never "an error happened" alone. An admin connection takes a
  snapshot before the rows and after each one: on PostgreSQL the fixture table's count and sum,
  the relations of `public`, the sequence, the large objects and the fixture object's content,
  replication slots, prepared transactions, temporary relations and the session's advisory locks;
  on MySQL and MariaDB the table's count and sum, the test database's tables and routines, two
  user locks, `max_connections` and `read_only`. After a neutralised row the session itself must
  show nothing left (no channel, holdable cursor or SQL-level prepared statement; the read-only
  default, `standard_conforming_strings` and `search_path` as before; MySQL's read-only setting,
  no user variable, the pinned `sql_mode`), and a write must still be refused (`25006`) or the
  test account's DDL (`1792`). The brief's M9 line (`SET SESSION TRANSACTION READ WRITE`, then
  `CREATE TABLE`) is judged by the snapshot too, and allows only `1792` or `1142`.
- **A stopped row passes only with the outcome its row allows** (the exact SQLSTATE or vendor
  code, `1a`, `protocol`, `gap` or `lost`, each only where it was observed) **and an unchanged
  side channel**. So a `gap` that passes is one whose effect the `ROLLBACK` undid. A gap after
  which the side channel shows a committed change is its own outcome, **committed; L6 gap**
  (detected, not stopped): it is never recorded as `gap`. The owner's `CALL` rows (T18 I-2) are
  not stopped rows; they record what L6 detected and what it missed (below). MariaDB rows are
  recorded, not judged (none may stay unknown, Q32); its label stays guarded (C19), and the test
  asserts that.
- Fixtures never touch the matrix's own objects (Q7): PostgreSQL's live in a database of their
  own, `hv_t23`, dropped first and last; MySQL's are `hv_t23_*` objects of the test database,
  dropped first and last. `hv_ro` is (re)created only when missing, as Task 18 does. What an
  escaped row would leave outside them is undone at teardown, and the escape fails the test:
  P31's replication slot (`DROP DATABASE` does not drop a physical slot), the server files of
  P14 and P33, and a changed `max_connections` (set back, and `RESET PERSIST`, by the account
  that changed it). An M4 `INTO OUTFILE` file could not be removed by SQL; it would fail the row
  (and every later run, as the file then exists). The fixture names are fixed, so two runs at once
  would collide; the controller's lock wrapper serialises the Docker runs.
- Under `--set full` every line (PostgreSQL 13–18, MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4) must be
  present, or the test fails; a narrower set skips its missing lines with a line on stderr (Q6).
- **Paths (accepted narrowing).** The rows run on every line directly; only the server-blocked
  rows of the `pr` lines (pg17, mysql84) run again **through `sshd-a`**. Across the suite, the
  connect matrix runs every TCP target directly and through `sshd-a`, and the cancel, TLS-pin and
  insecure-auth tests run each of their targets on both paths (the shared-session test is
  SSH-only by nature); the terminated-backend test runs through `sshd-a` on the `pr` lines
  only; the INSERT-cut and outage tests run directly only (their `ChaosProxy` sits in front of the
  database's published port), and SSH death is covered by the parked-session test. This narrows
  the brief's "every TCP test runs twice"; the controller accepted it (fix round 1, m4).
- Each target's results go to the Docker artifact `t23-bypass-<target>.tsv`; the tables below
  are generated from those of the final `--set full` run. SQLite has its own test without a
  server (below).

Cell values: a server code (SQLSTATE or vendor code), `1a`, `protocol`, `gap` (L6 found the
statement's effect after it ran and discarded the connection; the side channel shows nothing
persisted, because the `ROLLBACK` undid it), **committed; L6 gap** (L6 found it and discarded the
connection, but the change had been committed), `neutralised` (it ran; nothing was left), `lost`
(the connection ended and the session recovered; nothing changed; F2 below), **escaped** (a side
effect persisted and nothing caught it).

**PostgreSQL** (pg13 = 13, pg14 = 14, pg15 = 15, pg16 = 16, pg17 = 17, pg18 = 18)

| Row | Account | Statement | pg13 | pg14 | pg15 | pg16 | pg17 | pg18 |
|---|---|---|---|---|---|---|---|---|
| P1 | hv_ro | `WITH d AS (DELETE FROM hv_t23 RETURNING *) SELECT * FROM d` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P2 | hv_ro | `SELECT * INTO hv_t23_p2 FROM hv_t23` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P3 | hv_ro | `EXPLAIN (ANALYZE) DELETE FROM hv_t23` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P4 | hv_ro | `SELECT 1; DROP TABLE hv_t23` | 42601 | 42601 | 42601 | 42601 | 42601 | 42601 |
| P4 | hv_ro | `COMMIT; DROP TABLE hv_t23` | 1a | 1a | 1a | 1a | 1a | 1a |
| P5 | hv_ro | `SET TRANSACTION READ WRITE` | 25001 | 25001 | 25001 | 25001 | 25001 | 25001 |
| P5 | hv_ro | `SET transaction_read_only = off` | 25001 | 25001 | 25001 | 25001 | 25001 | 25001 |
| P5 | hv_ro | `SELECT set_config('transaction_read_only', 'off', true)` | 25001 | 25001 | 25001 | 25001 | 25001 | 25001 |
| P7 | hv_ro | `COMMIT` | 1a | 1a | 1a | 1a | 1a | 1a |
| P7 | hv_ro | `COMMIT AND CHAIN` | 1a | 1a | 1a | 1a | 1a | 1a |
| P7 | hv_ro | `/* x */ END` | 1a | 1a | 1a | 1a | 1a | 1a |
| P7 | hv_ro | `ROLLBACK` | 1a | 1a | 1a | 1a | 1a | 1a |
| P8 | hv_ro | `CALL hv_t23_commit()` | 2D000 | 2D000 | 2D000 | 2D000 | 2D000 | 2D000 |
| P9 | hv_ro | `DO $$ BEGIN INSERT INTO hv_t23 VALUES (9); END $$` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P14 | hv_ro | `COPY hv_t23 FROM '/etc/hostname'` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P26 | hv_ro | `CREATE TEMP TABLE hv_t23_tmp (a int)` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P37 | hv_ro | `SELECT nextval('hv_t23_seq')` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P37 | hv_ro | `SELECT setval('hv_t23_seq', 100)` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P38 | hv_ro | `SELECT hv_t23_sd_write()` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P38 | hv_ro | `SELECT hv_t23_dyn_write()` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P38 | hv_ro | `SELECT query_to_xml('DELETE FROM hv_t23 RETURNING a', true, true, '')` | 0A000 | 0A000 | 0A000 | 0A000 | 0A000 | 0A000 |
| P38 | hv_ro | `SELECT lo_from_bytea(0, '\x6876')` | gap | gap | gap | 25006 | 25006 | 25006 |
| P38 | hv_ro | `SELECT lo_create(0)` | gap | gap | gap | 25006 | 25006 | 25006 |
| P38 | hv_ro | `SELECT lo_put(230023, 0, '\x00')` | 42501 | 42501 | 42501 | 25006 | 25006 | 25006 |
| P38 | hv_ro | `SELECT lo_unlink(230023)` | 42501 | 42501 | 42501 | 25006 | 25006 | 25006 |
| P38 | hv_ro | `SELECT lo_import('/etc/hostname')` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P35 | hv_ro | `SELECT txid_current()` | gap | gap | gap | gap | gap | gap |
| P1 | hv_test | `WITH d AS (DELETE FROM hv_t23 RETURNING *) SELECT * FROM d` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P2 | hv_test | `SELECT * INTO hv_t23_p2 FROM hv_t23` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P3 | hv_test | `EXPLAIN (ANALYZE) DELETE FROM hv_t23` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P4 | hv_test | `SELECT 1; DROP TABLE hv_t23` | 42601 | 42601 | 42601 | 42601 | 42601 | 42601 |
| P4 | hv_test | `COMMIT; DROP TABLE hv_t23` | 1a | 1a | 1a | 1a | 1a | 1a |
| P5 | hv_test | `SET TRANSACTION READ WRITE` | 25001 | 25001 | 25001 | 25001 | 25001 | 25001 |
| P5 | hv_test | `SET transaction_read_only = off` | 25001 | 25001 | 25001 | 25001 | 25001 | 25001 |
| P5 | hv_test | `SELECT set_config('transaction_read_only', 'off', true)` | 25001 | 25001 | 25001 | 25001 | 25001 | 25001 |
| P7 | hv_test | `COMMIT` | 1a | 1a | 1a | 1a | 1a | 1a |
| P7 | hv_test | `COMMIT AND CHAIN` | 1a | 1a | 1a | 1a | 1a | 1a |
| P7 | hv_test | `/* x */ END` | 1a | 1a | 1a | 1a | 1a | 1a |
| P7 | hv_test | `ROLLBACK` | 1a | 1a | 1a | 1a | 1a | 1a |
| P8 | hv_test | `CALL hv_t23_commit()` | 2D000 | 2D000 | 2D000 | 2D000 | 2D000 | 2D000 |
| P9 | hv_test | `DO $$ BEGIN INSERT INTO hv_t23 VALUES (9); END $$` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P14 | hv_test | `COPY hv_t23 FROM '/etc/hostname'` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P26 | hv_test | `CREATE TEMP TABLE hv_t23_tmp (a int)` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P37 | hv_test | `SELECT nextval('hv_t23_seq')` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P37 | hv_test | `SELECT setval('hv_t23_seq', 100)` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P38 | hv_test | `SELECT hv_t23_sd_write()` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P38 | hv_test | `SELECT hv_t23_dyn_write()` | 25006 | 25006 | 25006 | 25006 | 25006 | 25006 |
| P38 | hv_test | `SELECT query_to_xml('DELETE FROM hv_t23 RETURNING a', true, true, '')` | 0A000 | 0A000 | 0A000 | 0A000 | 0A000 | 0A000 |
| P38 | hv_test | `SELECT lo_from_bytea(0, '\x6876')` | gap | gap | gap | 25006 | 25006 | 25006 |
| P38 | hv_test | `SELECT lo_create(0)` | gap | gap | gap | 25006 | 25006 | 25006 |
| P38 | hv_test | `SELECT lo_put(230023, 0, '\x00')` | gap | gap | gap | 25006 | 25006 | 25006 |
| P38 | hv_test | `SELECT lo_unlink(230023)` | gap | gap | gap | 25006 | 25006 | 25006 |
| P38 | hv_test | `SELECT lo_import('/etc/hostname')` | gap | gap | gap | 25006 | 25006 | 25006 |
| P35 | hv_test | `SELECT txid_current()` | gap | gap | gap | gap | gap | gap |
| P5 | hv_ro | `RESET transaction_read_only` | neutralised | neutralised | neutralised | 0A000 | 0A000 | 0A000 |
| P6 | hv_ro | `SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P6 | hv_ro | `SET default_transaction_read_only = off` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P6 | hv_ro | `RESET ALL` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P10 | hv_ro | `PREPARE hv_t23_s AS SELECT 1` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P15 | hv_ro | `SELECT pg_advisory_lock(230023)` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P15 | hv_ro | `LOCK TABLE hv_t23 IN ACCESS EXCLUSIVE MODE` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P17 | hv_ro | `SET search_path = pg_catalog` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P23 | hv_ro | `SET standard_conforming_strings = off` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P25 | hv_ro | `NOTIFY hv_t23, 'x'` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P25 | hv_ro | `SELECT pg_notify('hv_t23', 'x')` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P26 | hv_ro | `LISTEN hv_t23` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P26 | hv_ro | `DECLARE hv_t23_c CURSOR WITH HOLD FOR SELECT 1` | neutralised | neutralised | neutralised | neutralised | neutralised | neutralised |
| P11 | hv_ro | `SELECT pg_terminate_backend(<admin's backend>)` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P13 | hv_ro | `COPY (SELECT 1) TO PROGRAM 'true'` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P14 | hv_ro | `SELECT pg_read_file('/etc/hostname')` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P14 | hv_ro | `COPY hv_t23 TO '/tmp/hv_t23_copy'` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P16 | hv_ro | `SET ROLE hv_test` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P16 | hv_ro | `SET SESSION AUTHORIZATION hv_test` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P31 | hv_ro | `SELECT pg_create_physical_replication_slot('hv_t23_slot')` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P32 | hv_ro | `SELECT pg_switch_wal()` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P32 | hv_ro | `SELECT pg_logical_emit_message(true, 'hv_t23', 'x')` | gap | gap | gap | gap | gap | gap |
| P33 | hv_ro | `SELECT lo_export(230023, '/tmp/hv_t23_lo')` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P34 | hv_ro | `CHECKPOINT` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |
| P34 | hv_ro | `SELECT pg_stat_reset()` | 42501 | 42501 | 42501 | 42501 | 42501 | 42501 |

Through `sshd-a` (pg17): 54 server-blocked rows, 0 different from the direct leg.

**MySQL** (mysql80 = 8.0, mysql84 = 8.4)

| Row | Account | Statement | mysql80 | mysql84 |
|---|---|---|---|---|
| M1 | hv_ro | `/*!50000 DROP TABLE hv_t23_bypass */` | 1792 | 1792 |
| M1 | hv_ro | `/*!50000 INSERT INTO hv_t23_bypass VALUES (50) */` | 1142 | 1142 |
| M2 | hv_ro | `SELECT 1; DROP TABLE hv_t23_bypass` | 1064 | 1064 |
| M5 | hv_ro | `SELECT * FROM hv_t23_bypass FOR UPDATE` | 1142 | 1142 |
| M5 | hv_ro | `LOCK TABLES hv_t23_bypass WRITE` | protocol | protocol |
| M6 | hv_ro | `PREPARE hv_t23_s FROM 'DELETE FROM hv_t23_bypass'` | protocol | protocol |
| M8 | hv_ro | `CREATE TABLE hv_t23_m8 (a int)` | 1142 | 1142 |
| M8 | hv_ro | `TRUNCATE TABLE hv_t23_bypass` | 1792 | 1792 |
| M8 | hv_ro | `DELETE FROM hv_t23_bypass` | 1142 | 1142 |
| M12 | hv_ro | `HANDLER hv_t23_bypass OPEN` | protocol | protocol |
| M13 | hv_ro | `XA START 'hv_t23'` | protocol | protocol |
| M17 | hv_ro | `CREATE TEMPORARY TABLE hv_t23_tmp (a int)` | 1044 | 1044 |
| M9 | hv_ro | `SET SESSION TRANSACTION READ WRITE, then CREATE TABLE` | 1142 | 1142 |
| M5 | hv_ro | `SELECT GET_LOCK('hv_m5', 0)` | neutralised | neutralised |
| M1 | hv_test | `/*!50000 DROP TABLE hv_t23_bypass */` | 1792 | 1792 |
| M1 | hv_test | `/*!50000 INSERT INTO hv_t23_bypass VALUES (50) */` | 1792 | 1792 |
| M2 | hv_test | `SELECT 1; DROP TABLE hv_t23_bypass` | 1064 | 1064 |
| M5 | hv_test | `SELECT * FROM hv_t23_bypass FOR UPDATE` | 1792 | 1792 |
| M5 | hv_test | `LOCK TABLES hv_t23_bypass WRITE` | protocol | protocol |
| M6 | hv_test | `PREPARE hv_t23_s FROM 'DELETE FROM hv_t23_bypass'` | protocol | protocol |
| M8 | hv_test | `CREATE TABLE hv_t23_m8 (a int)` | 1792 | 1792 |
| M8 | hv_test | `TRUNCATE TABLE hv_t23_bypass` | 1792 | 1792 |
| M8 | hv_test | `DELETE FROM hv_t23_bypass` | 1792 | 1792 |
| M12 | hv_test | `HANDLER hv_t23_bypass OPEN` | protocol | protocol |
| M13 | hv_test | `XA START 'hv_t23'` | protocol | protocol |
| M17 | hv_test | `CREATE TEMPORARY TABLE hv_t23_tmp (a int)` | 1792 | 1792 |
| M9 | hv_test | `SET SESSION TRANSACTION READ WRITE, then CREATE TABLE` | 1792 | 1792 |
| M5 | hv_test | `SELECT GET_LOCK('hv_m5', 0)` | neutralised | neutralised |
| M1 | hv_ro | `SELECT 1 /*!50000 , (SELECT GET_LOCK('hv_t23_m1', 0)) */` | neutralised | neutralised |
| M3 | hv_ro | `SET sql_mode = 'NO_BACKSLASH_ESCAPES'` | neutralised | neutralised |
| M5 | hv_ro | `SELECT GET_LOCK('hv_t23_m5', 0)` | neutralised | neutralised |
| M9 | hv_ro | `SET SESSION TRANSACTION READ WRITE` | neutralised | neutralised |
| M9 | hv_ro | `SET @@session.transaction_read_only = 0` | neutralised | neutralised |
| M14 | hv_ro | `SET @hv_t23 = 1` | neutralised | neutralised |
| M4 | hv_ro | `SELECT a FROM hv_t23_bypass INTO OUTFILE '/tmp/hv_t23_out'` | 1227 | 1227 |
| M10 | hv_ro | `SET GLOBAL max_connections = 999` | lost | lost |
| M10 | hv_ro | `SET PERSIST max_connections = 999` | lost | lost |
| M10 | hv_ro | `RESET PERSIST max_connections` | 3615 | 3615 |
| M10 | hv_ro | `FLUSH TABLES` | 1227 | 1227 |
| M10 | hv_ro | `KILL <admin's backend>` | 1095 | 1095 |
| M11 | hv_ro | `CREATE FUNCTION hv_t23_udf RETURNS STRING SONAME 'hv_t23.so'` | protocol | protocol |
| M7 | hv_ro | `CALL hv_t23_escape()` | 1370 | 1370 |
| n-3 | hv_ro | `ALTER USER USER() IDENTIFIED BY '…'` | 1792 | 1792 |
| n-3 | hv_ro | `ALTER USER CURRENT_USER() IDENTIFIED BY '…'` | 1792 | 1792 |
| n-3 | hv_ro | `SET PASSWORD = '…'` | gap | gap |
| M7/I-2 | hv_test | `CALL hv_t23_escape()` | **committed; L6 gap** | **committed; L6 gap** |
| M7/I-2 | hv_test | `CALL hv_t23_escape_ro()` | **escaped** (committed; guarded) | **escaped** (committed; guarded) |

Through `sshd-a` (mysql84): 24 server-blocked rows, 0 different from the direct leg.

**MariaDB (recorded, not judged; the label stays guarded, C19)** (mariadb106 = 10.6, mariadb1011 = 10.11, mariadb114 = 11.4)

| Row | Account | Statement | mariadb106 | mariadb1011 | mariadb114 |
|---|---|---|---|---|---|
| M1 | hv_ro | `/*!50000 DROP TABLE hv_t23_bypass */` | 1792 | 1792 | 1792 |
| M1 | hv_ro | `/*!50000 INSERT INTO hv_t23_bypass VALUES (50) */` | 1142 | 1142 | 1142 |
| M2 | hv_ro | `SELECT 1; DROP TABLE hv_t23_bypass` | 1064 | 1064 | 1064 |
| M5 | hv_ro | `SELECT * FROM hv_t23_bypass FOR UPDATE` | 1792 | 1792 | 1792 |
| M5 | hv_ro | `LOCK TABLES hv_t23_bypass WRITE` | gap | gap | gap |
| M6 | hv_ro | `PREPARE hv_t23_s FROM 'DELETE FROM hv_t23_bypass'` | protocol | protocol | protocol |
| M8 | hv_ro | `CREATE TABLE hv_t23_m8 (a int)` | 1142 | 1142 | 1142 |
| M8 | hv_ro | `TRUNCATE TABLE hv_t23_bypass` | 1792 | 1792 | 1792 |
| M8 | hv_ro | `DELETE FROM hv_t23_bypass` | 1142 | 1142 | 1142 |
| M12 | hv_ro | `HANDLER hv_t23_bypass OPEN` | neutralised | neutralised | neutralised |
| M13 | hv_ro | `XA START 'hv_t23'` | 1400 | 1400 | 1400 |
| M17 | hv_ro | `CREATE TEMPORARY TABLE hv_t23_tmp (a int)` | 1044 | 1044 | 1044 |
| M9 | hv_ro | `SET SESSION TRANSACTION READ WRITE, then CREATE TABLE` | 1142 | 1142 | 1142 |
| M5 | hv_ro | `SELECT GET_LOCK('hv_m5', 0)` | neutralised | neutralised | neutralised |
| M1 | hv_test | `/*!50000 DROP TABLE hv_t23_bypass */` | 1792 | 1792 | 1792 |
| M1 | hv_test | `/*!50000 INSERT INTO hv_t23_bypass VALUES (50) */` | 1792 | 1792 | 1792 |
| M2 | hv_test | `SELECT 1; DROP TABLE hv_t23_bypass` | 1064 | 1064 | 1064 |
| M5 | hv_test | `SELECT * FROM hv_t23_bypass FOR UPDATE` | 1792 | 1792 | 1792 |
| M5 | hv_test | `LOCK TABLES hv_t23_bypass WRITE` | gap | gap | gap |
| M6 | hv_test | `PREPARE hv_t23_s FROM 'DELETE FROM hv_t23_bypass'` | protocol | protocol | protocol |
| M8 | hv_test | `CREATE TABLE hv_t23_m8 (a int)` | 1792 | 1792 | 1792 |
| M8 | hv_test | `TRUNCATE TABLE hv_t23_bypass` | 1792 | 1792 | 1792 |
| M8 | hv_test | `DELETE FROM hv_t23_bypass` | 1792 | 1792 | 1792 |
| M12 | hv_test | `HANDLER hv_t23_bypass OPEN` | neutralised | neutralised | neutralised |
| M13 | hv_test | `XA START 'hv_t23'` | 1400 | 1400 | 1400 |
| M17 | hv_test | `CREATE TEMPORARY TABLE hv_t23_tmp (a int)` | 1792 | 1792 | 1792 |
| M9 | hv_test | `SET SESSION TRANSACTION READ WRITE, then CREATE TABLE` | 1792 | 1792 | 1792 |
| M5 | hv_test | `SELECT GET_LOCK('hv_m5', 0)` | neutralised | neutralised | neutralised |
| M1 | hv_ro | `SELECT 1 /*!50000 , (SELECT GET_LOCK('hv_t23_m1', 0)) */` | neutralised | neutralised | neutralised |
| M3 | hv_ro | `SET sql_mode = 'NO_BACKSLASH_ESCAPES'` | neutralised | neutralised | neutralised |
| M5 | hv_ro | `SELECT GET_LOCK('hv_t23_m5', 0)` | neutralised | neutralised | neutralised |
| M9 | hv_ro | `SET SESSION TRANSACTION READ WRITE` | neutralised | neutralised | neutralised |
| M9 | hv_ro | `SET @@session.transaction_read_only = 0` | neutralised | neutralised | neutralised |
| M14 | hv_ro | `SET @hv_t23 = 1` | neutralised | neutralised | neutralised |
| M4 | hv_ro | `SELECT a FROM hv_t23_bypass INTO OUTFILE '/tmp/hv_t23_out'` | 1227 | 1227 | 1227 |
| M10 | hv_ro | `SET GLOBAL max_connections = 999` | 1227 | 1227 | 1227 |
| M10 | hv_ro | `SET PERSIST max_connections = 999` | 1064 | 1064 | 1064 |
| M10 | hv_ro | `RESET PERSIST max_connections` | 1064 | 1064 | 1064 |
| M10 | hv_ro | `FLUSH TABLES` | 1227 | 1227 | 1227 |
| M10 | hv_ro | `KILL <admin's backend>` | 1095 | 1095 | 1095 |
| M11 | hv_ro | `CREATE FUNCTION hv_t23_udf RETURNS STRING SONAME 'hv_t23.so'` | 1792 | 1792 | 1792 |
| M7 | hv_ro | `CALL hv_t23_escape()` | 1370 | 1370 | 1370 |
| n-3 | hv_ro | `ALTER USER USER() IDENTIFIED BY '…'` | 1064 | 1064 | 1064 |
| n-3 | hv_ro | `ALTER USER CURRENT_USER() IDENTIFIED BY '…'` | 1792 | 1792 | 1792 |
| n-3 | hv_ro | `SET PASSWORD = '…'` | gap | gap | gap |
| M7/I-2 | hv_test | `CALL hv_t23_escape()` | **committed; L6 gap** | **committed; L6 gap** | **committed; L6 gap** |
| M7/I-2 | hv_test | `CALL hv_t23_escape_ro()` | **committed; L6 gap** | **committed; L6 gap** | **committed; L6 gap** |

**SQLite (no server; runs on every PR).** A read-only session over a fresh file refused `INSERT`
(L9), `ATTACH '<dir>/n.db'` and `VACUUM INTO '<dir>/c.db'` (L2), `PRAGMA query_only = 0` (L1),
`CREATE TEMP TABLE` (L8), `writefile` and `load_extension` (L3), `PRAGMA writable_schema = 1`
(L4), `PRAGMA user_version = 7` and `PRAGMA journal_mode = WAL` (L9), and `BEGIN IMMEDIATE`
(1a), staying usable after each. Side channel: the directory holds only `db.sqlite`, and the
database is byte-identical.

**What the results say.**

- **PostgreSQL 13–18: every server-blocked row is stopped by L3, 1a or the protocol alone** — the
  superuser rows show it — and no row left a trace. `hv_ro` meets its missing privilege first on
  `COPY FROM` a file, `nextval`/`setval` and the large-object rows of PostgreSQL 13–15 (`42501`).
  P9's `DO` block is `25006`.
- **Large objects (T5 review m-5).** The read-only transaction refuses `lo_create`,
  `lo_from_bytea`, `lo_put`, `lo_unlink` and `lo_import` only from **PostgreSQL 16** (`25006`).
  On 13–15 they run: they assign a transaction ID, the guard's `ROLLBACK` undoes them, and L6's
  canary reports the gap and discards the connection. Nothing persisted on any line (the side
  channel counts the large objects and reads the fixture one). So on 13–15 it is L6, not L3, that
  stops them; m-5's "only the read-only transaction stops it" holds from 16.
- `RESET transaction_read_only` runs on 13–15 — it restores L4's `default_transaction_read_only
  = on`, a no-op — and is `0A000` from 16. `query_to_xml` cannot run a `DELETE` at all (`0A000`);
  a PL/pgSQL `EXECUTE` (`hv_t23_dyn_write`) stands for P38's dynamic SQL and is `25006`.
  `txid_current()` (P35) is a gap on every line: the canary trips with no write (the S-10 false
  positive class, fail closed).
- **P32 (finding F1).** `pg_logical_emit_message(true, …)` is a gap (rolled back). Its
  non-transactional form is executable by `PUBLIC` on 13–18: `hv_ro`, labelled enforced, writes a
  WAL record that no rollback undoes and no canary sees. Not in the tables; the failing test
  `zz_docker_m2b_findings::f1_pg_enforced_role_cannot_write_wal_messages` holds it.
- **MySQL 8.0/8.4: every server-blocked row is stopped by L4 or the protocol alone** — the owner
  rows are `1792` or `protocol` — and no row left a trace. The brief's M9 is `1792` for the owner
  (the reset and the replay restore the read-only setting; the snapshot shows no `hv_m9`) and
  `1142` for `hv_ro`. User locks (`GET_LOCK`, also inside an executable comment) do not survive
  the request.
- **T18 carries.** `CALL` of a procedure that commits, opens a read-write transaction, writes and
  commits: `hv_ro` has no `EXECUTE` (`1370`). The owner's `hv_t23_escape` (ends with no
  transaction) is a **committed; L6 gap** on MySQL and MariaDB: L6 detects it and discards the
  connection, but the row **was committed** — L3 and L4 cannot stop a procedure that overrides the
  session's read-only setting. The owner's `hv_t23_escape_ro` (ends inside a new read-only
  transaction) **escapes every runtime layer** on MySQL (on MariaDB it too ends as a committed
  gap); only L5 is honest: the account is labelled guarded (asserted). Both are the `EXECUTE`
  residual Task 18 recorded; least privilege (no `EXECUTE`) is the remedy. `SET GLOBAL` and
  `SET PERSIST` as `hv_ro` change nothing (side channel) but end the connection until the F2 fix
  lands (`lost`); `RESET PERSIST` is `3615` — nothing is persisted, so the server answers before
  any privilege check, and **L5 for `RESET PERSIST` is untested by this fixture**. Review n-3:
  `ALTER USER USER()` and `ALTER USER CURRENT_USER()` are `1792`, `SET PASSWORD` a gap (refused
  after its implicit commit, judged), and `hv_ro` still signs in with the matrix password.
- **MariaDB (recorded).** Like MySQL on the read-only setting (`1792`), with these differences:
  `LOCK TABLES` is preparable and commits implicitly (a gap, nothing persisted, discarded);
  `HANDLER … OPEN` runs (neutralised by `COM_RESET_CONNECTION`); `XA START` is `1400`;
  `SET PERSIST`, `RESET PERSIST` and `ALTER USER USER()` are syntax errors there
  (`CURRENT_USER()` is `1792`); `SET GLOBAL` is `1227` in both protocols. The label stays guarded
  (C19); nothing here is claimed as enforced.
- **Through `sshd-a`** (pg17 over the channel's stream, mysql84 through the loopback forwarder)
  every server-blocked row gave the direct leg's result.

**Findings.** Failing tests in `tests/zz_docker_m2b_findings.rs` (the file sorts last on
purpose: cargo stops at the first failing test binary, and a known-red finding must not hide the
rest of the Docker suite), routed by the controller:

- **F1** (role probe / remediation, T5/T18): above.
- **F2** (MySQL driver, T13, vendored `mysql_async`): on MySQL 8.0 and 8.4 an account without
  `SYSTEM_VARIABLES_ADMIN` that sends `SET GLOBAL …` or `SET PERSIST …` through the
  prepared-statement protocol (every user statement of a session) loses the connection: the
  client reads EOF and the server counts an aborted client. Over `COM_QUERY` the same text is
  `1227`, as is every other privilege refusal tried (`SET SESSION sql_log_bin`, `FLUSH TABLES`, a
  `SELECT` on `mysql.user`); MariaDB answers `1227` in both protocols. The user is told
  "connection lost, outcome unknown" for a statement the server refused, and the session
  reconnects. Nothing changes on the server.
- **F3** (session reset, T7; pool, T15) — **fixed** on `m2b-fix-f3` (merged into this branch;
  ADR section `m2b-t07 — internal value reads`): released PostgreSQL pooled sessions were never
  parked, because the reset's verify probe ran on the count-only extended protocol. Its test is
  now `docker_m2b_cancel_reconnect::a_released_pooled_session_is_parked_and_reused`, and the T17
  R2 test runs on pg17 as well as mysql84.

**Other conformance results (exit criteria E1–E3, E6–E8, E14).** `docker_m2b_connect.rs`: every
TCP target of the matrix (PostgreSQL 14–18, pg17-tls, MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4)
connects directly and through `sshd-a`, and pg17 and mysql84 also through the `sshd-a` → `sshd-b`
chain (the first hop with the encrypted key's passphrase, the second with the bastion password,
from the overlay); the bastions' host keys are trusted `Strict` from their published keys. Through
SSH the trail is `Ssh, Connect, [Tls,] SignIn, CheckServer`, the address private, `via` the last
hop, PostgreSQL on the channel's stream and MySQL through the forwarder. A MySQL-family server
without TLS needs `Disable` through SSH: its `Auto` falls back to plaintext on loopback only (the
open PO question Q41). Two profiles share one SSH session, and it stays up after one profile's
connection has closed on the server. `docker_m2b_tls_ssh.rs`: pg17-tls and mysql84 in `Auto` are
reviewed once, pinned (`TlsVerified::Pinned`, one `AddPin` for the database host, its in-network
name through SSH) and reconnect pinned headless; mysql84 `VerifyCa` with its container CA is
`CaOnly`; pg17-auth's md5 and cleartext roles are refused through SSH under `Auto`
(`auth.insecure_method`), allowed with `Disable` and on loopback, and SCRAM connects.
`docker_m2b_cancel_reconnect.rs`: a cancel ends `pg_sleep(60)` and a `SLEEP` over a table within
1 s on pg17, mysql84 and mariadb114, directly and through SSH, and an endless SQLite CTE (MySQL's
`SELECT SLEEP(60)` alone is answered with its result when killed, so it finishes,
`AlreadyFinished`, also within 1 s); a terminated backend inside a transaction gives `SessionLost
{ open_tx, temp_objects }` and `Lost` (T17 C1: never replayed; the brief's `Connected` predates
C1), and Retry connects without the old temporary table; an `INSERT` cut mid-flight is
`ConnectionLost { outcome_unknown: true }` and ran at most once; a 20 s outage reconnects within
the B19 schedule (bounded at 75 s: the sixth attempt is due 45 s ± 20 % after the loss, past the
brief's 45 s); the read-only guard's `BEGIN` answered by `FATAL 57P01` reconnects (T17 P1; the
named exception in `readonly.rs`: `lost_class` takes `recovery::connection_ending`, the reply is
`ConnectionLost`); an SSH chain's death closes the parked pooled sessions (`close_idle`, T17 R2;
a mutation run without it timed out on MySQL). The waits are conditions, never fixed sleeps
(fix round 1, I1): a request "has run for 600 ms" is read from the session's own `Busy { since }`,
and a released session "is parked" from `SessionManager::parked_count` (a read-only accessor added
to the pool as a named exception).

**Not done: MySQL client-certificate sign-in (T13 carry).** It needs a MySQL target whose
`ssl_ca` trusts a committed test CA and an account `REQUIRE X509`: a compose change and an init
script that only take effect on `cargo xtask docker up`, which only the controller runs. Routed
back.

## m2b-t07 — internal value reads (finding F3)

**The defect.** T23's F3: the reset's verify probe and the schema probe ran with
`ExecOpts::default()`, the extended protocol, which the PostgreSQL and MySQL drivers run
count-only (ADR 0002 m2b-t10, m2b-t13: the rows are counted, their values dropped). The verify
probe never read its `t`, so on PostgreSQL every pooled release broke its session and every
borrow opened a new connection. The schema probe read no row on PostgreSQL and MySQL, so a
re-probe forgot the context (`schema: None`) instead of updating it, and no reset or guarded
replay restored a schema. `FakeDriver` answered scripted rows on either protocol, so the T7 and
T15 tests passed.

**The rule.** Every statement of Hacivat's own whose values it reads runs through
`ExecOpts::values(max_rows)` (`hacivat-driver`: the simple protocol, whose rows every driver
returns as text): the verify probe (one row), the schema probe (`SCHEMA_PROBE_MAX_ROWS`, 128:
SQLite answers one row per attached database, at most 125) and, as before, the role probe, its
PostgreSQL identity check and the guard's canary (T18's `simple(..)`, the same options). User
statements stay on the extended protocol, whose command tag the read-only post-check reads
(review M2). Audited, reading no value: the environment plan, `ROLLBACK`, `SET autocommit`, the
guard's `before`, `after` (its notice codes only) and reset replay, and the side connection's
`KILL`/`pg_terminate_backend` (`batch`). A verify probe that answers no row, or anything but
true, still breaks the session (fail closed).

**The fake cannot hide it again.** `FakeDriver::execute` with `Protocol::Extended` returns a
scripted answer without its row values (the row count stays), on every engine, as the real
count-only path does; a script reaches a value read only through `ExecOpts::values`. Before the
session fix this turned 13 session tests red (pool 5, env_schema 4, manager_m2b, readonly and
session_actor 1 each); `fake::tests::the_count_only_path_keeps_no_values` holds the contract.

**Behaviour change.** With the context known after a re-probe, a pooled session's reset and a
read-only session's guarded replay now restore the schema (PostgreSQL the whole `search_path`,
M11), as the m2b-t07 section always specified.

**Tests.** Driver contract on a real server: `hacivat-driver-pg`
`docker_exec::internal_value_reads_decode_through_the_values_path` (after the driver's reset the
verify probe decodes `t` and the schema probe's five columns decode; the default path keeps no
value). Session over Docker, in `docker_m2b_cancel_reconnect.rs` (F3 moved there from
`zz_docker_m2b_findings.rs`): `a_released_pooled_session_is_parked_and_reused` and
`the_schema_probe_reads_the_servers_answer` on pg17 and mysql84, and T17 R2's
`ssh_death_closes_parked_pooled_sessions` now runs pg17 too (T23 deviation 7).
