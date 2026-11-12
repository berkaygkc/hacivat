# DataGrip 2024.3.5 — architecture overview (connectivity, execution, introspection, NoSQL)

> Scope: the *runtime architecture* behind connections, sessions, execution, introspection and the metadata model, as observable without decompilation. Detailed behaviour is in `02-connectivity-and-drivers.md`, `03-sessions-transactions-execution.md`, `04-introspection-and-metadata-model.md`, `05-nosql.md`. Editor, grid, DDL/diff, tools, settings and localization are covered by a separate document set (cross-referenced only).
>
> Method, legality and evidence-tag conventions: see `00-method-and-legal.md`.

---

## 1. Scope and sources

| Source | Version / hash | Licence |
|---|---|---|
| DataGrip `database-plugin.jar` descriptors (`META-INF/plugin.xml`, `DatabaseConnectivity.xml`, `DatabaseCore.xml`, `DatabasePlugin.xml`, `DatabaseRegistryKeys.xml`) and per-dialect XML (`com/intellij/database/dialects/<d>/<d>.xml`) | 2024.3.5 build 243.24978.79, jar SHA-256 `63c5…393d` | Proprietary (read as data) |
| `jdbc-console.jar` manifest + class names | SHA-256 `e34a…e57e` | Proprietary |
| Class-name inventory (`.refs/datagrip-extract/classes/*.txt`) | from the same jars | Proprietary (names only) |
| Shipped offline help (`help-datagrip-assets.jar`) | 2024.3.5 | Proprietary docs; online twin `https://www.jetbrains.com/help/datagrip/` |
| User's local `.idea/dataSources*` (structure only) | DataGrip 2024.3 | n/a |
| `DataGrip/redis-jdbc-driver@v1.5`, `DataGrip/mongo-jdbc-driver@v1.20` | GitHub | Apache-2.0 |

---

## 2. Executive summary

1. **Two-process design.** The IDE JVM holds UI, the metadata model, the SQL language engine and orchestration; JDBC drivers run in a **separate JVM** started from `jdbc-console.jar` (`Main-Class: com.intellij.database.remote.RemoteJdbcServer`) `[Observed: jdbc-console.jar!/META-INF/MANIFEST.MF]`. All JDBC objects are proxied (`RemoteConnection`, `RemoteStatement`, `RemoteResultSet`, …) `[Observed: classes: com/intellij/database/remote/jdbc/*]`.
2. **Everything is keyed by a DBMS id** (≈60 ids, e.g. `POSTGRES`, `REDIS`, `MONGO`), with **family sets** (`POSTGRES_LIKE`, `MYSQL_LIKE`, `MSSQL_LIKE`, `TRANSACT_SQL`, `DB2_LIKE`, `ORACLE_LIKE`, `PRESTO_LIKE`, `DOCUMENT_ORIENTED`, `PSEUDO_SUPPORTED`, `AWS_PROFILE_AUTH_SUPPORTED`) and an explicit **fallback chain** (e.g. `YUGABYTE→POSTGRES`, `TIDB→MYSQL`, `SYNAPSE→AZURE`) `[Observed: <dbms>, <addToHSet>, <extensionFallback> in dialect XMLs]`.
3. A "dialect" is **not one class** but a bundle of ~25 small services registered per DBMS id (`introspector`, `jdbcHelper`, `modelFacade`, `scriptGenerator`, `dmlHelper`, `errorHandler`, `typeSystem`, `gridColumnsManagerFactory`, `hookUpHelper`, …), each resolved through the fallback chain to `UNKNOWN` (generic JDBC) `[Observed: META-INF/DatabaseCore.xml extension points]`.
4. **Connection = ordered interceptor pipeline** over a "proto-connection" (URL + properties + process parameters): password-expiry and pgpass first, SSH (with Oracle TNS resolution *before* SSH), SSL, cloud-proxy, auth providers, forced-DBMS, logging, and `NoCredentialsAuthProvider` last `[Observed: connectionInterceptor registrations with order= attributes]`.
5. **Session ≠ connection ≠ client.** A *session* wraps one logical connection (and its tx/schema state); *clients* (consoles, files, grids) attach to sessions; a *router* (`SingleSessionRouter` / `MultipleSessionsRouter`) maps requests to connections; every request carries a **department** (console, data editor, introspection, quick-doc, …) `[Observed: classes: DatabaseSessionManager$Facade$*, DGDepartment$*; bundle:department.*]`.
6. **Introspection** is a family of tasks (complete, incremental, fragment, check-up-to-date, elevation, combined) driven by per-DBMS catalog query packs written as named SQL sections with **meta-directives** for version gating, incremental filtering and fragment narrowing `[Observed: *IntroQueries.sql]`.
7. The **metadata model** is a typed object tree persisted per data source as a flat XML ("dbm" serializer, `format-version="4.53"`) plus a separate **source store** (`storage_v2/_src_/…` zips) `[Observed: user .idea/dataSources/*]`.
8. **System catalogs ship pre-introspected** ("minicat" snapshots for 18 DBMSs) and are overlaid with virtual FKs/views (e.g. a `sessions` view over `pg_stat_activity`) `[Observed: *.minicat.xml.gzip, *-model-external-data.xml]`.
9. **NoSQL stores are forced into the same pipeline** via JDBC drivers that JetBrains maintains (Redis, MongoDB, DynamoDB, Couchbase, Cassandra), each with its own console language and per-type delegates `[Observed: redis.xml, mongo.xml; drivers XML]`.

