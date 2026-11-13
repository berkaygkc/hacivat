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
