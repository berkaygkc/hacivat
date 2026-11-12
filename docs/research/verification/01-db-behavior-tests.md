# 01 — Database Behaviour Verification (Docker)

> Empirical verification of database-behaviour claims made in `clients/sql-tooling-05-safety-classification-lint-format.md`, `datagrip/04-introspection-and-metadata-model.md`, `access/04-zero-credential-discovery-and-ephemeral-users.md` and `i18n/02-turkish-and-db-specific-pitfalls.md`. Every verdict below comes from a real server started with `docker run --rm` on 2026-11-11 (Apple Silicon host, Docker 28.3.2 arm64). Source docs were edited in place to point here.

## 1. Scope, environment and method

| Image (tag) | Exact server version | Platform | Used for |
|---|---|---|---|
| `postgres:16` | 16.15 (Debian 16.15-1.pgdg13+2) | arm64 | PG read-only, xmin, ephemeral users, Turkish ICU |
| `postgres:17` | 17.11 (Debian 17.11-1.pgdg13+2) | arm64 | same + xid wraparound simulation (`pg_resetwal`) |
| `postgres:18` | 18.6 (Debian 18.6-1.pgdg13+2) | arm64 | same |
| `mysql:8.0` | 8.0.46 | arm64 | read-only modes, executable comments, I_S times, ephemeral users, Turkish collation |
| `mysql:8.4` | 8.4.13 | arm64 | same |
| `keinos/sqlite3:latest` | SQLite 3.54.1 (2026-10-16) | arm64 | `mode=ro`, `query_only`, `VACUUM INTO`, `ATTACH`, `schema_version`, identifier case |
| `redis:7` | 7.4.12 | arm64 | default auth, ACL read-only users |
| `mongo:7` | 7.0.45 | arm64 | no-auth default, localhost exception, `read` role vs `$out`/`$merge` |
| `clickhouse/clickhouse-server:24.8` | 24.8.14.39 | arm64 | default-user network restriction, env visibility |
| `mcr.microsoft.com/mssql/server:2022-latest` | 16.0.4315.2 (2022 CU27-GDR) | **amd64 under emulation** | `Turkish_CI_AS` identifier resolution |

Method: each claim was turned into a minimal script; scripts and raw outputs were kept in a scratch area and the decisive lines are quoted below. Containers were named `hacivat-verify-*` and removed afterwards. Not tested: ClickHouse `readonly=1` profiles, Oracle, MariaDB, PG hot standby, logical replication slots (default `wal_level=replica`), MySQL killing of existing sessions after `ACCOUNT LOCK`.

Verdict vocabulary: **Confirmed** (behaves as the doc says), **Refuted** (doc is wrong; corrected), **Partially** (true with a material nuance; doc amended), **Not testable** (here).

## 2. Verdict table