---

## 3. Detailed findings

### 3.1 Process topology

```
┌──────────────────────────── IDE JVM ──────────────────────────────┐
│ UI (Database Explorer, consoles, Services tool window, grids)     │
│ DataSourceStorage (project + app level)  ─ dataSources*.xml       │
│ DatabaseDriverManager  ─ databaseDrivers/*.xml + user overrides   │
│ DatabaseArtifactLoader ─ database-artifacts.xml (maven/CDN)       │
│ DatabaseConnectionManager ─ interceptor pipeline ─────────┐       │
│ DatabaseSessionManager ─ sessions / routers / departments │       │
│ JdbcEngine / SmartStatements ─ operations, cancel         │       │
│ Introspection tasks ─ DBIntrospector per DBMS              │       │
│ Model (DasObject tree) ─ DataSourceModelStorage            │       │
│ DbSrcStorage ─ object sources (VFS "dbSrc")                │       │
└───────────────────────────────────────────────────────────┼───────┘
                                                            │ remote proxies
┌──────────────── driver host JVM ("jdbc-console") ─────────▼───────┐
│ RemoteJdbcServer → RemoteDriver/Connection/Statement/ResultSet    │
│ jdba layer: RemoteFacade → RemoteSession → RemoteSeance →         │
│             RemoteCursor (+ ExceptionRecognizer per DBMS)         │
│ JdbcHelper per DBMS (38+): PgJdbcHelper, RedisJdbcHelper, …       │
│ Driver jars (isolated class loader optional)                      │
│ Large values: FileBlob/FileClob (LobInfo$FileBlobInfo)            │
└───────────────────────────────────────────────────────────────────┘
```

Evidence:
- Main class and proxies `[Observed: jdbc-console.jar MANIFEST; classes: remote/jdbc/Remote*, remote/jdba/Remote{Facade,Session,Seance,Cursor}]`.
- Per-DBMS helpers living in the driver process `[Observed: classes: remote/jdbc/helpers/*JdbcHelper]` and per-DBMS exception recognizers `[Observed: classes: remote/jdba/jdbc/*ExceptionRecognizer]`.
- Typed remote exceptions `DBLoginFailedException`, `DBSessionIsClosedException`, `DBAccessDeniedException`, `NoTableOrViewException`, `DuplicateKeyException`, `DBTransactionIsAlreadyStartedException`, … `[Observed: classes: remote/jdba/exceptions/*]`.
- "The driver is started as a separate process in the JVM", with per-driver **VM options**, **VM environment** and **working directory** `[Observed: help:data-sources-and-drivers-dialog]`.
- Process failure is a first-class error (`DataSourceProcessExitedErrorInfo`) and OOM gets a dedicated message *"Database client process needs more memory to perform the request"* `[Observed: classes: BaseDatabaseErrorHandler$DataSourceProcessExitedErrorInfo; bundle:error.message.remote.oomerror]`.
- "Before connection" tasks run "before the first connection (at the start of a process in which the JDBC driver works)" `[Observed: help:data-sources-and-drivers-dialog]` → the process is long-lived and shared by connections of a data source.
- `[Inferred]` Transport is IntelliJ's `RemoteServer`/RMI pattern (class names `RemoteJdbcServerBase`, `RemoteCloseableImpl`, `JdbaRemoteObject`). `[Unknown]` exact granularity (one process per data source vs. per driver configuration).

Why the separate process (`[Inferred: general engineering, consistent with options observed]`): driver dependency conflicts across 60+ drivers; multiple versions of the same driver side by side; crash/OOM isolation; a driver that ignores `Statement.cancel()` can be killed; process-wide settings (JVM TLS truststores `-Djavax.net.ssl.*`, Kerberos `krb5.conf`, `user.timezone`, `NLS_LANG`, native OCI libraries) are scoped per process. The docs literally recommend fixing Oracle time-zone problems via `-Duser.timezone=…` and encoding via `NLS_LANG` in **the driver VM** `[Observed: help:data-sources-and-drivers-dialog]`.

