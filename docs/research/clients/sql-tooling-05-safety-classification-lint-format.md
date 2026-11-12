# SQL Language Tooling 05 — Statement Safety Classification, Linting and Formatting

Series: `01` landscape · `02` lexing & splitting · `03` parsing architecture · `04` completion & resolution · **`05` safety, lint, format**.

Part A (security-critical) specifies how Hacivat and Karagöz classify SQL statements by effect (read / write / DDL / DCL / transaction control / session state / procedural / external I/O / admin), the exhaustive catalogue of read-only **bypasses** per dialect, and the layered defence Karagöz must apply (classifier + single-statement protocol + server-side read-only transaction + least-privilege role + resource limits + audit). Part B covers lint rule frameworks and catalogues (sqruff/SQLFluff, postgres-language-server). Part C specifies the formatter pipeline (Wadler-style document printer with alignment and verification).

---

## 1. Scope and sources

| Source | Commit | License | Read |
|---|---|---|---|
| libpg_query / pg_query.rs | `27112f8af5de` / `20f3a182dc74` | BSD-3 + PostgreSQL / MIT | Parse tree node kinds, `ParseResult` table/function contexts, `statement_types`, `is_utility_stmt`, `fingerprint`, `normalize` |
| postgres-language-server `pgls_analyser`, `pgls_suppressions`, `pgls_pretty_print`, `pgls_typecheck` | `083c5d1bbf04` | MIT | Lint categories & rules, suppression comments, formatter with AST-equivalence verification, PREPARE probe |
| sqruff `crates/lib/src/rules/*`, `core/linter/core.rs`, `utils/reflow/*` | `c3d18575d6d4` | Apache-2.0 | Rule catalogue (codes, names, fixability), fix loop, reflow |
| pgcli / mycli destructive warnings | `dc9ce9447e0f` / `525f0315ef86` | BSD-3 | Prefix-based heuristics (anti-pattern for security) |
| datafusion-sqlparser-rs AST + `Visitor` | `8a176d98b37b` | Apache-2.0 | Multi-dialect statement enumeration |
| Vendor documentation (PostgreSQL `SET TRANSACTION`, `utility.c ClassifyUtilityCommandAsReadOnly` behaviour, function docs; MySQL `START TRANSACTION`, `read_only`/`super_read_only`, privileges, executable comments; SQL Server permissions, `ApplicationIntent`; SQLite `sqlite3_stmt_readonly`, `sqlite3_set_authorizer`, `query_only`; ClickHouse `readonly` setting/profiles/grants; Oracle `SET TRANSACTION READ ONLY`, `READ` vs `SELECT` privilege, autonomous transactions) | current | n/a | Facts tagged **[Inferred: vendor docs/source knowledge]**; each listed in §7 for conformance verification |

The research `access/` folder covers RBAC, approvals, masking and audit storage; this document covers the SQL-analysis side and the execution guards that depend on it.

---

## 2. Executive summary