| # | Claim (source doc) | Verdict | Anchor |
|---|---|---|---|
| 1 | PG: `BEGIN READ ONLY` + first query pins read-only; `SET TRANSACTION READ WRITE` / `SET transaction_read_only=off` / `set_config(...)` fail afterwards (sql-tooling-05 §2.7, P5) | Confirmed (16/17/18) | [pg-ro-pin](#pg-ro-pin) |
| 2 | PG RO txn blocks INSERT, DDL, COMMENT, GRANT, TRUNCATE, `nextval`/`setval` (non-temp), `EXPLAIN ANALYZE <dml>`, `SELECT INTO`, writable CTE, writes inside VOLATILE / SECURITY DEFINER functions, `query_to_xml('DELETE…')`, `lo_from_bytea` (sql-tooling-05 §2.6, §3.4) | Confirmed | [pg-ro-blocked](#pg-ro-blocked) |
| 3 | PG RO txn does **not** block `LOCK TABLE` (any mode), advisory locks (session locks survive ROLLBACK), `pg_terminate_backend`, `NOTIFY`/`pg_notify`, `SET`/`set_config`/`SET ROLE`, `COPY … TO PROGRAM`/file, `dblink_exec` (sql-tooling-05 §2.6, P11–P15, P25) | Confirmed; nuance: NOTIFY is only delivered on COMMIT | [pg-ro-escapes](#pg-ro-escapes) |
| 4 | PG `CREATE TEMP TABLE` is a "temporary object" bypass under RO (P26) | Partially — `CREATE TEMP TABLE`/`CREATE TEMP SEQUENCE` are **blocked** in a RO txn; DML on temp tables / `nextval` on temp sequences created *before* the txn is allowed; `DECLARE … WITH HOLD` and `LISTEN` are allowed | [pg-ro-temp](#pg-ro-temp) |
| 5 | PG `txid_current()` / `pg_current_xact_id()` are blocked in RO txn (sql-tooling-05 §3.4 "XID/admin probes") | **Refuted** — both succeed and assign an XID | [pg-canary](#pg-canary) |
| 6 | PG `txid_current_if_assigned() IS NULL` after the statement is a no-write canary (sql-tooling-05 §2.9, §3.6) | Partially — NULL after reads; non-NULL after local writes; **NULL after `dblink_exec` and `COPY TO PROGRAM`** (misses them); non-NULL after `LOCK … ACCESS EXCLUSIVE`, temp-table DML and `txid_current()` (false alarms) | [pg-canary](#pg-canary) |
| 7 | PG `lo_export` under RO: [Unknown] (sql-tooling-05 §3.4) | Resolved — allowed; writes the server file | [pg-ro-escapes](#pg-ro-escapes) |
| 8 | PG replication functions under RO: "Varies [Unknown]" | Resolved (partially) — `pg_create_physical_replication_slot` allowed and the slot **survives ROLLBACK**; `pg_logical_emit_message`, `pg_switch_wal` allowed; logical-slot functions not testable (`wal_level=replica`) | [pg-ro-escapes](#pg-ro-escapes) |
| 9 | PG `postgres_fdw` is a full RO bypass ("separate session") | Partially — remote transaction is **not** read-only (a remote view calling a writing function executed), but it is coordinated with the local txn: ROLLBACK undoes the remote write, COMMIT keeps it; direct INSERT into a foreign table is blocked locally | [pg-ro-escapes](#pg-ro-escapes) |
| 10 | PG mislabelled IMMUTABLE function evaluated at plan time by plain `EXPLAIN` (P21) | Confirmed — `EXPLAIN SELECT f_imm_dblink()` committed a row via dblink | [pg-ro-escapes](#pg-ro-escapes) |
| 11 | PG `DISCARD ALL` resets session state (advisory locks, prepared stmts, LISTEN, temp tables, SET ROLE, GUCs) | Confirmed; cannot run inside a transaction block | [pg-discard](#pg-discard) |
| 12 | MySQL `START TRANSACTION READ ONLY` blocks DML | Confirmed (ERROR 1792) | [mysql-ro](#mysql-ro) |
| 13 | MySQL DDL vs RO txn implicit-commit interplay: [Unknown] (M8) | Resolved — DDL (`CREATE`/`DROP`/`TRUNCATE`) **executes**: the implicit commit ends the RO txn first; subsequent DML in the same session commits. `SET SESSION TRANSACTION READ ONLY` does block DDL. `LOCK TABLES` also ends the RO txn | [mysql-ro](#mysql-ro) |
| 14 | MySQL session RO is bypassable (`SET SESSION TRANSACTION READ WRITE`, `SET @@transaction_read_only=0`) (M9) | Confirmed; inside an active RO txn `SET TRANSACTION READ WRITE` fails (ERROR 1568) | [mysql-ro](#mysql-ro) |
| 15 | MySQL "temporary tables allowed in RO transactions" (access/04 §3.14) | Partially — DML on an existing temp table allowed; `CREATE TEMPORARY TABLE` **blocked** inside `START TRANSACTION READ ONLY`; allowed under `super_read_only` | [mysql-ro](#mysql-ro) |
| 16 | MySQL executable comments `/*! */` execute; version gate; `/*M! */` (M1) | Confirmed — `/*!50000 … */` and `/*!80000 … */` run (incl. a whole `DROP TABLE` statement); `/*!99999 … */` skipped; `/*M! … */` ignored by MySQL | [mysql-comments](#mysql-comments) |
| 17 | MySQL optimizer hints `/*+ … */` change execution (M16) | Confirmed — `MAX_EXECUTION_TIME(500)` interrupted `SLEEP(2)`; `SET_VAR(sql_mode=…)` does **not** change lexing of the same statement; `SET_VAR(transaction_read_only=0)` does not lift RO | [mysql-comments](#mysql-comments) |
| 18 | MySQL `super_read_only` blocks writes server-wide but not per user (access/04 §3.14) | Confirmed; temp tables and `GET_LOCK` still work; an admin can switch it OFF, and `read_only` stays ON afterwards | [mysql-super-ro](#mysql-super-ro) |
| 19 | SQLite `VACUUM INTO` may work on a read-only DB [Inferred — verify] (L2) | Confirmed — works with `mode=ro` and `-readonly`; blocked by `PRAGMA query_only=1` (but a 0-byte file is still created) | [sqlite-ro](#sqlite-ro) |
| 20 | SQLite `ATTACH` new file writable from read-only main connection; `query_only` can be turned off (L1, L2) | Confirmed | [sqlite-ro](#sqlite-ro) |
| 21 | SQLite PRAGMA writes on `mode=ro` | Confirmed blocked (`user_version`, `journal_mode=WAL`); `writable_schema=1` accepted (session flag) but still no writes; temp tables writable | [sqlite-ro](#sqlite-ro) |
| 22 | Redis ACL `-@all +@read +@connection -@dangerous` / `+@read -@write` read-only users | Confirmed; `+@read -@write` still allows `KEYS`/`SORT_RO` (in `@dangerous`) | [redis-acl](#redis-acl) |
| 23 | Redis: Lua scripts run commands under the caller's ACL (Redis 7) [Inferred] | Confirmed (`EVAL` and `FCALL` both get "ACL failure in script"); **new finding**: `+@scripting` grants `SCRIPT FLUSH`, and when listed after `-@write` re-grants `FUNCTION FLUSH/LOAD/DELETE/RESTORE` | [redis-acl](#redis-acl) |
| 24 | MongoDB `read` role blocks `$out`/`$merge` | Partially — blocked for a pure `read` user; a user with `readWrite` on *another* database can `$out`/`$merge` data from the read-only DB into it; `$where`/`$function` JS run under `read` | [mongo-read](#mongo-read) |
| 25 | PG incremental introspection: `CREATE INDEX` does not change table `pg_class.xmin`; `ADD COLUMN` does; `COMMENT` only `pg_description`; `DROP` invisible (datagrip/04) | Confirmed on 16/17/18 + extended matrix (GRANT, CREATE TRIGGER, ALTER FUNCTION, SET DEFAULT, CHECK, DROP COLUMN) | [pg-xmin](#pg-xmin) |
| 26 | PG frozen tuples have huge age → never considered changed (datagrip/04 §PG, ALG-INTRO-02) | **Refuted** — freezing keeps the raw xmin; after >2^31 XIDs `age(xmin)` becomes **negative** and `age(xmin) <= N` matches (false re-reads); only bootstrap rows (xmin 1/2) get `INT_MAX` | [pg-xmin-freeze](#pg-xmin-freeze) |
| 27 | `txid_current()` "errors in read-only transaction" (datagrip/04 ALG-INTRO-02 edge cases) | **Refuted** for primaries (works and assigns an XID); standby not tested | [pg-canary](#pg-canary) |
| 28 | SQLite `PRAGMA schema_version` increments on every schema change; not on INSERT | Confirmed; also increments on `VACUUM`; temp schema has its own counter; rolled-back DDL does not increment | [sqlite-schema-version](#sqlite-schema-version) |
| 29 | MySQL `CREATE_TIME` changes on some ALTERs only / only with table rebuild; `UPDATE_TIME` unreliable (datagrip/04) | **Refuted** (CREATE_TIME) / Confirmed (UPDATE_TIME) — on 8.0.46 and 8.4.13 every tested `ALTER TABLE` (INSTANT add column, INPLACE add index, comment, rename column, COPY) changed CREATE_TIME; `CREATE TRIGGER` did not; 1-second granularity; UPDATE_TIME is NULL unless `information_schema_stats_expiry=0` and resets after ALTER | [mysql-is-times](#mysql-is-times) |
| 30 | Official images keep bootstrap passwords in `Config.Env` though entrypoints `unset` them (access/04 §2.3) | Confirmed; scrubbed from PID 1 only for postgres and mongo — **mysql and clickhouse PID 1 still carry them**; every `docker exec` process inherits `Config.Env` | [docker-env](#docker-env) |
| 31 | PG official image: local socket and loopback are `trust` (access/04 §3.6.1, §3.9) | Confirmed (pg_hba dump); `VALID UNTIL` is not enforced under trust | [local-auth](#local-auth) |
| 32 | MySQL official image: no socket-auth root (access/04 §3.6.2, §3.9) | Confirmed (`ERROR 1045 … using password: NO`) | [local-auth](#local-auth) |
| 33 | MongoDB: no auth without `MONGO_INITDB_ROOT_*`; localhost exception only while no users exist | Confirmed | [mongo-localhost](#mongo-localhost) |
| 34 | Redis official image: no auth, protected mode disabled | Confirmed | [redis-noauth](#redis-noauth) |
| 35 | ClickHouse default user without password restricted to `::1`/`127.0.0.1`; with `CLICKHOUSE_PASSWORD`, default user is replaced and opened to `::/0` | Confirmed; nuance: the container's own non-loopback IP is also accepted; `clickhouse-client` inside the container silently uses `CLICKHOUSE_PASSWORD` from the inherited env | [ch-default](#ch-default) |
| 36 | PG ephemeral-user templates (create/grant/comment/ALTER ROLE SET, PG16 `WITH INHERIT TRUE, SET FALSE`, CREATEROLE scoping, deactivate/drop) | Confirmed; nuances: object-level GRANTs by a non-owner admin only WARN ("no privileges were granted"); read tier can create temp tables | [pg-ephemeral](#pg-ephemeral) |
| 37 | MySQL ephemeral-user template | Partially — **template bug**: `hacivat_ddl` is never created, so its GRANT fails (ERROR 1410); rest works (ATTRIBUTE, default role, ACCOUNT LOCK, DROP USER) | [mysql-ephemeral](#mysql-ephemeral) |
| 38 | `SET ROLE` / `SET SESSION AUTHORIZATION` from an admin login does not contain the user (`RESET ROLE`) | Confirmed on PG (MySQL/SQL Server variants not run) | [set-role](#set-role) |
| 39 | PG with Turkish collation: `upper('i')='İ'`, `lower('ITEMS')='ıtems'`; unquoted identifiers fold ASCII-only (`MÜŞTERİ`→`mÜŞterİ`); `name` columns use `C`; ORDER BY tr vs root vs C word lists | Confirmed (ICU `tr-TR`; libc `tr_TR` locale is **not installed** in the official image); new: `'ITEMS' ILIKE 'items'` is **false** in a tr-ICU database | [pg-turkish](#pg-turkish) |
| 40 | MySQL `utf8mb4_tr_0900_ai_ci` | Confirmed ordering; `I=ı`, `i=İ`, `I≠i`, `ş≠s`; **`LOWER/UPPER` are not Turkish-tailored**; I_S `TABLE_NAME` is `utf8mb3_bin` (lctn=0) so the doc's "case-insensitive in practice" is refuted | [mysql-turkish](#mysql-turkish) |
| 41 | SQLite identifiers and `LIKE`/`upper`/`lower` are ASCII-only case-insensitive | Confirmed | [sqlite-case](#sqlite-case) |
| 42 | SQL Server `Turkish_CI_AS`: `information_schema.tables` fails; `ID`≠`id`; `@ID`≠`@id` | Confirmed (amd64 emulation); `@ID`/`#temp` behaviour depends on the **instance** collation (works with default instance collation, fails with `MSSQL_COLLATION=Turkish_CI_AS`) | [mssql-turkish](#mssql-turkish) |

## 3. Per-claim details

Scripts below are complete; run them with `docker exec -i <container> psql -U postgres < file.sql` (PG), `mysql -uroot -p<pw> --force -t < file.sql` (MySQL), etc.

### 3.1 PostgreSQL read-only transaction defence

Start: `docker run --rm -d --name hacivat-verify-pg16 -e POSTGRES_PASSWORD=s3cretPW postgres:16` (same for 17, 18).

Setup (`setup.sql`):

```sql
CREATE TABLE t(id int primary key, v text); INSERT INTO t VALUES (1,'a');
CREATE SEQUENCE s;
CREATE EXTENSION dblink; CREATE EXTENSION postgres_fdw;
CREATE ROLE ro LOGIN PASSWORD 'ro'; GRANT SELECT ON t TO ro;
SELECT lo_from_bytea(424242, 'hello'::bytea);
CREATE FUNCTION f_write() RETURNS int LANGUAGE plpgsql AS $$BEGIN INSERT INTO t VALUES (100,'fw'); RETURN 1; END$$;
CREATE FUNCTION f_write_sd() RETURNS int LANGUAGE plpgsql SECURITY DEFINER AS $$BEGIN INSERT INTO t VALUES (101,'sd'); RETURN 1; END$$;
CREATE FUNCTION f_imm_dblink() RETURNS int LANGUAGE plpgsql IMMUTABLE AS
  $$BEGIN PERFORM dblink_exec('dbname=postgres user=postgres', 'INSERT INTO t VALUES (300, ''explain-planner'')'); RETURN 1; END$$;
CREATE FUNCTION f_imm_write() RETURNS int LANGUAGE plpgsql IMMUTABLE AS $$BEGIN INSERT INTO t VALUES (301,'x'); RETURN 1; END$$;
CREATE TABLE fdw_log(x text);
CREATE FUNCTION public.f_log() RETURNS int LANGUAGE plpgsql AS $$BEGIN INSERT INTO public.fdw_log VALUES ('remote-write'); RETURN 1; END$$;
CREATE VIEW v_side AS SELECT f_log() AS r;
CREATE SERVER loop FOREIGN DATA WRAPPER postgres_fdw OPTIONS (host '127.0.0.1', dbname 'postgres');
CREATE USER MAPPING FOR postgres SERVER loop OPTIONS (user 'postgres', password 'x');
CREATE FOREIGN TABLE ft_side (r int) SERVER loop OPTIONS (table_name 'v_side');
CREATE FOREIGN TABLE ft_t (id int, v text) SERVER loop OPTIONS (table_name 't');
```

Every test uses the same envelope: `BEGIN READ ONLY; SELECT 1; <statement>; [canary]; ROLLBACK;`. Outputs were byte-identical across 16.15, 17.11 and 18.6 apart from stderr interleaving.

<a id="pg-ro-pin"></a>
#### pg-ro-pin — snapshot pin makes read-only irreversible — Confirmed

```sql
BEGIN READ ONLY; SET TRANSACTION READ WRITE; SHOW transaction_read_only; ROLLBACK;     -- before first query
BEGIN READ ONLY; SELECT 1; SET TRANSACTION READ WRITE; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SET transaction_read_only = off; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT set_config('transaction_read_only','off',true); ROLLBACK;
BEGIN READ ONLY; SELECT 1; RESET transaction_read_only; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE; SHOW transaction_read_only; ROLLBACK;
```

```
transaction_read_only = off                       -- (1) flip works before the first query
ERROR:  transaction read-write mode must be set before any query     -- (2),(3),(4)
ERROR:  parameter "transaction_read_only" cannot be reset            -- (5)
SET / transaction_read_only = on                  -- (6) only changes the default for later txns
```

<a id="pg-ro-blocked"></a>
#### pg-ro-blocked — writes blocked inside RO txn — Confirmed

| Statement | Result |
|---|---|
| `INSERT INTO t …` | `cannot execute INSERT in a read-only transaction` |
| `CREATE TABLE z(a int)` | `cannot execute CREATE TABLE in a read-only transaction` |
| `COMMENT ON TABLE t IS 'x'` | `cannot execute COMMENT in a read-only transaction` |
| `GRANT SELECT ON t TO PUBLIC` | `cannot execute GRANT in a read-only transaction` |
| `TRUNCATE t` | `cannot execute TRUNCATE TABLE in a read-only transaction` |
| `SELECT nextval('s')` / `setval('s',100)` | `cannot execute nextval() / setval() in a read-only transaction` |
| `EXPLAIN (ANALYZE) DELETE FROM t` | `cannot execute DELETE in a read-only transaction` |
| `SELECT query_to_xml('DELETE FROM t RETURNING 1', …)` | `DELETE is not allowed in a non-volatile function` |
| `SELECT * INTO x_new FROM t` | `cannot execute SELECT INTO in a read-only transaction` |
| `WITH d AS (DELETE …) SELECT * FROM d` | `cannot execute SELECT in a read-only transaction` |
| `SELECT f_write()` / `SELECT f_write_sd()` (SECURITY DEFINER) | `cannot execute INSERT in a read-only transaction` |
| `SELECT f_imm_write()` (IMMUTABLE with INSERT) | `INSERT is not allowed in a non-volatile function` |
| `SELECT lo_from_bytea(0,'x')` | `cannot execute lo_from_bytea() in a read-only transaction` |
| `INSERT INTO ft_t …` (postgres_fdw foreign table) | `cannot execute INSERT in a read-only transaction` |
| `COPY t FROM '/tmp/copy_file_out'` | `cannot execute COPY FROM in a read-only transaction` |
| `CALL p_commit()` (procedure with COMMIT) inside txn block | `invalid transaction termination` |

<a id="pg-ro-escapes"></a>
#### pg-ro-escapes — what a RO txn does not stop — Confirmed (with nuances)

```sql
BEGIN READ ONLY; SELECT 1; LOCK TABLE t IN ACCESS EXCLUSIVE MODE;
  SELECT mode FROM pg_locks WHERE relation='t'::regclass AND pid=pg_backend_pid(); ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT pg_advisory_lock(42); SELECT pg_advisory_xact_lock(43); ROLLBACK;
SELECT objid, mode, granted FROM pg_locks WHERE locktype='advisory' AND pid=pg_backend_pid();
BEGIN READ ONLY; SELECT 1; SET search_path = public; SELECT set_config('statement_timeout','0',false);
  SET ROLE ro; SELECT current_user; RESET ROLE; SELECT current_user; ROLLBACK;
BEGIN READ ONLY; SELECT 1; NOTIFY ch, 'from-ro-rollback'; SELECT pg_notify('ch','x'); LISTEN ch2; ROLLBACK;
BEGIN READ ONLY; SELECT 1; NOTIFY ch, 'from-ro-commit'; COMMIT;
BEGIN READ ONLY; SELECT 1; COPY t TO PROGRAM 'cat > /tmp/copy_program_pwned'; ROLLBACK;
BEGIN READ ONLY; SELECT 1; COPY t TO '/tmp/copy_file_out'; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT lo_export(424242, '/tmp/lo_export_out'); ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT length(pg_read_file('/etc/hostname')) > 0; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT dblink_exec('dbname=postgres user=postgres', 'CREATE TABLE dbl_made(x int)'); ROLLBACK;
SELECT to_regclass('dbl_made') IS NOT NULL AS dblink_table_persisted;
BEGIN READ ONLY; SELECT 1; SELECT * FROM ft_side; ROLLBACK;   SELECT count(*) FROM fdw_log;
BEGIN READ ONLY; SELECT 1; SELECT * FROM ft_side; COMMIT;     SELECT count(*) FROM fdw_log;
BEGIN READ ONLY; SELECT 1; EXPLAIN SELECT f_imm_dblink(); ROLLBACK;  SELECT count(*) FROM t WHERE id=300;
BEGIN READ ONLY; SELECT 1; SELECT pg_terminate_backend(pid) FROM pg_stat_activity
  WHERE query LIKE '%victim_sleep%' AND pid <> pg_backend_pid(); ROLLBACK;
BEGIN READ ONLY; SELECT 1; CHECKPOINT; SELECT pg_stat_reset(); ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT pg_create_physical_replication_slot('rs1'); ROLLBACK;
SELECT pg_drop_replication_slot('rs1');       -- succeeds => slot survived ROLLBACK
BEGIN READ ONLY; SELECT 1; SELECT pg_logical_emit_message(true,'p','m'); ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT pg_switch_wal() IS NOT NULL; ROLLBACK;
BEGIN READ ONLY; SELECT 1; COMMIT; INSERT INTO t VALUES (7,'after-commit');   -- P7
```

Listener for NOTIFY (separate session): `psql -c "LISTEN ch" -c "select pg_sleep(10)" -c "select 1"`. Victim for terminate: `docker exec -d … psql -c "select pg_sleep(60) /* victim_sleep */"`.

Key outputs:

```
LOCK TABLE / mode = AccessExclusiveLock                       -> allowed
 objid 42 | ExclusiveLock | t                                  -> session advisory lock survives ROLLBACK (xact lock 43 released)
current_user = ro ... RESET ... current_user = postgres        -> SET/set_config/SET ROLE allowed in RO txn
NOTIFY / pg_notify / LISTEN                                    -> allowed
listener: Asynchronous notification "ch" with payload "from-ro-commit" received   -> only the COMMITted one arrives
COPY 1  (TO PROGRAM)  ; COPY 1 (TO file) ; lo_export = 1       -> files /tmp/copy_program_pwned, /tmp/copy_file_out, /tmp/lo_export_out created (owner postgres)
read_file_ok = t
dblink_exec = CREATE TABLE ; dblink_table_persisted = t        -> full bypass
ft_side returns r=1 inside RO txn; fdw_log rows unchanged after ROLLBACK; +1 after COMMIT
QUERY PLAN Result ... ; explain_side_effect_rows = 1           -> plain EXPLAIN ran the IMMUTABLE fn; dblink write committed
terminated = t
CHECKPOINT ; pg_stat_reset                                     -> allowed
pg_create_physical_replication_slot = (rs1,) ; later pg_drop_replication_slot('rs1') succeeds
pg_logical_emit_message = 0/15F9A60 ; switched = t
INSERT 0 1 / rows_after_commit = 1                              -> after the user's COMMIT the session is plain read-write
```

<a id="pg-ro-temp"></a>
#### pg-ro-temp — temporary objects — Partially

```sql
BEGIN READ ONLY; SELECT 1; CREATE TEMP TABLE tt(a int); ROLLBACK;
BEGIN READ ONLY; SELECT 1; CREATE TEMP SEQUENCE ts; ROLLBACK;
BEGIN READ ONLY; SELECT 1; DECLARE c CURSOR WITH HOLD FOR SELECT 1; ROLLBACK;
CREATE TEMP TABLE dummy(x int); CREATE TEMP SEQUENCE tseq;
BEGIN READ ONLY; SELECT 1; INSERT INTO dummy VALUES (1); SELECT nextval('tseq');
  SELECT txid_current_if_assigned() IS NULL; ROLLBACK;
```

```
ERROR:  cannot execute CREATE TABLE in a read-only transaction
ERROR:  cannot execute CREATE SEQUENCE in a read-only transaction
DECLARE CURSOR
INSERT 0 1 ; temp_nextval = 1 ; canary_null = f
```

<a id="pg-canary"></a>
#### pg-canary — XID functions and the `txid_current_if_assigned()` canary — Partially / Refuted

```sql
BEGIN READ ONLY; SELECT 1; SELECT * FROM t; SELECT txid_current_if_assigned() IS NULL; ROLLBACK;
BEGIN; INSERT INTO t VALUES (2,'b'); SELECT txid_current_if_assigned() IS NULL; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT txid_current() > 0; SELECT txid_current_if_assigned() IS NULL; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT pg_current_xact_id() IS NOT NULL; ROLLBACK;
BEGIN READ ONLY; SELECT 1; SELECT pg_snapshot_xmin(pg_current_snapshot()) IS NOT NULL; ROLLBACK;
```

```
canary_null_after_read = t
canary_null_after_write = f
txid_current_ok = t ; canary_null = f          -- txid_current() works in a RO txn and assigns an XID
pg_current_xact_id_ok = t
snapshot_xmin_ok = t
LOCK TABLE … ACCESS EXCLUSIVE -> canary_null = f   (false alarm: AEL needs an XID)
COPY … TO PROGRAM            -> canary_null = t   (missed)
dblink_exec(… CREATE TABLE)  -> canary_null = t   (missed)
temp-table INSERT            -> canary_null = f   (false alarm)
```

Conclusion: the canary detects local XID assignment only. It is a useful tripwire for "a local write slipped through", not a proof of no side effects.

<a id="pg-discard"></a>
#### pg-discard — DISCARD ALL — Confirmed

```sql
SET ROLE ro; SELECT pg_advisory_lock(77); RESET ROLE; PREPARE ps AS SELECT 1; LISTEN chx;
SET search_path = pg_catalog; CREATE TEMP TABLE tmp_d(a int); SET ROLE ro;
SELECT current_user, (SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND pid=pg_backend_pid()) adv,
       (SELECT count(*) FROM pg_prepared_statements) prep, (SELECT count(*) FROM pg_listening_channels()) listen;
BEGIN; DISCARD ALL; ROLLBACK;
DISCARD ALL;
SELECT current_user, current_setting('search_path'), … , to_regclass('pg_temp.tmp_d');
```

```
 ro | adv 2 | prep 1 | listen 1
ERROR:  DISCARD ALL cannot run inside a transaction block
 postgres | "$user", public | 0 | 0 | 0 | <null>
```

### 3.2 MySQL read-only modes

Start: `docker run --rm -d --name hacivat-verify-mysql80 -e MYSQL_ROOT_PASSWORD=rootPW mysql:8.0` (and `mysql:8.4`). Results identical on 8.0.46 and 8.4.13.

<a id="mysql-ro"></a>
#### mysql-ro — START TRANSACTION READ ONLY, DDL interplay, session RO — Confirmed / Resolved / Partially

```sql
CREATE DATABASE app; USE app; CREATE TABLE t(id int primary key, v varchar(20)); INSERT INTO t VALUES (1,'a');
CREATE TABLE drop_me(a int);
START TRANSACTION READ ONLY; INSERT INTO t VALUES (2,'b'); ROLLBACK;                               -- (a)
START TRANSACTION READ ONLY; SELECT 1; CREATE TABLE ddl_in_ro(a int);                              -- (b)
  SELECT COUNT(*) FROM information_schema.TABLES WHERE TABLE_SCHEMA='app' AND TABLE_NAME='ddl_in_ro';
  INSERT INTO t VALUES (3,'after-ddl'); ROLLBACK; SELECT COUNT(*) FROM t WHERE id=3;
START TRANSACTION READ ONLY; DROP TABLE drop_me; ROLLBACK;                                         -- (c)
START TRANSACTION READ ONLY; TRUNCATE TABLE t; ROLLBACK;                                           -- (d)
START TRANSACTION READ ONLY; CREATE TEMPORARY TABLE tmp1(a int); ROLLBACK;                         -- (e)
CREATE TEMPORARY TABLE tmp2(a int);
START TRANSACTION READ ONLY; INSERT INTO tmp2 VALUES (1); SELECT COUNT(*) FROM tmp2; ROLLBACK;     -- (f)
START TRANSACTION READ ONLY; SET TRANSACTION READ WRITE; SET SESSION transaction_read_only = 0;
  SELECT @@transaction_read_only; INSERT INTO t VALUES (4,'x'); ROLLBACK;                         -- (g)
SET SESSION TRANSACTION READ ONLY; INSERT INTO t VALUES (5,'x');                                   -- (h)
SET SESSION TRANSACTION READ WRITE; INSERT INTO t VALUES (5,'x');                                  -- (i)
SET SESSION TRANSACTION READ ONLY; SET @@transaction_read_only = 0; INSERT INTO t VALUES (6,'x');  -- (j)
SET SESSION TRANSACTION READ ONLY; CREATE TABLE ddl_in_ro2(a int);                                 -- (k)
START TRANSACTION READ ONLY; SELECT GET_LOCK('hv',1); ROLLBACK; SELECT IS_USED_LOCK('hv') = CONNECTION_ID();  -- (l)
START TRANSACTION READ ONLY; SELECT 1; LOCK TABLES t WRITE; INSERT INTO t VALUES (77,'x'); UNLOCK TABLES; ROLLBACK;  -- (m)
START TRANSACTION READ ONLY; SELECT * FROM t FOR UPDATE; ROLLBACK;                                 -- (n)
```

```
(a) ERROR 1792 (25006): Cannot execute statement in a READ ONLY transaction.
(b) ddl_table_exists = 1 ; row3_visible = 1 ; row3_after_rollback = 1     -> DDL ran, implicit commit ended the RO txn, later INSERT committed
(c) drop_me_exists = 0                                                      -> DROP ran
(d) rows in t after TRUNCATE-in-RO = 0                                      -> TRUNCATE ran
(e) ERROR 1792 …                                                            -> CREATE TEMPORARY TABLE blocked in RO txn
(f) temp DML in RO txn rows = 1                                             -> DML on existing temp table allowed
(g) ERROR 1568 (25001): Transaction characteristics can't be changed while a transaction is in progress
    @@transaction_read_only = 0 ; INSERT -> ERROR 1792                     -> session var flips, current txn stays RO
(h) ERROR 1792 ; (i) row5 = 1 ; (j) row6 = 1                               -> session RO trivially bypassed
(k) ERROR 1792 ; ddl2_exists = 0                                            -> session-level RO does block DDL
(l) got = 1 ; lock_still_held = 1                                           -> GET_LOCK survives ROLLBACK
(m) row after LOCK TABLES in RO txn = 1                                     -> LOCK TABLES implicitly commits; following INSERT persisted
(n) ERROR 1792                                                              -> SELECT … FOR UPDATE blocked in RO txn
```

<a id="mysql-comments"></a>
#### mysql-comments — executable comments and optimizer hints — Confirmed

```sql
SELECT 1 /*!50000 , 2 */ ;                                  -- 2 columns
SELECT 1 /*!99999 , SLEEP(5) */ AS gated;                   -- 1 column, returns immediately
SELECT 1 /*!80000 , 'ver8-gate-ran' */ ;                    -- 2 columns
/*!50000 DROP TABLE IF EXISTS ddl_in_ro */;                 -- whole statement inside a comment: table dropped
SELECT 1 /*M! , 'mariadb-only' */ AS m_comment;             -- MySQL ignores /*M! */ (1 column)
SELECT /*+ MAX_EXECUTION_TIME(500) */ SLEEP(2) AS slept;    -- slept = 1 (interrupted)
SELECT /*+ SET_VAR(sql_mode='NO_BACKSLASH_ESCAPES') */ 'a\\b' AS v;   -- v = a\b  (lexing unchanged)
START TRANSACTION READ ONLY; INSERT /*+ SET_VAR(transaction_read_only=0) */ INTO t VALUES (9,'hint'); ROLLBACK;  -- ERROR 1792
```

<a id="mysql-super-ro"></a>
#### mysql-super-ro — super_read_only — Confirmed

```sql
SET GLOBAL super_read_only = ON; SELECT @@global.read_only, @@global.super_read_only;   -- 1 | 1
INSERT INTO t VALUES (10,'sro');      -- ERROR 1290: running with the --super-read-only option
CREATE TEMPORARY TABLE tmp_sro(a int); INSERT INTO tmp_sro VALUES (1);   -- allowed (1 row)
SELECT GET_LOCK('sro',0);             -- 1
SET GLOBAL super_read_only = OFF; INSERT INTO t VALUES (10,'sro-off');   -- root (SUPER) re-enables writes
SELECT @@global.read_only, @@global.super_read_only;   -- 1 | 0   (read_only stays ON; non-SUPER users still blocked: ERROR 1290 --read-only)
```

### 3.3 SQLite

Run: `docker run --rm -i --entrypoint sh keinos/sqlite3:latest < lite.sh` (SQLite 3.54.1).

<a id="sqlite-ro"></a>
#### sqlite-ro — read-only connections vs VACUUM INTO / ATTACH / PRAGMA — Confirmed

```sh
cd /tmp; sqlite3 a.db "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); INSERT INTO t VALUES (1,'a');"
sqlite3 'file:a.db?mode=ro' "INSERT INTO t VALUES (2,'b');"
sqlite3 'file:a.db?mode=ro' "VACUUM INTO '/tmp/copy_ro.db';"; ls -la /tmp/copy_ro.db
sqlite3 -readonly a.db "VACUUM INTO '/tmp/copy_ro2.db';"; ls -la /tmp/copy_ro2.db
sqlite3 a.db "PRAGMA query_only=1; VACUUM INTO '/tmp/copy_qo.db';"; ls -la /tmp/copy_qo.db
sqlite3 'file:a.db?mode=ro' "ATTACH '/tmp/new_att.db' AS x; CREATE TABLE x.z(a); INSERT INTO x.z VALUES (1); SELECT count(*) FROM x.z;"
sqlite3 'file:a.db?mode=ro' "ATTACH 'file:/tmp/new_att2.db?mode=rwc' AS y; CREATE TABLE y.z(a);"
sqlite3 a.db "PRAGMA query_only=1; ATTACH '/tmp/new_att3.db' AS q; CREATE TABLE q.z(a);"; ls -la /tmp/new_att3.db
sqlite3 'file:a.db?mode=ro' "PRAGMA user_version=7;"
sqlite3 'file:a.db?mode=ro' "PRAGMA journal_mode=WAL;"
sqlite3 'file:a.db?mode=ro' "PRAGMA query_only=0; INSERT INTO t VALUES (3,'c');"
sqlite3 'file:a.db?mode=ro' "PRAGMA writable_schema=1; SELECT 'writable_schema set';"
sqlite3 a.db "PRAGMA query_only=1; PRAGMA query_only=0; INSERT INTO t VALUES (4,'d'); SELECT count(*) FROM t WHERE id=4;"
sqlite3 'file:a.db?mode=ro' "CREATE TEMP TABLE tt(a); INSERT INTO tt VALUES (1); SELECT count(*) FROM tt;"
```

```
attempt to write a readonly database                      (INSERT on mode=ro)
-rw-r--r-- 8192 /tmp/copy_ro.db                            (VACUUM INTO on mode=ro: full copy written)
-rw-r--r-- 8192 /tmp/copy_ro2.db                           (VACUUM INTO on -readonly)
attempt to write a readonly database ; -rw-r--r-- 0 /tmp/copy_qo.db     (query_only blocks, empty file left)
attached write ok|1 ; 8192 /tmp/new_att.db                 (ATTACH + write from a mode=ro main connection)
uri attach rwc ok
attempt to write a readonly database ; 0 /tmp/new_att3.db  (query_only blocks ATTACH-write, empty file left)
attempt to write a readonly database  x3                   (user_version, journal_mode, query_only=0 + INSERT on mode=ro)
writable_schema set                                        (flag accepted; file still read-only)
query_only escaped|1                                       (query_only=1 can be reset by the session)
temp write ok on ro|1
```

<a id="sqlite-schema-version"></a>
#### sqlite-schema-version — PRAGMA schema_version — Confirmed

```
baseline: 1 | after INSERT: 1 | after CREATE INDEX: 2 | after ADD COLUMN: 3
after CREATE TEMP TABLE (main, temp): 3 1 | after CREATE VIEW: 4 | after DROP VIEW: 5
after rolled-back CREATE: 5 | after VACUUM: 6 | data_version (per-connection): 1
```

<a id="sqlite-case"></a>
#### sqlite-case — identifiers / LIKE / upper / lower are ASCII-only — Confirmed

```sh
sqlite3 b.db "CREATE TABLE Müşteri(a); CREATE TABLE items(a);"
sqlite3 b.db "SELECT count(*) FROM müşteri; SELECT count(*) FROM ITEMS; SELECT name FROM sqlite_schema;"   # 0 / 0 / Müşteri, items
sqlite3 a.db "SELECT count(*) FROM MÜŞTERİ;"     # no such table (Ü≠ü, İ≠i)
sqlite3 a.db "SELECT upper('iı'), lower('İI'), 'I' LIKE 'i', 'İ' LIKE 'i', 'Ş' LIKE 'ş';"   # Iı | İi | 1 | 0 | 0
```

### 3.4 Redis ACL

<a id="redis-noauth"></a>
#### redis-noauth — Confirmed

```
docker run --rm -d --name hacivat-verify-redis redis:7
redis-cli CONFIG GET protected-mode   -> no
redis-cli ACL LIST                    -> user default on nopass sanitize-payload ~* &* +@all
docker run --rm redis:7 redis-cli -h <container-ip> PING   -> PONG   (other container, no password)
```

<a id="redis-acl"></a>
#### redis-acl — read-only ACL users and what escapes — Confirmed + new finding

```sh
ACL SETUSER hv_r on '>pw1' resetkeys '~*' resetchannels -@all +@read +@connection -@dangerous
ACL SETUSER ro2  on '>pw2' '~*' +@read -@write
ACL SETUSER ro3  on '>pw3' '~*' '&*' +@read -@write +@scripting
FUNCTION LOAD "#!lua name=lib
redis.register_function('wr', function(keys, args) return redis.call('SET', keys[1], 'from-fn') end)"
# then, per user: redis-cli --user U --pass P <command>
```

| Command | hv_r (`-@all +@read +@connection -@dangerous`) | ro2 (`+@read -@write`) | ro3 (`+@read -@write +@scripting`) |
|---|---|---|---|
| `GET k` | v | v | v |
| `SET k x` | NOPERM | NOPERM | NOPERM |
| `KEYS *` | NOPERM | **allowed** | **allowed** |
| `SORT_RO lst` | NOPERM | allowed | allowed |
| `EVAL "return redis.call('SET','k','from-eval')" 1 k` | NOPERM (eval) | NOPERM (eval) | `ERR ACL failure in script: … no permissions to run the 'set' command` |
| `FCALL wr 1 k` | NOPERM | NOPERM | `ERR ACL failure in script` |
| `EVAL_RO "return redis.call('GET','k')" 1 k` | NOPERM | NOPERM | v |
| `FUNCTION FLUSH` | NOPERM | NOPERM | **OK** (library deleted) |
| `SCRIPT FLUSH` | NOPERM | NOPERM | **OK** |
| `CLIENT LIST` / `CLIENT KILL` / `CLIENT PAUSE` | NOPERM | NOPERM | NOPERM |
| `CONFIG SET maxmemory 1` | NOPERM | NOPERM | NOPERM |
| `PUBLISH ch m` | NOPERM | NOPERM | NOPERM |
| `SELECT 1` | OK | NOPERM | NOPERM |
| `SORT lst STORE dst` / `FLUSHALL` / `SHUTDOWN` | NOPERM | NOPERM | NOPERM |
| `DEBUG SLEEP 0` | ERR DEBUG command not allowed (server option) | same | same |

Category intersections on 7.4.12: `@read ∩ @dangerous = keys sort_ro`; `@scripting ∩ @write = function|delete function|flush function|load function|restore`; `@connection ∩ @dangerous = client|kill client|list client|no-evict client|pause client|unblock client|unpause`. Rules are applied left-to-right, so `+@scripting` after `-@write` re-grants the four FUNCTION write subcommands; `SCRIPT FLUSH` is `@scripting` but not `@write`.

### 3.5 MongoDB

<a id="mongo-localhost"></a>
#### mongo-localhost — no-auth default and localhost exception — Confirmed

```
docker run --rm -d --name hacivat-verify-mongo-noauth mongo:7
docker run --rm -d --name hacivat-verify-mongo-bare   mongo:7 --auth
docker run --rm -d --name hacivat-verify-mongo-auth   -e MONGO_INITDB_ROOT_USERNAME=admin -e MONGO_INITDB_ROOT_PASSWORD=mongoPW mongo:7
# no-auth image, from another container, no credentials:
listDatabases -> ["admin","config","local"]
# --auth with zero users, from ANOTHER container:
listDatabases: Command listDatabases requires authentication
createUser:    Command createUser requires authentication
# --auth with zero users, from 127.0.0.1 inside the container:
db.createUser({user:"first",…,roles:["root"]}) -> OK (localhost exception)
second createUser -> Command createUser requires authentication   (exception closed)
# image with MONGO_INITDB_ROOT_* (users exist), localhost without credentials:
createUser: Command createUser requires authentication
```

<a id="mongo-read"></a>
#### mongo-read — `read` role vs `$out`/`$merge` — Partially

```js
db.getSiblingDB("admin").createUser({user:"hv_ab12cd34_r", pwd:"R4ndomPW", roles:[{role:"read", db:"app"}],
  customData:{hv:1,sid:"s1"}, mechanisms:["SCRAM-SHA-256"]});
db.getSiblingDB("admin").createUser({user:"hv_mixed", pwd:"MixPW",
  roles:[{role:"read", db:"app"},{role:"readWrite", db:"scratch"}]});
// as each user:
app.orders.aggregate([{$out:"copy"}]); app.orders.aggregate([{$merge:{into:"copy2"}}]);
app.orders.aggregate([{$out:{db:"scratch",coll:"exfil"}}]); app.orders.aggregate([{$merge:{into:{db:"scratch",coll:"exfil2"}}}]);
app.orders.find({$where:"this.t > 5"}); app.orders.aggregate([{$addFields:{x:{$function:{body:"function(){return 1}",args:[],lang:"js"}}}}]);
```

```
hv_ab12cd34_r: insertOne / $out / $merge (same or other db) / createIndex / killOp -> Unauthorized
hv_ab12cd34_r: find OK, $where OK, $function OK
hv_mixed:      $out same db -> Unauthorized ; $out {db:"scratch"} -> OK ; $merge into scratch -> OK
admin view:    scratch collections = ["exfil2","exfil"]
```

### 3.6 PostgreSQL catalog `xmin` for incremental introspection

<a id="pg-xmin"></a>
#### pg-xmin — which DDL changes which catalog row's xmin — Confirmed + extended

Script (`xmin.sql`): creates `xs.x(a int, b int)`, `xs.fn()`, a trigger function, then after each DDL snapshots `xmin` of the table's `pg_class` row, `pg_attribute` row for `a`, max `pg_attribute.xmin`, max `pg_description.xmin`, `pg_proc` row of `xs.fn`, `pg_type` row of the table. Core loop:

```sql
CREATE INDEX xi1 ON xs.x(a); CREATE INDEX xi2 ON xs.x(b); ALTER TABLE xs.x ADD COLUMN c int;
COMMENT ON TABLE xs.x IS 't'; COMMENT ON COLUMN xs.x.a IS 'ca'; GRANT SELECT ON xs.x TO PUBLIC;
GRANT SELECT (a) ON xs.x TO PUBLIC;
CREATE TRIGGER tr1 BEFORE INSERT ON xs.x FOR EACH ROW EXECUTE FUNCTION xs.trgf();
CREATE TRIGGER tr2 BEFORE UPDATE ON xs.x FOR EACH ROW EXECUTE FUNCTION xs.trgf();
ALTER FUNCTION xs.fn() STABLE; CREATE OR REPLACE FUNCTION xs.fn() RETURNS int LANGUAGE sql AS 'SELECT 2';
ALTER TABLE xs.x ALTER COLUMN a SET DEFAULT 5; ALTER TABLE xs.x ADD CONSTRAINT ck CHECK (a > 0);
DROP INDEX xs.xi2; DROP TRIGGER tr2 ON xs.x; ALTER TABLE xs.x DROP COLUMN c;
INSERT INTO xs.x VALUES (1,1); VACUUM ANALYZE xs.x;
```

PG 16.15 output (PG 18.6 identical in shape, XIDs shifted by +22):

```
 step                          | pg_class | att_a | max_attr | max_descr | pg_proc | pg_type | natts
 0 baseline                    |   812    |  812  |   812    |           |   813   |   812   |  2
 1 CREATE INDEX (first)        |   812    |  812  |   812    |           |   813   |   812   |  2
 2 CREATE INDEX (second)       |   812    |  812  |   812    |           |   813   |   812   |  2
 3 ADD COLUMN                  |   822    |  812  |   822    |           |   813   |   812   |  3
 4 COMMENT ON TABLE            |   822    |  812  |   822    |    824    |   813   |   812   |  3
 5 COMMENT ON COLUMN           |   822    |  812  |   822    |    826    |   813   |   812   |  3
 6 GRANT table                 |   828    |  812  |   822    |    826    |   813   |   812   |  3
 7 GRANT column                |   828    |  830  |   830    |    826    |   813   |   812   |  3
 8 CREATE TRIGGER (first)      |   832    |  830  |   830    |    826    |   813   |   812   |  3
 9 CREATE TRIGGER (second)     |   832    |  830  |   830    |    826    |   813   |   812   |  3
 10 ALTER FUNCTION             |   832    |  830  |   830    |    826    |   836   |   812   |  3
 11 CREATE OR REPLACE FUNCTION |   832    |  830  |   830    |    826    |   838   |   812   |  3
 12 SET DEFAULT                |   832    |  840  |   840    |    826    |   838   |   812   |  3
 13 ADD CHECK                  |   842    |  840  |   840    |    826    |   838   |   812   |  3
 14 DROP INDEX                 |   842    |  840  |   840    |    826    |   838   |   812   |  3
 15 DROP TRIGGER (one of two)  |   842    |  840  |   840    |    826    |   838   |   812   |  3
 16 DROP COLUMN                |   842    |  840  |   848    |    826    |   838   |   812   |  3
 18 INSERT data                |   842    |  840  |   848    |    826    |   838   |   812   |  3
 19 VACUUM ANALYZE             |   842    |  840  |   848    |    826    |   838   |   812   |  3
 index xi1: own pg_class xmin 818 = pg_index xmin 818 ; trigger tr1: pg_trigger xmin 832
```

Reading: table `pg_class.xmin` moves on ADD COLUMN, table GRANT (relacl), the **first** trigger (relhastriggers) and CHECK (relchecks) — not on CREATE INDEX (any), a second trigger, SET DEFAULT, column GRANT, COMMENT, DROP INDEX/TRIGGER. Column-level changes (column GRANT, SET DEFAULT, DROP COLUMN = `attisdropped`) move `pg_attribute.xmin`. Functions: `pg_proc.xmin` moves on ALTER FUNCTION and CREATE OR REPLACE. Removed objects (DROP INDEX/TRIGGER) leave no trace → existence queries required. Therefore triggers, indexes, constraints and defaults must be tracked by their own catalog rows' xmin (`pg_trigger`, `pg_index`/index `pg_class`, `pg_constraint`, `pg_attrdef`).

<a id="pg-xmin-freeze"></a>
#### pg-xmin-freeze — freezing and wraparound — Refuted (doc corrected)

```sql
SELECT xmin, age(xmin) FROM pg_class WHERE oid='xs.x'::regclass;   -- 842 | 15
VACUUM (FREEZE) pg_class;
SELECT xmin, age(xmin) FROM pg_class WHERE oid='xs.x'::regclass;   -- 842 | 15   (raw xmin kept; relfrozenxid age 0)
```

Wraparound simulation on `postgres:17` (throw-away cluster; `wrap.sh`):

```sh
initdb -D /tmp/d; pg_ctl -D /tmp/d -o "-p 5555 -k /tmp -c listen_addresses=" -w start
psql -c "CREATE TABLE w(a int)"; for db in postgres template1; do psql -d $db -c "VACUUM FREEZE"; done
pg_ctl -D /tmp/d -w stop
X=$(( 2147483648 + 2000 )); SEG=$(printf "%04X" $(( X / 1048576 )))
dd if=/dev/zero of=/tmp/d/pg_xact/$SEG bs=262144 count=1; pg_resetwal -x $X -D /tmp/d; pg_ctl … start
```

```
before jump                    | xmin 738 | age 1
bootstrap-xmin row (int4pl)    | xmin 1   | age 2147483647
after jump of 2^31+2000 xids   | xmin 738 | age -2147482386 | age(xmin) <= 100000 → t
pg_class rows passing age(xmin) <= 100000: 220 of 416
SELECT txid_current() -> ERROR … HINT: Execute a database-wide VACUUM in that database.   (wraparound protection; reads still work)
```

Conclusion: false negatives do not occur (a row changed after the watermark always has a small non-negative age), but in long-lived clusters ancient frozen rows produce **negative** ages and are re-read on every incremental pass. Filter with `age(xmin) BETWEEN 0 AND :txage` (or treat a negative age as "unchanged") and fall back to full introspection if the stored watermark is older than 2^31 XIDs.

### 3.7 MySQL `information_schema.TABLES` timestamps

<a id="mysql-is-times"></a>
#### mysql-is-times — CREATE_TIME / UPDATE_TIME — Refuted / Confirmed

```sh
q() { mysql -uroot -prootPW -N app -e "$1"; }
show() { echo "$1 | default: $(q "SELECT CONCAT_WS(' / ',CREATE_TIME,IFNULL(UPDATE_TIME,'NULL'),TABLE_COMMENT) FROM information_schema.TABLES WHERE TABLE_SCHEMA='app' AND TABLE_NAME='tt'") | expiry=0: $(q "SET SESSION information_schema_stats_expiry=0; SELECT …same…")"; }
q "CREATE TABLE tt(id int primary key, a int)"; show CREATE; sleep 2
q "INSERT INTO tt VALUES (1,1)"; show INSERT; sleep 2
q "ALTER TABLE tt ADD COLUMN b int"; … ADD INDEX ia(a); … COMMENT='c1'; … MODIFY a bigint; … RENAME COLUMN b TO b2;
q "CREATE TRIGGER trg BEFORE INSERT ON tt FOR EACH ROW SET NEW.a=NEW.a"; … ADD COLUMN c int, ALGORITHM=COPY
```

8.0.46 (8.4.13 identical pattern):

```
after CREATE          | 15:08:31 / NULL       | expiry=0: 15:08:31 / NULL
after INSERT          | 15:08:31 / NULL       | expiry=0: 15:08:31 / 15:08:33
ADD COLUMN (INSTANT)  | 15:08:35 / NULL       | expiry=0: 15:08:35 / NULL
ADD INDEX (INPLACE)   | 15:08:37 / NULL       | expiry=0: 15:08:37 / NULL
COMMENT change        | 15:08:39 / NULL / c1
MODIFY type (COPY)    | 15:08:41 / NULL / c1
RENAME COLUMN         | 15:08:43 / NULL / c1
CREATE TRIGGER        | 15:08:43 / NULL / c1     <- unchanged
ADD COLUMN ALG=COPY   | 15:08:47 / NULL / c1
defaults: information_schema_stats_expiry = 86400
```

CREATE_TIME is a usable per-table "last DDL" hint on 8.0/8.4 (second granularity; triggers need `information_schema.TRIGGERS.CREATED`, routines `ROUTINES.LAST_ALTERED`). UPDATE_TIME is a cached statistic (NULL unless `information_schema_stats_expiry=0`) and resets after ALTER — unusable for DDL detection.

### 3.8 Zero-credential discovery facts (official images)

<a id="docker-env"></a>
#### docker-env — bootstrap secrets in `Config.Env` and process env — Confirmed + nuance

```
docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' hacivat-verify-<c> | grep PASSWORD
  pg16/pg18:  POSTGRES_PASSWORD=s3cretPW
  mysql84:    MYSQL_ROOT_PASSWORD=rootPW
  mongo-auth: MONGO_INITDB_ROOT_USERNAME=admin / MONGO_INITDB_ROOT_PASSWORD=mongoPW
  ch-pw:      CLICKHOUSE_PASSWORD=chPW
PID 1 environ (read as the process owner: docker exec -u <user> … /proc/1/environ):
  postgres (PID 1 = postgres):   no POSTGRES_* left (PG_MAJOR, PG_VERSION, PGDATA only)
  mongod:                        no MONGO_INITDB_* left
  mysqld:                        MYSQL_ROOT_PASSWORD=rootPW still present
  clickhouse-server:             CLICKHOUSE_PASSWORD=chPW still present
docker exec hacivat-verify-pg18 env | grep POSTGRES_PASSWORD -> POSTGRES_PASSWORD=s3cretPW   (every exec'd process gets Config.Env)
```

Reading `/proc/1/environ` as container root fails ("Permission denied") because Docker drops `CAP_SYS_PTRACE`; it succeeds as the owning user.

<a id="local-auth"></a>
#### local-auth — PG trust / MySQL socket — Confirmed

```
docker exec hacivat-verify-pg18 psql -U postgres -Atc "select version()"   -> works, no password
$PGDATA/pg_hba.conf (non-comment lines):
  local all all trust ; host all all 127.0.0.1/32 trust ; host all all ::1/128 trust
  local replication all trust ; host replication all 127.0.0.1/32 trust ; host replication all ::1/128 trust
  host all all all scram-sha-256
CREATE ROLE hv_expired LOGIN PASSWORD 'x' VALID UNTIL '2000-01-01';
psql -U hv_expired (socket)        -> logs in (trust ignores VALID UNTIL)
psql -h 127.0.0.1 -U hv_expired    -> logs in
ALTER ROLE hv_expired NOLOGIN; psql -U hv_expired -> FATAL: role "hv_expired" is not permitted to log in
docker exec hacivat-verify-mysql80 mysql -uroot -e "select 1"
  -> ERROR 1045 (28000): Access denied for user 'root'@'localhost' (using password: NO)
mysql.user root rows: root@% and root@localhost, both caching_sha2_password (8.0.46 and 8.4.13)
```

<a id="ch-default"></a>
#### ch-default — ClickHouse default user — Confirmed + nuances

```
hacivat-verify-ch (no env), users.d/default-user.xml: <networks><ip>::1</ip><ip>127.0.0.1</ip></networks>
  clickhouse-client --host 127.0.0.1        -> default
  clickhouse-client --host <own eth0 IP>    -> default        (own host address also accepted)
  from another container                   -> Code: 516 … default: Authentication failed
hacivat-verify-ch-pw (CLICKHOUSE_PASSWORD=chPW), users.d/default-user.xml: <default remove="remove"/> then default with <ip>::/0</ip>, password, access_management 0
  from another container, no password      -> Code: 516
  from another container, --password chPW  -> default
  docker exec … clickhouse-client (no --password) -> default   (client reads CLICKHOUSE_PASSWORD from inherited env)
  env -u CLICKHOUSE_PASSWORD clickhouse-client     -> Code: 516
```

### 3.9 Ephemeral users and `SET ROLE`

<a id="pg-ephemeral"></a>
#### pg-ephemeral — PG template — Confirmed (16/17/18)

Setup (`eph-setup.sql`): `app_owner` owns schema `app` with `app.orders`; `CREATE ROLE karagoz_admin LOGIN CREATEROLE …; GRANT pg_read_all_data, pg_write_all_data TO karagoz_admin WITH ADMIN OPTION;` then, as `karagoz_admin`, the doc's one-time setup and per-session statements verbatim (with `GRANT hacivat_read TO "hv_ab12cd34_r" WITH INHERIT TRUE, SET FALSE`). Then log in over TCP to the container's eth0 IP (forces the `scram-sha-256` hba line):

```
ALTER ROLE app_owner CONNECTION LIMIT 5 (as karagoz_admin) -> ERROR: permission denied to alter role   (PG16+ CREATEROLE scoping)
GRANT USAGE ON SCHEMA app / GRANT SELECT ON ALL TABLES IN SCHEMA app (as non-owner admin)
  -> WARNING: no privileges were granted for "app" / "orders"   (warning only, not an error)
wrong password -> FATAL: password authentication failed for user "hv_ab12cd34_r"
hv_ab12cd34_r | via_tcp t | default_transaction_read_only on ; orders_visible 2
INSERT -> ERROR: cannot execute INSERT in a read-only transaction
SET default_transaction_read_only = off; INSERT -> ERROR: permission denied for table orders
nextval('app.orders_id_seq') -> ERROR: permission denied for sequence orders_id_seq
SET ROLE hacivat_read -> ERROR: permission denied to set role "hacivat_read"     (SET FALSE works)
CREATE TEMP TABLE tmp_x(a int) -> CREATE TABLE                                     (TEMP privilege via PUBLIC)
CREATE TABLE public.x_hv(a int) -> ERROR: permission denied for schema public
-- expiry/deactivate/delete, as karagoz_admin:
ALTER ROLE hv_ab12cd34_r VALID UNTIL '2000-01-01'
  new TCP login -> FATAL: password authentication failed ; already-open session keeps querying (count = 2)
ALTER ROLE … NOLOGIN; REVOKE hacivat_read FROM …; SELECT pg_terminate_backend(…); DROP ROLE hv_ab12cd34_r -> DROP ROLE ; remaining hv_% = 0
```

<a id="mysql-ephemeral"></a>
#### mysql-ephemeral — MySQL template — Partially (template bug)

```
CREATE ROLE IF NOT EXISTS 'hacivat_read', 'hacivat_write'; … GRANT … ON `app`.* TO 'hacivat_ddl';
  -> ERROR 1410 (42000): You are not allowed to create a user with GRANT       (hacivat_ddl never created)
CREATE USER 'hv_ab12cd34_r'@'%' IDENTIFIED BY … WITH MAX_USER_CONNECTIONS 4 PASSWORD EXPIRE NEVER ATTRIBUTE '{…}'  -> OK
information_schema.USER_ATTRIBUTES -> {"hv": 1, "exp": "2026-11-12", "sid": "s1"}
login over TCP: CURRENT_ROLE() = `hacivat_read`@`%` ; SELECT ok (4 rows)
INSERT -> ERROR 1142: INSERT command denied ; CREATE TEMPORARY TABLE -> ERROR 1044 (no CREATE TEMPORARY TABLES)
SET ROLE NONE; SELECT … -> ERROR 1142: SELECT command denied   (privileges only via the role)
ALTER USER … ACCOUNT LOCK; new login -> ERROR 3118: Account is locked.
DROP USER 'hv_ab12cd34_r'@'%' -> OK
```

(Note: the session test initially hit `ERROR 1290 … --read-only` because the earlier `super_read_only` test leaves `read_only=ON` after `SET GLOBAL super_read_only=OFF`; rerun after `SET GLOBAL read_only=OFF`.)

<a id="set-role"></a>
#### set-role — `SET ROLE` does not contain the user — Confirmed (PG)

```sql
-- as superuser postgres:
SET ROLE ro; SELECT current_user; INSERT INTO t VALUES (8,'x'); RESET ROLE; SELECT current_user;
SET SESSION AUTHORIZATION ro; SELECT current_user, session_user; RESET SESSION AUTHORIZATION; SELECT current_user, session_user;
```

```
ro ; ERROR: permission denied for table t ; RESET ; postgres
ro | ro ; RESET ; postgres | postgres
```

Also observed: `SET ROLE` is allowed inside a `READ ONLY` transaction (§3.1). MySQL `SET ROLE` and SQL Server `EXECUTE AS`/`REVERT` were not run.

### 3.10 Turkish / collation pitfalls

<a id="pg-turkish"></a>
#### pg-turkish — PostgreSQL ICU `tr-TR` — Confirmed + new finding

The official Debian images ship only `C`, `C.utf8`, `en_US.utf8`, `POSIX` (`locale -a`), so a libc `tr_TR.UTF-8` database cannot be created without building a custom image; ICU was used:

```sql
CREATE DATABASE trdb TEMPLATE template0 ENCODING 'UTF8' LOCALE_PROVIDER icu ICU_LOCALE 'tr-TR' LOCALE 'C.UTF-8';
\c trdb
SELECT lower('ITEMS'), upper('items'), lower('I'), upper('i'), lower('İ'), upper('ı');
SELECT 'ITEMS' ILIKE 'items', 'İTEMS' ILIKE 'items';
SELECT lower('ITEMS' COLLATE "C"), lower('ITEMS' COLLATE "und-x-icu"), lower('ITEMS' COLLATE "tr-x-icu");
CREATE TABLE ITEMS(ID int); CREATE TABLE MÜŞTERİ(ID int);
SELECT relname, length(relname) FROM pg_class WHERE relname IN ('items','ıtems','mÜŞterİ','müşteri');
SELECT count(*) FROM Items; SELECT count(*) FROM müşteri;
SELECT relname FROM pg_class WHERE lower(relname) = lower('ITEMS');
SELECT table_name FROM information_schema.tables WHERE lower(table_name) = lower('ITEMS');
SELECT pg_collation_for(relname), pg_collation_for('x'::text) FROM pg_class LIMIT 1;
SELECT count(*) FROM pg_class WHERE relname ILIKE 'ITEMS';  SELECT count(*) FROM pg_class WHERE relname ILIKE 'İTEMS';
-- 15-word list ordered with the db default, "tr-x-icu", "und-x-icu", "C"
```

```
lower_items ıtems | upper_items İTEMS | ı | İ | i | I
ilike_ascii f | ilike_dotted t                                  <- 'ITEMS' ILIKE 'items' is FALSE in a tr database
C: items | und: items | tr: ıtems
relname items (5) ; mÜŞterİ (7)                                 <- ASCII-only folding of unquoted identifiers
SELECT … FROM Items -> 0 rows (works) ; FROM müşteri -> ERROR: relation "müşteri" does not exist
lower(relname) = lower('ITEMS') -> 0 rows ; information_schema variant -> 0 rows   <- catalog-query pitfall reproduced
pg_collation_for(relname) "C" ; text "default" ; relname ILIKE 'ITEMS' -> 1 ; relname ILIKE 'İTEMS' -> 0
db default (tr-TR) | cam çay gül ğ ılık Irmak istek İzmir oda ördek sabun şeker uçak ünlü zil
tr-x-icu           | cam çay gül ğ ılık Irmak istek İzmir oda ördek sabun şeker uçak ünlü zil
und-x-icu          | cam çay ğ gül Irmak istek İzmir ılık oda ördek sabun şeker uçak ünlü zil
C                  | Irmak cam gül istek oda sabun uçak zil çay ördek ünlü ğ İzmir ılık şeker
```

Identical on 16.15, 17.11, 18.6.

<a id="mysql-turkish"></a>
#### mysql-turkish — `utf8mb4_tr_0900_ai_ci` — Confirmed + corrections

```sql
SELECT 'I'='ı' COLLATE utf8mb4_tr_0900_ai_ci, 'i'='İ' COLLATE utf8mb4_tr_0900_ai_ci, 'I'='i' COLLATE utf8mb4_tr_0900_ai_ci,
       'ş'='s' COLLATE utf8mb4_tr_0900_ai_ci, 'é'='e' COLLATE utf8mb4_tr_0900_ai_ci, 'I'='i' COLLATE utf8mb4_0900_ai_ci, 'ı'='i' COLLATE utf8mb4_0900_ai_ci;
SELECT LOWER('ITEMS' COLLATE utf8mb4_tr_0900_ai_ci), UPPER('items' COLLATE utf8mb4_tr_0900_ai_ci), UPPER('ı' COLLATE utf8mb4_tr_0900_ai_ci), LOWER('İ' COLLATE utf8mb4_tr_0900_ai_ci);
SELECT 'ITEMS' LIKE 'items' COLLATE utf8mb4_tr_0900_ai_ci, 'İTEMS' LIKE 'items' COLLATE utf8mb4_tr_0900_ai_ci;
CREATE DATABASE trdb CHARACTER SET utf8mb4 COLLATE utf8mb4_tr_0900_ai_ci; … word list ORDER BY (tr / 0900 / bin)
CREATE TABLE Items(ID int, Müşteri int); SELECT … FROM items; SELECT … WHERE ıd / İD / MÜŞTERİ / MUSTERI IS NULL;
SELECT COUNT(*) FROM information_schema.tables WHERE TABLE_SCHEMA='trdb' AND TABLE_NAME='ITEMS';
SELECT COUNT(*) FROM information_schema.tables WHERE TABLE_SCHEMA='trdb' AND LOWER(TABLE_NAME)=LOWER('ITEMS');
```

```
I=ı 1 | i=İ 1 | I=i 0 | ş=s 0 | é=e 1 | I=i (root) 1 | ı=i (root) 0
LOWER tr: items | UPPER tr: ITEMS | UPPER(ı): I | LOWER(İ): i      <- case mapping NOT Turkish-tailored
LIKE: 'ITEMS' LIKE 'items' 0 | 'İTEMS' LIKE 'items' 1
tr_0900_ai_ci: cam çay gül ğ ılık Irmak istek İzmir oda ördek sabun şeker uçak ünlü zil
0900_ai_ci:    cam çay ğ gül Irmak istek İzmir ılık oda ördek sabun şeker uçak ünlü zil
bin:           Irmak cam gül istek oda sabun uçak zil çay ördek ünlü ğ İzmir ılık şeker
FROM items (table is Items, lower_case_table_names=0) -> ERROR 1146 Table 'trdb.items' doesn't exist
columns: İD -> matches ID ; MÜŞTERİ -> matches Müşteri ; ıd -> ERROR 1054 ; MUSTERI -> ERROR 1054
I_S TABLE_NAME = 'ITEMS' -> 0 (COLLATION(TABLE_NAME) = utf8mb3_bin) ; LOWER(TABLE_NAME)=LOWER('ITEMS') -> 1
```

<a id="mssql-turkish"></a>
#### mssql-turkish — SQL Server `Turkish_CI_AS` — Confirmed (amd64 emulation)

```
docker run --rm -d --platform linux/amd64 --name hacivat-verify-mssql -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD=… mcr.microsoft.com/mssql/server:2022-latest
docker run … --name hacivat-verify-mssql-tr … -e MSSQL_COLLATION=Turkish_CI_AS …   (instance-level Turkish)
/opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P … -C < script.sql
```

Database `trdb COLLATE Turkish_CI_AS` on a default-collation instance (`SQL_Latin1_General_CP1_CI_AS`):

```
INFORMATION_SCHEMA.TABLES -> 0 rows (works)
information_schema.tables -> Msg 208 Invalid object name 'information_schema.tables'.
Information_Schema.Tables -> Msg 208
sys.objects -> 111 ; SYS.OBJECTS -> 111 (no 'i' in the name) ; sys.indexes -> 187 ; SYS.INDEXES -> Msg 208
table Items queried as items -> Msg 208 ; as İTEMS -> Msg 208
table items queried as İTEMS -> 0 rows (works) ; as ITEMS -> Msg 208
column ID queried as id -> Msg 207 Invalid column name 'id'.
DECLARE @ID …; SELECT @id -> 1 (works: variables follow the instance collation)
CREATE TABLE #Tmp …; SELECT FROM #tmp -> works (tempdb = instance collation)
LOWER(N'ITEMS') -> ıtems ; UPPER(N'items') -> İTEMS
csdb COLLATE Latin1_General_CS_AS: SYS.OBJECTS -> Msg 208 ; information_schema.tables -> Msg 208 ; INFORMATION_SCHEMA.TABLES -> works
```

Instance collation `Turkish_CI_AS` (`MSSQL_COLLATION`):

```
DECLARE @ID int = 1; SELECT @id   -> Msg 137 Must declare the scalar variable "@id".
DECLARE @id int = 1; SELECT @İD   -> 1
CREATE TABLE #Items …; SELECT FROM #items -> Msg 208 Invalid object name '#items'.
master: information_schema.tables -> Msg 208
```

## 4. Cleanup

All containers started for this report (`hacivat-verify-pg16`, `-pg17`, `-pg18`, `-pgwrap`, `-mysql80`, `-mysql84`, `-redis`, `-mongo-noauth`, `-mongo-auth`, `-mongo-bare`, `-ch`, `-ch-pw`, `-mssql`, `-mssql-tr`, plus the one-shot `-sqlite`, `-sqlite2`, `-rediscli`) were removed with `docker rm -f`; `docker ps -a --filter name=hacivat-verify-` returns nothing. Pulled images were left in the local cache.