### 3.2 Registries

| Registry | Backing data | Notes |
|---|---|---|
| DBMS registry | `<dbms id=… instance=…>` beans | ~60 ids; `order="before X"` for detection precedence (e.g. `SYBASE` before `MSSQL`, `MYSQL_AURORA` before `MYSQL`, `DB2_LUW` before `DB2`) `[Observed]` |
| Family sets ("HSet") | `<addToHSet item set>` | Sets can contain sets (`AZURE ∈ AZURE_LIKE ∈ MSSQL_LIKE ∈ TRANSACT_SQL`) `[Observed]` |
| Extension fallback | `<extensionFallback dbms fallbackDbms>` | 9 edges observed (§3.3) |
| Driver catalog | `databaseDrivers/*.xml` (52 files) | 78 drivers, 115 URL templates, `based-on` inheritance, abstract bases `[Observed]` |
| Artifact catalog | `resources/database-artifacts.xml` | 64 artifacts, item types maven(238)/license(245)/native(14)/pack(11); `constraint`/`unsupported` ranges against DB and IDE versions; channels `[Observed]` |
| Option providers | `<optionProvider>` | Per-DS typed options (24 observed) |
| Model external data | `<modelExternalData config=…>` | System-catalog overlays |

### 3.3 DBMS ids, families and fallbacks (complete list)

`[Observed: all <dbms>, <addToHSet>, <extensionFallback> in database-plugin.jar]`

| Group | DBMS ids | Families / fallback |
|---|---|---|
| PostgreSQL family | `POSTGRES`, `REDSHIFT`, `GREENPLUM`, `COCKROACH`, `YUGABYTE` | all ∈ `POSTGRES_LIKE`; `YUGABYTE→POSTGRES` fallback, `YUGABYTE ∈ PSEUDO_SUPPORTED`; `REDSHIFT ∈ AWS_PROFILE_AUTH_SUPPORTED` |
| MySQL family | `MYSQL`, `MARIADB`, `MYSQL_AURORA`, `MEMSQL` (SingleStore), `TIDB`, `VITESS`, `OCEANBASE`, `GITBASE` | ∈ `MYSQL_LIKE`; `MYSQL_AURORA ∈ MYSQL` + fallback `→MYSQL`; `TIDB/VITESS/OCEANBASE → MYSQL`; `MEMSQL/TIDB/VITESS/OCEANBASE ∈ PSEUDO_SUPPORTED` |
| T-SQL family | `MSSQL`, `MSSQL_LOCALDB`, `AZURE`, `SYNAPSE`, `SYBASE` | `MSSQL, AZURE_LIKE ∈ MSSQL_LIKE ∈ TRANSACT_SQL`; `SYBASE ∈ TRANSACT_SQL`; `MSSQL_LOCALDB→MSSQL`; `SYNAPSE→AZURE` |
| Oracle | `ORACLE`, `NETSUITE` | `∈ ORACLE_LIKE`; `NETSUITE→ORACLE` |
| Db2 | `DB2`, `DB2_LUW`, `DB2_ZOS`, `DB2_IS` | `∈ DB2_LIKE` |
| Embedded/other SQL | `SQLITE`, `H2`, `HSQLDB`, `DERBY`, `CLICKHOUSE`, `SNOWFLAKE`, `VERTICA`, `EXASOL`, `HIVE`, `SPARK`, `BIGQUERY` | `SPARK ∈ PSEUDO_SUPPORTED` |
| Document/KV | `REDIS`, `MONGO`, `DYNAMO`, `COUCHBASE` | `∈ DOCUMENT_ORIENTED`; `DYNAMO ∈ AWS_PROFILE_AUTH_SUPPORTED` |
| Wide column | `CASSANDRA` | — |
| Generic (basic support) | `ATHENA`, `PRESTO`, `TRINO`, `HANA`, `FIREBIRD`, `INFORMIX`, `IMPALA`, `NETEZZA`, `PHOENIX`, `INGRES`, `TERADATA`, `OPENEDGE`, `TIBERO`, `FILEMAKER`, `CLOUD_SPANNER`, `ZEN`, `FRONTBASE`, `IGNITE`, `MONET`, `DENODO`, `IRIS`, `DRILL`, `CRATE`, `GBASE`, `SQLANYWHERE`, `ELASTICSEARCH`, `SALESFORCE`, `KDB`, `TIMESTREAM` | `PRESTO, TRINO ∈ PRESTO_LIKE`; `TRINO→PRESTO`; `ATHENA ∈ AWS_PROFILE_AUTH_SUPPORTED` |
| Catch-all | `UNKNOWN` | root of every fallback chain; generic JDBC introspector |