1. **A parser-based classifier is necessary but never sufficient** for read-only enforcement. Side effects hide in functions (`pg_terminate_backend`, `dblink_exec`, `nextval`, `GET_LOCK`, `xp_cmdshell`), operators/casts/views that call functions, dynamic SQL (`EXECUTE`, `query_to_xml('…')`, `PREPARE … FROM @s`, `sp_executesql`), procedures with internal `COMMIT`, MySQL executable comments, and *parser differentials* between our lexer and the server's. Karagöz must therefore enforce on the server (read-only transaction with pinned snapshot, least-privilege role, single-statement protocol) **and** classify (UX, early denial, audit, object-level policy).
2. **Classify on the server's own grammar where possible.** For PostgreSQL use libpg_query (PG 18.6 grammar = server grammar); unknown syntax ⇒ deny. For other dialects, use our strict parser plus sqlparser-rs as a second opinion and take the most dangerous result; any parse failure ⇒ deny. [Observed: `pg_query.rs/src/query.rs:36`; ALG-SQL-LAND-01 in doc 01]
3. **Prefix heuristics are trivially bypassable** (`WITH d AS (DELETE …) SELECT …`, `EXPLAIN ANALYZE DELETE …`, `SELECT … INTO new_table`, `SELECT pg_terminate_backend(…)`, leading comments, `/*!*/`). pgcli/mycli use them only for *warnings*; Hacivat must not use them for decisions. [Observed: `pgcli/packages/parseutils/__init__.py:17-53`; `mycli/packages/utils/sql_utils.py:341-400`]
4. **Effect model, not a single label:** each statement yields a set of effects (`ReadData`, `ReadMetadata`, `LockRows`, `LockTables`, `WriteData`, `WriteTemp`, `Ddl`, `Dcl`, `TxnControl`, `SessionState`, `PrivilegeChange`, `ProceduralCode`, `DynamicSql`, `ExternalFileRead/Write`, `ExternalNetwork`, `ProcessExec`, `AdminControl`, `Notify`, `SequenceAdvance`, `ResourceHeavy`, `InfoDisclosure`, `Unknown`) + touched objects + reasons; a *policy* maps effects to allow/deny/confirm. Coarse classes (Read / Write / DDL / …) are derived for UI.
5. **Walk the whole tree, not the top node:** data-modifying CTEs, `SELECT INTO`, locking clauses (`FOR UPDATE/SHARE`), `EXPLAIN ANALYZE <stmt>` (executes!), `EXPLAIN` without ANALYZE (does not execute but the planner may constant-fold IMMUTABLE functions), `COPY` direction and target (`TO PROGRAM`, `TO 'file'`), `DECLARE CURSOR` inner query, `CREATE TABLE AS`/`CREATE MATERIALIZED VIEW`, rules, `PREPARE` (inner effects + persistent state) and `EXECUTE` (opaque ⇒ Unknown).
6. **PostgreSQL read-only transaction** (`BEGIN … READ ONLY`) blocks writes to non-temporary relations, all DDL, `COMMENT`, `GRANT/REVOKE`, `TRUNCATE`, `nextval/setval` on non-temp sequences, and `EXPLAIN ANALYZE`/`EXECUTE` of such — including writes inside functions and SPI (so `query_to_xml('DELETE …')` fails). It does **not** block: `pg_terminate_backend`/`pg_cancel_backend`, advisory locks, `LOCK TABLE` (any mode — DoS), `NOTIFY`/`pg_notify`, `set_config`/`SET` (incl. `SET ROLE`), `pg_sleep`, file-reading functions, `COPY … TO PROGRAM/file` (read direction), `dblink`/`dblink_exec` (opens a *separate* connection — full bypass), HTTP extensions (`pgsql-http`), untrusted PL functions doing OS-level I/O, `pg_logical_slot_get_changes` (consumes CDC). Further verified details (corrected after verification): `CREATE TEMP TABLE`/`CREATE TEMP SEQUENCE` **are** blocked inside a RO txn, but DML on temp tables and `nextval` on temp sequences created earlier in the session are allowed; `NOTIFY` is accepted but only delivered on COMMIT (ROLLBACK discards it); `lo_export` and `COPY … TO 'file'` write server files; `pg_create_physical_replication_slot` succeeds and the slot survives ROLLBACK; `postgres_fdw` opens a remote transaction that is **not** read-only (remote views/functions can write) but is rolled back/committed together with the local one, while direct writes to foreign tables are blocked locally. [Observed: docker test, postgres:16/17/18 (16.15, 17.11, 18.6), see verification/01-db-behavior-tests.md#pg-ro-escapes; `pgsql-http`, untrusted PL and logical slots not tested]
7. **Pin the snapshot to make read-only irreversible:** PostgreSQL only allows switching a transaction to READ WRITE *before its first snapshot*; after `BEGIN READ ONLY; SELECT 1;`, any `SET TRANSACTION READ WRITE`, `SET transaction_read_only = off` or `set_config('transaction_read_only','off',…)` fails ("transaction read-write mode must be set before any query"). Combined with the extended query protocol (one statement per `Parse`), the user cannot escape the transaction except by ending it (`COMMIT`/`ROLLBACK`), after which nothing else of theirs runs in that request. `RESET transaction_read_only` also fails ("cannot be reset"). [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-ro-pin]
8. **Least privilege is the only robust layer** for MySQL, SQL Server and Oracle, where session-level read-only modes are weaker (MySQL DDL implicit commit semantics, SQL Server has no read-only session mode, Oracle autonomous transactions/definer-rights code). Karagöz's discovery/onboarding (access/ docs) should create or recommend a dedicated read-only database principal per access level.
9. **Engine-assisted checks exist and should be used:** SQLite `sqlite3_stmt_readonly()` + `sqlite3_set_authorizer` + `SQLITE_OPEN_READONLY`; ClickHouse `readonly=1` in a server-side profile (cannot be changed by the session) + revoked source grants; PG `txid_current_if_assigned() IS NULL` after execution as a canary that no XID (no local write) occurred — (corrected after verification) it is a tripwire, not a proof: it stays NULL after `dblink_exec` and `COPY … TO PROGRAM`, and turns non-NULL (false alarm) after `LOCK … IN ACCESS EXCLUSIVE MODE`, temp-table DML and `txid_current()`. [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-canary]
10. **Lexical modes are attack surface:** MySQL `SET NAMES gbk`-style charset switches, `sql_mode` (`ANSI_QUOTES`, `NO_BACKSLASH_ESCAPES`), PG `standard_conforming_strings`, T-SQL `QUOTED_IDENTIFIER` change how the *next* statement lexes. Karagöz pins these at session start and denies statements that change them.
11. **Resource and disclosure risks remain under read-only:** unbounded scans/cartesian joins, `pg_sleep`, lock waits, `generate_series(1, 1e12)`, reading other sessions' query text (`pg_stat_activity`), server files (`pg_read_file`, `LOAD_FILE`, `OPENROWSET(BULK)`), SSRF via ClickHouse `url()`/`s3()`. Enforce statement/lock/idle timeouts, row/byte caps, and deny-listed functions.
12. **Lint framework:** rules declare code, category, severity, dialects, required analysis level (syntax / semantic / catalog / live DB), and fix safety; suppressions via comments (pgls: `-- pgls-ignore <rule>`, `-- pgls-ignore-start/-end`, `-- pgls-ignore-all`). [Observed: `pgls_suppressions/src/lib.rs:250-370`, `parser.rs:60-95`]
13. **Rule catalogues to draw from:** sqruff (≈78 rules: AL aliasing, AM ambiguous, CP capitalisation, CV convention, LT layout, RF references, ST structure, + PG/T-SQL/Oracle specific) and pgls analyser (≈80 rules: correctness, destructive, safety (migration locking), style, typecheck). Layout/capitalisation rules are better implemented by the formatter than by lint fixes. [Observed: rule files listed in §3.8–3.9]
14. **Formatter: Wadler/Prettier document IR** over the lossless CST, comments attached as leading/trailing/dangling, an alignment pre-pass (measure cells flat → pad) for column definitions, `AS` aliases and `SET` assignments, keyword case normalisation on keyword tokens only, and **verification**: re-lex the output and require the trivia-free token stream to be identical modulo keyword case (catches `- -` → `--`, lost newline after `--` comments, PG string-continuation newline loss, `a $1` → `a$1` merges); for PG additionally compare libpg_query fingerprints/normalised ASTs like pgls. [Observed: `pgls_pretty_print/src/lib.rs:200-330`]
15. **Never format what does not parse cleanly:** statements with `ERROR` nodes or unsupported kinds are emitted verbatim; pgls refuses to format when a comment cannot be placed rather than drop it. [Observed: `pgls_pretty_print/src/lib.rs:263-275`]

---

## 3. Detailed findings — Part A: safety classification

### 3.1 Threat model (Karagöz)

| Actor | Path | Goal | Notes |
|---|---|---|---|
| Legitimate user with read-only grant | Hacivat → P2P → Karagöz | Accidentally run a write | UX warnings + hard deny |
| Malicious/compromised user with read-only grant | Hacivat (or custom client) → Karagöz | Escalate to write/DDL/admin, exfiltrate beyond grant, DoS | Must assume a fully custom client: **all** client-side checks are bypassed; Karagöz is the enforcement point (README: authorization enforceable at Karagöz) |
| AI agent via Hacivat's local MCP server | MCP tool call → Hacivat → Karagöz | Prompt-injected agent issues destructive SQL | Same as above + Hacivat MCP policy (read-only by default) |
| Perde compromise | control plane | Issue a forged capability | Out of scope here (access/ docs); Karagöz still applies local policy ceilings |

Security goal for "read-only" access level: **no persistent change** to database state (data, schema, privileges, configuration, replication state), **no external side effects** (files, network, processes, other sessions), **bounded resource use**, and **no access outside granted objects**.

### 3.2 Effect taxonomy

| Effect | Meaning | Default under read-only policy |
|---|---|---|
| `ReadData` | Reads table/view rows | allow (subject to object policy) |
| `ReadMetadata` | Catalog/`SHOW`/`DESCRIBE`/`EXPLAIN` (no execution) | allow |
| `InfoDisclosure` | Reads server internals beyond normal metadata (other sessions' queries, settings, files, `version()`) | configurable (default allow metadata, deny files/activity of others) |
| `LockRows` | `SELECT … FOR UPDATE/SHARE` | deny |
| `LockTables` / `AdvisoryLock` | `LOCK TABLE`, `GET_LOCK`, `pg_advisory_lock` | deny |
| `WriteData` | INSERT/UPDATE/DELETE/MERGE/UPSERT/REPLACE/COPY FROM/LOAD DATA/TRUNCATE | deny |
| `WriteTemp` | writes to temp tables/table variables only | deny in v1 (allow later per policy) |
| `SequenceAdvance` | `nextval`, `NEXT VALUE FOR`, identity consumption | deny |
| `Ddl` | CREATE/ALTER/DROP/RENAME/COMMENT/SECURITY LABEL/REFRESH MATVIEW/REINDEX/CLUSTER/VACUUM/ANALYZE | deny |
| `Dcl` | GRANT/REVOKE/CREATE ROLE/ALTER ROLE/OWNER changes | deny |
| `PrivilegeChange` | SET ROLE, SET SESSION AUTHORIZATION, EXECUTE AS, SETUSER | deny |
| `TxnControl` | BEGIN/COMMIT/ROLLBACK/SAVEPOINT/PREPARE TRANSACTION/XA/SET TRANSACTION | deny (agent owns transactions) |
| `SessionState` | SET (GUCs, sql_mode, NAMES), USE, RESET, DISCARD, PREPARE, DECLARE CURSOR WITH HOLD, LISTEN, temp objects | deny in v1; allowlist of harmless SETs later (`SET statement_timeout` lower than ceiling, `SET TIME ZONE`) |
| `ProceduralCode` | DO, CALL, anonymous blocks, EXECUTE (prepared), user functions of unknown effect | deny unless function is allowlisted |
| `DynamicSql` | SQL text executed from strings (EXECUTE IMMEDIATE, sp_executesql, query_to_xml, PREPARE FROM) | deny |
| `ExternalFileRead` / `ExternalFileWrite` | `pg_read_file`, `COPY TO 'file'`, `INTO OUTFILE`, `LOAD_FILE`, `OPENROWSET(BULK)`, `lo_export`, `VACUUM INTO`, ClickHouse `file()` | deny |
| `ExternalNetwork` | dblink, FDWs, linked servers, `url()`, `s3()`, `remote()`, HTTP extensions, `UTL_HTTP` | deny |
| `ProcessExec` | `COPY TO/FROM PROGRAM`, `xp_cmdshell`, untrusted PL | deny |
| `AdminControl` | kill/cancel sessions, ALTER SYSTEM, CHECKPOINT, reload conf, replication control, SHUTDOWN, FLUSH | deny |
| `Notify` | NOTIFY/pg_notify (messages to other sessions) | deny |
| `ResourceHeavy` | flagged constructs (cross joins without predicates, `pg_sleep`, huge `generate_series`, recursive CTE without obvious bound) | allow with limits / confirm |
| `Unknown` | anything not understood | **deny** |

Coarse class for UI = the "highest" of: `Unknown` > `AdminControl/ProcessExec/External*` > `Dcl/PrivilegeChange` > `Ddl` > `WriteData/SequenceAdvance/WriteTemp` > `Lock*` > `TxnControl/SessionState/ProceduralCode/DynamicSql/Notify` > `ReadData/ReadMetadata/InfoDisclosure`.

### 3.3 PostgreSQL statement catalogue (libpg_query node → base effects)

| Node(s) | Base effects | Deep checks |
|---|---|---|
| `SelectStmt` | ReadData | `intoClause` ⇒ Ddl+WriteData (SELECT INTO creates a table); `lockingClause` ⇒ LockRows; CTEs with Insert/Update/Delete/Merge ⇒ WriteData; all `FuncCall`/operators ⇒ ALG-SQL-SAFE-03; `RangeFunction` (functions in FROM) ⇒ SAFE-03; sublinks recursively |
| `InsertStmt`, `UpdateStmt`, `DeleteStmt`, `MergeStmt` | WriteData | ON CONFLICT, RETURNING functions; target relation recorded as write |
| `TruncateStmt` | WriteData + Ddl-like (`RESTART IDENTITY`, `CASCADE`) | — |
| `CopyStmt` | `is_from` ⇒ WriteData; `filename` set ⇒ ExternalFileRead/Write; `is_program` ⇒ ProcessExec; `TO STDOUT` ⇒ ReadData; `query` ⇒ inner effects | `COPY (DELETE … RETURNING *) TO STDOUT` ⇒ WriteData |
| `ExplainStmt` | ReadMetadata | options contain `analyze` (any truthy form: `ANALYZE`, `ANALYZE true`, `ANALYSE`, `(ANALYZE 1)`) ⇒ effects of inner statement; without ANALYZE: inner function calls still assessed (planner may evaluate IMMUTABLE/STABLE functions during constant folding/estimation) |
| `CreateTableAsStmt` (incl. `CREATE MATERIALIZED VIEW`, `SELECT INTO` deparsed form) | Ddl + WriteData | inner query |
| `ViewStmt`, `RuleStmt`, `CreateTrigStmt`, `CreateFunctionStmt`, all `Create*`/`Alter*`/`Drop*`/`Rename*`, `CommentStmt`, `SecLabelStmt`, `RefreshMatViewStmt`, `ReindexStmt`, `ClusterStmt`, `VacuumStmt` (VACUUM and ANALYZE), `CreateExtensionStmt`, `ImportForeignSchemaStmt`, `CreateSubscriptionStmt`/`CreatePublicationStmt`, `AlterDatabaseSetStmt`, `AlterRoleSetStmt` | Ddl | — |
| `GrantStmt`, `GrantRoleStmt`, `CreateRoleStmt`, `AlterRoleStmt`, `DropRoleStmt`, `ReassignOwnedStmt`, `DropOwnedStmt`, `AlterDefaultPrivilegesStmt`, `AlterOwnerStmt` | Dcl | — |
| `TransactionStmt` (BEGIN/START/COMMIT/ROLLBACK/SAVEPOINT/RELEASE/ROLLBACK TO/PREPARE TRANSACTION/COMMIT PREPARED/ROLLBACK PREPARED) | TxnControl | `COMMIT/ROLLBACK PREPARED` affect *other* transactions ⇒ also WriteData/AdminControl |
| `VariableSetStmt` (SET/SET LOCAL/RESET/SET TRANSACTION/SET SESSION CHARACTERISTICS) | SessionState | names: `role`, `session_authorization` ⇒ PrivilegeChange; `transaction_read_only`, `default_transaction_read_only`, `transaction_isolation`, `session_replication_role`, `row_security`, `search_path`, `standard_conforming_strings`, `backslash_quote`, `client_encoding`, `statement_timeout`, `lock_timeout` … ⇒ flagged; `RESET ALL` ⇒ SessionState (undoes agent's pins) |
| `VariableShowStmt` | ReadMetadata | `SHOW ALL` ⇒ InfoDisclosure |
| `DiscardStmt` | SessionState | — |
| `DoStmt` | ProceduralCode | language `plpgsql`/untrusted ⇒ also DynamicSql potential |
| `CallStmt` | ProceduralCode | procedures may COMMIT (only outside explicit txn blocks) |
| `PrepareStmt` | SessionState + inner effects | — |
| `ExecuteStmt` | ProceduralCode + Unknown | target prepared statement opaque |
| `DeallocateStmt` | SessionState | — |
| `DeclareCursorStmt` | ReadData (inner) + SessionState if `WITH HOLD` | inner query effects |
| `FetchStmt` (FETCH/MOVE), `ClosePortalStmt` | ReadData / SessionState | — |
| `ListenStmt`, `UnlistenStmt` | SessionState | — |
| `NotifyStmt` | Notify | — |
| `LockStmt` | LockTables | any mode (even ACCESS SHARE held until txn end) |
| `LoadStmt` (`LOAD 'lib'`) | AdminControl + ProcessExec | — |
| `CheckPointStmt`, `AlterSystemStmt` | AdminControl | — |
| `ConstraintsSetStmt` (`SET CONSTRAINTS`) | TxnControl | — |
| `CreatedbStmt`/`DropdbStmt`/`AlterDatabaseStmt` | Ddl (and cannot run in txn block) | — |
| anything else / new node kinds | Unknown | — |

`pg_query.rs` already exposes a coarse view (`ParseResult::tables()` with `Context::{Select, DML, DDL, Call}`, `functions()`, `statement_types()`), which is a useful cross-check but not sufficient (no locking clauses, no COPY direction, no EXPLAIN ANALYZE semantics). [Observed: `pg_query.rs/src/parse_result.rs:59-270`]

### 3.4 PostgreSQL function / expression side-effect catalogue (deny unless explicitly allowed)

| Category | Functions (pg_catalog unless noted) | Blocked by READ ONLY txn? |
|---|---|---|
| Session/backend control | `pg_terminate_backend`, `pg_cancel_backend`, `pg_reload_conf`, `pg_rotate_logfile`, `pg_log_backend_memory_contexts`, `pg_promote`, `pg_wal_replay_pause/resume`, `pg_switch_wal`, `pg_create_restore_point`, `pg_backup_start/stop`, `pg_stat_reset*`, `pg_stat_statements_reset` (ext) | No (most require privileges) [Observed: docker test, postgres:16/17/18 for `pg_terminate_backend`, `pg_stat_reset`, `pg_switch_wal`, `CHECKPOINT`; see verification/01-db-behavior-tests.md#pg-ro-escapes] |
| Configuration | `set_config(name, value, is_local)` | No (but read-only flag cannot be flipped after snapshot) |
| Locks | `pg_advisory_lock*`, `pg_advisory_xact_lock*`, `pg_try_advisory_*` (session-level locks survive ROLLBACK!) | No |
| Sequences | `nextval`, `setval`, (`currval`/`lastval` read) | Yes for non-temp sequences |
| Large objects | `lo_import`, `lo_export` (server file write), `lo_create`, `lo_unlink`, `lo_put`, `lo_from_bytea`, `lo_truncate`, `lo_open(…, INV_WRITE)` | Writes yes; `lo_export` file write **not blocked** (corrected after verification: it writes the server file inside a RO txn) — deny [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-ro-escapes] |
| Server files | `pg_read_file`, `pg_read_binary_file`, `pg_ls_dir`, `pg_ls_*dir`, `pg_stat_file` | No (privilege-gated: `pg_read_server_files`) |
| Notifications | `pg_notify` | No |
| Replication | `pg_create_physical/logical_replication_slot`, `pg_drop_replication_slot`, `pg_copy_*_slot`, `pg_logical_slot_get_changes`/`_binary_changes` (consumes!), `pg_replication_slot_advance`, `pg_logical_emit_message`, `pg_replication_origin_*` | Varies — deny. Observed: `pg_create_physical_replication_slot` and `pg_logical_emit_message` are **not** blocked and the slot survives ROLLBACK; logical-slot functions untested [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-ro-escapes] |
| Dynamic SQL executors | `query_to_xml`, `query_to_xmlschema`, `query_to_xml_and_xmlschema`, `cursor_to_xml*`, `ts_stat(text)`, `ts_rewrite(tsquery, text)`, `database_to_xml*`, `schema_to_xml*` (heavy) | Inner writes yes (SPI under read-only); still deny (DynamicSql) |
| Cross-connection | `dblink`, `dblink_exec`, `dblink_send_query`, `dblink_open`, `dblink_connect*` (ext `dblink`); `postgres_fdw` objects | **No — separate session** for dblink (writes commit independently, observed). postgres_fdw: remote txn is not read-only but follows the local COMMIT/ROLLBACK (corrected after verification) [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-ro-escapes] |
| Network/OS (extensions) | `http_*` (pgsql-http), `net.http_*` (pg_net; queue table writes blocked), untrusted PL (`plpython3u`, `plperlu`, `plsh`) functions | Network/OS effects **not** blocked |
| Time/DoS | `pg_sleep`, `pg_sleep_for`, `pg_sleep_until`; huge `generate_series`, `repeat`, `string_agg` over cross joins | No |
| Info disclosure | `pg_stat_activity`/`pg_stat_get_activity` (others' query text), `pg_settings`/`current_setting` of sensitive GUCs, `pg_hba_file_rules`, `pg_file_settings`, `inet_server_addr`, `version()` | No |
| XID/admin probes | `txid_current()`/`pg_current_xact_id()` (assigns XID) | **No** (corrected after verification: both succeed inside a RO txn and assign an XID) [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-canary] |
| Implicit function calls | user-defined **operators** (`pg_operator.oprcode`), **casts** (`pg_cast.castfunc`), domain CHECKs, type I/O functions, column defaults (on write only), views whose definitions call volatile functions, RLS policies | Writes inside yes; other effects no |

Rules: (a) only an allowlist of builtin functions is "known read-only" (math, string, date/time, JSON/array accessors, aggregates, window functions, catalog info functions `pg_get_*`, `format_type`, `has_*_privilege`, `current_user`, `now()`); (b) user-defined functions are assessed via catalog metadata (ALG-SQL-SAFE-03); (c) any non-builtin operator or cast in the statement ⇒ resolve to its function or mark Unknown.

### 3.5 Bypass catalogue and defences (exhaustive, by dialect)

#### PostgreSQL

| # | Bypass | Example | Defence |
|---|---|---|---|
| P1 | Write hidden in CTE | `WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d` | Deep AST walk; RO txn |
| P2 | `SELECT INTO` creates table | `SELECT * INTO x FROM t` | `intoClause` check; RO txn |
| P3 | `EXPLAIN ANALYZE` executes | `EXPLAIN (ANALYZE) DELETE FROM t` | Option parsing; RO txn |
| P4 | Multi-statement escape | `COMMIT; DELETE FROM t` | Extended protocol (single statement), splitter requires 1 statement |
| P5 | Flip transaction to RW | `SET TRANSACTION READ WRITE`, `SET transaction_read_only=off`, `SELECT set_config('transaction_read_only','off',true)` | Snapshot pinned before user statement; classifier denies these SETs/functions |
| P6 | Session defaults | `SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE`, `SET default_transaction_read_only=off`, `RESET ALL` | Deny SessionState; ROLLBACK reverts transactional SETs; DISCARD ALL / connection reset after each request |
| P7 | End the transaction | `COMMIT`, `ROLLBACK`, `COMMIT AND CHAIN` | Deny TxnControl; if it ever slips through, single-statement protocol means nothing else runs; agent detects "no transaction in progress" on its ROLLBACK and discards the connection |
| P8 | Procedure with internal COMMIT | `CALL p()` | Deny ProceduralCode; inside explicit txn block, procedure COMMIT raises "invalid transaction termination" |
| P9 | Anonymous code | `DO $$ … $$` | Deny |
| P10 | Prepared statement indirection | `PREPARE s AS DELETE …; EXECUTE s` (2 requests) | Deny PREPARE/EXECUTE; DISCARD ALL between requests |
| P11 | Side-effect builtins | `SELECT pg_terminate_backend(pid) FROM pg_stat_activity` | Function denylist/allowlist; role without `pg_signal_backend` |
| P12 | Cross-connection writes | `SELECT dblink_exec('dbname=x', 'DROP TABLE t')` | Deny dblink; agent role has no EXECUTE on dblink; recommend not installing |
| P13 | Server process execution | `COPY t TO PROGRAM 'curl …'` | Deny COPY to program/file; role without `pg_execute_server_program`/`pg_write_server_files` |
| P14 | Server file read | `SELECT pg_read_file('/etc/passwd')`, `COPY t FROM '/file'` | Deny; role without `pg_read_server_files` |
| P15 | Locks / DoS | `LOCK TABLE t IN ACCESS EXCLUSIVE MODE` (allowed in RO txn); `SELECT pg_advisory_lock(1)` (survives rollback); `SELECT … FOR UPDATE` | Deny LockTables/AdvisoryLock/LockRows; `lock_timeout`; `pg_advisory_unlock_all()`/DISCARD ALL after request |
| P16 | Privilege switch | `SET ROLE admin`, `SET SESSION AUTHORIZATION` | Deny; agent role not member of powerful roles (`NOINHERIT` doesn't prevent SET ROLE — membership does) |
| P17 | search_path hijack | `SET search_path = evil, pg_catalog` then later statements | Deny SET; agent pins `search_path` per request |
| P18 | Shadowed functions | user creates `public.lower()`-like or operator in a schema earlier in search_path | Pin `search_path` with `pg_catalog` first; classifier resolves names exactly as server; non-builtin ⇒ SAFE-03 |
| P19 | Custom operator / cast calling volatile function | `SELECT 1 ### 2` | Resolve operators/casts via catalog; non-builtin ⇒ Unknown |
| P20 | View or RLS policy calling volatile functions | `SELECT * FROM v` | RO txn contains writes; non-write effects (dblink in a view) only stoppable by privileges/extension absence |
| P21 | IMMUTABLE-mislabelled function evaluated at plan time | `EXPLAIN SELECT evil_immutable()` | Deny unknown functions even under EXPLAIN [Observed: docker test, postgres:16/17/18 — plain `EXPLAIN` of an IMMUTABLE plpgsql function calling `dblink_exec` committed a row inside a RO txn; see verification/01-db-behavior-tests.md#pg-ro-escapes] |
| P22 | Two-phase commit manipulation | `COMMIT PREPARED 'gid'` of another session | Deny TxnControl; role lacks ownership |
| P23 | Lexical-mode parser differential | `SET standard_conforming_strings = off` then `'\';DROP …'` | Deny; pin `standard_conforming_strings=on`; libpg_query parses with the same GUC value the server uses |
| P24 | Unicode/encoding tricks | `client_encoding` switch to SJIS/GBK | Pin `client_encoding=UTF8`; deny `SET client_encoding`/`SET NAMES` |
| P25 | Notifications | `NOTIFY ch, 'payload'` / `pg_notify` | Deny Notify |
| P26 | Temporary objects persisting in pooled sessions | `CREATE TEMP TABLE` / `DECLARE … WITH HOLD` / `LISTEN` | Deny; DISCARD ALL after request. (corrected after verification) `CREATE TEMP TABLE/SEQUENCE` is rejected inside a RO txn, but `DECLARE … WITH HOLD` and `LISTEN` are accepted, and temp objects created outside the RO txn stay writable; `DISCARD ALL` resets locks, prepared statements, LISTEN, temp tables, role and GUCs but cannot run inside a transaction block [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-ro-temp and #pg-discard] |
| P27 | Long-running reads holding back vacuum / bloat | huge read in REPEATABLE READ | `statement_timeout`, `idle_in_transaction_session_timeout`, row caps |
| P28 | Replication slot consumption | `SELECT pg_logical_slot_get_changes('slot', …)` | Deny; role lacks REPLICATION |
| P29 | psql meta-commands smuggled | `\! rm -rf`, `\gexec` | Client-command pre-pass rejects (agent never runs psql) |
| P30 | Syntax newer than agent's parser | PG 19 statement | libpg_query parse error ⇒ deny with "upgrade agent" reason |

#### MySQL / MariaDB

| # | Bypass | Example | Defence |
|---|---|---|---|
| M1 | Executable comments | `SELECT 1 /*!50000 , (SELECT GET_LOCK('x',10)) */`, `/*M! … */` | Lexer surfaces executable comments as code; classifier expands **all** of them regardless of version gate; deny if expansion doesn't parse. MySQL 8.0/8.4 run `/*!50000 …*/` and `/*!80000 …*/` bodies (even a whole `DROP TABLE` statement), skip `/*!99999 …*/`, ignore `/*M! …*/` [Observed: docker test, mysql:8.0/8.4, see verification/01-db-behavior-tests.md#mysql-comments] |
| M2 | Multi-statement | `SELECT 1; DROP TABLE t` | Never set `CLIENT_MULTI_STATEMENTS`; single-statement requirement |
| M3 | `sql_mode`/charset differential | `SET sql_mode='NO_BACKSLASH_ESCAPES'`, `SET NAMES gbk` (0xbf27 backslash-eating) | Pin `sql_mode` and `utf8mb4`; deny SET of these; classify with server's modes |
| M4 | File write/read | `SELECT … INTO OUTFILE/DUMPFILE`, `LOAD DATA [LOCAL] INFILE`, `LOAD_FILE()` | Deny; no `FILE` privilege; client disables LOCAL INFILE |
| M5 | Locks / DoS | `SELECT … FOR UPDATE`/`LOCK IN SHARE MODE`, `LOCK TABLES`, `GET_LOCK()`, `SLEEP()`, `BENCHMARK()`, `DO SLEEP(9999)` | Deny; `max_execution_time` (SELECT only), `lock_wait_timeout`, `innodb_lock_wait_timeout` |
| M6 | Dynamic SQL | `PREPARE s FROM @q; EXECUTE s` | Deny PREPARE/EXECUTE/DEALLOCATE |
| M7 | Definer-rights stored functions | `SELECT definer_fn()` writing with owner privileges | Deny unknown functions; `START TRANSACTION READ ONLY` blocks table changes inside |
| M8 | Implicit commit by DDL | DDL inside a read-only transaction | Deny DDL; privileges. (corrected after verification) `CREATE`/`DROP`/`TRUNCATE` inside `START TRANSACTION READ ONLY` **execute**: the implicit commit ends the RO txn first and later DML in the session commits; `LOCK TABLES` likewise ends it. Only session-level `SET SESSION TRANSACTION READ ONLY` makes DDL fail (ERROR 1792) — keep both, plus privileges [Observed: docker test, mysql:8.0 (8.0.46) and mysql:8.4 (8.4.13), see verification/01-db-behavior-tests.md#mysql-ro] |
| M9 | Session read-only reset | `SET SESSION TRANSACTION READ WRITE`, `SET @@transaction_read_only=0` | Deny SET; per-request `START TRANSACTION READ ONLY`; `COM_RESET_CONNECTION` after request |
| M10 | Admin | `KILL`, `SHUTDOWN`, `RESTART`, `FLUSH`, `RESET`, `PURGE BINARY LOGS`, `INSTALL PLUGIN/COMPONENT`, `CLONE`, `CHANGE REPLICATION SOURCE`, `SET GLOBAL/PERSIST` | Deny; no SUPER/SYSTEM_VARIABLES_ADMIN/etc. |
| M11 | UDFs | `CREATE FUNCTION f RETURNS STRING SONAME 'lib_mysqludf_sys.so'`; `sys_exec()` | Deny; no INSERT on `mysql.func`, no CREATE ROUTINE |
| M12 | `HANDLER` statements | `HANDLER t OPEN` (session state) | Deny |
| M13 | XA transactions | `XA START`/`XA PREPARE` leaving dangling txns | Deny |
| M14 | User variables as side channel/state | `SET @x := …` across requests | Deny SET; reset connection |
| M15 | `--` comment differential | `SELECT 1--1` vs `-- ` | Lexer implements MySQL rule exactly |
| M16 | Optimizer hints changing resource use | `/*+ MAX_EXECUTION_TIME(0) */`, `SET_VAR(...)` hint | Parse hints; deny `SET_VAR` and execution-time hints exceeding ceiling. Observed: `MAX_EXECUTION_TIME(500)` interrupts `SLEEP(2)`; `SET_VAR(sql_mode=…)` does not change lexing of the same statement; `SET_VAR(transaction_read_only=0)` does not lift a RO txn [Observed: docker test, mysql:8.0/8.4, see verification/01-db-behavior-tests.md#mysql-comments] |

#### SQL Server

| # | Bypass | Example | Defence |
|---|---|---|---|
| S1 | No session read-only mode | any DML in batch | Permissions (db_datareader + DENY), readable secondary (`ApplicationIntent=ReadOnly` on AG), database snapshot |
| S2 | Batch with many statements, no terminators | `SELECT 1 DELETE t` | Classify whole batch; every statement incl. inside IF/WHILE/TRY |
| S3 | Transaction escape | `COMMIT` inside batch; `@@TRANCOUNT` games | Deny TxnControl; wrapper checks `@@TRANCOUNT` |
| S4 | Dynamic SQL | `EXEC('DROP …')`, `sp_executesql`, `EXEC (@s)` | Deny DynamicSql |
| S5 | OS / external | `xp_cmdshell`, `sp_OACreate`, `sp_send_dbmail`, CLR assemblies, `OPENROWSET(BULK …)`, `OPENROWSET('SQLNCLI', …, 'EXEC …')`, `OPENDATASOURCE`, linked servers `EXEC … AT srv`, `OPENQUERY` with DML, `BULK INSERT`, `BACKUP … TO DISK` | Deny; permissions; `Ad Hoc Distributed Queries` disabled |
| S6 | Impersonation | `EXECUTE AS LOGIN='sa'`, `SETUSER`, `REVERT` | Deny PrivilegeChange; no IMPERSONATE permission |
| S7 | Non-transactional side effects even if rolled back | `NEXT VALUE FOR seq`, identity increments, `WAITFOR`, `sp_set_session_context` (affects RLS predicates), `SET CONTEXT_INFO` | Deny |
| S8 | Admin | `KILL`, `SHUTDOWN`, `RECONFIGURE`, `sp_configure`, `DBCC …`, `CHECKPOINT`, `ALTER SERVER CONFIGURATION` | Deny |
| S9 | Temp objects | `SELECT … INTO #t`, `CREATE TABLE #t` (allowed for everyone in tempdb) | Deny in v1 (WriteTemp) |
| S10 | Lexical mode | `SET QUOTED_IDENTIFIER OFF` | Pin; deny |
| S11 | `GO` + count | `GO 100000` | Client command rejected by agent |
| S12 | Locking hints | `SELECT … WITH (TABLOCKX, HOLDLOCK)`, `UPDLOCK` | Parse table hints; deny lock-escalating hints |

#### SQLite

| # | Bypass | Example | Defence |
|---|---|---|---|
| L1 | Turn off query_only | `PRAGMA query_only = 0` | Open with `SQLITE_OPEN_READONLY`; authorizer denies `SQLITE_PRAGMA` except allowlist [Observed: docker test, SQLite 3.54.1 — `query_only=0` re-enables writes on a RW-opened file; on `mode=ro` writes still fail; see verification/01-db-behavior-tests.md#sqlite-ro] |
| L2 | New files | `ATTACH 'new.db' AS x` (works and is writable even from a `mode=ro` main connection), `VACUUM INTO '/tmp/copy.db'` (works on `mode=ro` and `-readonly` connections; `PRAGMA query_only=1` blocks both but still leaves a 0-byte file) [Observed: docker test, keinos/sqlite3 (SQLite 3.54.1), see verification/01-db-behavior-tests.md#sqlite-ro] | Deny ATTACH/DETACH/VACUUM via classifier + authorizer (`SQLITE_ATTACH`) |
| L3 | Extensions / file functions | `load_extension()`, CLI `readfile()/writefile()/edit()` | Keep load_extension disabled; never register fileio functions |
| L4 | Schema corruption | `PRAGMA writable_schema=1` | `SQLITE_DBCONFIG_DEFENSIVE`; deny PRAGMA |
| L5 | App-defined functions with side effects | functions registered by host | Agent registers none |
| L6 | Statement read-only check | — | Call `sqlite3_stmt_readonly()` before stepping (note: returns true for BEGIN/COMMIT/ATTACH — not sufficient alone) |
| L7 | DoS | recursive CTE without bound, `randomblob(1e9)` | Progress handler timeout; `sqlite3_limit` (LENGTH, SQL_LENGTH, VARIABLE_NUMBER) |

#### ClickHouse

| # | Bypass | Example | Defence |
|---|---|---|---|
| C1 | Session settings | `SET readonly = 0`, `SETTINGS max_execution_time=0` | Server-side profile `readonly=1` (cannot be changed by session) + settings constraints |
| C2 | SSRF / external reads | `SELECT * FROM url('http://169.254.169.254/…', CSV)`, `s3()`, `remote()`, `mysql()`, `postgresql()`, `file()`, `jdbc()`, `odbc()`, `hdfs()`, dictionaries with external sources | Revoke source grants (`URL`, `S3`, `REMOTE`, `FILE`, …); classifier denies table functions not allowlisted |
| C3 | Writes through table functions | `INSERT INTO FUNCTION s3(…)` | readonly blocks INSERT; classifier denies |
| C4 | Admin | `SYSTEM …`, `KILL QUERY`, `OPTIMIZE`, `ALTER … DELETE` mutations | readonly + grants |
| C5 | DoS | huge scans | Profile limits: `max_execution_time`, `max_memory_usage`, `max_rows_to_read`, `max_result_rows`, quotas |

#### Oracle (basics)

| # | Bypass | Example | Defence |
|---|---|---|---|
| O1 | Autonomous transactions | `SELECT f() FROM dual` where `f` has `PRAGMA AUTONOMOUS_TRANSACTION` and commits DML | Deny unknown functions; no EXECUTE grants on such code |
| O2 | Definer-rights packages | `DBMS_*`, `UTL_FILE`, `UTL_HTTP`, `UTL_TCP`, `DBMS_SCHEDULER`, `DBMS_SQL`, `DBMS_JAVA` | Revoke from PUBLIC/agent; deny |
| O3 | Locks via SELECT privilege | `SELECT … FOR UPDATE`, `LOCK TABLE` | Grant `READ` (12c+) instead of `SELECT` (READ forbids locking) [Inferred] |
| O4 | Session changes | `ALTER SESSION SET CURRENT_SCHEMA=…`, `SET ROLE` | Deny |
| O5 | Dynamic SQL | `EXECUTE IMMEDIATE` in anonymous blocks | Deny anonymous PL/SQL |
| O6 | DB links | `SELECT * FROM t@link`, `DELETE FROM t@link` | Deny `@link` references unless allowlisted |
| O7 | RO txn ended by DDL/COMMIT | `SET TRANSACTION READ ONLY` is per-transaction | Deny DDL/TxnControl; privileges |

#### Cross-cutting

- **Parser differential:** any token the classifier lexes differently from the server is a potential bypass (comments, quotes, escapes, charsets, `--`, `#`, executable comments, Unicode homoglyphs in keywords are *not* keywords — fine; NUL bytes — reject; overlong statements truncated by drivers — reject before sending). Mitigation: libpg_query for PG; for others exhaustive lexer conformance tests against servers; deny on any lexer error.
- **Size/complexity limits** on input (bytes, tokens, nesting) to prevent classifier DoS; deny on limit hits.
- **Unicode normalisation is not applied by servers** — classifier must compare identifiers/keywords exactly as servers do (ASCII keyword matching).
- **Audit everything** including denials (fingerprint + normalised text + reasons), because classifiers will have gaps.

### 3.6 Server-side enforcement mechanisms (summary)

| Dialect | Transaction-level | Session-level | Principal-level | Engine checks |
|---|---|---|---|---|
| PostgreSQL | `BEGIN READ ONLY` + pin snapshot (`SELECT 1`) + ROLLBACK | `default_transaction_read_only=on` (role/db setting), `statement_timeout`, `lock_timeout`, `idle_in_transaction_session_timeout`, pinned `search_path`/`client_encoding`/`standard_conforming_strings`; DISCARD ALL after | Dedicated role: `GRANT SELECT` on allowed objects or `pg_read_all_data`; no membership in powerful roles; REVOKE EXECUTE on dangerous functions from PUBLIC where feasible; no `pg_read_server_files`/`pg_execute_server_program`/`pg_signal_backend` | `txid_current_if_assigned() IS NULL` after execution (canary) |
| MySQL/MariaDB | `START TRANSACTION READ ONLY` + ROLLBACK | `SET SESSION TRANSACTION READ ONLY`, `max_execution_time`, pinned `sql_mode`/charset; `COM_RESET_CONNECTION` | `SELECT`, `SHOW VIEW` only; no FILE/PROCESS/SUPER/EXECUTE/LOCK TABLES/CREATE TEMPORARY TABLES | replicas with `super_read_only=ON` |
| SQL Server | `BEGIN TRAN` + `@@TRANCOUNT` check + ROLLBACK (weak) | `SET LOCK_TIMEOUT`, `SET XACT_ABORT ON`; `sp_reset_connection` | `db_datareader` + `DENY INSERT, UPDATE, DELETE, EXECUTE, ALTER, CONTROL, IMPERSONATE`; no server roles | `ApplicationIntent=ReadOnly` to readable secondary; database snapshots |
| SQLite | — | `PRAGMA query_only=1` (set by agent) | file opened `SQLITE_OPEN_READONLY` (OS file permissions) | `sqlite3_stmt_readonly`, authorizer, `DEFENSIVE`, limits, progress handler |
| ClickHouse | — | profile `readonly=1` + constraints | users/roles with SELECT grants; revoke sources | quotas, limits |
| Oracle | `SET TRANSACTION READ ONLY` + ROLLBACK | `ALTER SESSION` pinned by agent | `READ` object privileges; no EXECUTE on risky packages; revoke from PUBLIC | resource manager profiles |

### Part B — Linting

### 3.7 Frameworks observed

- **pgls:** `pgls_analyse` (registry, categories, filters, metadata) + `pgls_analyser` (rules); rules run per statement with the libpg_query AST (+ catalog for typecheck rules); suppressions: `-- pgls-ignore <category/rule>` (next statement/line), `-- pgls-ignore-start … -- pgls-ignore-end`, `-- pgls-ignore-all` at file top (legacy `pgt-` prefix accepted). [Observed: `pgls_suppressions/src/lib.rs:250-370`, `parser.rs:60-95`; `ARCHITECTURE.md`]
- **sqruff:** rules visit the segment tree with crawlers; `is_fix_compatible` flags; fix loop runs up to `runaway_limit` iterations re-linting after each round; layout rules use the reflow engine (`depth_map`, `rebreak`, `reindent`, `respace`). [Observed: `crates/lib/src/core/linter/core.rs:684-740`; `crates/lib/src/utils/reflow/*`]

### 3.8 sqruff rule catalogue (summarised in our words; ✎ = auto-fix available)

| Group | Rules |
|---|---|
| Aliasing (AL) | AL01 table aliasing implicit/explicit ✎; AL02 column aliasing implicit/explicit ✎; AL03 expressions need an alias; AL04 table aliases unique per clause; AL05 unused table alias ✎; AL06 alias length bounds; AL07 forbid table aliases ✎; AL08 column aliases unique; AL09 self-aliased columns (`a AS a`) ✎; AL10 derived tables need an alias |
| Ambiguous (AM) | AM01 DISTINCT with GROUP BY; AM02 bare UNION (require DISTINCT/ALL) ✎; AM03 mixed implicit/explicit ORDER BY directions ✎; AM04 outer query column count unknown (`*`); AM05 qualify join type (INNER/LEFT…) ✎; AM06 inconsistent GROUP/ORDER BY reference styles (names vs positions); AM07 set operation branches column count mismatch; AM08 implicit cross join ✎; AM09 LIMIT/OFFSET without ORDER BY |
| Capitalisation (CP) | CP01 keywords ✎; CP02 unquoted identifiers ✎; CP03 function names ✎; CP04 boolean/null literals ✎; CP05 data types ✎ |
| Convention (CV) | CV01 `!=` vs `<>` ✎; CV02 COALESCE over IFNULL/NVL ✎; CV03 select trailing comma ✎; CV04 `COUNT(*)` vs `COUNT(1)` ✎; CV05 `IS NULL` instead of `= NULL` ✎; CV06 statement terminator ✎; CV07 no brackets around top-level statements ✎; CV08 LEFT JOIN over RIGHT JOIN; CV09 blocked words; CV10 preferred quote style for literals ✎; CV11 casting style (`::` vs `CAST`) ✎; CV12 JOIN … ON instead of WHERE joins ✎; CV13 final CTE select is `SELECT *` |
| Jinja (JJ) | JJ01 tag padding ✎ |
| Layout (LT) | LT01 spacing ✎; LT02 indentation ✎; LT03 operator line position ✎; LT04 leading/trailing commas ✎; LT05 long lines ✎; LT06 function name followed by `(` ✎; LT07 CTE closing bracket newline ✎; LT08 blank line after CTE ✎; LT09 select targets on new lines ✎; LT10 SELECT modifiers on the SELECT line ✎; LT11 set operators surrounded by newlines ✎; LT12 single trailing newline ✎; LT13 no leading blank lines ✎; LT14 clause keyword newlines ✎; LT15 max consecutive blank lines ✎ |
| References (RF) | RF01 references must exist in FROM; RF02 qualify when multiple tables; RF03 consistent qualification in single-table queries ✎; RF04 keywords as identifiers; RF05 special characters in identifiers; RF06 unnecessary quoting ✎; RF07 alias referenced in its own OVER |
| Structure (ST) | ST01 redundant `ELSE NULL` ✎; ST02 unnecessary CASE ✎; ST03 unused CTE; ST04 flatten nested CASE in ELSE ✎; ST05 subqueries in FROM/JOIN → CTEs ✎; ST06 select column order (wildcards, simple, then calculations) ✎; ST07 prefer explicit ON over USING ✎; ST08 `DISTINCT(` misuse ✎; ST09 join condition order ✎; ST10 constant expressions; ST11 unused joined table; ST12 consecutive semicolons ✎ |
| Dialect-specific | PG01 excessive locks in DDL; PG02 FKs as `NOT VALID` then validate; TQ02 multi-statement procedure bodies wrapped in BEGIN/END ✎; TQ03 empty batches ✎; TQ04 `AS` aliasing over `alias = expr` ✎; OR01 empty batches ✎ |

[Observed: `crates/lib/src/rules/<group>/<code>.rs` `name()`/`description()`/`is_fix_compatible()`]

### 3.9 pgls analyser catalogue (summarised)

- **correctness:** avoid `ALTER TYPE … ADD VALUE` pitfalls, ban `CREATE INDEX CONCURRENTLY` inside transactions, duplicate column assignments, non-transactional statements in transactions, uncommitted transactions at file end, identifier too long (> 63 bytes), invalid DROP TYPE signature, transaction nesting.
- **destructive:** adding required field, alter identity, DELETE/UPDATE without WHERE, drop column/constraint/database/default/domain/extension/function/generated expression/NOT NULL/schema/sequence/table/type/view, `SET SCHEMA`, TRUNCATE (and CASCADE), changing column type, renaming column/object/table.
- **safety (online migration locking, Squawk-style):** add serial column, field with volatile default, FK/PK constraint addition, NOT NULL field, exclusion constraint, attach partition, create/enable/disable triggers, wide lock windows, domain constraints, drop trigger, `VACUUM FULL`, constraint missing `NOT VALID`, unique constraint without index, `lock_timeout`/`statement_timeout`/`idle_in_transaction_session_timeout` requirements, multiple ALTER TABLE, robust statements (`IF [NOT] EXISTS`), CONCURRENTLY for index create/drop/reindex/detach/refresh matview, separate constraint validation, statements while holding ACCESS EXCLUSIVE.
- **style:** ban `char`, creating enums, prefer `bigint`, identity, `jsonb`, `text`, `timestamptz`, enum value ordering, require table schema.
- **typecheck:** ambiguous column, assignment/function/operator/return type mismatches, generated column assignment, INSERT column mismatch, invalid cast, invalid MERGE, missing FROM entry, unknown column/function/relation/schema/type.
[Observed: `crates/pgls_analyser/src/lint/{correctness,destructive,safety,style,typecheck}/*.rs`]

### 3.10 Proposed Hacivat initial rule set

| Tier | Rules | Analysis level |
|---|---|---|
| Execution guard (also used by confirm dialogs) | delete/update without WHERE; DROP/TRUNCATE; ALTER that rewrites/locks large tables (PG safety set); statements with Write effects in a connection marked "production" | syntax + safety classifier |
| Correctness | unknown relation/column/function (certain only); ambiguous column; `= NULL`; `NOT IN (subquery)` with nullable column; implicit cross join; set-operation column count; aggregate/non-aggregate mix without GROUP BY (dialect-aware: MySQL `ONLY_FULL_GROUP_BY`); LIMIT without ORDER BY; derived table without alias; unused CTE; alias used in WHERE (dialect-aware) | semantic + catalog |
| Migration safety (PG first) | port pgls safety ideas: CONCURRENTLY, NOT VALID, timeouts, volatile defaults | syntax |
| Style | handled by formatter; a few lint-only: RF04 keywords as identifiers, RF05 special chars, CV02/CV11 preferences | syntax |

### Part C — Formatting

### 3.11 Approaches compared

| Approach | Example | Pros | Cons |
|---|---|---|---|
| AST pretty-printer with layout events | pgls (`LayoutEvent::GroupStart/GroupEnd/IndentStart/IndentEnd`, line width, comment maps, AST re-parse verification, comment idempotency loop ≤ 6 passes) [Observed: `pgls_pretty_print/src/{lib.rs,renderer.rs,emitter.rs}`] | Canonical output; semantic verification | Needs complete AST; comments must be re-attached; PG only |
| Lint-fix reflow | sqruff LT rules + reflow engine [Observed: `utils/reflow/*`] | Works on partially parsed code; configurable rule by rule | Iterative fix loops; slower; less canonical |
| Wadler/Prettier document IR | prettier, biome_formatter, ruff_formatter [Inferred] | Proven line-fitting; groups/indent/conditional breaks; comments as line-suffix | Need per-node doc builders; alignment needs extension |

### 3.12 Whitespace-sensitive SQL constructs (formatter hazards)

1. Line comments must be followed by a newline (never join the next token onto a `--`/`#` line).
2. PG adjacent string literals require a newline between them (`'a'\n'b'`); joining onto one line is a syntax error.
3. MySQL `-- ` needs a following space; `1 - -1` must not become `1--1`.
4. Operator merging: `< =` → `<=`, `- -` → `--`, `/ *` → `/*`, `: :` → `::`, `= >` → `=>`.
5. Identifier merging: removing space between a word and `$1`/`$$`/`#` can change tokens (`a $1` → `a$1`).
6. Client-command lines (`GO`, `DELIMITER`, `\…`, `/`) must stay on their own lines.
7. Dollar-quoted bodies, string contents, executable comments' internal layout: preserve verbatim (optionally format embedded SQL bodies as separate documents).
8. Hints `/*+ … */` must stay directly after their keyword.
9. T-SQL: statements without terminators — the formatter must not insert/remove newlines that act as separators only in `GO` contexts; inserting `;` is a semantic-free improvement only where allowed.

## 4. Algorithm specifications

### ALG-SQL-SAFE-01: Statement safety classification
- **Problem:** Determine the effects, touched objects and decision-relevant reasons of a SQL request for a dialect, conservatively (fail closed), identically in Hacivat (advisory) and Karagöz (authoritative).
- **Inputs / outputs:** `(sql: &str, dialect: DialectId, server_version, lex_modes (pinned), catalog: Option<CatalogView> (functions, operators, casts, views), policy_profile)` → `SafetyReport { items: Vec<StmtReport>, decision: Allow | Deny{reasons} | Confirm{reasons} }`, `StmtReport { effects: EffectSet, class: CoarseClass, reads: Set<ObjRef>, writes: Set<ObjRef>, calls: Set<FnRef>, flags: RiskFlags, reasons: Vec<ReasonCode>, confidence: Exact | Conservative }`.
- **Algorithm:**
  1. **Pre-checks:** reject if `len > max_bytes`, contains NUL, invalid UTF-8, or the lexer reports any error token (unterminated literal/comment, unknown char) → `Deny(LexError)`.
  2. **Client-command pre-pass** (ALG-SQL-SPLIT-02): any `Command`/`Data` segment → `Deny(ClientCommand)`.
  3. **Split** (execution mode): require exactly one `Exact` statement (SQL Server: one batch); otherwise `Deny(MultiStatement|Ambiguous)`.
  4. **MySQL/MariaDB executable comments:** produce an *expanded* token stream where every `/*!…*/` and `/*M!…*/` body is inlined (regardless of version gate); also classify the non-expanded stream; take the union of effects (the server executes one of them depending on version; we don't trust our version knowledge).
  5. **Parse strictly:** PG → libpg_query `parse` (error ⇒ `Deny(ParseError)`); others → `sql-parser` Strict mode (doc 03 §3.9) and, if enabled, sqlparser-rs with the matching dialect; any failure ⇒ `Deny(ParseError)`; both succeed ⇒ classify both and union effects.
  6. **Top-level dispatch** by statement kind using the dialect catalogue (§3.3 for PG; analogous tables for others) → base effects + flags.
  7. **Deep walk** over every node (iteratively, with depth limit):
     - relation references: in read positions → `reads`; in write targets (INSERT/UPDATE/DELETE/MERGE targets, COPY FROM targets, TRUNCATE, SELECT INTO target, CREATE … AS) → `writes`; DB-link / 4-part / foreign references → `ExternalNetwork` unless allowlisted.
     - CTEs: data-modifying ⇒ `WriteData`.
     - locking clauses / table hints ⇒ `LockRows`/`LockTables`.
     - function calls (`FuncCall`, table functions in FROM, MySQL/T-SQL builtins, ClickHouse table functions) → ALG-SQL-SAFE-03 → merge effects.
     - operators and casts: builtin (known `pg_catalog` operator/cast) → none; otherwise resolve via catalog (oprcode/castfunc) → SAFE-03; unresolvable → `Unknown`.
     - string literals passed to known SQL-executing functions/procedures → `DynamicSql`.
     - `EXPLAIN`: if any ANALYZE option is truthy (accept all spellings and boolean forms: `ANALYZE`, `ANALYSE`, `ANALYZE true|on|1|yes`) → effects of inner statement; else `ReadMetadata` + function assessment of the inner statement (plan-time evaluation); under read-only profiles an EXPLAIN whose inner statement has any non-read effect is still denied (conservative: option spellings and future EXPLAIN options may execute).
     - `COPY`: per §3.3.
     - `SET`/`RESET`/`SET TRANSACTION`/`SET ROLE` etc. → SessionState + specific flags per variable name (case-insensitive, also `SET LOCAL`, `set_config()` calls with literal names; non-literal names ⇒ `Unknown`).
     - procedural statements (DO, CALL, EXECUTE, anonymous blocks, EXEC proc) → `ProceduralCode` (+ `Unknown` for opaque targets).
     - views referenced: if catalog available and policy requires, mark `reads` on the view; optional expansion of view definitions for function assessment (`Conservative` otherwise).
  8. **Combine** effects across nodes; derive `CoarseClass`; collect reasons (e.g. `pg.select.into_clause`, `pg.explain.analyze`, `fn.denylisted(pg_terminate_backend)`, `cte.data_modifying`, `unknown.function(public.f)`).
  9. **Policy evaluation:** for each effect, policy says Allow/Deny/Confirm; any Deny ⇒ Deny; else any Confirm ⇒ Confirm (Hacivat UI; Karagöz may route to approval workflow per access/ docs); `Unknown` ⇒ Deny under every built-in read-only profile.
  10. **Object policy:** check `reads`/`writes`/`calls` against granted objects (patterns per schema/table); unknown objects (unresolved) ⇒ treat as not granted.
- **Data structures:** `EffectSet` bitflags (u32); `ReasonCode` enum with args; per-dialect static tables: statement-kind → effects, function denylist/allowlist (versioned), variable-name flags; `ObjRef{db, schema, name, kind}`.
- **Complexity:** O(statement size) + catalog lookups; libpg_query parse µs–ms.
- **Edge cases (exhaustive list):** empty input (Deny: no statement); comment-only input; statement with trailing `;` plus trailing comment; `SELECT` with `INTO` in the middle of UNION branches (PG: only allowed in the first); `INSERT … SELECT` from a CTE with writes; `WITH RECURSIVE` + data-modifying CTE; `EXPLAIN (FORMAT JSON, ANALYZE)` vs `EXPLAIN (ANALYZE false)` (not executing — but treat `false` as non-executing only if literally false/off/0); `EXPLAIN EXECUTE s`; `EXPLAIN CREATE TABLE AS …` (no ANALYZE → not executed, but denied under read-only by the conservative EXPLAIN rule); `COPY (SELECT …) TO STDOUT` (read); `COPY t TO STDOUT` (read); `COPY t FROM STDIN` (write); `\copy` (client) → rejected in pre-pass; functions called with schema qualification `pg_catalog.pg_sleep`; quoted function names `"pg_sleep"`; case variants `PG_SLEEP` (fold per dialect); overloaded denylisted names in user schemas (`public.pg_sleep` — deny by *name* conservatively); functions inside default expressions of a VALUES/INSERT (write already denied); `SELECT * FROM t TABLESAMPLE SYSTEM(…)` (read); `SELECT … FOR UPDATE SKIP LOCKED` (LockRows); `SELECT … FOR KEY SHARE` (LockRows); `LOCK TABLE t IN ACCESS SHARE MODE` (LockTables); `SET LOCAL statement_timeout = 0` (SessionState: raising timeout ceiling → deny); `SHOW` statements (ReadMetadata; `SHOW ALL` InfoDisclosure); `VALUES (1)` (read); `TABLE t` (read); `PREPARE` of a SELECT (SessionState); `DECLARE c CURSOR FOR …` without HOLD (read + SessionState within txn); `FETCH ALL` (read); `CHECKPOINT`; MySQL `SELECT … INTO @v` (SessionState), `INTO OUTFILE` (ExternalFileWrite); MySQL `REPLACE`, `INSERT IGNORE`, `INSERT … ON DUPLICATE KEY UPDATE`; MySQL `CALL`; MySQL `DO expr`; MySQL `SHOW PROCESSLIST` (InfoDisclosure); T-SQL batch with variables and `IF` blocks; T-SQL `SELECT … INTO #t`; T-SQL `WAITFOR DELAY`; T-SQL `EXEC sp_who` (ProceduralCode); SQLite `PRAGMA table_info(t)` (ReadMetadata; allowlisted pragma), `PRAGMA journal_mode=WAL` (Ddl-ish/SessionState: deny); ClickHouse `SELECT … SETTINGS max_threads=1` (SessionState: allow only allowlisted settings below ceilings), `SELECT * FROM numbers(1e12)` (ResourceHeavy), `url()` (ExternalNetwork); Oracle `SELECT … FROM dual@link`; Unicode identifiers; parser limit hit (Deny).
- **Test cases (PG unless noted; → decision under `read_only` profile):**
  1. `SELECT * FROM t` → Allow (ReadData, reads {t}).
  2. `WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d` → Deny (WriteData via CTE).
  3. `SELECT * INTO x FROM t` → Deny (Ddl+WriteData).
  4. `EXPLAIN SELECT 1` → Allow; `EXPLAIN ANALYZE DELETE FROM t` → Deny; `EXPLAIN (ANALYZE off) DELETE FROM t` → Deny (plan-only would not execute, but the conservative rule denies EXPLAIN whose inner statement is not a read).
  5. `SELECT pg_terminate_backend(123)` → Deny (AdminControl).
  6. `SELECT nextval('s')` → Deny (SequenceAdvance).
  7. `SELECT * FROM t FOR UPDATE` → Deny (LockRows).
  8. `SET TRANSACTION READ WRITE` → Deny; `SELECT set_config('transaction_read_only','off',true)` → Deny.
  9. `COMMIT` → Deny; `SELECT 1; DELETE FROM t` → Deny (MultiStatement).
  10. `COPY t TO STDOUT` → Allow (if `COPY` allowed by policy) ; `COPY t TO PROGRAM 'x'` → Deny (ProcessExec).
  11. `SELECT dblink_exec('…','DROP TABLE t')` → Deny (ExternalNetwork+DynamicSql).
  12. `SELECT query_to_xml('select 1', true, true, '')` → Deny (DynamicSql).
  13. `DO $$BEGIN END$$` → Deny; `CALL p()` → Deny.
  14. `SELECT 1 ### 2` with user-defined operator → Deny (Unknown) without catalog; with catalog: per oprcode function assessment.
  15. `SELECT my_stable_sql_fn(x) FROM t` (catalog: STABLE, LANGUAGE sql, not SECURITY DEFINER) → Allow with `Conservative` flag.
  16. MySQL `SELECT 1 /*!99999 , SLEEP(100) */` → Deny (SLEEP in expanded variant).
  17. MySQL `SELECT * FROM t INTO OUTFILE '/tmp/x'` → Deny.
  18. T-SQL `SELECT 1 EXEC('DROP TABLE t')` (one batch) → Deny.
  19. SQLite `PRAGMA query_only=0` → Deny; `ATTACH 'x.db' AS x` → Deny; `VACUUM INTO 'y.db'` → Deny.
  20. ClickHouse `SELECT * FROM url('http://…', CSV)` → Deny.
  21. `SELEC 1` → Deny (ParseError).
- **Prior art and evidence:** pg_query table/function contexts and statement types [Observed: `pg_query.rs/src/parse_result.rs:59-270`]; pgls destructive rule set (static detection of destructive statements) [Observed: `pgls_analyser/src/lint/destructive/*`]; pgcli/mycli prefix heuristics as negative example [Observed: `pgcli/packages/parseutils/__init__.py:17-53`]; PG read-only transaction semantics [Inferred: PG docs].
- **Used by:** Hacivat (advisory, MCP policy) | Karagöz (authoritative)

### ALG-SQL-SAFE-02: Karagöz execution pipeline (defence in depth)
- **Problem:** Execute an authorised request such that, even if ALG-SQL-SAFE-01 misses something, server-side mechanisms prevent persistent or external effects for read-only access, and every request is audited.
- **Inputs / outputs:** `(request{sql, params, connection_id, access_level, request_id, user, client_meta}, policy)` → result stream (capped) or denial; audit events (`requested`, `decided`, `executed|denied|failed`, with fingerprint, normalised SQL, effects, reasons, timings, row counts).
- **Algorithm:**
  1. Authenticate the caller (capability from Perde, verified locally) and authorise `access_level` for `connection_id`.
  2. Apply input limits (bytes, tokens, params count/size).
  3. Run ALG-SQL-SAFE-01 with the **agent-pinned** lexical modes and server version; on Deny → audit + return reasons (message IDs).
  4. Check out a connection from a pool dedicated to `(connection_id, access_level)`, authenticated as the least-privilege principal for that level (never the admin/discovery credential).
  5. Dialect-specific guarded execution:
     - **PG:** session initialised once with `SET search_path = pg_catalog, <allowed schemas>`, `client_encoding=UTF8`, `standard_conforming_strings=on`, `statement_timeout`, `lock_timeout`, `idle_in_transaction_session_timeout`, `default_transaction_read_only=on`; per request: `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` → `SELECT 1` (snapshot pinned; read-only now irreversible) → user statement via extended protocol (`Parse` unnamed / `Bind` params / `Describe` / `Execute` with row limit and portal suspension for streaming) → `SELECT txid_current_if_assigned()` (expect NULL; else alarm + mark connection tainted) → `ROLLBACK` (if server replies "no transaction in progress" → the user statement ended the txn → audit alarm, discard connection) → `DISCARD ALL` or connection discard.
     - **MySQL/MariaDB:** no `CLIENT_MULTI_STATEMENTS`, `LOCAL INFILE` disabled; session `SET SESSION sql_mode=<pinned>`, `NAMES utf8mb4`, `max_execution_time`, `lock_wait_timeout`, `innodb_lock_wait_timeout`, `SESSION TRANSACTION READ ONLY`; per request `START TRANSACTION READ ONLY` → statement via `COM_STMT_PREPARE/EXECUTE` (or `COM_QUERY` if not preparable) → `ROLLBACK` → `COM_RESET_CONNECTION`.
     - **SQL Server:** principal with read-only permissions; `ApplicationIntent=ReadOnly` when an AG secondary exists; `SET XACT_ABORT ON; SET LOCK_TIMEOUT n; SET QUOTED_IDENTIFIER ON; BEGIN TRAN` → batch → `IF @@TRANCOUNT > 0 ROLLBACK` (if 0 → user ended txn → alarm) → `sp_reset_connection` (driver pool reset).
     - **SQLite:** connection opened `SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX`, `DEFENSIVE`, authorizer installed, `PRAGMA query_only=1`, limits; `sqlite3_prepare_v2` must consume the full text (tail empty or trivia) and `sqlite3_stmt_readonly` must be true; progress-handler timeout.
     - **ClickHouse:** user with readonly profile and source grants revoked; per-query `max_execution_time`, `max_result_rows`, `result_overflow_mode=break` sent as query settings only if lower than profile ceilings.
     - **Oracle:** principal with `READ` grants; `SET TRANSACTION READ ONLY` → statement → `ROLLBACK`.
  6. Stream results with row/byte caps and cancellation (server-side cancel on cap or client disconnect).
  7. Audit outcome; on any anomaly (canary failure, txn ended by user, unexpected command tag such as `INSERT 0 1`), raise a security event to Perde and quarantine the pool.
- **Data structures:** per-(connection, level) pools with init scripts; `ExecutionGuard` trait per dialect; audit event schema (access/ docs).
- **Complexity:** two extra round trips per request for PG (`BEGIN … SELECT 1`, canary+ROLLBACK can be pipelined).
- **Edge cases:** server is a hot standby (PG: already read-only; `SELECT 1` and canary still fine; `txid_current_if_assigned` OK); PgBouncer in transaction-pooling mode between Karagöz and PG (session SETs not preserved → set per transaction with `SET LOCAL`; `DISCARD ALL` disallowed in some configs — prefer direct connections); statements that cannot run inside a transaction block (`VACUUM`, `CREATE DATABASE`) fail — fine (denied anyway); long-running cursor streaming exceeding `idle_in_transaction_session_timeout` while client is slow (buffer or cap); `statement_timeout` vs result streaming; replicas lagging (inform user); MySQL `max_execution_time` applies only to SELECT; SQL Server snapshot isolation not enabled (default READ COMMITTED locking reads can block writers — set `READ COMMITTED SNAPSHOT` recommendation or `SET TRANSACTION ISOLATION LEVEL SNAPSHOT` if enabled); SQLite database file replaced underneath (open by fd); driver auto-retry semantics (disable).
- **Test cases:** run the bypass catalogue (§3.5) as an integration test suite against Docker servers with the classifier **disabled** (server layers alone must block every persistent-write case in P1–P10, M2, M7–M9, S3, L1–L2, C1, C3) and **enabled** (all cases denied before execution); canary test: a SECURITY DEFINER function that writes must fail under RO; `COMMIT` statement smuggled with classifier disabled → agent detects txn termination and discards connection.
- **Prior art and evidence:** pgls typecheck uses fresh connections closed after use to avoid cached state [Observed: `pgls_typecheck/src/lib.rs:62-68`]; PG/MySQL/MSSQL/SQLite/ClickHouse/Oracle mechanisms [Inferred: vendor docs; to verify §7].
- **Used by:** Karagöz (Hacivat direct-connection mode reuses the same guards for its local "read-only connection" toggle)

### ALG-SQL-SAFE-03: Function and operator side-effect assessment
- **Problem:** Decide the effects of a function call (or an operator/cast implemented by a function) given dialect builtin knowledge and, when available, catalog metadata.
- **Inputs / outputs:** `(fn: {schema?, name, arg_count, arg_types?}, dialect, server_version, catalog: Option<FnCatalog>, search_path_pinned)` → `(EffectSet, Confidence, ReasonCode)`.
- **Algorithm:**
  1. Resolve the name exactly as the server would (pinned search path; PG: `pg_catalog` first). If the call is schema-qualified, use it. If ambiguous/unresolvable → step 5.
  2. If `(schema, name)` ∈ **denylist** (versioned per dialect, matched case-insensitively for unquoted names and also by bare name regardless of schema for builtin-looking names) → its listed effects (Admin, ExternalNetwork, DynamicSql, Lock, Sleep/ResourceHeavy, SequenceAdvance, InfoDisclosure…).
  3. If ∈ **allowlist** (curated read-only builtins for the dialect/version) → `∅` (pure), `Exact`.
  4. If catalog metadata available (user-defined or extension function):
     - PG: `prokind` (`p` procedure ⇒ ProceduralCode), `provolatile` (`i`/`s` with `prolang` ∈ {sql, plpgsql} ⇒ treat as read-only `Conservative`, since PG forbids data modification inside non-volatile SQL/PL functions; `v` ⇒ `Unknown`), `prolang` C/internal non-builtin or untrusted PL ⇒ `Unknown`; `prosecdef` (SECURITY DEFINER) ⇒ add flag `PrivilegeAmplification` (RO txn still applies); extension membership (`pg_depend` → extension name) ⇒ check extension denylist (dblink, postgres_fdw, http, pg_net, adminpack, file_fdw, plpython3u…).
     - MySQL: `ROUTINES.SQL_DATA_ACCESS` (`NO SQL`/`READS SQL DATA` ⇒ Conservative read; `MODIFIES SQL DATA`/`CONTAINS SQL` ⇒ Unknown), `SECURITY_TYPE` DEFINER flag; UDFs from `mysql.func` ⇒ Unknown.
     - SQL Server: user functions (`sys.objects` type FN/IF/TF) — functions cannot perform DML on persistent tables (T-SQL restriction) but can call extended procs? (scalar UDFs can call `xp_` extended stored procedures) ⇒ Conservative unless body references `xp_`/`OPENQUERY` (requires definition text) ⇒ Unknown otherwise for CLR functions.
     - Oracle: any user function ⇒ Unknown (autonomous transactions, definer rights) unless allowlisted.
     - ClickHouse: table functions per allowlist; UDFs (SQL lambdas) ⇒ pure; executable UDFs ⇒ Unknown.
  5. No metadata ⇒ `Unknown`.
- **Data structures:** versioned TOML/JSON data files per dialect: `allow.toml` (by name and arity), `deny.toml` (name → effects, reason), `extensions.toml`.
- **Complexity:** O(1) per call with hash maps.
- **Edge cases:** function with the same name as a denylisted builtin in another schema (deny by name — conservative); variadic calls; named-argument calls (`f(a => 1)`); functions used as table sources (`FROM generate_series(…)` — allowlisted but `ResourceHeavy` if bounds non-constant or huge); aggregates with FILTER; window functions; `CALL` of procedures; overloaded builtins with side-effecting variants (e.g. `lo_open` with write mode); functions taking SQL text (`query_to_xml`) — always DynamicSql; MySQL builtins with side effects (`GET_LOCK`, `RELEASE_LOCK`, `SLEEP`, `BENCHMARK`, `LOAD_FILE`, `MASTER_POS_WAIT`, `SOURCE_POS_WAIT`, `RAND` (no), `LAST_INSERT_ID(expr)` sets session state); T-SQL `NEXT VALUE FOR`, `NEWSEQUENTIALID` (DEFAULT only), `OPENROWSET`, `OPENQUERY`, `OPENDATASOURCE`, `OPENXML` (read).
- **Test cases:** PG `lower('A')` → ∅; `pg_sleep(1)` → ResourceHeavy (deny by default); `public.pg_sleep(1)` (user function shadowing) → Deny (by name); `my_fn()` STABLE plpgsql → Conservative read; `my_fn()` VOLATILE → Unknown; `my_c_fn()` C → Unknown; MySQL `GET_LOCK('a', 1)` → AdvisoryLock; MySQL routine with `READS SQL DATA` → Conservative read.
- **Prior art and evidence:** pg_query `functions()` with `Context::Call` [Observed: `parse_result.rs:221-250`]; pgls catalog snapshot of functions (behaviour/volatility used in typing) [Observed: `pgls_catalog/src/snapshot/functions.rs`]; PG non-volatile function write restriction [Inferred: PG docs "Function Volatility Categories"].
- **Used by:** Hacivat | Karagöz

### ALG-SQL-LINT-01: Lint engine with suppressions and fixes
- **Problem:** Run configurable rules over statements at the right analysis level, report diagnostics with optional fixes, honour suppression comments, and apply fixes safely.
- **Inputs / outputs:** `(file syntax + per-statement semantic results, config, rule registry)` → `Vec<Diagnostic{rule, severity, range, message_id, args, fixes: Vec<Fix{edits, safety: Safe|Unsafe}>}>`; `apply_fixes(file, selection) -> new text`.
- **Algorithm:**
  1. Build the active rule set: defaults < user settings < project config (`.hacivat/sql.toml`) < file-level suppression (`-- hacivat-ignore-all <rule|group>` before the first statement) ; filter by dialect and by available analysis level (catalog rules disabled without a snapshot; DB-probe rules only on explicit request).
  2. Parse suppression comments: `-- hacivat-ignore <rule>[, <rule>] [: reason]` applies to the next statement (or the same line if trailing); `-- hacivat-ignore-start <rule>` … `-- hacivat-ignore-end <rule>` ranges. Record each suppression with a "used" flag. (Syntax mirrors pgls `-- pgls-ignore`, `-ignore-start/-end`, `-ignore-all`.)
  3. For each statement with a clean enough tree (rules declare whether they accept trees with `ERROR` nodes; most don't), dispatch node-kind subscriptions to rules (visitor), collecting diagnostics; semantic rules query `sql-sema` (`resolve`, `scopes`); catalog rules use the effective catalog; certainty: only `Missing` lookups produce "unknown X".
  4. Filter diagnostics by suppressions (mark used); emit `unused-suppression` diagnostics for unused ones (warning).
  5. Fix application: collect `Safe` fixes (or selected ones); sort by range; drop overlapping edits (keep higher-priority rule); apply; re-lex/re-parse; abort the round if new parse errors appear (revert); repeat until no fixes or `max_rounds` (sqruff `runaway_limit` analogue, default 10); verify via ALG-SQL-FMT-01 step 7 token comparison for rules that claim semantic preservation.
- **Complexity:** O(nodes × subscribed rules) per statement; incremental through salsa per statement.
- **Edge cases:** suppression for unknown rule name (diagnostic); suppression inside a string (ignored, lexer knows); rules on statements with errors (skip unless opted in); conflicting fixes; fixes that change semantics (must be `Unsafe`, not auto-applied); generated code regions; huge files (rule time budgets).
- **Test cases:** `DELETE FROM t;` → `destructive.delete-without-where` (Confirm in execution guard); same with `-- hacivat-ignore destructive.delete-without-where` above → suppressed; unused suppression → warning; `SELECT a FROM t WHERE b = NULL` → `correctness.null-comparison` with Safe fix `IS NULL`.
- **Prior art and evidence:** pgls suppressions [Observed: `pgls_suppressions/src/{lib.rs,parser.rs}`]; sqruff fix loop [Observed: `core/linter/core.rs:684-740`].
- **Used by:** Hacivat

### ALG-SQL-FMT-01: Formatter pipeline (Wadler-style printer with alignment and verification)
- **Problem:** Produce consistently formatted SQL from the lossless CST, preserving comments and semantics, with column alignment options, deterministic and idempotent.
- **Inputs / outputs:** `(file or range, dialect, FormatOptions{line_width=100, indent=2|4|tab, keyword_case=Upper|Lower|Preserve, type_case, function_case, identifier_case=Preserve, comma=Trailing|Leading, logical_operator=Before|After, align_aliases, align_column_defs, align_assignments, blank_lines_between_statements=1..2, semicolon_newline})` → edits (minimal diff) or `Err(reason)` per statement.
- **Algorithm:**
  1. **Select statements**: for a range format, the statements intersecting the range; skip statements whose tree contains `ERROR` nodes, unsupported (`UNKNOWN_STMT`) kinds, or client commands → emitted verbatim.
  2. **Comment attachment**: walk tokens; each comment becomes `Leading(node)` (own-line comment before a node), `Trailing(node)` (same-line comment after a node's last token), or `Dangling(node)` (inside an otherwise empty construct, e.g. `()`), choosing the deepest enclosing node whose boundaries are adjacent (prettier/ruff strategy). Blank lines before a leading comment are preserved (max 1).
  3. **Build Doc** from typed AST with per-node builders. Doc primitives: `Text`, `Space`, `Line` (space | newline), `SoftLine` (empty | newline), `HardLine`, `Group(id, docs)`, `Indent(doc)`, `Align(n, doc)`, `IfBreak(break_doc, flat_doc, group_id)`, `Fill(docs)` (paragraph-like), `LineSuffix(doc)` (trailing comments), `BreakParent` (forces enclosing groups to break — used after line comments and `HardLine`), `AlignGroup(id)` + `AlignCell(group, col, doc)`.
     Key builders: statement = `Group(clause_1 Line clause_2 …)` where each clause is `Group(keyword Indent(SoftLine body))`; select list = `Indent(SoftLine join(items, "," Line))`; boolean chains flattened into one group with breaks before/after `AND`/`OR`; parenthesised subqueries `( Indent(SoftLine query) SoftLine )`; JOIN as its own clause line with `ON` condition indented; `CASE` with `WHEN` lines; `IN (…)` lists as `Fill`; function args as group with soft breaks; CREATE TABLE column list one per line when broken; set operations with keyword on its own line.
     Keyword tokens are emitted with case transform (ASCII-only); identifiers, literals, operators emitted verbatim; ensure required spaces between adjacent tokens via a `needs_space(prev_token, next_token, dialect)` table (prevents merges listed in §3.12).
  4. **Alignment pre-pass** for each `AlignGroup` (e.g. select items' `AS` aliases, column definitions name/type/constraints, `SET a = …` assignments): render each cell flat to measure width (Unicode display width of the text — East Asian wide chars count 2); if any row's cells don't fit flat within `line_width` or contain hard breaks, that row is excluded; compute per-column max width over included rows (cap at `max_align_width`, e.g. 40; overflow rows excluded); replace `AlignCell` with `Text(cell) + Text(padding)`; alignment is disabled for the group if fewer than 2 rows remain.
  5. **Print** with the Wadler/Leijen algorithm: stack of `(indent, mode: Flat|Break, doc)`; for a `Group` in Break-capable context, test `fits(rest_width, [(indent, Flat, group)] + rest_commands_until_first_line_in_break_mode)`; `LineSuffix` contents are flushed before the next newline; `BreakParent` propagated in a pre-pass (groups containing hard breaks are pre-broken). Trailing whitespace trimmed; final newline per options.
  6. **Assemble** file: original inter-statement trivia normalised (blank lines clamped), verbatim statements untouched, semicolons placed per options (never added in dialects/contexts where they change meaning: T-SQL batches before `GO`, Oracle PL/SQL units ending in `/`).
  7. **Verify** each formatted statement: (a) re-lex output with the same dialect/modes; compare the sequence of non-trivia tokens with the input's: kinds equal, texts equal except keyword tokens compared case-insensitively → otherwise `Err(NotTokenEquivalent)` and keep original; (b) comments: multiset of comment texts equal (whitespace-normalised) → otherwise `Err(CommentLost)`; (c) PG: libpg_query fingerprint and (optionally) normalised AST equality as pgls does; (d) whitespace-sensitive checks implicit in (a) (string continuation requires newline: the lexer sees one `STRING` + `STRING`; if formatter put them on one line, PG parse fails → (c) catches; add explicit rule in `needs_newline(prev, next)`).
  8. **Idempotency** (test-time always; runtime optional): `format(format(x)) == format(x)`; pgls allows up to 6 passes for comment layouts before failing [Observed: `pgls_pretty_print/src/lib.rs:213-257`].
- **Data structures:** Doc arena (`Vec<DocNode>` with indices), measured-width cache per node, printer command stack.
- **Complexity:** O(n) doc building + O(n · w) worst case printing (fits lookahead bounded by line width); alignment O(rows × cols).
- **Edge cases (exhaustive list):** statements with errors (verbatim); comments between every token (`SELECT/*a*/1/*b*/FROM/*c*/t`); line comment at end of statement before `;` (keep newline before `;` or move `;` before the comment — must stay token-equivalent and not comment out the `;`); `--` comment inside a select list item; comments inside CASE before END (pgls special-cases "before CASE END" [Observed: `emitter.rs:58`]); very long identifiers/literals exceeding width (no break possible — overflow allowed); deeply nested subqueries (indent cap/relative); dollar-quoted bodies (verbatim or format embedded SQL as a nested document with indentation preserved inside? default verbatim); executable comments (verbatim); MySQL hints; T-SQL batches with `GO`; psql meta-commands; empty statements `;;`; trailing whitespace inside strings (untouched); Windows line endings (preserve the file's dominant EOL); tabs vs spaces; Unicode width for alignment; keywords used as identifiers (`SELECT "select"` or unreserved keywords like `name`, `type` — case transform must apply only to tokens the *parser* classified as keywords, not to identifiers spelled like keywords); quoted identifier case (never change); `NULL`/`TRUE` literal case option; operator spacing for unary minus (`- 1` vs `-1`), PG `::` (no spaces), `->>` spacing; `IN (` list with thousands of items (Fill); generated INSERT with 10k VALUES rows (format performance; alignment disabled above N rows).
- **Test cases:**
  1. `select a,b from t where x=1 and y=2` (width 100) → `SELECT a, b FROM t WHERE x = 1 AND y = 2`.
  2. Same with width 20 → clause-per-line layout with `a,`/`b` on separate lines and `AND` on its own line.
  3. `SELECT 1 - -1` → stays `SELECT 1 - -1` (never `1--1`).
  4. PG `SELECT 'a'\n'b'` → newline between literals preserved.
  5. `SELECT a -- c\nFROM t` → comment stays at end of the `a` line; `FROM` on the next line.
  6. `CREATE TABLE t (id int primary key, name text not null)` with `align_column_defs` → names and types aligned in columns.
  7. Formatting twice → identical output.
  8. Statement with syntax error → returned unchanged, diagnostic `format.skipped-has-errors`.
- **Prior art and evidence:** pgls pretty printer (layout events, comment maps, AST verification, idempotency loop) [Observed: `pgls_pretty_print/src/{lib.rs:200-330,renderer.rs,emitter.rs}`]; sqruff reflow/LT rules [Observed: `utils/reflow/*`, `rules/layout/*`]; Wadler "A prettier printer" / Prettier doc IR [Inferred].
- **Used by:** Hacivat

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **`sql-safety` crate** (shared): ALG-SQL-SAFE-01/03 + versioned data files (statement tables, function allow/deny lists per dialect/version, extension lists) + policy engine primitives. Karagöz is authoritative; Hacivat uses it for UI (badges "Read", "Write", "DDL" per statement; confirmation dialogs; MCP tool-call gating).
2. **Karagöz `ExecutionGuard` per dialect** implementing ALG-SQL-SAFE-02; onboarding wizard (access/ docs) provisions least-privilege principals per access level and verifies them (e.g. PG: check role attributes and memberships; refuse to run read-only level with a superuser or `pg_write_all_data` member).
3. **Audit**: store `fingerprint` (PG: libpg_query; others: own normaliser), normalised SQL, effects, reasons, decision, parser versions, row counts. Perde receives audit summaries (README: audit enforceable at Karagöz).
4. **Policy profiles** shipped: `read_only` (ReadData + ReadMetadata only), `read_only_plus_explain`, `read_write_dml` (Write but no DDL/DCL/admin; confirmations for unbounded UPDATE/DELETE), `ddl` (migrations), `admin` (no classifier denial, still audit). Custom profiles from Perde (open-core split TBD).
5. **Hacivat lint/format**: `sql-lint` with the initial rule set (§3.10) and `sql-fmt` (ALG-SQL-FMT-01); format-on-save off by default; "format selection".
6. **Testing**: a `bypass-corpus` (all §3.5 rows as SQL files with expected decisions) run in CI against real servers in Docker for each dialect; property tests: classifier never returns Allow when the server-side guard reports a write (differential fuzzing with sqlsmith-like generators for PG).

## 6. i18n considerations

- Denial reasons, effects and lint messages are `ReasonCode`/rule IDs with typed args → Fluent messages (English + Turkish in v1). SQL fragments in messages (keywords, object names) are inserted as code spans, never translated.
- Audit logs store codes + args (language-neutral); Perde renders them in the viewer's locale.
- Formatter keyword casing is ASCII-only (`select` → `SELECT` with Rust `to_ascii_uppercase`) — never locale-aware (Turkish `i` → `İ` bug class); identifier case options (if ever offered) must also be ASCII-only and off by default.
- Alignment uses Unicode display width (CJK/Turkish characters render width 1 except wide ones); test with Turkish identifiers containing `ğ ş ı İ ç ö ü` and combining marks.

## 7. Risks, pitfalls, open questions

- **Items to verify on real servers (conformance matrix):** (update: PG `LOCK TABLE`, `NOTIFY`, `lo_export`, physical replication slots, `CREATE TEMP TABLE`, `CHECKPOINT`, the snapshot pin on PG 16–18, MySQL DDL vs `START TRANSACTION READ ONLY` and SQLite `VACUUM INTO` were verified — see verification/01-db-behavior-tests.md; PG 13–15, logical slots, Oracle and ClickHouse remain open) PG behaviour of `LOCK TABLE`, `NOTIFY`, `lo_export`, replication-slot functions, `CREATE TEMP TABLE`, `CHECKPOINT` inside `READ ONLY` transactions; the snapshot-pin trick across PG 13–18; MySQL DDL vs `START TRANSACTION READ ONLY` implicit-commit interplay; SQLite `VACUUM INTO` on read-only connections; Oracle `READ` privilege semantics; ClickHouse `readonly=1` handling of query-level `SETTINGS`.
- **False denials** will frustrate users (e.g. harmless `SET search_path` in pasted scripts); mitigate with clear reasons, per-policy allowlists for specific SETs, and Hacivat-side automatic stripping of benign session prefixes *only* with user confirmation.
- **Catalog-dependent decisions** (function volatility) require a trustworthy, fresh catalog at Karagöz; an attacker with CREATE privileges could change a function between classification and execution (TOCTOU) — read-only levels must not have CREATE anywhere; server-side RO txn remains the backstop.
- **sqlparser-rs as second opinion** may accept syntax the server parses differently; union-of-effects is safe only if *both* parse; if one fails we deny — accept some false denials.
- **Formatter risks:** semantic drift through token merges or lost newlines; mitigated by token-equivalence verification and refusal to format erroneous statements. **Open:** default style (uppercase keywords, trailing commas, 2-space indent?) — decide with UI/UX research; provide presets ("compact", "aligned", "pgls-like", "sqlfluff-like").
- **Open:** whether to offer an *approval* path for denied statements (four-eyes) — belongs to access/ research; the classifier must provide precise reasons to make approvals meaningful.
