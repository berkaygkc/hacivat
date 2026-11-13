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

Rejected: trusting `rows_affected` alone (DDL has none); treating an unknown transaction state
as clean (the session may not have recorded it, and the guard cannot then confirm that the
transaction survived).

**Input limits (`check_input`).** At most `MAX_STATEMENT_BYTES` (1 MiB) in UTF-8 bytes, not
characters, and no NUL (C-string APIs would cut the statement there and run a different one).

**SQLite (B21).** Hardening: `query_only`, `SQLITE_DBCONFIG_DEFENSIVE` on, `ATTACH_CREATE` and
`ATTACH_WRITE` off, `LIMIT_ATTACHED` 0, `LIMIT_LENGTH` 100,000,000, `LIMIT_SQL_LENGTH` 1 MiB (the
input limit), `LIMIT_VARIABLE_NUMBER` 32,766. The authorizer denies by default and allows only
`Read`, `Select`, `Recursive`, functions other than `load_extension`, `readfile`, `writefile`,
`edit` and `fts3_tokenizer`, and the pragmas of `SQLITE_READ_PRAGMAS` **without an argument**
(Q16: `query_only`, `journal_mode`, `trusted_schema` and `foreign_keys` are readable; with an
argument each is a setter and stays denied). Names are compared with ASCII case folding
(INV-3) and must match whole: a schema prefix (`main.query_only`), padding or a NUL is denied
(SQLite passes the bare pragma name and the schema separately, so this never denies a real
read). The `match` names every `SqliteAction`, so a new action needs a decision; `Other(code)`
is always denied. The authorizer returns `Deny`, never `Ignore`: `Ignore` turns a denied column
read into NULL and lets the statement run.

Known cost: the name-taking pragmas (`table_info`, `table_xinfo`, `index_list`, `index_info`,
`index_xinfo`, `foreign_key_list`) are only useful with an argument, so on a read-only
connection they are denied in both the `PRAGMA x(t)` and the `pragma_x('t')` forms (the
table-valued form prepares the same pragma and asks the authorizer again: the sqlite3 shell's
`.auth on` trace shows `PRAGMA "table_info" "t"` for `SELECT … FROM pragma_table_info('t')`, and
`PRAGMA "query_only" NULL "main"` for `PRAGMA main.query_only`). Introspection of a
read-only SQLite profile reads `sqlite_schema` or uses a separate connection. Allowing a name
argument for those six only is a one-line change that needs a controller ruling.

**Role probe (MCP-09 step 1).** PostgreSQL runs five statements (role flags; membership in
`pg_read_server_files`, `pg_write_server_files`, `pg_execute_server_program`,
`pg_signal_backend`, `pg_write_all_data`; any owned or writable user relation; TEMP on the
database; `dblink`, `postgres_fdw`, `pg_net`, `http`). Least privilege is all of: no flag set,
no predefined-role row, no writable relation, no egress extension. TEMP adds `TempPrivilege`
and keeps least privilege (Q21: the read-only transaction already refuses `CREATE TEMP`,
#pg-ro-temp; the reset drops temp objects). MySQL and MariaDB run `SHOW GRANTS`; least
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
not `GRANT`/`REVOKE`; an unterminated quote, unbalanced parentheses or an unquoted `;`; no
`TO`; a second `ON`/`TO` after the grantee; an empty or quoted privilege; a privilege without a
row in the table (`PROCESS`, `SHOW DATABASES`, dynamic `*_ADMIN` privileges); any engine
without a probe. An `EgressExtension` warning carries the name from the probe's list, never
the server's text.

**Labels (`access_label`, D-035).** Not read-only → `ReadWrite`. Read-only →
`ReadOnlyEnforced` only for PostgreSQL or MySQL whose findings say least privilege **and**
carry no warning that denies it (only PostgreSQL's `TempPrivilege` is neutral), so
self-contradicting findings are guarded. Everything else is `ReadOnlyGuarded`: MariaDB (C19:
guarded until it passes the bypass harness in the Docker matrix), SQLite, other engines, no
probe, a failed probe. Rejected: labelling MariaDB enforced on a clean `SHOW GRANTS`.

**Remediation.** PostgreSQL: `CREATE ROLE <role> LOGIN PASSWORD '<password>'`,
`GRANT pg_read_all_data TO <role>`, and `-- optional: REVOKE TEMP ON DATABASE <database> FROM
PUBLIC;` as a comment (revoking from `PUBLIC` affects every role, and TEMP alone keeps the label
enforced). No `NOINHERIT`: a non-inheriting member would not get `pg_read_all_data` and could
read nothing (Q21). MySQL: `CREATE USER '<role>'@'%' IDENTIFIED BY '<password>'`,
`GRANT SELECT, SHOW VIEW ON <database>.* TO '<role>'@'%'`, which the probe reads as least
privilege (tested). MariaDB, SQLite and every other engine: `None`, since no role change makes
their label enforced. The strings carry only the `<role>`, `<password>` and `<database>`
placeholders; the UI shows and copies them unchanged.

**Tests.** `crates/core/hacivat-guard/tests/{plans,post,sqlite_authorizer,probe,labels}.rs`
(50 tests): exact plan values; every gap and its order; unknown transaction state; tags in any
case, with and without row counts; byte-counted input limits; every pragma of the list with
and without an argument, in both cases, and malformed names; every write, attach and raw
action code (-1..=40 and the `i32` extremes) denied; every PostgreSQL warning and every
malformed probe shape; MySQL grant lines with odd whitespace, case, quoting, column lists,
role grants, grant options, partial revokes and a smuggled second statement; labels, including
self-contradicting findings; the exact remediation strings.