"Basic support" per docs = JDBC-metadata introspection + SQL:2016 highlighting, no error highlighting `[Observed: help:other-databases]`. The docs' basic list (Athena, Ignite, Spark, DuckDB, Elasticsearch, Firebird, Spanner, Informix, IRIS, Mimer, OpenEdge, Phoenix, Presto, HANA, SingleStore, Tarantool, Teradata, TiDB, Tibero, Trino, YugabyteDB) matches `GenericSQL` dialect drivers + `PSEUDO_SUPPORTED`.

### 3.4 Per-DBMS service bundle (extension points keyed by DBMS)

`[Observed: META-INF/DatabaseCore.xml, DatabaseConnectivity.xml, dialect XMLs]`

| Extension point | Role | Example (PostgreSQL) |
|---|---|---|
| `dialect` / `sql.dialect` | Database dialect and SQL language dialect | `PgDialect` |
| `jdbcHelper` | Driver-side adaptations (runs in driver process) | `PgJdbcHelper` |
| `jdbcMetadataWrapper` | Fixes broken `DatabaseMetaData` for the JDBC introspector | `PgBaseMetadataWrapper` |
| `jdbcSourceLoader` | Loads object sources via JDBC | `PgBaseJdbcSourceLoader` |
| `introspector` | Native introspector factory | `PgIntrospector$Factory` |
| `modelFacade` | Model meta-model, kinds, properties | `PgModelFacade` |
| `scriptGenerator`, `namingService`, `definitionProvider`, `sqlObjectBuilder` | DDL generation (other doc set) | |
| `dmlHelper`, `predicatesHelper`, `hookUpHelper`, `gridHelper`, `gridColumnsManagerFactory` | Grid ↔ SQL/commands (other doc set; NoSQL parts in `05`) | |
| `domainRegistry`, `typeSystem`, `geoHelper` | Value domains/types | `PgDomains`, `Pg83TypeSystem` |
| `executionEnvironmentHelper` | Session environment: time zone manager, search path | `PgBaseExecutionEnvironmentHelper$PgBaseTimeZoneManager` |
| `errorHandler`, `errorProvider`, `runtimeErrorFixProvider` | Error classification and quick fixes | `PgErrorHandler` |
| `explainPlanProvider`, `routineExecutionHelper`, `dataImporter` | (other doc set) | |
| `databaseViewStructureExtension` | Explorer tree shaping | `PgDvStructureExtension` |
| `driversConfig`, `modelExternalData` | Data files | `postgres-drivers.xml`, `pg-model-external-data.xml` |
| `optionProvider` | Typed per-DS options | `PgTemplateDatabaseOption`, `PgSystemSchemasOption`, `PgNoDatabaseXminOption` |
| `connectionInterceptor` | Connect pipeline step (not keyed; self-selecting) | `PgPassAuthProvider` (first), `PgBaseUrlCheckingInterceptor`, `PgCloudSQLProxyInterceptor` |
| `introspectorStatsProvider` | Introspection statistics/diagnostics | MySQL, Oracle, Snowflake |
| `configValidator`, `urlParamEditorProvider`, `urlEditorInspector`, `dataSourceDetector` | Config UI validation, URL parameter editors | `MsTypeDescriptorFactory` (SQL Server instance browser) |

Full extension-point list in this plugin (73): `addToHSet artifactsConfig cli.runTargetProvider configValidator connectionInterceptor consoleProvider consoleRunContextParametersTuner dataAuditor databaseViewStructureExtension dataConsumer dataImporter dataProducer dataSourceDetector dataSourceManager dbms debuggerFacade definitionProvider dialect dmlHelper domainRegistry driversConfig errorHandler errorProvider executionEnvironmentHelper explainPlanProvider extensionFallback geoHelper gridColumnsManagerFactory gridHelper hookUpHelper introspector introspectorStatsProvider jdbcHelper jdbcMetadataWrapper jdbcSourceLoader linkedDataSourceHelper modelExternalData modelFacade modelRelationProvider namingService objectEditorFactory objectEditorModelFactory optionProvider parameterPatternProvider predicatesHelper processParamProvider queryParametersProvider queryValidator routineExecutionHelper runConsoleAvailable runtimeErrorFixProvider schemaDiffCustomization scriptGenerator selectInProvider sql.* (9) sqlEffectAnalyzer sqlObjectBuilder synchronizeHandler toDatabaseScriptTranslator typeSystem urlEditorInspector urlParamEditorProvider urlParamEditorUiProvider virtualFileDataSourceProvider` `[Observed]`.

