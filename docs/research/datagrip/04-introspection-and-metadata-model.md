# DataGrip 2024.3.5 — introspection and the metadata model

> Covers: introspection triggers and task types, native vs JDBC introspectors, the catalog-query-pack format and its meta-directives, incremental strategies per DBMS (PostgreSQL xmin age, SQL Server modify_date + checksums, Oracle obj$.mtime / last_ddl_time + hashes, MySQL/MariaDB full-per-schema + fragments, SQLite, ClickHouse, Redis, MongoDB, others), existence diffs for drops, fragment introspection, refresh after DDL, Oracle levels L1–L3, source loading and storage, scope patterns and object filters, pre-introspected system catalogs (minicat), model external data (virtual objects), the object-kind hierarchy, per-kind properties, references, persistence format and versioning.
> Explorer/tree UX, DDL generation and schema diff are covered by the other doc set. Conventions: `00-method-and-legal.md`.

---

## 1. Scope and sources

| Source | Version | Licence |
|---|---|---|
| `com/intellij/database/dialects/*/introspector/*IntroQueries.sql`: `PgIntroQueries.sql` (1,360 lines), `RsIntroQueries.sql` (776), `GPlumIntroQueries.sql` (1,240), `CRoachIntroQueries.sql` (576), `MsIntroQueries.sql` (935), `OraIntroQueries.sql` (2,160), `AseIntroQueries.sql` (291), `CassIntroQueries.sql` (170) | DataGrip 2024.3.5 | Proprietary — **described, not copied** |
| Class-name inventory: `introspection/*`, `dialects/*/introspector/*IntroQueries$*` (query-holder classes for MySQL, SQLite, ClickHouse, Redis, SQL Server, PG-base), `OraIntrospector$OraDroppedStuffDetector` | 2024.3.5 | names only |
| `*.minicat.xml.gzip` (18 DBMSs), `*-model-external-data.xml` (12) | 2024.3.5 | Proprietary |
| `messages/DatabaseBundle.properties` (introspection progress, levels, options, scope/filter hints) | 2024.3.5 | Proprietary |
| Help: `oracle-introspection-levels`, `schemas`, `data-sources-and-drivers-dialog`, `database-explorer`, `virtual-foreign-keys`, `virtual-columns`, `virtual-views`, `how-to-fetch-more-fields-for-mongodb-introspection`, `redis`; web `https://www.jetbrains.com/help/datagrip/2024.3/introspection-levels.html`, `https://www.jetbrains.com/datagrip/whatsnew/2024-3/` | 2024.3 | Proprietary docs |
| User's own model caches (`.idea/dataSources/<uuid>.xml`, `storage_v2`) — **element/attribute names only** | DataGrip 2024.3 | n/a |
| Docker `postgres:16-alpine` validation of xmin/age semantics | 16.x | PostgreSQL licence |
| `DataGrip/mongo-jdbc-driver@v1.20` | GitHub | Apache-2.0 |

---

## 2. Executive summary