### 3.5 Which DBMSs have a native introspector

`[Observed: <introspector dbms=…> registrations]`

| Native catalog introspector | JDBC-metadata introspector (`JdbcIntrospector$Factory`) |
|---|---|
| POSTGRES, REDSHIFT, GREENPLUM, COCKROACH, MYSQL, MARIADB, MEMSQL, MSSQL, AZURE (SYNAPSE via fallback), SYBASE, ORACLE, DB2_LUW, SQLITE, H2, CLICKHOUSE, SNOWFLAKE, VERTICA, EXASOL, HIVE, BIGQUERY, CASSANDRA, COUCHBASE, DYNAMO, REDIS | UNKNOWN (all generic DBMSs), VITESS, YUGABYTE, and DBMSs without their own registration (e.g. MONGO, HSQLDB, DERBY, SPARK, other Db2 flavours) resolve to the generic one through fallback `[Inferred: absence of registration + fallback to UNKNOWN]` |

MongoDB relies on the driver's own `DatabaseMetaData` (collections as tables, sampled fields as columns) `[Observed: oss-mongo MongoDatabaseMetaData.java, MetaCollection.java@v1.20]`. Any DBMS can be forced to JDBC metadata with the expert option *Introspect using JDBC metadata* `[Observed: help:data-sources-and-drivers-dialog; JdbcIntrospectorOptionProvider]`.

### 3.6 Persistence layout

`[Observed: user .idea structure; help:managing-data-sources; DatabaseConnectivity.xml services]`

| File | Level | Content (names only) | Shareable |
|---|---|---|---|
| `.idea/dataSources.xml` | project | `<data-source name source uuid>` → `driver-ref`, `synchronize`, `jdbc-driver`, `jdbc-url`, `working-dir`, `auto-commit`, `libraries/library/url` | yes (VCS) |
| `.idea/dataSources.local.xml` | project, per machine | `database-info(dbms, product, version, exact-version, driver-name, driver-version, exact-driver-version, jdbc-version)`, `identifier-quote-string`, `extra-name-characters`, `case-sensitivity(plain-identifiers, quoted-identifiers)`, `secret-storage`, `user-name`, `auth-provider`, `ssh-properties(enabled, ssh-config-id)`, `schema-mapping/introspection-scope/node(kind, qname, negative)`, `substitutions(use-mini-cat)` | no |
| `.idea/dataSources/<uuid>.xml` | cache | metadata model (`dbm` serializer, `format-version="4.53"`) | no (cache) |
| `.idea/dataSources/<uuid>/storage_v2/_src_/…` | cache | object sources in zips, per database/schema | no |
| `options/databaseDrivers.xml` (app) | IDE | user driver overrides | via Settings Sync |
| `options/sshConfigs.xml` (app) or project `.idea` | IDE/project | SSH configurations, referenced by id | configurable |
| `jdbc-drivers/<Artifact>/<version>/…` (app) | IDE | downloaded driver jars | no |
| OS keychain / KeePass | OS | passwords ("DataGrip stores passwords in native keychains on macOS and Linux and in the KeePass storage on Windows") | no |

Global (application-level) data sources exist too and can be exported via *Manage IDE Settings* `[Observed: help:managing-data-sources]`.

### 3.7 Main runtime flows (one-paragraph each)

1. **Connect**: request with *requestor* and *department* → `DatabaseConnectionManager` builds a proto-connection from data source + driver → runs interceptors in order (§`02` ALG-CONN-01) → driver process opens `RemoteConnection` → post-connect: DBMS detection (product/version/quote string/case sensitivity cached in `dataSources.local.xml`), session environment (time zone, schema/search path, read-only, startup script) → session marked connected (green dot).
2. **Execute**: console statement → session router picks/creates the session connection → `JdbcEngine` operation (query/update/transaction ops) → results streamed into grid; cancel/escalation path (§`03`).
3. **Introspect**: trigger (auto-sync after DDL, manual refresh, schedule, scope change, level change) → task (complete/incremental/fragment) → native introspector runs catalog query pack on its own connection (department "introspection") → model delta merged → persisted → explorer and resolve updated (§`04`).

---

## 4. Algorithm specifications

High-level algorithms are specified in the sibling docs:
- `02`: ALG-URL-01..03 (URL templates), ALG-CONN-01..04 (interceptor pipeline, SSH tunnel manager, auth resolution, error classification).
- `03`: ALG-SESS-01..05 (routing, cancellation escalation, keep-alive/auto-disconnect timers, read-only classification, schema switching).
- `04`: ALG-INTRO-01..08, ALG-SCOPE-01..02.
- `05`: ALG-REDIS-01..04, ALG-MONGO-01..03.