1. Introspection is a set of **tasks**: general/combined, database-level, schema-level in *complete*, *incremental* or *fragment* mode, *brief* (names), *check up-to-dateness*, *elevation* (raise one object's level), *refresh affected objects* (after DDL) `[Observed: classes introspection/*Task; bundle:introspector.progress.status.*]`.
2. **Native introspectors** exist for 24 DBMSs; everything else uses JDBC `DatabaseMetaData` (with per-DBMS metadata *wrappers* that patch broken drivers). Users can force JDBC metadata `[Observed]`.
3. Catalog queries ship as **named SQL sections** (`---- Name ----`) with **meta-directives**: version gates (`#V91:` inline `a #: b #.` / `#V91>` line), mode gates (`#INC>` incremental-only line, `#INC: … #: … #.`, `#FRAG>` fragment-only, `#FULL:`), privilege gates (`#SUPER`, `#DBA>`, `#CLASSIC`, `#AZURE`), catalog-prefix macros (`#CAT`, `#CAT_objects` → `ALL_`/`DBA_` or `[db].sys`), placeholders (`#TXAGE`, `#SRCTXAGE`, `:since`, `:schema_id`, `:[*list]`, `OIDS`) `[Observed]`.
4. **Pairs of "existent" + "retrieve" queries**: a cheap id-only query lists what currently exists (to detect **drops**), and a full query filtered by a change marker returns what changed (to detect **creates/alters**) `[Observed: RetrieveExistentX / RetrieveX, ListExistentObjects, ListExistentIndices…]`.
5. **PostgreSQL family** uses tuple `xmin` ages: per object `StateNumber` (= xmin), per namespace `IntrospectionStateNumber`; incremental queries keep rows whose `age(xmin) ≤ #TXAGE`; the watermark is derived from the current xid and the **oldest in-progress transaction id** in `pg_locks`; server restart time is recorded `[Observed]`.
6. **SQL Server** uses `sys.*.modify_date ≥ :since` per schema, a database **fingerprint** (db id + create date + service broker guid + server name), and **checksums** for things without timestamps (custom types, permissions at schema/db/server level, extended properties) `[Observed]`.
7. **Oracle** uses `sys.obj$.mtime` (fast path with `SELECT ANY DICTIONARY`/`SELECT_CATALOG_ROLE`) or `ALL_/DBA_OBJECTS.last_ddl_time ≥ :since`, schema-id checks (drop/recreate), order-independent **hashes** for comments, db links, tablespaces, invalid-object lists, and **levels L1–L3** with auto thresholds `[Observed]`.
8. **MySQL/MariaDB**: no change marker in the shipped resources; introspection is full per schema; **2024.3 added fragment introspection and smart refresh after DDL** for MySQL/MariaDB (2024.2 did it for Oracle/SQL Server) `[Observed: whatsnew 2024.3; help "Starting from version 2024.2, for Oracle and Microsoft SQL Server…"; sqlEffectAnalyzer registered for ORACLE, MSSQL, MYSQL, MARIADB]`.
9. **Scope** = per-DS tree of database/schema nodes (with patterns and negation) persisted in `dataSources.local.xml`; **schema pattern** text syntax `db1:sc1,sc2|db2:sc3` with `@`, `*`, `{regex}`; **object filter** `type:[-]regex` joined by `||` `[Observed: bundle hints, help]`.
10. **System catalogs** are not introspected by default; **pre-introspected snapshots** (minicat) provide them for completion; registry `database.snapshots.auto=true` `[Observed]`.
11. **Model**: ~237 object kinds (singular/plural message keys), a typed tree persisted as XML with `serializer="dbm"`, `format-version="4.53"`, flat elements `<kind id name parent>` with typed child properties; sources stored separately in zips with per-file meta `[Observed]`.
12. **External data**: virtual views/FKs/columns per DS in a user file; shipped overlays add a `sessions` view (PG `pg_stat_activity`, MySQL `show processlist`, …) and hundreds of virtual FKs between catalog tables (e.g. `pg_class.relnamespace → pg_namespace.oid`) `[Observed]`.

---

## 3. Detailed findings

### 3.1 Triggers, tasks and modes

| Trigger | Evidence | Resulting task |
|---|---|---|
| DS created / OK in DS dialog with *Auto sync* | `[Observed: help Auto sync]` | complete (first), then incremental |
| DDL executed in a console (Auto sync on) | `[Observed: help]` | **affected objects only** for Oracle, SQL Server (2024.2), MySQL/MariaDB (2024.3); otherwise schema-level refresh (incremental where supported) |
| Manual *Refresh* (Ctrl+F5) | `[Observed]` | incremental for the selection; single object → fragment ("Refresh Selected Object") |
| *Diagnostics → Force Refresh* | `[Observed: help:schemas]` | clears cache, complete introspection |
| Scope change (Schemas tab, N of M) | `[Observed]` | introspect newly selected namespaces |
| *Automatic introspection interval N minutes* | `[Observed: IntrospectionScheduleOptionProvider]` | periodic incremental |
| Oracle level change | `[Observed: help]` | re-introspect at new level |
| Opening source of an object below L3 (Oracle) | `[Observed: help "fragmental introspection"]` | fragment (sources) |
| Cache staleness check (*Warn when editing outdated DDL*) | `[Observed: help; CheckUptodatenessIntrospectionTask]` | state check of specific objects |

Progress vocabulary `[Observed: bundle]`: "Performing complete|incremental introspection", "Introspecting schema {0} in full|incremental|fragment mode [at level {1}]", "using JDBC metadata", "external schema", "shared schema" (Redshift datashares), "Introspecting database at level 1: processing N regular schemas incrementally / N system schemas completely" and an "old algorithm" variant; ~70 "Retrieving <kind>" phases; "Waiting for previous introspection to finish…" (one introspection per DS at a time).

Task classes `[Observed]`: `GeneralIntrospectionTask`, `CombinedIntrospectionTask`, `DatabaseIntrospectionTask`, `DatabaseFragmentIntrospectionTask`, `BriefIntrospectionTask`, `CheckUptodatenessIntrospectionTask`, `ElevationIntrospectionTask`, `DefiniteIntrospectionTask`/`IndefiniteIntrospectionTask`, `IntrospectionMode`, `IntrospectionScopes`, `FamilyFragment(s)Builder`, `AffectedObjectMention`/`AffectedObjectScriptedMention`, `Impactees`, `DBIntrospectionOptions$SourceLoading`, `DBIntrospectorFeatures`, `DatabaseIntrospectionSession$IntrospectionPce` (cancellation).

### 3.2 Query-pack format and meta-directives

`[Observed: all *IntroQueries.sql; semantics Inferred from consistent usage]`

| Directive | Count | Meaning |
|---|---|---|
| `---- Name ----` | ~300 sections | named query; looked up by name from code |
| `#V<ver>: A #: B #.` | ~90 | if server version ≥ ver then A else B (inline; `#:` else branch optional) e.g. `#V91:`, `#V120:` (PG 12.0), `#V1044126` (CockroachDB internal numbering) |
| `#V<ver>> line` | ~50 | include line only if version ≥ ver |
| `#N12: A #: B #.` | 2 | negated version gate (version < 12) |
| `#INC> line` | 204 | include line only in **incremental** mode |
| `#INC: A #: B #.` | 18 | incremental vs full variant (e.g. different ORDER BY) |
| `#FULL:` / `#FULL>` | 4 | full-mode only |
| `#FRAG> line` / `#FRAG:` | 82 | include only in **fragment** mode (typically `and name in (:[*f_names])`) |
| `#INCSRC> line` | 6 | incremental filter for **source** queries (separate watermark) |
| `#TXAGE` / `#SRCTXAGE` | 137 / 11 | PG: max xid age considered "changed since last time" (model / sources) |
| `:since` / `:src_since` | 69 / 2 | timestamp watermark (SQL Server `modify_date`, Oracle `mtime`/`last_ddl_time`) |
| `:schema_id`, `:schema_name`, `:database_name`, `:oid`, `:type_id`… | — | bind parameters |
| `:[*name]` | many | list expansion (IN-list) |
| `OIDS` | 19 | literal list substitution of OIDs (state checks) |
| `#SUPER:`/`#SUPER>` | 4 | only when connected as superuser (PG user mappings) |
| `#DBA>` / `#CATDU_*` | 3 | DBA dictionary access |
| `#CAT` (MSSQL) | 165 | catalog prefix: `[db].sys` (cross-database introspection without `USE`) `[Inferred]` |
| `#CAT_<view>` (Oracle) | 50+ | `ALL_<view>` or `DBA_<view>` depending on privileges `[Inferred from CheckBaseDictionariesAccess]` |
| `#AZURE: A #: B #.`, `#CLASSIC:` | 3 | Azure SQL vs on-prem variants (e.g. identify current DB by name vs id) |
| `#ISTABLETYPE`, `#DBIDCOLUMN`, `#SHARING`, `#PUBLIC`, `#UMENABLED` | few | feature switches (table types, DB id column, Oracle 12c+ sharing, PUBLIC schema, user mappings enabled) |

Oracle statements end with `/`, others with `;`. Several queries pack many fields into a single fixed-width string per row (type code + zero-padded owner id + object id + flags + name) to minimise round-trips and parsing over slow dictionaries `[Observed: RetrieveAllMajorNamesFast]`.

### 3.3 Per-DBMS strategies

#### 3.3.1 PostgreSQL (and Greenplum, CockroachDB; Redshift partially)

Sections `[Observed: PgIntroQueries.sql]`: ServerStartupTime, IsSuperUser, ListDatabases, ListSchemas, roles, tablespaces, languages, event triggers, **StateNumber**, **CurrentXid**, comments (existent + retrieve at server/database/schema level), **RetrieveStates**, sequences (`pg_sequence` for ≥10, `pg_sequence_parameters` before), data types (+domain checks, enum labels, composite attributes), tables (`relkind` r/m/v/f/p; inheritance ancestors/successors; partition key/bound; persistence; options; access method), foreign tables, columns, indices (+index columns), constraints, used argument types, routines, aggregates, extensions (+members), operators, operator classes/families, AM ops/procs, rules, triggers, view/rule/function **sources**, access methods, casts, collations, FDWs, foreign servers, user mappings (superuser variant), referencing schemas, relations (dependencies), time zones, DateStyle, security policies.

Catalogs used (by frequency): `pg_class`, `pg_description`, `pg_proc`, `pg_namespace`, `pg_constraint`, `pg_opfamily`, `pg_opclass`, `pg_type`, `pg_rewrite`, `pg_operator`, `pg_language`, `pg_collation`, `pg_trigger`, `pg_policy`, `pg_foreign_server`, `pg_foreign_data_wrapper`, `pg_extension`, `pg_event_trigger`, `pg_am`, `pg_cast`, `pg_index`, `pg_attribute`, `pg_user_mapping(s)`, `pg_tablespace`, `pg_shdescription`, `pg_inherits`, `pg_enum`, `pg_amproc`, `pg_amop`, `pg_aggregate`, `pg_sequence`, `pg_roles`, `pg_locks`, `pg_database`, `pg_attrdef`, `pg_timezone_names/abbrevs`, plus functions `pg_get_expr`, `pg_get_indexdef`, `pg_get_viewdef`, `pg_get_triggerdef`, `pg_get_ruledef`, `pg_get_partkeydef`, `pg_get_function_sqlbody`, `pg_get_function_result/arguments`, `pg_get_userbyid`, `pg_tablespace_location`, `pg_indexam_has_property`, `pg_is_in_recovery`, `txid_current` `[Observed]`.

Incremental mechanism `[Observed + validated]`:
- Every catalog row's `xmin` = id of the transaction that wrote that row version. The model stores it per object as `StateNumber` and per root/database/schema `IntrospectionStateNumber` `[Observed: cache property names]`.
- Incremental queries add `age(xmin) <= #TXAGE` → "rows written within the last TXAGE transactions" `[Observed]`. Columns: a table is re-read if its `pg_class` row **or any of its `pg_attribute` rows** is young `[Observed: RetrieveColumns #INC lines]`. Comments: filtered by `pg_description.xmin` independently `[Observed]`. Sources: separate `#SRCTXAGE` watermark (sources can be loaded later/lazily).
- `CurrentXid` = `txid_current() mod 2^32` (null on standby — `pg_is_in_recovery()` ⇒ incremental impossible on hot standby, since a standby cannot assign xids) `[Observed]`.
- `StateNumber` query = the **oldest transaction id holding a lock** (`pg_locks.transactionid` ordered by age desc) `[Observed]` → `[Inferred]` watermark = min(oldest running xid, current xid) so that objects being modified by still-open transactions are re-read next time (their xmin is older than "now" but they become visible later).
- Validation (Docker PG 16) `[Observed: runtime]`: CREATE gives new xmin; `ALTER TABLE … ADD COLUMN` rewrites the `pg_class` row (new xmin) and adds `pg_attribute` row; `COMMENT ON COLUMN` changes **only** `pg_description` (table xmin unchanged); **`CREATE INDEX` did not change the table's `pg_class.xmin`** (relhasindex is updated *in place*), but the index's own `pg_class` row is new → index queries must use their own xmin; `DROP` is invisible to xmin → requires existence queries. Extended matrix on PG 16/17/18 [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-xmin]: table `pg_class.xmin` also moves on table-level `GRANT` (relacl), the **first** `CREATE TRIGGER` (relhastriggers) and `ADD CONSTRAINT … CHECK` (relchecks) — but **not** on a second trigger, `SET DEFAULT`, column-level `GRANT`, `DROP INDEX`/`DROP TRIGGER`; column `GRANT`, `SET DEFAULT` and `DROP COLUMN` (attisdropped) move only `pg_attribute.xmin`; `ALTER FUNCTION`/`CREATE OR REPLACE FUNCTION` move `pg_proc.xmin`; `pg_type`/`pg_namespace` rows are untouched by these DDLs. Hence triggers, indexes, constraints and defaults must be tracked by their own rows (`pg_trigger`, `pg_index`, `pg_constraint`, `pg_attrdef`).
- `RetrieveStates` takes a list of known OIDs and returns their current xmin from all relevant catalogs (`OIDS` substitution) `[Observed]` → used to check whether cached objects are still current (outdated-DDL warning) and to resolve uncertain cases.
- `ServerStartupTime` stored as `root/StartupTime` `[Observed]` → `[Inferred]` a restart (or restore/failover to another node) invalidates xid-based assumptions → complete introspection.
- `PgNoDatabaseXminOption` "Do not use xmin in queries to pg_database" for servers lacking it `[Observed]`.
- xid wraparound: `age()` handles modular arithmetic. (corrected after verification) Frozen tuples do **not** get a huge age: since PG 9.4 freezing keeps the raw `xmin`, so `age(xmin)` is unchanged by `VACUUM FREEZE`; only bootstrap rows (xmin 1/2) report `2147483647`. Once more than 2^31 XIDs have passed since a row was written, its `age(xmin)` turns **negative** and `age(xmin) <= TXAGE` matches it (spurious re-reads; no false negatives). Filter with `age(xmin) BETWEEN 0 AND TXAGE`; if `TXAGE` exceeds ~2^31 or the cache is very old → full introspection. [Observed: docker test, postgres:16/18 (freeze) and postgres:17 with `pg_resetwal -x` jump of 2^31+2000 (age −2147482386; 220 of 416 `pg_class` rows matched), see verification/01-db-behavior-tests.md#pg-xmin-freeze]

#### 3.3.2 Amazon Redshift, Greenplum, CockroachDB

- Redshift adds datashares, shared/external schemas & tables (Spectrum/Data Catalog), late-binding ("boundless") views' columns, groups/users/permissions; still xmin-based for local objects `[Observed: RsIntroQueries sections]`.
- Greenplum adds distribution keys, external tables, partitions `[Observed]`.
- CockroachDB mirrors PG sections, uses `[SHOW ROLES]` for role options, comments out some source-incremental filters `[Observed]`.

#### 3.3.3 SQL Server / Azure SQL / Synapse / Sybase ASE

Sections `[Observed: MsIntroQueries.sql]`: QueryCurrentSessionInfo (is_azure, db/schema id+name), **QueryFingerprint**, ListDatabases, ListSchemas (default schema of current principal), **ListExistentObjects**, **ListExistentIndices** (synthetic id = object_id×100000 + index_id mod 30000), ListExistentFulltextIndices, RetrieveStates (object_id → modify_date), **CalculateCustomTypesCheckSums**, alias/table types, tables (+temporal periods, partitions), views, objects, columns (+type columns), indices, fulltext indices, keys, FKs, checks, triggers, synonyms, sequences, rules, **Calculate{Schema,Db,Server}PermissionsChecksum** + Retrieve*Permissions, **CalculateExtendedPropertiesChecksum** + extended properties, sources (with/without sys), arguments, security policies/predicates, linked servers, collations, logins, roles, Always Encrypted keys (CMK, CEK, values), assemblies, partition functions/ranges/schemes, filegroups.

Catalogs: `sys.all_objects`, `sys.objects`, `sys.types`, `sys.table_types`, `sys.tables`, `sys.periods`, `sys.partitions`, `sys.views`, `sys.all_columns`, `sys.indexes`, `sys.index_columns`, `sys.fulltext_indexes`, `sys.key_constraints`, `sys.foreign_keys`, `sys.foreign_key_columns`, `sys.check_constraints`, `sys.default_constraints`, `sys.computed_columns`, `sys.identity_columns`, `sys.masked_columns`, `sys.triggers`, `sys.trigger_events`, `sys.synonyms`, `sys.sequences`, `sys.sql_modules`/`sys.all_sql_modules`, `sys.syscomments`, `sys.all_parameters`, `sys.extended_properties`, `sys.database_permissions`, `sys.server_permissions`, `sys.database_principals`, `sys.server_principals`, `sys.security_policies/predicates`, `sys.column_master_keys`, `sys.column_encryption_keys(_values)`, `sys.assemblies`, `sys.partition_functions/parameters/range_values/schemes`, `sys.destination_data_spaces`, `sys.filegroups`, `sys.databases` `[Observed]`.

Incremental mechanism:
- `#INC> and modify_date >= cast(:since as datetime)` on objects/tables/views/types; child objects (columns, indexes, keys…) are re-read when the **parent** object's modify_date is recent (`parent_id in (…)`) `[Observed]`.
- Watermark per schema stored as `schema/IntrospectionTimestamp` `[Observed: cache]`; `[Inferred]` it's the server time at start of the previous introspection (not client time) to avoid clock skew.
- Drops: `ListExistentObjects`/`Indices`/`FulltextIndices` per schema `[Observed]`.
- Things without modify_date (permissions, extended properties/comments, user-defined types) → **checksums** compared with stored `PermissionsCheckSum`, `DescriptionsCheckSum` `[Observed: queries + cache property names]`; on mismatch the whole category is re-read.
- **Fingerprint** `database_id.create_date/service_broker_guid@server` stored as `database/Fingerprint` → database dropped & recreated or restored under the same name ⇒ full introspection `[Observed]`.
- Caveat `[Inferred: SQL Server semantics]`: `modify_date` on a table changes for ALTER TABLE and index/constraint changes, but **not** for some operations (e.g. `sp_rename` of a column updates it; granting permissions does not → hence permission checksums).
- Sybase ASE: QueryCurrentSessionInfo, databases, schemas, major names, **RetrieveStates**, logins/roles/users, grants, procedures, columns & arguments, defaults, indices, FKs (+wide column lists), checks, triggers, sources `[Observed: AseIntroQueries.sql]`.

#### 3.3.4 Oracle

Sections `[Observed: OraIntroQueries.sql]`: RetrieveSessionInfo, RetrieveServerInfo (container: CDB root/PDB), **CheckBaseDictionariesAccess** (`SELECT_CATALOG_ROLE` / `SELECT ANY DICTIONARY`), **CheckFastDictionariesAccess** (probe `sys.obj$`, `type$`, `ind$`, `ts$`, `con$`, `cdef$`, `col$`, `dependency$`, `trigger$`, `procedureinfo$`, `syn$`), RetrieveAllSchemas (incl. PUBLIC; current user first), RetrieveSchemaInfo (user id + min(created) + max(last_ddl_time) per owner), **RetrieveAllMajorNames[Fast]** (packed rows; `#INC> mtime >= :since` or `last_ddl_time >= :since`; excludes recycle bin `BIN$%`, LOB index segments `SYS_IL%$$`, secondary objects, sub-objects), RetrieveAllObjectIdentifiers, **CheckSchemaId**, **RetrieveStates[Fast]** (`obj$.mtime`), **ListFragmentRegularIds/TypeIds**, synonyms (+fragment), column names (Fast/Full/Inc), minor object names, constraint names, inner routine names, argument names (several version variants incl. `Slow10`, `Full10`), sequences, object/collection types, type attributes, clusters (with fallback when no permissions), tables, MV logs, MVs, views, columns, column defaults (XML vs LONG fetch variants), cluster indices, table-cluster columns (DBA/USER), indices (+columns, +expressions), keys, FKs, checks (predicates via XML / 12c / LONG), constraint columns, triggers (+update columns), partitions, packages, inner routines (11/10), type methods (11/10), inner routine arguments, single routines (+arguments), **CalculateDbLinksHash** + db links (+with source), **CalculateCommentsHash** + comments, dependencies (fast/normal/fragment), MV/view/trigger sources (XML/12/LONG variants), **ListSourceNames** + **RetrievePortionOfSources** (sources fetched in name batches from `ALL_SOURCE`), **ListInvalidObjects[Fast]** (`obj$.status > 1`), RetrieveErrors (`ALL_ERRORS`), RetrieveDebugInfoPresence, RetrieveNativeDefinitions (`DBMS_METADATA`), **CalculateTablespacesHash{Dba,User}** + tablespaces, data files, database users.

Strategy:
- **Two dictionary paths**: *fast* (`sys.*$` base tables; requires privileges) and *standard* (`ALL_*`/`DBA_*` views via `#CAT_*` macros) `[Observed]`. The base-table path avoids the notoriously slow `ALL_*` views (privilege checks per row).
- Incremental watermark: `mtime` (fast) / `last_ddl_time` (standard) `≥ :since` `[Observed]`.
- **Schema identity**: `CheckSchemaId` compares the stored user id; a schema dropped/recreated with the same name ⇒ full `[Observed query; consequence Inferred]`.
- **Hash checks** for categories without reliable timestamps: comments (order-independent sum of `object_id × prime + hash(text)`), db links, tablespaces `[Observed]`.
- Drop detection: id lists (`RetrieveAllObjectIdentifiers`, `OraDroppedStuffDetector`) `[Observed]`.
- Invalid objects and compile errors are introspected (to flag invalid packages/views) `[Observed]`.
- Option *Fetch LONGs* and multiple LONG/XML/12c variants: LONG columns (`DATA_DEFAULT`, `TEXT` of views, `SEARCH_CONDITION`) can't be filtered/substr'ed in SQL; DataGrip has variants converting them via XML (`DBMS_XMLGEN`) or 12c `*_VC` columns `[Observed section names]`.
- **Levels L1/L2/L3** with defaults by schema type and object count (current: L3 if N ≤ 1000, L2 if ≤ 3000, else L1; non-current: L2 if ≤ 3000 else L1; system: L2 if ≤ 100 else L1); set per database or schema, inherited by children; icons ("pills") blue = explicit, grey = inherited; Auto sync re-introspects on change; registry `database.introspection.level.auto.select.strategy` (`mini`, `simple`, default) `[Observed: help + registry]`.
- Level definitions: L1 = names of all supported objects and their signatures (except index column names and private package variables); L2 = everything except source code; L3 = everything `[Observed]`.
- **Introspect server objects** option (users, tablespaces, data files) `[Observed]`.

#### 3.3.5 MySQL / MariaDB (and MemSQL/SingleStore, Aurora; TiDB/OceanBase via fallback; Vitess via JDBC)

- No shipped `.sql` pack; queries are code-embedded. Query-holder classes `MysqlBaseIntroQueries$` `{Schema, Table, View, Column, AutoIncValue, CheckConstraint, Collation, Constraint, ConstraintColumn, ForeignKey, IndexColumn, Routine, Parameter, Trigger, Event, User, RoleGrant, SchemaGrant, ColumnGrant, RoutineGrant, Subpartition}` `[Observed]` → `[Inferred]` sources: `information_schema.SCHEMATA, TABLES, VIEWS, COLUMNS, STATISTICS, TABLE_CONSTRAINTS, KEY_COLUMN_USAGE, REFERENTIAL_CONSTRAINTS, CHECK_CONSTRAINTS (8.0.16+/MariaDB 10.2+), ROUTINES, PARAMETERS, TRIGGERS, EVENTS, PARTITIONS, COLLATIONS`, `mysql.user`, `mysql.role_edges`, `SCHEMA_PRIVILEGES`, `COLUMN_PRIVILEGES`, `mysql.procs_priv`; `AUTO_INCREMENT` values from `TABLES`.
- Options: *MysqlRoutineRetrievalOption* (routine bodies via `SHOW CREATE` vs `information_schema` — the latter hides bodies without privileges `[Inferred]`), *MariaGenExprRetrievalOption* ("Use SHOW CREATE TABLE … Until MariaDB 10.2.5 the 'generation_expression' was missing in information_schema.columns") `[Observed]`.
- `MysqlIntrospectorStatsProvider` `[Observed]` → statistics (e.g. `information_schema_stats_expiry` issues in 8.0) `[Inferred]`.
- **Incremental**: help lists incremental support only for "Oracle, Microsoft SQL Server, PostgreSQL and similar" `[Observed]`. MySQL has `TABLES.UPDATE_TIME/CREATE_TIME` but `CREATE_TIME` changes on some ALTERs only and data-dictionary caching (8.0 `information_schema_stats_expiry`) makes it unreliable → full per schema. (corrected after verification) On MySQL 8.0.46/8.4.13, `CREATE_TIME` changed on **every** tested `ALTER TABLE` (INSTANT add column, INPLACE add index, table comment, rename column, COPY rebuild) and is not subject to the stats cache; it did not change on `CREATE TRIGGER`; granularity is one second. `UPDATE_TIME` is a cached statistic (NULL unless `information_schema_stats_expiry=0`, reset by ALTER) and is useless for DDL detection. [Observed: docker test, mysql:8.0/8.4, see verification/01-db-behavior-tests.md#mysql-is-times] **2024.3**: fragment introspection (single object refresh) + smart refresh after DDL via `MysqlBaseAffectAnalyzer` `[Observed]`.
- Model shape: Root → {Collation, User/Role, Schema}; Schema → {Table, View, Routine, Event}; Table → {Column, Key, ForeignKey, Index, Check, Trigger, Partition → Subpartition} `[Observed: prior note from model classes; consistent with holders]`.

#### 3.3.6 SQLite

- Query holders `SqliteIntroQueries$` `{SchemaInfo, MajorInfo, ColumnInfo, IndexInfo, IndexColumnInfo, ForeignKeyInfo, CollationInfo, FunctionInfo, ModuleInfo, SourceInfo}` `[Observed]` → `[Inferred]` `PRAGMA database_list` (main/temp/attached → schemas), `sqlite_master`/`sqlite_schema` (major objects + `sql` sources), `PRAGMA table_xinfo` (columns incl. generated/hidden), `PRAGMA index_list/index_xinfo`, `PRAGMA foreign_key_list`, `PRAGMA collation_list`, `PRAGMA function_list` (→ routines + arguments; observed in cache as `routine`/`argument`/`package`), `PRAGMA module_list` (virtual table modules).
- Cache has `schema/LastIntrospectionLocalTimestamp` but no state numbers `[Observed]` → full introspection each time (cheap: local file). `[Unknown]` whether `PRAGMA schema_version` short-circuits it; recommended for Hacivat (ALG-INTRO-05).
- `SqliteJdbcSourceLoader`, `SqliteMetadataWrapper` `[Observed]`; options *Register REGEXP function*, temp dir `[Observed]`.

#### 3.3.7 ClickHouse

- Query holders `CHouseIntroQueries$` `{TableInfo, ColumnInfo, FunctionInfo, RoleInfo, UserInfo}` `[Observed]` → `[Inferred]` `system.databases`, `system.tables` (engine, engine_full, uuid, create query), `system.columns` (default_kind/expression, comment), `system.functions` (incl. SQL UDFs), `system.users`, `system.roles`.
- Cache properties: table `Engine`, `EngineParams`, `Uuid`, `Comment`; column `ColumnKind` (default kind: DEFAULT/MATERIALIZED/ALIAS/EPHEMERAL), `DefaultExpression`; no state numbers `[Observed]` → full per database. `system.tables.metadata_modification_time` exists and could drive incremental (Hacivat opportunity).
- `databaseTerm=schema` maps databases to schemas; `CHouseAutoSessionOption` for session_id `[Observed]`; minicat for system database `[Observed]`.

#### 3.3.8 Redis, MongoDB and other NoSQL — summary (details in `05-nosql.md`)

| DBMS | Introspector | Catalog source | Incremental |
|---|---|---|---|
| Redis | native `RedisIntrospector` (holders: ServerInfo, DbCountInfo, KeyspaceInfo, KeysScanInfo, ModulesInfo, SlotsInfo) | `INFO server`, `CONFIG GET databases`, `INFO keyspace`, `SCAN … MATCH … COUNT …` + `TYPE`, `MODULE LIST`, `CLUSTER SLOTS` `[Inferred mapping]` | none (full scan per DB; key filter + COUNT hint) |
| MongoDB | generic JDBC (driver metadata) | `listDatabases`, `listCollections`, first N documents per collection (N = `fetch_documents_for_metainfo`, default 10), `listIndexes` `[Observed: oss-mongo@v1.20; help]` | none |
| Cassandra | native, `CassIntroQueries.sql` | `system_schema.{keyspaces,tables,columns,indexes,views,types,functions,aggregates,triggers}`, `system_virtual_schema.*`, legacy `system.schema_*` (2.x), `system_auth.{roles,users}` `[Observed]` | none |
| DynamoDB, Couchbase | native | `ListTables`/`DescribeTable`; Couchbase `system:keyspaces`/buckets-scopes-collections `[Inferred]` | none |

#### 3.3.9 Others (brief)

Native introspectors also for Db2 LUW, H2, Hive, Snowflake (`SFlakeShowStatementLimitOption`, stage properties option, stats provider), Vertica, Exasol, BigQuery (parallel metadata threads property) `[Observed]`. Generic DBMSs use JDBC `getSchemas/getTables/getColumns/getPrimaryKeys/getImportedKeys/getIndexInfo/getProcedures/getFunctions` via `JdbcIntrospector`, optionally patched by a `jdbcMetadataWrapper` `[Inferred: JDBC API]`.

### 3.4 Per-DBMS summary table

| Aspect | PostgreSQL | MySQL / MariaDB | SQL Server | Oracle | SQLite | ClickHouse | Redis | MongoDB |
|---|---|---|---|---|---|---|---|---|
| Introspector | native SQL pack | native (code) | native SQL pack | native SQL pack | native (pragmas) | native (code) | native (commands) | JDBC (driver) |
| Namespace model | Root→Database→Schema | Root→Schema | Root→Database→Schema | Root→Schema (users) | Root→Schema (attached DBs) | Root→Schema (=database) | Root→Schema (=DB index) | Root→Schema (=database) |
| Change marker | `xmin` age (`#TXAGE`) | none (full/schema) | `modify_date` | `obj$.mtime` / `last_ddl_time` | none | none | none | none |
| Watermark storage | `IntrospectionStateNumber` (root/db/schema) + `StateNumber` per object | — | `IntrospectionTimestamp` per schema | `:since` per schema `[Inferred]` | `LastIntrospectionLocalTimestamp` | — | — | — |
| Drop detection | Existent-id queries per kind | full reload | `ListExistent*` | id lists, dropped-stuff detector | full | full | full | full |
| Non-timestamped categories | comments by `pg_description.xmin` | — | checksums (types, permissions, ext. properties) | hashes (comments, db links, tablespaces) | — | — | — | — |
| Identity guard | server startup time | — | DB fingerprint | schema user id | file path | — | — | — |
| Fragment (single object) | yes (`Refresh Selected Object`) `[Inferred: general]` | **yes since 2024.3** | yes (`#FRAG>`) | yes (`ListFragment*`) | — | — | — | — |
| Smart refresh after DDL | schema incremental | **affected objects (2024.3)** | **affected objects (2024.2)** | **affected objects (2024.2)** | full | full | — | — |
| Levels | no | no | no | **L1–L3** | no | no | no | no |
| Sources | `pg_get_*def`, `prosrc`, `pg_get_function_sqlbody` | `SHOW CREATE`/`information_schema` | `sys.sql_modules` | `ALL_SOURCE` batches, `DBMS_METADATA` | `sqlite_master.sql` | `create_table_query` | — | — |
| Pre-introspected catalogs | `pg_catalog`, `information_schema` | `information_schema`, `mysql`, `performance_schema`, `sys` | `sys`, `INFORMATION_SCHEMA` | `SYS`, `SYSTEM` | — | `system` | — | — |
| Virtual `sessions` view | `pg_stat_activity` | `show processlist` | (ms-model-external-data) | `V$SESSION` | — | (chouse file) | — | — |

### 3.5 Existence diff and merge

The model merge rule `[Inferred from paired queries + StateNumber]`:
1. For a namespace in incremental mode: `E` = set of ids from "existent" queries (cheap), `C` = rows from incremental "retrieve" queries (changed or new), `M` = cached children.
2. Delete `M \ E` (dropped). Upsert all of `C` (replace properties, keep identity by object id). Keep `M ∩ E \ C` untouched.
3. Child collections of a re-read parent are replaced wholesale (e.g. all columns of a re-read table).
4. Objects referencing deleted ones keep **name-based** references (re-resolved later) — FKs store `RefTableId` and names `[Observed: cache properties]`.

### 3.6 Scope, patterns and filters

- Persisted scope tree in `dataSources.local.xml`: `schema-mapping/introspection-scope/node(kind, qname, negative)` nested up to database→schema, `name(qname)` children for explicit names, plus `substitutions(use-mini-cat)` `[Observed: structure]`.
- Text syntax (Schemas tab "Schema pattern") `[Observed: bundle:DataSourceSchemasPanel.schema.pattern.hint; help]`: `db1:sc1,sc2|db2:sc3`; `@` = default database/schema; `*` = every; `{regexp}` = names matching regex; "If only schemas supported, list them after `*`"; examples `*:*`, `@:*`, `@:@`, `{t.*}:@`, `*:dbo|@:@|db1:s1,s2,s3`.
- The tree editor supports regex filter nodes under any node, nested; "Multiple patterns combine multiplicities, not intersect them"; "All schemas" vs "Default schema" vs filter node `[Observed: help:schemas]`.
- Registry `database.scopeRefresh.listSchemasFromAllDatabases=true` — "visit every accessible database… On some RDBMS (PostgreSQL, Azure) this can lead to a noticeable decrease of performance" `[Observed]`.
- *Track databases/schemas creation and deletion* updates the selectable list `[Observed]`.
- **Object filter** `[Observed: bundle:DataSourceSchemasPanel.object.filter.hint; help]`: "A `<pattern>` or a list of `<type>:[-]<pattern>` separated by `||`"; types listed in help: aggregate, collation, event, fdw, ftable, mview, operator, package, role, routine, sequence, synonym, table, user, view, vtable; pattern = Java regex; `-` excludes; examples `table:CUR_.*`, `table:-OLD_.*||routine:DO.*||view:V_.*`. Filtered-out objects are **not resolved** in SQL ("DataGrip does not resolve objects that are not displayed").
- PG options *Show internal system schemas*, *Show template databases*; registry `database.postgres.introspect.system.schemas=false` `[Observed]`.

### 3.7 Pre-introspected catalogs (minicat) and model external data

- 18 gzip snapshots in the same `dbm` XML format (nested form), sizes 7 KB (Derby) … 3.9 MB (Oracle) `[Observed]`. PG snapshot contains ~3,900 routines, ~13,700 arguments, ~1,400 operators, ~140 views/tables, etc. `[Observed counts]`.
- Used for schemas not selected for introspection ("Use pre-introspected objects for system catalogs that are not introspected"; lightning icon; per-schema override) `[Observed: help:schemas]`. Registry `database.snapshots.auto=true` ("Automatically inject pre-introspected system objects"), `database.view.show.mini.cat=false` `[Observed]`.
- `[Inferred]` one snapshot per DBMS family/version line; mismatch with the actual server version is tolerated (completion only).
- **External data** (`*-model-external-data.xml`): keyed by family (`POSTGRES_LIKE`, `MYSQL_LIKE`, …) with `object-set` → `root`/`database`/`table path` → virtual `view name query` and virtual FK edges `table path from to` (PG: 324 table edges, 47 views) `[Observed]`. Users get the same via *Virtual objects and attributes* file (virtual FKs, virtual columns with expressions, virtual views with statements) `[Observed: help:data-sources-and-drivers-dialog; virtual-* topics]`.

### 3.8 Object kinds and hierarchy

- ~237 kinds (`ObjectKind.<kind>` singular + plural keys = 473 entries) covering all DBMSs, from `root, database, schema, table, column…` to DBMS-specific (`dictionary_layouts` ClickHouse, `masking_policies` Snowflake, `bufferpools` Db2, `availability_groups` SQL Server, Redis `string, list, set, sorted_set, hash_table, stream, json_document, data_structure`) `[Observed: DatabaseDynamicBundle]`. 20 `extraFamily.*` keys = synthetic folders `[Observed]`.
- Generic interfaces (names): `DasObject`, `DasNamed`, `DasNamespace`, `DasTable`, `DasColumn`, `DasIndex`, `DasConstraint`, `DasTableKey`, `DasForeignKey`, `DasTrigger`, `DasRoutine`, `DasArgument`, `DasSynonym`, `DasUserDefinedType`, `DasObjectWithSource`; `ObjectPath` (kind+name chain) `[Observed: prior class inventory]`.
- Hierarchies (from caches and model classes):
  - **PostgreSQL**: Root → {Role, Tablespace, Database}; Database → {Schema, AccessMethod, Cast, Extension, EventTrigger, ForeignDataWrapper → ForeignServer → UserMapping, Language}; Schema → {Table, View, MaterializedView, ForeignTable, Routine → Argument, Aggregate, Sequence, ObjectType (domain/enum/composite/range), Collation, Operator, OperatorClass, OperatorFamily}; Table → {Column, Key, ForeignKey, Index, Check, Trigger, Rule, SecurityPolicy}.
  - **SQL Server**: Root → {Login, LinkedServer, Collation, Database}; Database → {Schema, Role, Assembly, Filegroup, PartitionFunction, PartitionScheme, ColumnMasterKey, ColumnEncryptionKey}; Schema → {Table, View, Routine, AliasType, TableType, Sequence, Synonym, Default/Rule}; Table → {Column, Key, ForeignKey, Index (incl. fulltext), Check, Trigger, SecurityPolicy}.
  - **Oracle**: Root → {Schema (users), Role, Tablespace → DataFile, DbLink(public)}; Schema → {Table, View, MaterializedView, MaterializedLog, Package (+Body, inner Routine → Argument), Routine, ObjectType (+Method, Attribute), CollectionType, Sequence, Synonym, Cluster, DbLink, Trigger}; Table → {Column, Key, ForeignKey, Index, Check, Trigger, Partition}.
  - **MySQL**: see §3.3.5. **SQLite**: Root → Schema → {Table, View, Trigger, Routine(function_list) → Argument, Package(modules), Collation}. **ClickHouse**: Root → {User, Role, Schema → {Table(Engine), View, MaterializedView, Dictionary}, Routine}. **Redis**: Root → Schema(db index) → key kinds. **MongoDB**: Root → Schema(database) → Table(collection) → Column(field, nested) + Index.

### 3.9 Properties per kind (observed in caches)

`[Observed: element names under each kind in the user's caches; values not read]`

| Kind | PostgreSQL properties | SQL Server properties |
|---|---|---|
| root | `ServerVersion`, `StartupTime`, `IntrospectionStateNumber`, `DateStyle`, `TimeZones`, `Grants` | `ServerVersion`, `Grants`, `PermissionsCheckSum` |
| database | `ObjectId`, `OwnerName`, `Comment`, `Current`, `Grants`, `IntrospectionStateNumber` | `ObjectId`, `CollationName`, `Comment`, `Current`, `ExtendedProperties`, `Fingerprint`, `Grants`, `Online`, `PermissionsCheckSum` |
| schema | `ObjectId`, `StateNumber`, `OwnerName`, `Comment`, `Current`, `Grants`, `IntrospectionStateNumber`, `LastIntrospectionLocalTimestamp` | `ObjectId`, `Current`, `Grants`, `DescriptionsCheckSum`, `PermissionsCheckSum`, `IntrospectionTimestamp`, `LastIntrospectionLocalTimestamp` |
| table | `ObjectId`, `StateNumber`, `OwnerName`, `AccessMethodId` | `ObjectId`, `CreatedTimestamp`, `ModifiedTimestamp`, `DataRetentionPeriod`, `LobDataSpaceId`, `PartitionDataSpaceId`, `LedgerViewId` |
| column | `Position`, `StoredType`, `TypeId`, `NotNull`, `DefaultExpression`, `SequenceId`, `IdentityKind`, `StateNumber` | `Position`, `StoredType`, `NotNull`, `CollationName`, `Comment`, `Computed`, `DefaultConstraint{Id,Name,Inline,DefaultExpression}`, `SequenceIdentity`, `RowGuid` |
| index | `ColNames`, `ReverseColNames`, `CollationIds/Names/ParentNames`, `ClassIds/Names/ParentNames`, `AccessMethodId`, `Condition`, `Primary`, `Unique`, `NameSurrogate`, `ObjectId`, `StateNumber` | `ColNames`, `IncludedColNames`, `Clustering`, `Unique`, `Position`, `PartitionDataSpaceId`, `NameSurrogate` |
| key | `Primary`, `UnderlyingIndexId`, `Deferrable`, `InitiallyDeferred`, `NameSurrogate`, … | `Primary`, `UnderlyingIndexName`, timestamps |
| foreign-key | `ColNames`, `RefTableId`, `RefKeyColPositions`, `OnDelete`, `OnUpdate`, `NameSurrogate` | `ColNames`, `RefTableId`, `RefTableName`, `RefColNames`, `OnDelete/OnUpdate`, `Disabled`, timestamps |
| routine | `LanguageName`, `VolatilityKind`, `ConcurrencyKind`(parallel), `Strict`, `ReturnsSet`, `Cost`, `Rows`, `ResultsDefinition`, `PgRoutineKind`, `SourceTextLength`, `OwnerName` | `RoutineType`, `SourceTextLength`, `ExtendedProperties`, timestamps |
| trigger | `CallRoutineId`, `Events`, `Turn` (BEFORE/AFTER/INSTEAD) | `Events`, `Turn`, `Disabled`, `SourceTextLength` |
| others (PG) | sequence `StartValue`, `SequenceIdentity`; extension `Version`, `ExtSchemaId`, `MemberIds`; role `CanLogin`, `SuperRole`, `CreateDb`, `CreateRole`, `Inherit`, `Replication`, `BypassRls`, `RoleGrants`; operator/cast/opclass properties; security-policy `UsingExpression`, `CheckExpression`, `Permissive`, `RoleIds` | |

Conventions visible here: **references stored both by id and by name** (`TypeId`+`StoredType`; `CastFunctionId`+`CastFunctionName`+`CastFunctionParentName`) so links survive partial reloads; `NameSurrogate` marks server-generated names (affects DDL generation); `SourceTextLength` keeps only the length in the model while text lives in the source store.

### 3.10 Persistence, versioning and caching

- Model file: `.idea/dataSources/<ds-uuid>.xml`, root `<dataSource name>` → `<database-model serializer="dbm" dbms="…" family-id="…" format-version="4.53">`, then a **flat list** of `<kind id="N" parent="P" name="…">` elements with property child elements `[Observed]`. Integer ids are model-local; `ObjectId` holds the server id (OID, object_id, obj#).
- Format versioning: `format-version` + a chain of converters `[Observed attribute; converters per prior note]`. Registry `database.storage.output.buffer=10000000` (buffer for writing schema to disk) `[Observed]`.
- **Sources**: `.idea/dataSources/<uuid>/storage_v2/_src_/{database|schema}/<name>.<6-char hash>[.meta]`, nested `…/schema/<name>.<hash>.zip`; zips contain `<kind>/<name>.<hash>.sql` + `.meta` (stored, uncompressed); kinds observed `routine`, `table`, `view`, `materialized-view`; meta lines `#n:<name>` and a `!<md>` record with a numeric checksum and **offset/segment markers** (e.g. `[D, Q, D, ;]`) `[Observed: structure]` → `[Inferred]` per-source fingerprint + segmentation for quick diff/outdated detection. Actions: Clear stored sources and local modifications; Load Sources (All / All excl. system / None) `[Observed]`.
- Local modifications of sources ("Database Changes") are kept separately until submitted (other doc set).
- Diagnostics: *Generate Introspector Diagnostic Files*, *Dump model*, *Schema Refresh Time* view option `[Observed: bundle]`.

---

## 4. Algorithm specifications

### ALG-INTRO-01: Query-pack preprocessor

- **Problem:** Store per-DBMS catalog SQL as data files with version/mode/privilege variants and render the concrete SQL at runtime.
- **Inputs / outputs:** `pack_text`, `section_name`, `ctx{server_version, mode∈{Full,Inc,Frag}, flags:{super,dba,azure,…}, params}` → `(sql, binds)`.
- **Algorithm:**
  1. Split file into sections at lines matching `^---- (\w+) ----$`.
  2. Line pass: for lines starting with a gate `#X>` evaluate gate; keep the remainder or drop the line.
  3. Inline pass: parse `#X: A #: B #.` (nestable, `#:` optional) and replace with A or B (or empty).
  4. Gate evaluation: `#V<n>` → `version_code ≥ n` (encode `major*10+minor` for PG `91`=9.1, `120`=12.0; per-DBMS encoders); `#N<n>` → `<`; `#INC`/`#FULL`/`#FRAG`/`#INCSRC` → mode; `#SUPER`, `#DBA`, `#AZURE`, `#CLASSIC`, feature flags.
  5. Macros: `#CAT` → catalog prefix (e.g. `[db].sys`), `#CAT_x` → `ALL_x`/`DBA_x`, numeric placeholders (`#TXAGE`) → literal integers (validated).
  6. Binds: `:name` → positional/named bind; `:[*name]` → expand list into `(?,?,…)`; empty list → `(NULL)` or skip the query.
- **Data structures:** `HashMap<String, Section{raw:String}>`; compiled cache keyed by `(section, version_code, mode, flags)`.
- **Complexity:** O(n) per render; cache results.
- **Edge cases:** directives inside string literals (avoid: require directives at token boundaries); nested inline gates; a `union` left dangling by a removed branch (DataGrip places `union` inside the gated block — Hacivat linter must check SQL validity for each variant); version strings of forks (Cockroach `#V1044126`); list expansion beyond DBMS limits (Oracle 1000-item IN limit → chunk); SQL injection — never interpolate names, always bind.
- **Test cases:** `select a, #V91: b #: null #. as x` with v=9.0 → `select a, null as x`; `#INC> where age(xmin) <= #TXAGE` in Full → line removed; Frag with `:[*f_names]=["t1","t2"]` → `in (?, ?)`; Oracle 1500 names → 2 chunks.
- **Prior art and evidence:** `[Observed: *IntroQueries.sql]`.
- **Used by:** Hacivat (catalog SQL as `include_str!` packs, one file per DBMS, CI renders all variants against Docker matrices).

### ALG-INTRO-02: PostgreSQL incremental introspection by xmin age

- **Problem:** Re-read only catalog rows changed since the last introspection.
- **Inputs / outputs:** cached namespace `{last_xid, objects with state_number}`, live connection → `Delta`.
- **Algorithm:**
  1. If server in recovery (`pg_is_in_recovery()`), or `startup_time` changed, or cache format/version changed, or `last_xid` unknown → **full**.
  2. `cur = txid_current() mod 2^32` (or `pg_current_xact_id()` on ≥13 — note: assigns an xid; on a read-only/standby connection use `pg_snapshot_xmin(pg_current_snapshot())`/`txid_snapshot_xmin` instead) and `oldest_running = min(xid in pg_locks where transactionid not null)` (or snapshot xmin).
  3. `txage = (cur - last_watermark) mod 2^32 + safety` where `last_watermark` is the previous run's `min(oldest_running, cur)`. If `txage > 2^31 - margin` → full.
  4. For each kind: run *existent ids* query and *retrieve* query with `age(xmin) <= txage` (columns: table OR any attribute young; comments: description xmin; indexes: index's own `pg_class.xmin` or `pg_index.xmin`).
  5. Merge (ALG-INTRO-07). Store new watermark = `min(oldest_running, cur)` captured **at the start** of this run.
- **Data structures:** per namespace `{watermark_xid: u32, startup_time: i64}`; per object `state_number: u32`.
- **Complexity:** cheap queries on large catalogs (seq scans of catalogs filtered by `age(xmin)` — no index on xmin; still fast).
- **Edge cases (exhaustive):** in-place catalog updates (`relhasindex`, `relpages`, `reltuples`, `relfrozenxid`) don't change xmin — fine for DDL but beware relying on table xmin for index existence (validated); `VACUUM FREEZE` keeps the raw xmin, so frozen rows keep their real age and, after >2^31 XIDs, get a negative age that `age(xmin) <= txage` matches — use `age(xmin) BETWEEN 0 AND txage` (corrected after verification; [Observed: docker test, see verification/01-db-behavior-tests.md#pg-xmin-freeze]); xid wraparound (age handles mod 2^32; if more than 2^31 transactions passed → full); long-running transactions that commit DDL after our run: their xid < our watermark? We use oldest running xid as watermark, so their rows (xmin = their xid ≥ oldest_running) remain within age next time ✔; prepared transactions (`pg_prepared_xacts`) similar — include them; logical replicas/standbys: no xids → full; PgBouncer: `txid_current()` assigns an xid (writes!) — prefer snapshot functions; read-only transaction: (corrected after verification) on a primary `txid_current()`/`pg_current_xact_id()` succeed even inside `READ ONLY` transactions but assign an XID (and trip a `txid_current_if_assigned()` canary); they fail only on a hot standby (standby not tested) — use snapshot xmin (`pg_snapshot_xmin(pg_current_snapshot())` works in RO txns) [Observed: docker test, postgres:16/17/18, see verification/01-db-behavior-tests.md#pg-canary]; `DROP` detection only via existent queries; `pg_database` without xmin on some forks → option to skip; extensions altering catalogs via `ALTER EXTENSION UPDATE` → normal xmin.
- **Test cases (validated on PG 16):** create t1 → t1 young; alter t2 add column → t2 and new attribute young, t1 not; comment on t1.a → only description young (table must still pick up the comment via comments query); create index i1 on t1 → i1 young, t1 not; drop t2 → absent from existent ids → removed.
- **Prior art and evidence:** `[Observed: PgIntroQueries StateNumber/CurrentXid/#INC/#TXAGE; cache StateNumber/IntrospectionStateNumber/StartupTime; Docker validation]`.
- **Used by:** Hacivat (PG, Greenplum, Redshift local objects, CockroachDB where supported, YugabyteDB if catalogs behave).

### ALG-INTRO-03: SQL Server incremental by modify_date + checksums

- **Problem:** Incremental refresh without xids.
- **Algorithm:**
  1. Compute fingerprint (`database_id`, `create_date`, `service_broker_guid`, server name); mismatch → full.
  2. `since = previous run's server start time` (read `SYSDATETIME()` at start) minus a small skew margin (e.g. 1 s, datetime precision 3.33 ms).
  3. Existent: `object_id`s per schema (+ composite index ids, fulltext). Delete missing.
  4. Changed: objects with `modify_date >= since`; for changed tables re-read children (columns, indexes, keys, FKs, checks, triggers, partitions); routines/views: re-read sources.
  5. Checksums: compute `CHECKSUM_AGG`/sum-of-checksums for user types, permissions (schema/db/server), extended properties; compare with stored; mismatch → reload category.
  6. Store new `since`, checksums.
- **Edge cases:** `modify_date` not updated by permission changes (checksums), by some `sp_rename`s on older versions, by statistics; triggers' `modify_date` on parent? (DDL triggers are database-level); Azure SQL can't `USE` other DBs → per-DB connection (connection-bounds); `#CAT` prefix for cross-db queries needs `VIEW DEFINITION` per db; clock: always server time; contained AGs/failover → fingerprint changes (create_date same, server name differs) → full.
- **Test cases:** `ALTER TABLE t ADD c int` → t re-read; `GRANT SELECT ON t TO u` → permissions checksum mismatch → permissions reloaded only; restore DB from backup → fingerprint change → full.
- **Prior art:** `[Observed: MsIntroQueries; cache Fingerprint/IntrospectionTimestamp/*CheckSum]`.
- **Used by:** Hacivat.

### ALG-INTRO-04: Oracle incremental + levels

- **Problem:** Scale to schemas with 10⁵ objects over slow dictionaries.
- **Algorithm:**
  1. Probe privileges: fast base tables (`sys.obj$`…) accessible? catalog role? → choose path (`obj$` vs `DBA_*` vs `ALL_*`).
  2. Per schema: `CheckSchemaId` (user id) → mismatch → full; count objects → auto level per thresholds (current/non-current/system) unless explicit.
  3. L1: major names (packed rows) + minor names (columns/constraints/arguments) only; L2: + details (columns types, constraints, partitions, args); L3: + sources (batched `ALL_SOURCE` by names, LONG-safe view text variants).
  4. Incremental: `mtime/last_ddl_time >= since`; existent ids for drops; hashes for comments/db links/tablespaces; invalid object list; compile errors.
  5. Fragment: for a single object (opened by the user at a lower level) fetch its details/sources (`ListFragmentRegularIds` + per-kind queries with name lists).
- **Edge cases:** recycle bin (`BIN$…`) and system-generated LOB segments excluded; editions (EBR) — `[Unknown]`; synonyms pointing across schemas/db links; PUBLIC pseudo-schema; CDB/PDB (`con_id`); LONG columns can't be filtered — need XML/12c variants; 1,000-item IN limit; `last_ddl_time` updated by `GRANT` on the object (false positives fine).
- **Test cases:** 5,000-object non-current schema → auto L1; opening a package body at L1 → fragment loads source; `GRANT` only → object re-read (harmless).
- **Prior art:** `[Observed: OraIntroQueries; help:oracle-introspection-levels]`.
- **Used by:** Hacivat (Oracle feature-gated).

### ALG-INTRO-05: SQLite change detection

- **Problem:** Avoid re-reading a large SQLite schema needlessly; handle attached databases.
- **Algorithm:** per schema (main/temp/attached): read `PRAGMA <schema>.schema_version` (increments on every schema change) and the file identity (path + inode/size/mtime for main and attached files); if unchanged → skip; else full re-read of that schema via `sqlite_schema` + `PRAGMA table_xinfo/index_list/index_xinfo/foreign_key_list`. Temp schema is per-connection → always introspect on the session's own connection (or skip).
- **Edge cases:** `schema_version` is per-database-file and can be written by `PRAGMA schema_version=` (rare); WAL mode (fine); file replaced by another with same schema_version → identity check; read-only/immutable URIs; virtual tables (module may be unavailable → columns unknown); generated columns (`table_xinfo.hidden` 2/3); `WITHOUT ROWID`; STRICT tables; WSL lock errors (DataGrip has a dedicated error).
- **Test cases:** `CREATE TABLE` → schema_version+1 → re-read; `INSERT` only → no re-read; attach second DB → new schema node. Verified on SQLite 3.54.1: INSERT → unchanged; CREATE INDEX, ADD COLUMN, CREATE VIEW, DROP VIEW → +1 each; rolled-back CREATE → unchanged; `VACUUM` → +1 (spurious re-read, harmless); `CREATE TEMP TABLE` bumps only `temp.schema_version`. [Observed: docker test, keinos/sqlite3, see verification/01-db-behavior-tests.md#sqlite-schema-version]
- **Prior art:** SQLite docs; `[Observed: SQLite introspector holders; cache LastIntrospectionLocalTimestamp]` — DataGrip's own use of schema_version `[Unknown]`.
- **Used by:** Hacivat.

### ALG-INTRO-06: MySQL/MariaDB fallback strategy (full per schema + fragments + affected objects)

- **Problem:** No reliable change marker.
- **Algorithm:**
  1. Schema-level change hint: compute a cheap **signature** per schema = hash of `(TABLE_NAME, TABLE_TYPE, CREATE_TIME, UPDATE_TIME?, TABLE_COMMENT, VERSION, ENGINE, ROW_FORMAT, AUTO_INCREMENT excluded)` from `information_schema.TABLES` + `(ROUTINE_NAME, LAST_ALTERED)` from `ROUTINES` + `(TRIGGER_NAME, CREATED)` from `TRIGGERS` + `(EVENT_NAME, LAST_ALTERED)` from `EVENTS`. In MySQL 8 set `information_schema_stats_expiry=0` for the session first.
  2. If signature unchanged → skip schema (heuristic, may miss column-level ALTERs that don't touch CREATE_TIME on some versions → therefore also hash `information_schema.COLUMNS` per schema (`GROUP_CONCAT`/`MD5` aggregate in SQL to avoid transferring rows) — this is cheap enough for most schemas.
  3. If changed → full re-read of the schema (or only tables whose per-table hash changed).
  4. After user DDL: parse statement → affected objects (ALG-INTRO-08) → fragment re-read.
- **Edge cases:** `CREATE_TIME` changes on `ALTER` with table rebuild only (corrected after verification: on 8.0.46/8.4.13 it changes on every tested `ALTER TABLE`, including INSTANT/INPLACE ones, but not on `CREATE TRIGGER`; one-second granularity — two DDLs in the same second are indistinguishable, so keep the COLUMNS hash) [Observed: docker test, mysql:8.0/8.4, see verification/01-db-behavior-tests.md#mysql-is-times]; `UPDATE_TIME` is data-change and NULL for InnoDB on some versions (observed: NULL unless `information_schema_stats_expiry=0`, reset by ALTER — exclude it from the signature); MariaDB `information_schema` without `generation_expression` (<10.2.5); privileges hiding routine bodies; Vitess/TiDB partial `information_schema`; huge schemas (10⁵ tables) — aggregate hash in SQL.
- **Test cases:** `ALTER TABLE t MODIFY c bigint` → columns hash changes → t re-read; data `INSERT` → no schema change detected.
- **Prior art:** `[Observed: help (incremental not listed for MySQL); whatsnew 2024.3 fragment + smart refresh]`; heuristic is Hacivat's own `[Inferred]`.
- **Used by:** Hacivat.

### ALG-INTRO-07: Existence diff merge

- **Problem:** Apply an incremental result to the cached model correctly.
- **Inputs / outputs:** `M` (cached children of namespace per kind), `E` (existent ids per kind), `C` (changed objects with children) → new `M'` + `Delta{created, altered, dropped}`.
- **Algorithm:** for each kind in dependency order (types before tables before indexes): `dropped = M.ids − E`; `created = C.ids − M.ids`; `altered = C.ids ∩ M.ids` where properties differ; remove dropped (cascade children); upsert C (replace child collections of altered parents); rebuild name indexes; re-resolve name-based references; emit delta to UI/resolve caches.
- **Edge cases:** id reuse (PG OID wrap/reuse; Oracle obj# reuse) → compare `(id, name, kind)`; rename = same id new name (keep identity → explorer keeps expansion state); object moved between schemas (`ALTER … SET SCHEMA`: PG same OID, different namespace) → must handle cross-namespace moves (treat as drop+create if the other namespace not introspected); missing parent for a changed child (partial reads) → mark parent dirty and fetch; existent query failing due to permissions → do **not** treat as "all dropped" (abort incremental, keep cache).
- **Test cases:** rename table → same id, new name, children intact; `E` empty because of permission error → no deletions; drop table with indexes → indexes removed.
- **Prior art:** `[Observed: Existent/Retrieve pairs]`.
- **Used by:** Hacivat.

### ALG-INTRO-08: Refresh after DDL (affected-object analysis)

- **Problem:** After the user runs DDL, refresh only what it could have changed.
- **Algorithm:** parse executed statements (successful ones) → collect `(action, kind, qualified name)` mentions: `CREATE/ALTER/DROP <kind> <name>`, `RENAME`, `COMMENT ON`, `GRANT/REVOKE`, `CREATE INDEX ON t` (affects t's indexes), `ALTER TABLE t ADD FOREIGN KEY … REFERENCES u` (affects t and u's referencing set); resolve unqualified names with the session's current schema/search path at execution time; for dynamic/opaque statements (PL blocks, `EXECUTE IMMEDIATE`, procedures) → fall back to namespace incremental refresh; batch mentions and run fragment introspection; if the DBMS lacks fragment support → namespace incremental.
- **Edge cases:** DDL inside a transaction not yet committed (other sessions can't see it; refresh in the same session or after commit — DataGrip `[Unknown]`); `DROP SCHEMA … CASCADE`; `ALTER TYPE` affecting many columns; temp tables (single-session only); errors mid-script (only successful statements count); "indirect" changes (triggers creating objects) are missed — docs explicitly warn and suggest full refresh `[Observed: help Auto sync]`.
- **Test cases:** `ALTER TABLE s.t RENAME TO u` → fragment refresh `s.t`→`s.u`; `CREATE INDEX i ON t(a)` → refresh t's indexes; `DO $$ … $$` → schema incremental.
- **Prior art:** `[Observed: help Auto sync; sqlEffectAnalyzer registrations; AffectedObjectMention classes; whatsnew 2024.3]`.
- **Used by:** Hacivat; Karagöz (audit "objects affected" field can reuse the analyzer).

### ALG-SCOPE-01: Schema pattern grammar and matcher

- **Problem:** Parse and evaluate `db:sch,sch|db:sch` patterns.
- **Grammar (EBNF):**
  ```
  pattern  := clause ( '|' clause )*
  clause   := dbpart ( ':' schlist )?          // if DBMS has no databases, clause := schlist
  dbpart   := term
  schlist  := term ( ',' term )*
  term     := '@' | '*' | '{' regex '}' | name
  name     := identifier | quoted               // quoting: "..." or `...` per DBMS [Inferred]
  ```
- **Semantics:** `@` = current/default database or schema (as configured/detected); `*` = all; `{re}` = full-match Java/Rust regex (case-sensitivity per DBMS identifier rules); `name` = exact (case per DBMS folding). A clause selects pairs `(db, schema)`; the result is the **union** of clauses ("multiplicities combine, not intersect"). Missing `:schlist` → database only (e.g. DB nodes) `[Inferred]`. "If only schemas supported, list them after `*`" → in single-level DBMSs write `*:s1,s2`.
- **Algorithm:** parse → AST; evaluate against the list of `(db, [schemas])` from `ListDatabases`/`ListSchemas`; return selected set; persist as tree nodes (`kind`, `qname`, `negative`) for the UI tree; negative nodes subtract after union.
- **Edge cases:** names containing `:`, `,`, `|`, `{` (require quoting); regex with `|` inside `{…}` (brace-balanced scan); empty pattern = nothing selected; `@` when current DB not detected ("not detected"); case-insensitive DBMS (MySQL on Windows, SQL Server CI collation) vs PG case-sensitive quoted names; Unicode/Turkish dotted I in names (no locale folding).
- **Test cases:** `*:*` → everything; `@:@` → only current schema; `*:dbo|@:@|db1:s1,s2,s3`; `{t.*}:@` → current schema in DBs starting with t; `db{1:x` → syntax error at 2.
- **Prior art:** `[Observed: bundle:DataSourceSchemasPanel.schema.pattern.hint; help:schemas]`.
- **Used by:** Hacivat; Perde (policy scopes could reuse the same syntax for "databases/schemas this role can see").

### ALG-SCOPE-02: Object filter grammar and matcher

- **Grammar:** `filter := item ('||' item)*`; `item := (type ':')? '-'? regex`; `type ∈ {aggregate, collation, event, fdw, ftable, mview, operator, package, role, routine, sequence, synonym, table, user, view, vtable, …}`; an item without type applies to all kinds `[Observed: "A <pattern> or a list of <type>:[-]<pattern>"]`.
- **Semantics `[Inferred]`:** for an object of kind k and name n: let `P` = items applicable to k (typed with k, or untyped); if `P` has positive items, the object is included iff some positive item fully matches n; then it is excluded if any negative item matches. Objects of kinds without applicable items are included.
- **Edge cases:** `||` vs regex alternation `|` (single bar stays inside regex); regex anchoring (full match vs find — DataGrip examples `CUR_.*` imply full match); invalid regex → error, no filtering; filtered objects are excluded from resolve (completion/navigation) — must also exclude from Karagöz-side policy? No: filters are UX only, never security.
- **Test cases:** `table:-OLD_.*||routine:DO.*||view:V_.*` → tables except OLD_*, routines DO*, views V_*; `table:actor.*` → only actor tables; `.*_tmp` (untyped positive) → only objects ending `_tmp` of all kinds.
- **Prior art:** `[Observed: help + bundle hint]`.
- **Used by:** Hacivat.

### ALG-MODEL-01: Model persistence with format versioning

- **Problem:** Store the model per DS crash-safely, load lazily, migrate across versions.
- **Algorithm:** write to `<uuid>.model.tmp` then atomic rename; header `{serializer, dbms, family, format_version, app_version}`; flat records `(id, parent, kind, name, props…)` sorted parent-first; sources in a separate content-addressed store (`<hash>.sql.zst`) referenced by `(object id, source kind, length, hash)`; on load, if `format_version` < current → run migration chain; if > current or migration missing → discard cache (it's a cache!) and re-introspect; per-namespace watermarks stored alongside.
- **Edge cases:** concurrent IDE instances (file lock); partial writes (tmp+rename); huge models (stream, don't DOM); secrets never in model; user-specific external data stored separately.
- **Test cases:** kill during write → old file intact; version bump with migration → loads; unknown future version → cache discarded, warning.
- **Prior art:** `[Observed: dbm format-version 4.53, storage_v2 layout]`.
- **Used by:** Hacivat; Perde could cache a *published* model snapshot per managed DB (signed by Karagöz) to give new team members instant completion.

---

## 5. Implications for Hacivat / Karagöz / Perde

1. Author catalog SQL as **versioned query packs** with a tiny directive language (ALG-INTRO-01); test every variant against a Docker matrix (PG 12–17, MySQL 5.7/8.0/8.4, MariaDB 10.6/11, SQL Server 2019/2022, Oracle Free 23ai) in CI.
2. Implement the **existent/retrieve split** for every DBMS from the start; it is what makes incremental and drop detection correct.
3. PG: xmin-age incremental (validated). SQL Server: modify_date + checksums + fingerprint. Oracle: mtime + levels. MySQL: hash signatures (improvement). SQLite: `schema_version`. ClickHouse: `metadata_modification_time` (improvement).
4. **Karagöz introspection mode**: Karagöz can run introspection locally next to the DB (fast, no P2P round-trips) and ship deltas to Hacivat; deltas are just ALG-INTRO-07 outputs. Permissions: the agent must filter the model by the user's grants (a user must not see objects they cannot access) — DataGrip doesn't need this; we do.
5. Ship **minicat-equivalent snapshots** generated in CI from real servers for system catalogs; plus virtual FKs/`sessions` view overlays (great for an "Activity" panel).
6. Model: arena with stable ids, server ids, dual id+name references, `NameSurrogate`-like flags, separate source store; schema/obj filters as pure UX.

## 6. i18n considerations

- ~237 object kinds need singular + plural labels (DataGrip uses separate keys; Turkish needs plural forms rarely but **case suffixes** for sentences like "Retrieving tables" → "Tablolar alınıyor"); use whole-phrase keys per progress message, not "Retrieving {kind}".
- Regex in filters/patterns must use **Unicode-aware, locale-independent** case folding (`(?i)` with simple case folding); Turkish dotless/dotted I must not be folded by locale.
- Sorting objects in the explorer: natural sort with ICU collation per UI locale is fine for display, but identity/matching must stay byte/ordinal.
- Progress/phase names appear in logs → log stable keys.

## 7. Risks, pitfalls, open questions

- `[Unknown]` How DataGrip computes `#TXAGE` exactly (watermark arithmetic) and whether `StateNumber` uses `pg_locks` oldest xid as described (strongly suggested by the query).
- `[Unknown]` MySQL/SQLite/ClickHouse internal queries (code-embedded); our mapping is inferred from holder names and DBMS catalogs.
- `[Unknown]` Whether PostgreSQL supports fragment introspection in 2024.3 (`#FRAG` directives appear only in MSSQL/Oracle packs).
- Pitfall: relying on table xmin for index changes (validated: in-place update). Pitfall: permission errors in existence queries must not cause mass deletions.
- Pitfall: SQL Server `modify_date` precision and clock — use server time and ≥ with overlap.