One architectural algorithm belongs here:

### ALG-ARCH-01: DBMS capability resolution with family fallback

- **Problem:** Given a detected DBMS id and a capability (e.g. "introspector", "error handler"), find the most specific implementation, falling back to parents and finally the generic one.
- **Inputs / outputs:** `dbms: DbmsId`, `cap: CapabilityKind` → `Option<Arc<dyn Capability>>`.
- **Algorithm:**
  1. `cur = dbms`. Loop: if `registry[(cur, cap)]` exists return it.
  2. Else `cur = fallback[cur]` (explicit edge, e.g. `TIDB → MYSQL`); if none, `cur = UNKNOWN` unless `cur == UNKNOWN` → return `None`.
  3. Family *sets* are **not** used for implementation lookup; they are predicates used by features (e.g. "show AWS profile auth if dbms ∈ AWS_PROFILE_AUTH_SUPPORTED", "use POSTGRES_LIKE external data") `[Inferred: external-data files are keyed by `dbms="POSTGRES_LIKE"`/`"MYSQL_LIKE"` while services are keyed by concrete ids]`.
- **Data structures:** `HashMap<(DbmsId, CapabilityKind), Arc<dyn Capability>>`; `HashMap<DbmsId, DbmsId>` fallback; `HashMap<SetId, HashSet<Member>>` where members may be sets (transitive closure precomputed at startup).
- **Complexity:** O(chain length) ≤ 3 in practice.
- **Edge cases:** cycles in fallback (reject at registration); a DBMS forced by the user (*Forced DBMS* option) overrides detection; detection ambiguity resolved by declared order (`SYBASE` before `MSSQL`); unknown product name → `UNKNOWN`; capability present for family member but intentionally disabled (register a "Null" implementation rather than relying on absence).
- **Test cases:** `(TIDB, introspector)` → MySQL introspector; `(SYNAPSE, introspector)` → Azure/MSSQL introspector; `(YUGABYTE, introspector)` → explicit JDBC introspector (own registration wins over fallback); `(MONGO, introspector)` → generic; `is_member(AZURE, TRANSACT_SQL)` → true (nested sets); `(UNKNOWN, gridColumnsManagerFactory)` → generic.
- **Prior art and evidence:** `[Observed: extensionFallback, addToHSet, DbmsExtension$Bean]`.
- **Used by:** Hacivat (driver/dialect registry); Karagöz (classifier tables per DBMS).

---

## 5. Implications for Hacivat / Karagöz / Perde

1. **Keep the two-tier separation but not the JVM.** Hacivat (Rust) should run drivers in-process as async tasks with **one actor per connection** (owns socket, tx state, cancel token). Keep the *message boundary* serializable so selected drivers (e.g. Oracle via OCI, ODBC, future plugin drivers) can be moved into a helper process (`hacivat-dbhost`) without touching the UI. This gives DataGrip's isolation benefits only where needed.
2. **Capability registry keyed by `DbmsId` with explicit fallback + family predicates** (ALG-ARCH-01). Do not create a monolithic `Dialect` trait; prefer ~10 small traits (`Introspector`, `ErrorClassifier`, `SessionEnv`, `ReadOnlyClassifier`, `GridAdapter`, `KeepAlive`, `Canceller`, `UrlCodec`, `AuthProviders`, `ConsoleLanguage`).
3. **Config split exactly like DataGrip**: shareable (`hacivat-datasources.toml`, VCS-friendly, no secrets, no machine paths) vs local (`.local.toml`: user name, secret refs, SSH refs, detected server info, introspection scope) vs cache (model). Perde-managed connections add a third origin: "remote/managed" (read-only to the user, issued by Perde, endpoints are Karagöz peers).
4. **Karagöz** must re-implement the server-side half of several client features because traffic is P2P: read-only enforcement, statement classification, audit (department/requestor tags are a ready-made audit vocabulary: *console, data editor, introspection, export, …*). Mirror DataGrip's `department` concept as an audit field.
5. **Perde** never needs drivers; it only needs the data-source *descriptor* schema (shareable part) and capability flags per DBMS for its UI.

## 6. i18n considerations

- All user-visible states in this architecture (session states, department names, progress messages such as "Introspecting schema {0} in incremental mode") are message keys with parameters `[Observed: bundle:introspector.progress.status.*]`. Hacivat must make **department names, progress phases and error classes** catalog keys from day one, because they surface in UI *and* in Karagöz audit logs (audit should store the stable key + parameters, and render per viewer locale).
- Turkish pitfall: DBMS ids and family names must be compared with ASCII/ordinal case folding (never locale-aware `to_lowercase` — `I`→`ı` in tr-TR).

## 7. Risks, pitfalls, open questions

- `[Unknown]` Whether one driver process serves all data sources using the same driver configuration or one per data source. Observable proxy: "VM options" exist on both driver and data source → probably per data source configuration.
- `[Unknown]` IPC protocol (likely RMI). Irrelevant for Hacivat, but matters if we ever mimic per-process isolation.
- Risk: DataGrip's JDBC-centric model forces NoSQL into tables; Hacivat should model NoSQL natively (see `05`) while still offering a tabular projection for grids.

---

## Appendix A. Official-docs feature inventory (connectivity, sessions, introspection, NoSQL)

Every row was verified against the shipped offline help of 2024.3.5 (`help-datagrip-assets.jar`); the URL is the online twin. Base: `https://www.jetbrains.com/help/datagrip/`. "Doc" = where Hacivat research details it.

| # | Feature | Official page | Doc |
|---|---|---|---|
| 1 | Data source = connection configuration; project vs global level; copy/paste as XML; export/import global DS via settings | `managing-data-sources.html` | 01 §3.6 |
| 2 | Data Sources and Drivers dialog: General / Options / SSH-SSL / Schemas / Advanced / Kubernetes tabs; Problems panel | `data-sources-and-drivers-dialog.html` | 02 |
| 3 | Connection types per DBMS (default, URL only, Unix socket, SID/Service/TNS/Long, standalone/cluster, SRV, IAM cluster/region, In-memory/Embedded) | `data-sources-and-drivers-dialog.html`, per-DBMS pages (`postgresql.html`, `oracle.html`, `redis.html`, `mongodb.html`, `amazon-redshift.html`, …) | 02 §3.3 |
| 4 | URL templates (`{var}`, `[optional]`) and template names = connection types | `data-sources-and-drivers-dialog.html` (Drivers → General) | 02 §3.4 |
| 5 | Password storage: Never / Until restart / For session / Forever; IDE password store (Keychain/KeePass); Set Empty | `data-sources-and-drivers-dialog.html`, `managing-data-sources.html` | 02 §3.8 |
| 6 | Driver files: download, choose version, custom JARs, driver class, user drivers | `jdbc-drivers.html`, `connectivity-problems.html` | 02 §3.6 |
| 7 | Complete vs basic support (JDBC-metadata introspection, SQL:2016 highlighting) and basic-support vendor list | `other-databases.html` | 01 §3.3 |
| 8 | Test Connection (ping, keep-alive, SSL; copy info) | `data-sources-and-drivers-dialog.html` | 02 §3.13 |
| 9 | Authentication options: pgpass, User & Password, No auth, AWS Profile, Kerberos, Windows domain, Azure AD password/interactive/integrated, Mongo SCRAM/AWS IAM/X.509/GSSAPI/PLAIN, BigQuery methods | `postgresql.html`, `amazon-redshift.html`, `microsoft-sql-server.html`, `azure-sql-database.html`, `oracle.html`, `mongodb.html`, `connect-to-bigquery.html`, `connect-to-oracle-by-using-kerberos.html` | 02 §3.8 |
| 10 | SSH tunnel with shared SSH configurations; auth Password / Key pair / OpenSSH config & agent; local port; manual tunnels (PuTTY, Pageant, ssh-agent) | `configuring-ssh-and-ssl.html`, `settings-tools-ssh-configurations.html` | 02 §3.10 |
| 11 | SSH config: parse `~/.ssh/config`, keep-alive, strict host key checking, hashed known_hosts, HTTP/SOCKS proxy | `settings-tools-ssh-configurations.html` | 02 §3.10 |
| 12 | SSL: CA/client cert/key, truststore IDE/Java/System, Require/Verify CA/Full Verification, copy from DS | `configuring-ssh-and-ssl.html`, `cannot-connect-to-a-database-because-of-sslhandshakeexception.html`, `how-to-connect-to-cassandra-with-ssl.html` | 02 §3.11 |
| 13 | Kubernetes port forwarding | `data-sources-and-drivers-dialog.html` | 02 §3.10 |
| 14 | Cloud: Cloud SQL (proxy drivers), Oracle Cloud wallets, Redshift Okta, Heroku without SSL validation | `connect-to-google-cloud-sql-instances.html`, `connect-to-cloud-sql-instances-postgresql.html`, `connect-to-oracle-cloud-by-using-wallets.html`, `connect-to-amazon-redshift-with-okta.html`, `how-to-connect-to-heroku-postgres.html` | 02 §3.12 |
| 15 | MySQL over Unix sockets / named pipes; SQL Server LocalDB; Oracle OCI | `how-to-connect-to-mysql-with-unix-sockets.html`, `how-to-connect-to-mysql-with-named-pipes.html`, `connecting-to-sql-server-express-localdb.html`, `how-to-connect-to-oracle-with-oci.html` | 02 |
| 16 | Advanced tab: driver properties, VM options, VM environment, working directory; expert options (disable incremental introspection, no xmin on pg_database, JDBC metadata introspector, isolate class path, IDE proxy) | `data-sources-and-drivers-dialog.html` | 02 §3.7 |
| 17 | Troubleshooting connections (network, ports table, driver versions) | `connectivity-problems.html`, `reporting-connection-issues.html` | 02 §3.13 |
| 18 | Sessions: automatic management (2024.1+), Services tool window, switch/attach/detach/rename/close sessions | `managing-connection-sessions.html` (title "Sessions") | 03 §3.1 |
| 19 | Single session mode; single database mode | `configuring-database-connections.html` | 03 §3.3, §3.9 |
| 20 | Transaction mode Auto/Manual, isolation, Submit/Commit/Roll back, Submit changes immediately | `configuring-database-connections.html`, `managing-connection-sessions.html` | 03 §3.4 |
| 21 | Read-only data source | `configuring-database-connections.html`, `data-sources-and-drivers-dialog.html` | 03 §3.7 |
| 22 | Keep-alive query, auto-disconnect, startup script, time zone | `configuring-database-connections.html` | 03 §3.6, §3.9 |
| 23 | Session templates; before-connection tasks; Enable DBMS_OUTPUT | `data-sources-and-drivers-dialog.html` | 03 §3.9 |
| 24 | Cancel running statements; double-cancel deactivation; 10-second warning; cancel connection creation | `run-a-query.html` | 03 §3.5 |
| 25 | Unsafe-query warning (UPDATE/DELETE without WHERE) | `query-execution.html` | 03 §3.8 |
| 26 | Default schema selector; PostgreSQL/Redshift search path; Switch schema Automatic/Manual/Disable | `schemas.html`, `data-sources-and-drivers-dialog.html` | 03 §3.9 |
| 27 | Auto sync; smart refresh of affected objects after DDL (Oracle/SQL Server 2024.2; MySQL/MariaDB 2024.3) | `data-sources-and-drivers-dialog.html`; `https://www.jetbrains.com/datagrip/whatsnew/2024-3/` | 04 §3.1 |
| 28 | Load sources for All / All excl. system / None; Load Sources action | `data-sources-and-drivers-dialog.html` | 04 §3.10 |
| 29 | Schemas tab: schema pattern syntax, object filter syntax, show internal system schemas, show template databases | `data-sources-and-drivers-dialog.html`, `configuring-database-connections.html` | 04 §3.6 |
| 30 | Show/hide schemas, N of M selector, pattern filter nodes, All Namespaces, Force Refresh | `schemas.html`, `show-all-databases-or-schemas.html` | 04 §3.6 |
| 31 | Pre-introspected objects for system catalogs (lightning icon) | `schemas.html` | 04 §3.7 |
| 32 | Oracle introspection levels L1–L3, defaults by object count, fragment introspection | `oracle-introspection-levels.html`, `https://www.jetbrains.com/help/datagrip/2024.3/introspection-levels.html` | 04 §3.3.4 |
| 33 | Automatic introspection interval; track DB/schema creation & deletion; warn when editing outdated DDL | `data-sources-and-drivers-dialog.html` | 04 §3.1 |
| 34 | Virtual foreign keys / columns / views; external-data file | `virtual-foreign-keys.html`, `virtual-columns.html`, `virtual-views.html` | 04 §3.7 |
| 35 | Introspection diagnostics & reporting | `reporting-introspection-issues.html`, `cannot-find-a-database-object-in-the-database-tree-view.html` | 04 §3.10 |
| 36 | Redis: standalone/cluster, default key filter (SCAN), COUNT hint, cluster+SSH node list, no data editing | `redis.html` | 05 §3.1 |
| 37 | MongoDB: SRV, auth mechanisms, `fetch_documents_for_metainfo`, editing documents, SQL for MongoDB, Show JS Script | `mongodb.html`, `how-to-fetch-more-fields-for-mongodb-introspection.html`, `editing-data-in-mongodb.html`, `sql-for-mongodb.html`, `documentdb.html` | 05 §3.2 |
| 38 | Cassandra (keyspaces, SSL via keystores), DynamoDB (AWS profile, region), Couchbase Query (Query service only, no editing) | `apache-cassandra.html`, `how-to-connect-to-cassandra-with-ssl.html`, `dynamodb.html`, `couchbase-query.html` | 05 §3.3–3.5 |
